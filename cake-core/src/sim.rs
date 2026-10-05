//! The simulation: state, and the fixed-tick step that advances it.
//!
//! Everything here is deterministic. Entities live in a `Vec` in id order,
//! ids only ever increase, and ties break on id. There are no hash maps and
//! no randomness. Two peers that start from [`Sim::new`] and step through the
//! same commands hold bit-identical state, which [`Sim::checksum`] lets them
//! confirm.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::command::{Command, MAX_UNITS_PER_COMMAND, PlaceError};
use crate::geom::{Pos, R_MID, UNIT, isqrt, scaled, sector_center, span_of};
use crate::stats::{self, Kind, Shot};

pub type EntityId = u32;
/// A player's index around the ring: seat `i` starts in sector `i`. Sectors
/// are only a starting layout; nothing here keeps anyone in theirs.
pub type Seat = u8;

pub const MAX_SEATS: usize = 8;

/// A mobile unit whose destination moved less than this last tick, and is
/// this close, has arrived: it is being jostled at the goal, not walking.
const STALL_RADIUS: i64 = 40 * UNIT;
const STALL_TICKS: u8 = 5;
const DEPLOY_STALL_RADIUS: i64 = 15 * UNIT;
/// A group spread wider than this converges on the target rather than
/// keeping its shape.
const FORMATION_SPREAD: i64 = 80 * UNIT;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Order {
    Idle,
    Move(Pos),
    AttackMove(Pos),
    Attack(EntityId),
    /// Build a turret of this kind there.
    Build(Pos, Kind),
    Repair(EntityId),
    Deploy(Pos),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entity {
    pub id: EntityId,
    pub owner: Seat,
    pub kind: Kind,
    pub pos: Pos,
    pub hp: i32,
    pub order: Order,
    /// Who this entity is currently shooting or chasing.
    pub target: Option<EntityId>,
    pub cooldown: u32,
    /// Structures: construction ticks done. Utilities: deploy ticks done.
    pub progress: u32,
    /// Structures under construction neither shoot nor count for income.
    pub complete: bool,
    /// Presentation: which way the entity faces, in its local
    /// `(tangential, radial)` frame, scaled to 1000.
    pub facing: (i64, i64),
    /// Being thrown by a blast: milli-units per tick in the local
    /// `(tangential, radial)` frame, fading each tick.
    pub push: (i64, i64),
    /// Bit `s` is set when seat `s` can see this entity (its owner always
    /// can). Refreshed every tick.
    pub seen_by: u8,
    stall: u8,
    last_goal_d: i64,
    /// A utility that became an economy building. Removed without dying.
    consumed: bool,
}

impl Entity {
    pub fn stats(&self) -> &'static stats::Stats {
        self.kind.stats()
    }

    pub fn radius(&self) -> i64 {
        self.stats().radius
    }

    pub fn max_hp(&self) -> i32 {
        self.stats().hp
    }

    pub fn seen_by_seat(&self, seat: Seat) -> bool {
        self.seen_by & (1 << seat) != 0
    }

    /// Can this entity shoot right now?
    pub fn armed(&self) -> bool {
        self.complete && self.stats().weapon.is_some()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Player {
    pub alive: bool,
    pub supply: i64,
    pub queue: VecDeque<Kind>,
    /// Work done on the front of the queue, in hundredths of a tick at the
    /// base pace: it is done at `build_ticks * 100` (see
    /// [`Sim::production_pct`]).
    pub progress: u32,
    pub rally: Pos,
    pub hq: EntityId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Winner(Seat),
    /// Everyone fell on the same tick.
    Draw,
}

/// Things that happened during the last step, for presentation. Not part of
/// the game state, and not in the checksum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Shot {
        from: Pos,
        to: Pos,
        owner: Seat,
        kind: Kind,
    },
    Died {
        owner: Seat,
        kind: Kind,
        pos: Pos,
    },
    Completed {
        owner: Seat,
        kind: Kind,
        pos: Pos,
    },
    /// A projectile arrived: a bullet or missile hit, or plasma burst.
    Impact {
        at: Pos,
        owner: Seat,
        /// Who fired it.
        kind: Kind,
        target: Option<EntityId>,
    },
    /// A building exploded, throwing units back within `radius`.
    Blast {
        at: Pos,
        owner: Seat,
        radius: i64,
    },
    Eliminated(Seat),
}

/// Something in flight between a shooter and its target.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Projectile {
    pub owner: Seat,
    /// What fired it: damage multipliers are the shooter's.
    pub shooter: Kind,
    pub shot: Shot,
    pub pos: Pos,
    /// Where it was a tick ago, for drawing it between ticks.
    pub prev: Pos,
    pub target: EntityId,
    /// Where it is headed: the target, while it lives (for plasma, where the
    /// target was when it was fired).
    pub aim: Pos,
    /// Missiles: heading, in milli-units per tick in the local frame.
    pub vel: (i64, i64),
    pub damage: i32,
    pub age: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sim {
    pub tick: u32,
    pub players: Vec<Player>,
    /// In id order: see [`Sim::get`].
    pub entities: Vec<Entity>,
    next_id: EntityId,
    /// In firing order.
    pub projectiles: Vec<Projectile>,
    pub outcome: Option<Outcome>,
    #[serde(skip)]
    pub events: Vec<Event>,
}

/// What one entity decided to do this tick, computed against a frozen state
/// and applied afterwards.
struct Decision {
    order: Order,
    target: Option<EntityId>,
    /// Walk toward this point until within the given distance.
    goal: Option<(Pos, i64)>,
    /// Face this point (a target), overriding the direction of travel.
    face: Option<Pos>,
    effect: Effect,
}

enum Effect {
    None,
    Repair(EntityId),
    LayTurret(Pos, Kind),
    DeployTick(Pos),
}

/// Supply income per tick for a seat with `econ` finished economy buildings.
fn income_of(econ: usize) -> i64 {
    stats::HQ_INCOME + econ as i64 * stats::ECON_INCOME
}

/// How fast an HQ with `econ` finished economy buildings builds, in percent
/// of the base pace.
fn production_pct_of(econ: usize) -> u32 {
    100 + econ as u32 * stats::ECON_PRODUCTION_PCT
}

impl Sim {
    /// A fresh match for `seats` players, each with an HQ at the centre of
    /// its sector, starting supply and one utility.
    pub fn new(seats: usize) -> Sim {
        assert!(
            (1..=MAX_SEATS).contains(&seats),
            "a match seats 1 to {MAX_SEATS} players"
        );
        let mut sim = Sim {
            tick: 0,
            players: Vec::with_capacity(seats),
            entities: Vec::new(),
            next_id: 0,
            projectiles: Vec::new(),
            outcome: None,
            events: Vec::new(),
        };
        for seat in 0..seats {
            let at = Pos::new(sector_center(seat, seats), R_MID);
            let hq = sim.spawn(seat as Seat, Kind::Hq, at, true);
            sim.players.push(Player {
                alive: true,
                supply: stats::STARTING_SUPPLY,
                queue: VecDeque::new(),
                progress: 0,
                rally: at,
                hq,
            });
            sim.spawn(
                seat as Seat,
                Kind::Utility,
                at.displaced(0, -40 * UNIT),
                true,
            );
        }
        sim
    }

    /// Put a finished entity on the map directly, outside the rules. For
    /// scenarios and tests; a match only ever grows through commands.
    pub fn place(&mut self, owner: Seat, kind: Kind, pos: Pos) -> EntityId {
        self.spawn(owner, kind, pos, true)
    }

    pub fn seats(&self) -> usize {
        self.players.len()
    }

    /// The entity with this id, if it is alive. Entities are in id order, so
    /// this is a binary search.
    pub fn get(&self, id: EntityId) -> Option<&Entity> {
        self.index_of(id).map(|i| &self.entities[i])
    }

    fn index_of(&self, id: EntityId) -> Option<usize> {
        self.entities.binary_search_by_key(&id, |e| e.id).ok()
    }

    pub fn player(&self, seat: Seat) -> Option<&Player> {
        self.players.get(seat as usize)
    }

    pub fn is_alive(&self, seat: Seat) -> bool {
        self.player(seat).is_some_and(|p| p.alive)
    }

    /// `seat`'s finished economy buildings.
    fn econ_count(&self, seat: Seat) -> usize {
        self.entities
            .iter()
            .filter(|e| e.owner == seat && e.kind == Kind::Econ && e.complete)
            .count()
    }

    /// Supply income per tick for `seat`, in milli-supply.
    pub fn income(&self, seat: Seat) -> i64 {
        if !self.is_alive(seat) {
            return 0;
        }
        income_of(self.econ_count(seat))
    }

    /// How fast `seat`'s HQ builds, in percent of the base pace.
    pub fn production_pct(&self, seat: Seat) -> u32 {
        production_pct_of(self.econ_count(seat))
    }

    /// Per-seat tallies in one pass over the entities: finished economy
    /// buildings, and mobile units. The economy and production phases both
    /// want them, and each used to rescan per seat.
    fn seat_tallies(&self) -> ([usize; MAX_SEATS], [usize; MAX_SEATS]) {
        let mut econ = [0; MAX_SEATS];
        let mut units = [0; MAX_SEATS];
        for e in &self.entities {
            if e.kind == Kind::Econ {
                econ[e.owner as usize] += usize::from(e.complete);
            } else if e.kind.is_mobile() {
                units[e.owner as usize] += 1;
            }
        }
        (econ, units)
    }

    pub fn unit_count(&self, seat: Seat) -> usize {
        self.entities
            .iter()
            .filter(|e| e.owner == seat && e.kind.is_mobile())
            .count()
    }

    /// Can `seat` put an economy building at `at`? `at` is first pulled
    /// inside the band far enough for the building to fit.
    pub fn econ_site(&self, seat: Seat, at: Pos) -> Result<Pos, PlaceError> {
        let at = at.inset(stats::ECON.radius);
        for e in &self.entities {
            if (e.kind == Kind::Econ || e.kind == Kind::Hq)
                && e.pos.within(at, stats::ECON_SPACING - 1)
            {
                return Err(PlaceError::EconSpacing);
            }
        }
        self.structure_site(seat, at, stats::ECON.radius)
    }

    /// Can `seat` put a turret of `kind` at `at`? `at` is first pulled
    /// inside the band.
    pub fn turret_site(&self, seat: Seat, at: Pos, kind: Kind) -> Result<Pos, PlaceError> {
        let radius = kind.stats().radius;
        self.structure_site(seat, at.inset(radius), radius)
    }

    fn structure_site(&self, seat: Seat, at: Pos, radius: i64) -> Result<Pos, PlaceError> {
        for e in &self.entities {
            if e.kind == Kind::Hq
                && e.owner != seat
                && e.pos.within(at, stats::ENEMY_HQ_CLEARANCE - 1)
            {
                return Err(PlaceError::EnemyHq);
            }
            if e.kind.is_structure() && e.pos.within(at, e.radius() + radius + stats::STRUCTURE_GAP)
            {
                return Err(PlaceError::Overlaps);
            }
        }
        Ok(at)
    }

    /// Advance one tick, applying `cmds` first, in order.
    pub fn step(&mut self, cmds: &[(Seat, Command)]) {
        self.events.clear();
        if self.outcome.is_none() {
            for (seat, cmd) in cmds {
                self.apply(*seat, cmd);
            }
            self.economy();
            self.production();
            self.vision();
            self.act();
            self.drift();
            self.separate();
            self.combat();
            self.cleanup();
        }
        self.tick += 1;
    }

    /// A checksum of the whole game state, for comparing peers.
    pub fn checksum(&self) -> u64 {
        // postcard is platform-independent (varints, fixed endianness), so
        // native and wasm peers hash the same bytes.
        let bytes = postcard::to_allocvec(self).expect("the sim always serialises");
        crate::hash::fnv1a(&bytes)
    }

    fn spawn(&mut self, owner: Seat, kind: Kind, pos: Pos, complete: bool) -> EntityId {
        let id = self.next_id;
        self.next_id += 1;
        let max = kind.stats().hp;
        self.entities.push(Entity {
            id,
            owner,
            kind,
            pos: pos.inset(kind.stats().radius),
            hp: if complete {
                max
            } else {
                (max * stats::FRAME_HP_PCT / 100).max(1)
            },
            order: Order::Idle,
            target: None,
            cooldown: 0,
            progress: 0,
            complete,
            facing: (0, -1000),
            push: (0, 0),
            seen_by: 1 << owner,
            stall: 0,
            last_goal_d: i64::MAX,
            consumed: false,
        });
        id
    }

    // ---- Commands --------------------------------------------------------

    fn apply(&mut self, seat: Seat, cmd: &Command) {
        if !self.is_alive(seat) {
            return;
        }
        match cmd {
            Command::Produce(kind) => {
                let cost = kind.stats().cost;
                let p = &mut self.players[seat as usize];
                if kind.is_mobile() && p.queue.len() < stats::QUEUE_MAX && p.supply >= cost {
                    p.supply -= cost;
                    p.queue.push_back(*kind);
                }
            }
            Command::CancelProduce => {
                let p = &mut self.players[seat as usize];
                if let Some(kind) = p.queue.pop_back() {
                    p.supply += kind.stats().cost;
                    if p.queue.is_empty() {
                        p.progress = 0;
                    }
                }
            }
            Command::SetRally(to) => {
                self.players[seat as usize].rally = Pos::new(to.a, to.r);
            }
            Command::Move { units, to } => self.order_group(seat, units, *to, Order::Move),
            Command::AttackMove { units, to } => {
                self.order_group(seat, units, *to, Order::AttackMove)
            }
            Command::Attack { units, target } => {
                if self.get(*target).is_none_or(|t| t.owner == seat) {
                    return;
                }
                for i in self.own_units(seat, units) {
                    if self.entities[i].stats().weapon.is_some() {
                        self.give(i, Order::Attack(*target));
                    }
                }
            }
            Command::Stop { units } => {
                for i in self.own_units(seat, units) {
                    self.give(i, Order::Idle);
                }
            }
            Command::Build { unit, at, kind } => {
                if kind.is_turret()
                    && let Some(i) = self.own_utility(seat, *unit)
                {
                    self.give(i, Order::Build(at.inset(kind.stats().radius), *kind));
                }
            }
            Command::Deploy { unit, at } => {
                if let Some(i) = self.own_utility(seat, *unit) {
                    self.give(i, Order::Deploy(at.inset(stats::ECON.radius)));
                }
            }
            Command::Repair { units, target } => {
                if self.get(*target).is_none_or(|t| t.owner != seat) {
                    return;
                }
                for i in self.own_units(seat, units) {
                    if self.entities[i].kind == Kind::Utility && self.entities[i].id != *target {
                        self.give(i, Order::Repair(*target));
                    }
                }
            }
        }
    }

    fn give(&mut self, i: usize, order: Order) {
        let e = &mut self.entities[i];
        e.order = order;
        e.target = None;
        e.progress = 0;
        e.stall = 0;
        e.last_goal_d = i64::MAX;
    }

    /// Indices of the listed ids that are `seat`'s own mobile units, in id
    /// order, without duplicates.
    fn own_units(&self, seat: Seat, ids: &[EntityId]) -> Vec<usize> {
        let mut out: Vec<usize> = ids
            .iter()
            .take(MAX_UNITS_PER_COMMAND)
            .filter_map(|id| self.index_of(*id))
            .filter(|i| {
                let e = &self.entities[*i];
                e.owner == seat && e.kind.is_mobile()
            })
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    fn own_utility(&self, seat: Seat, id: EntityId) -> Option<usize> {
        self.index_of(id)
            .filter(|i| self.entities[*i].owner == seat && self.entities[*i].kind == Kind::Utility)
    }

    /// Give a group a destination, keeping its shape when it is compact.
    fn order_group(&mut self, seat: Seat, ids: &[EntityId], to: Pos, order: fn(Pos) -> Order) {
        let group = self.own_units(seat, ids);
        let Some(&first) = group.first() else {
            return;
        };
        let anchor = self.entities[first].pos;
        let offsets: Vec<(i64, i64)> = group
            .iter()
            .map(|i| anchor.offset_to(self.entities[*i].pos))
            .collect();
        let n = offsets.len() as i64;
        let cx = offsets.iter().map(|o| o.0).sum::<i64>() / n;
        let cy = offsets.iter().map(|o| o.1).sum::<i64>() / n;
        let rel: Vec<(i64, i64)> = offsets.iter().map(|o| (o.0 - cx, o.1 - cy)).collect();
        let compact = rel
            .iter()
            .all(|r| r.0 * r.0 + r.1 * r.1 <= FORMATION_SPREAD * FORMATION_SPREAD);
        for (k, i) in group.into_iter().enumerate() {
            let dest = if compact {
                to.displaced(rel[k].0, rel[k].1)
            } else {
                to
            };
            let radius = self.entities[i].radius();
            self.give(i, order(dest.inset(radius)));
        }
    }

    // ---- Economy and production -----------------------------------------

    fn economy(&mut self) {
        let (econ, _) = self.seat_tallies();
        for (seat, buildings) in econ.iter().enumerate().take(self.seats()) {
            let income = if self.is_alive(seat as Seat) {
                income_of(*buildings)
            } else {
                0
            };
            self.players[seat].supply += income;
        }
    }

    fn production(&mut self) {
        let (econ, units) = self.seat_tallies();
        for seat in 0..self.seats() {
            let s = seat as Seat;
            let p = &self.players[seat];
            if !p.alive {
                continue;
            }
            let Some(&kind) = p.queue.front() else {
                continue;
            };
            if p.progress < kind.stats().build_ticks * 100 {
                self.players[seat].progress += production_pct_of(econ[seat]);
                continue;
            }
            if units[seat] >= stats::UNIT_CAP {
                continue;
            }
            let Some(hq) = self.get(p.hq) else {
                continue;
            };
            let rally = p.rally;
            let (dt, dr) = hq.pos.offset_to(rally);
            let reach = hq.radius() + kind.stats().radius + 2 * UNIT;
            let (dt, dr) = if dt == 0 && dr == 0 {
                (0, -reach)
            } else {
                scaled(dt, dr, reach)
            };
            let at = hq.pos.displaced(dt, dr);
            let far = hq.pos.dist(rally) > reach;
            let id = self.spawn(s, kind, at, true);
            if far {
                let i = self.entities.len() - 1;
                debug_assert_eq!(self.entities[i].id, id);
                self.give(i, Order::Move(rally.inset(kind.stats().radius)));
            }
            let p = &mut self.players[seat];
            p.queue.pop_front();
            p.progress = 0;
        }
    }

    // ---- Vision ----------------------------------------------------------

    fn vision(&mut self) {
        let n = self.entities.len();
        let mut seen: Vec<u8> = self.entities.iter().map(|e| 1u8 << e.owner).collect();
        // Sight reaches a short way along the ring, and `within` rejects a
        // pair on angle alone (see `geom::span_of`), so a sliding window over
        // the angle order visits every pair that could pass the exact test,
        // and only those. What a pair contributes is an OR of bits, so the
        // order pairs come in cannot show: this is the full scan's result.
        if n > 1 {
            let all = (1u16 << self.seats()) - 1;
            let max_vision = self
                .entities
                .iter()
                .map(|e| e.stats().vision)
                .max()
                .unwrap_or(0);
            let max_radius = self.entities.iter().map(Entity::radius).max().unwrap_or(0);
            let half = span_of(max_vision + max_radius);
            let ring = angle_order(&self.entities);
            let (mut lo, mut hi) = (0usize, 0usize);
            for c in n..2 * n {
                let (centre, ti) = ring[c];
                while hi < ring.len() && ring[hi].0 - centre <= half {
                    hi += 1;
                }
                while centre - ring[lo].0 > half {
                    lo += 1;
                }
                if seen[ti] as u16 == all {
                    continue;
                }
                let target = &self.entities[ti];
                for &(_, vi) in &ring[lo..hi] {
                    if vi == ti {
                        continue;
                    }
                    let viewer = &self.entities[vi];
                    let bit = 1u8 << viewer.owner;
                    if seen[ti] & bit == 0
                        && viewer
                            .pos
                            .within(target.pos, viewer.stats().vision + target.radius())
                    {
                        seen[ti] |= bit;
                    }
                }
            }
        }
        for (e, s) in self.entities.iter_mut().zip(seen) {
            e.seen_by = s;
        }
    }

    // ---- Orders, movement and abilities -----------------------------------

    /// The target `e` should engage within `range` (plus the target's radius):
    /// its current one if still valid, otherwise the nearest visible enemy.
    fn engage(&self, e: &Entity, range: i64) -> Option<EntityId> {
        let valid = |t: &Entity| {
            t.owner != e.owner && t.seen_by_seat(e.owner) && e.pos.within(t.pos, range + t.radius())
        };
        if let Some(t) = e.target.and_then(|id| self.get(id))
            && valid(t)
        {
            return Some(t.id);
        }
        self.entities
            .iter()
            .filter(|t| valid(t))
            .min_by_key(|t| (e.pos.dist2(t.pos), t.id))
            .map(|t| t.id)
    }

    /// How close `e` should stand to shoot `target`.
    fn reach(e: &Entity, target: &Entity) -> i64 {
        let range = e.stats().weapon.as_ref().map_or(0, |w| w.range);
        (range + target.radius() - UNIT).max(0)
    }

    fn decide(&self, e: &Entity) -> Decision {
        let mut d = Decision {
            order: e.order.clone(),
            target: None,
            goal: None,
            face: None,
            effect: Effect::None,
        };

        if e.kind.is_structure() {
            if e.armed() {
                let range = e.stats().weapon.as_ref().map_or(0, |w| w.range);
                d.target = self.engage(e, range);
            }
            d.face = d.target.and_then(|t| self.get(t)).map(|t| t.pos);
            return d;
        }

        let armed = e.stats().weapon.is_some();
        let chase = |d: &mut Decision, target: EntityId| {
            d.target = Some(target);
            if let Some(t) = self.get(target) {
                d.goal = Some((t.pos, Self::reach(e, t)));
                d.face = Some(t.pos);
            }
        };

        match e.order {
            Order::Idle => {
                if armed && let Some(t) = self.engage(e, stats::acquire_range(e.kind)) {
                    chase(&mut d, t);
                }
            }
            Order::Move(to) => d.goal = Some((to, 0)),
            Order::AttackMove(to) => {
                if armed && let Some(t) = self.engage(e, stats::acquire_range(e.kind)) {
                    chase(&mut d, t);
                } else {
                    d.goal = Some((to, 0));
                }
            }
            Order::Attack(target) => match self.get(target) {
                Some(t) if t.owner != e.owner && t.seen_by_seat(e.owner) => chase(&mut d, target),
                _ => d.order = Order::Idle,
            },
            Order::Build(at, kind) => {
                if e.pos.within(at, stats::BUILD_RANGE) {
                    d.effect = Effect::LayTurret(at, kind);
                } else {
                    d.goal = Some((at, stats::BUILD_RANGE - UNIT));
                }
            }
            Order::Repair(target) => match self.get(target) {
                Some(t) if t.owner == e.owner && (!t.complete || t.hp < t.max_hp()) => {
                    let reach = stats::REPAIR_RANGE + t.radius();
                    if e.pos.within(t.pos, reach) {
                        d.effect = Effect::Repair(target);
                        d.face = Some(t.pos);
                    } else {
                        d.goal = Some((t.pos, reach - UNIT));
                    }
                }
                _ => d.order = Order::Idle,
            },
            Order::Deploy(at) => {
                if e.pos.within(at, UNIT) {
                    d.effect = Effect::DeployTick(at);
                } else {
                    d.goal = Some((at, 0));
                }
            }
        }
        d
    }

    fn act(&mut self) {
        for i in 0..self.entities.len() {
            let d = self.decide(&self.entities[i]);
            let e = &mut self.entities[i];
            e.order = d.order;
            e.target = d.target;

            if let Some((goal, within)) = d.goal {
                let dist = e.pos.dist(goal);
                // A destination order that stops making progress near its
                // goal is being jostled by neighbours: call it arrived. A
                // utility headed to deploy settles for where it stands, but
                // only if that is close to the site it was sent to.
                let stall_radius = match e.order {
                    Order::Move(_) | Order::AttackMove(_) if e.target.is_none() => STALL_RADIUS,
                    Order::Deploy(_) => DEPLOY_STALL_RADIUS,
                    _ => 0,
                };
                let travelling = stall_radius > 0;
                if travelling {
                    if dist < stall_radius && dist + e.stats().speed / 4 > e.last_goal_d {
                        e.stall += 1;
                    } else {
                        e.stall = 0;
                    }
                    e.last_goal_d = dist;
                }
                if dist <= within || (travelling && e.stall >= STALL_TICKS) {
                    if travelling {
                        e.order = match e.order {
                            Order::Deploy(_) => Order::Deploy(e.pos),
                            _ => Order::Idle,
                        };
                        e.stall = 0;
                        e.last_goal_d = i64::MAX;
                    }
                } else {
                    let step = e.stats().speed.min(dist - within);
                    let (t, r) = e.pos.offset_to(goal);
                    e.facing = scaled(t, r, 1000);
                    e.pos = e.pos.step_toward(goal, step).0.inset(e.radius());
                }
            }
            if let Some(face) = d.face {
                let (t, r) = e.pos.offset_to(face);
                if t != 0 || r != 0 {
                    e.facing = scaled(t, r, 1000);
                }
            }

            match d.effect {
                Effect::None => {}
                Effect::Repair(target) => self.repair(target),
                Effect::LayTurret(at, kind) => self.lay_turret(i, at, kind),
                Effect::DeployTick(at) => self.deploy_tick(i, at),
            }
        }
    }

    fn repair(&mut self, target: EntityId) {
        let tick = self.tick;
        let Some(t) = self.index_of(target) else {
            return;
        };
        let t = &mut self.entities[t];
        let max = t.max_hp();
        if t.complete {
            if tick.is_multiple_of(stats::REPAIR_INTERVAL) {
                t.hp = (t.hp + stats::REPAIR_AMOUNT).min(max);
            }
            return;
        }
        let build = t.stats().build_ticks.max(1);
        let frame = max * stats::FRAME_HP_PCT / 100;
        t.progress += 1;
        // Grow from the frame's HP to full across the build, keeping any
        // damage taken meanwhile.
        let grown = |p: u32| frame + (max - frame) * p.min(build) as i32 / build as i32;
        t.hp = (t.hp + grown(t.progress) - grown(t.progress - 1)).min(max);
        if t.progress >= build {
            t.complete = true;
            let (owner, kind, pos) = (t.owner, t.kind, t.pos);
            self.events.push(Event::Completed { owner, kind, pos });
        }
    }

    fn lay_turret(&mut self, builder: usize, at: Pos, kind: Kind) {
        let owner = self.entities[builder].owner;
        let cost = kind.stats().cost;
        let site = self.turret_site(owner, at, kind);
        let p = &mut self.players[owner as usize];
        let order = match site {
            Ok(at) if p.supply >= cost => {
                p.supply -= cost;
                Order::Repair(self.spawn(owner, kind, at, false))
            }
            _ => Order::Idle,
        };
        self.give(builder, order);
    }

    fn deploy_tick(&mut self, i: usize, at: Pos) {
        let owner = self.entities[i].owner;
        // The site is checked when deploying begins and again when it ends:
        // someone may have claimed the spot in between.
        let check = self.entities[i].progress == 0
            || self.entities[i].progress + 1 >= stats::ECON.build_ticks;
        if check && self.econ_site(owner, at).is_err() {
            self.give(i, Order::Idle);
            return;
        }
        let e = &mut self.entities[i];
        e.progress += 1;
        if e.progress >= stats::ECON.build_ticks {
            e.consumed = true;
            e.hp = 0;
            let pos = at.inset(stats::ECON.radius);
            self.spawn(owner, Kind::Econ, pos, true);
            self.events.push(Event::Completed {
                owner,
                kind: Kind::Econ,
                pos,
            });
        }
    }

    // ---- Separation --------------------------------------------------------

    /// Push overlapping bodies apart: mobile units share the overlap, and
    /// structures do not move.
    fn separate(&mut self) {
        let n = self.entities.len();
        let mut push: Vec<(i64, i64)> = vec![(0, 0); n];
        // Overlapping bodies lie within two radii of one another along the
        // ring, so the same sliding window as vision's visits every pair that
        // could pass the exact test, and only those. Each pair is handled
        // once, from the smaller index's window, and each pair's contribution
        // to the pushes is its own integer summand, so the sums are the full
        // scan's whatever the order the pairs come in.
        let max_radius = self.entities.iter().map(Entity::radius).max().unwrap_or(0);
        let half = span_of(2 * max_radius);
        let ring = angle_order(&self.entities);
        let (mut lo, mut hi) = (0usize, 0usize);
        for c in n..2 * n {
            let (centre, ai) = ring[c];
            while hi < ring.len() && ring[hi].0 - centre <= half {
                hi += 1;
            }
            while centre - ring[lo].0 > half {
                lo += 1;
            }
            let a = &self.entities[ai];
            if a.consumed {
                continue;
            }
            for &(_, bi) in &ring[lo..hi] {
                if bi <= ai {
                    continue;
                }
                let b = &self.entities[bi];
                if b.consumed || (a.kind.is_structure() && b.kind.is_structure()) {
                    continue;
                }
                let min = a.radius() + b.radius();
                if !a.pos.within(b.pos, min) {
                    continue;
                }
                let (t, r) = a.pos.offset_to(b.pos);
                let d = crate::geom::isqrt(t * t + r * r);
                let overlap = min - d;
                // Coincident bodies part along the ring, by id.
                let (ux, uy) = if d == 0 {
                    (1000, 0)
                } else {
                    scaled(t, r, 1000)
                };
                let (share_a, share_b) = match (a.kind.is_mobile(), b.kind.is_mobile()) {
                    (true, true) => (overlap / 2, overlap - overlap / 2),
                    (true, false) => (overlap, 0),
                    (false, true) => (0, overlap),
                    (false, false) => (0, 0),
                };
                push[ai].0 -= ux * share_a / 1000;
                push[ai].1 -= uy * share_a / 1000;
                push[bi].0 += ux * share_b / 1000;
                push[bi].1 += uy * share_b / 1000;
            }
        }
        for (e, (t, r)) in self.entities.iter_mut().zip(push) {
            if t != 0 || r != 0 {
                e.pos = e.pos.displaced(t, r).inset(e.radius());
            }
        }
    }

    // ---- Combat ------------------------------------------------------------

    fn combat(&mut self) {
        let mut hits: Vec<(usize, i32)> = Vec::new();
        for i in 0..self.entities.len() {
            let e = &mut self.entities[i];
            e.cooldown = e.cooldown.saturating_sub(1);
            let e = &self.entities[i];
            if e.cooldown > 0 || !e.armed() {
                continue;
            }
            let Some(t) = e.target.and_then(|id| self.index_of(id)) else {
                continue;
            };
            let target = &self.entities[t];
            let weapon = e.stats().weapon.as_ref().expect("armed");
            if !e.pos.within(target.pos, weapon.range + target.radius()) {
                continue;
            }
            self.events.push(Event::Shot {
                from: e.pos,
                to: target.pos,
                owner: e.owner,
                kind: e.kind,
            });
            if weapon.shot == Shot::Melee {
                let damage = weapon.damage * stats::damage_pct(e.kind, target.kind) / 100;
                hits.push((t, damage));
            } else {
                let projectile = launch(e, target, weapon, self.projectiles.len());
                self.projectiles.push(projectile);
            }
            self.entities[i].cooldown = weapon.cooldown;
        }
        self.fly(&mut hits);
        for (t, damage) in hits {
            self.entities[t].hp -= damage;
        }
    }

    /// Move everything in flight, and collect what lands.
    fn fly(&mut self, hits: &mut Vec<(usize, i32)>) {
        let mut landed = Vec::new();
        for (n, p) in self.projectiles.iter_mut().enumerate() {
            p.prev = p.pos;
            p.age += 1;
            let target = self
                .entities
                .binary_search_by_key(&p.target, |e| e.id)
                .ok()
                .map(|i| &self.entities[i]);
            // Whether it arrived, and whether that was on its target.
            let (arrived, on_target) = match p.shot {
                Shot::Melee => (true, true),
                Shot::Bullet { speed } => {
                    if let Some(t) = target {
                        p.aim = t.pos;
                    }
                    let (pos, arrived) = p.pos.step_toward(p.aim, speed);
                    p.pos = pos;
                    (arrived, true)
                }
                Shot::Plasma { speed, .. } => {
                    let (pos, arrived) = p.pos.step_toward(p.aim, speed);
                    p.pos = pos;
                    (arrived, true)
                }
                Shot::Missile { speed, turn } => {
                    if let Some(t) = target {
                        p.aim = t.pos;
                    }
                    // Steer part of the way toward the target, at full speed.
                    let (dt, dr) = p.pos.offset_to(p.aim);
                    let want = scaled(dt, dr, speed);
                    let steered = (
                        p.vel.0 + (want.0 - p.vel.0) * turn / 1000,
                        p.vel.1 + (want.1 - p.vel.1) * turn / 1000,
                    );
                    p.vel = scaled(steered.0, steered.1, speed);
                    p.pos = p.pos.displaced(p.vel.0, p.vel.1);
                    let reach = speed + target.map_or(0, |t| t.radius());
                    let hit = p.pos.within(p.aim, reach);
                    // Out of fuel, it bursts where it is, harmlessly.
                    (hit || p.age >= stats::MISSILE_FUEL, hit)
                }
            };
            if arrived {
                landed.push((n, on_target));
            }
        }
        if landed.is_empty() {
            return;
        }
        for &(n, on_target) in &landed {
            let p = &self.projectiles[n];
            let target = self.index_of(p.target);
            match p.shot {
                Shot::Plasma { splash, .. } => {
                    // Everything of the enemy's in the burst: full strength
                    // in the middle, half at the edge.
                    for (i, e) in self.entities.iter().enumerate() {
                        if e.owner == p.owner {
                            continue;
                        }
                        let reach = splash + e.radius();
                        if !p.aim.within(e.pos, reach) {
                            continue;
                        }
                        let d = p.aim.dist(e.pos);
                        let pct = 100 - 50 * d.min(reach) / reach.max(1);
                        let damage = p.damage * stats::damage_pct(p.shooter, e.kind) / 100;
                        hits.push((i, damage * pct as i32 / 100));
                    }
                }
                _ => {
                    if let Some(t) = target.filter(|_| on_target) {
                        let e = &self.entities[t];
                        hits.push((t, p.damage * stats::damage_pct(p.shooter, e.kind) / 100));
                    }
                }
            }
            self.events.push(Event::Impact {
                at: p.pos,
                owner: p.owner,
                kind: p.shooter,
                target: target.map(|t| self.entities[t].id),
            });
        }
        // Take the landed ones out, keeping firing order.
        let mut keep = vec![true; self.projectiles.len()];
        for &(n, _) in &landed {
            keep[n] = false;
        }
        let flown = std::mem::take(&mut self.projectiles);
        self.projectiles = flown
            .into_iter()
            .zip(keep)
            .filter_map(|(p, keep)| keep.then_some(p))
            .collect();
    }

    /// Units thrown by blasts slide, slowing as they go.
    fn drift(&mut self) {
        for e in &mut self.entities {
            if e.push == (0, 0) {
                continue;
            }
            e.pos = e.pos.displaced(e.push.0, e.push.1).inset(e.radius());
            e.push = (e.push.0 * 3 / 4, e.push.1 * 3 / 4);
            if e.push.0.abs() + e.push.1.abs() < UNIT / 10 {
                e.push = (0, 0);
            }
        }
    }

    /// A building exploding at `at`: every mobile unit within its blast is
    /// thrown away from it, harder the closer it stood and the lighter it is.
    fn blast(&mut self, at: Pos, owner: Seat, blast: &stats::Blast) {
        for e in &mut self.entities {
            let mass = e.stats().mass;
            if mass == 0 || e.hp <= 0 || !at.within(e.pos, blast.radius) {
                continue;
            }
            let (t, r) = at.offset_to(e.pos);
            let d = isqrt(t * t + r * r);
            let (ux, uy) = if d == 0 {
                (1000, 0)
            } else {
                scaled(t, r, 1000)
            };
            let strength = blast.force * (blast.radius - d) / blast.radius / mass;
            e.push.0 += ux * strength / 1000;
            e.push.1 += uy * strength / 1000;
        }
        self.events.push(Event::Blast {
            at,
            owner,
            radius: blast.radius,
        });
    }

    // ---- Deaths, elimination, victory ------------------------------------

    fn cleanup(&mut self) {
        let mut fallen = [false; MAX_SEATS];
        for e in &self.entities {
            if e.hp <= 0 && e.kind == Kind::Hq {
                fallen[e.owner as usize] = true;
            }
        }
        // The dead, and everything a fallen player owned, go out with a bang.
        let mut blasts = Vec::new();
        for e in &self.entities {
            let dies = e.hp <= 0 || fallen[e.owner as usize];
            if !dies || e.consumed {
                continue;
            }
            self.events.push(Event::Died {
                owner: e.owner,
                kind: e.kind,
                pos: e.pos,
            });
            if let Some(blast) = &e.stats().blast {
                blasts.push((e.pos, e.owner, blast));
            }
        }
        for seat in (0..self.seats() as Seat).filter(|&s| fallen[s as usize]) {
            let p = &mut self.players[seat as usize];
            p.alive = false;
            p.queue.clear();
            p.progress = 0;
            self.events.push(Event::Eliminated(seat));
        }
        self.entities
            .retain(|e| e.hp > 0 && !fallen[e.owner as usize]);
        for (at, owner, blast) in blasts {
            self.blast(at, owner, blast);
        }

        if fallen.iter().any(|&f| f) && self.seats() > 1 {
            let alive: Vec<Seat> = (0..self.seats() as Seat)
                .filter(|s| self.is_alive(*s))
                .collect();
            self.outcome = match alive.as_slice() {
                [] => Some(Outcome::Draw),
                [winner] => Some(Outcome::Winner(*winner)),
                _ => None,
            };
        }
    }
}

/// The entities in angle order, copied three times over: at a turn below,
/// at none, and at a turn above. Windows are taken for centres in the middle
/// copy, so a window reaches across the seam at angle zero either way:
/// backward into the lower copy, forward into the upper one.
fn angle_order(entities: &[Entity]) -> Vec<(i64, usize)> {
    const TURN: i64 = 1 << 32;
    let mut sorted: Vec<(i64, usize)> = entities
        .iter()
        .enumerate()
        .map(|(i, e)| (e.pos.a.0 as i64, i))
        .collect();
    sorted.sort_unstable_by_key(|&(a, _)| a);
    let n = sorted.len();
    let mut ring = Vec::with_capacity(3 * n);
    ring.extend(sorted.iter().map(|&(a, i)| (a - TURN, i)));
    ring.extend(sorted.iter().copied());
    ring.extend(sorted.iter().map(|&(a, i)| (a + TURN, i)));
    ring
}

/// A projectile leaving `from` for `target`. Missiles set off sideways,
/// alternately to either side, and curve in.
fn launch(from: &Entity, target: &Entity, weapon: &stats::Weapon, count: usize) -> Projectile {
    let (dt, dr) = from.pos.offset_to(target.pos);
    let vel = match weapon.shot {
        Shot::Missile { speed, .. } => {
            let (ux, uy) = scaled(dt, dr, 1000);
            // Turned about 70 degrees off the line to the target.
            let (c, s) = (342, if count.is_multiple_of(2) { 940 } else { -940 });
            let side = ((ux * c - uy * s) / 1000, (ux * s + uy * c) / 1000);
            scaled(side.0, side.1, speed)
        }
        _ => (0, 0),
    };
    Projectile {
        owner: from.owner,
        shooter: from.kind,
        shot: weapon.shot,
        pos: from.pos,
        prev: from.pos,
        target: target.id,
        aim: target.pos,
        vel,
        damage: weapon.damage,
        age: 0,
    }
}
