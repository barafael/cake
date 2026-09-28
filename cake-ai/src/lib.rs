//! Scripted bots.
//!
//! A bot is just another seat: it reads the simulation and returns
//! [`Command`]s, which the host sequences like anyone else's. It has no
//! privileged path into the game. It does read the whole state, fog included,
//! which is fine for a sparring partner.
//!
//! Every bot has a [`Personality`], and plays it:
//!
//! - **Its land is implicit.** Sectors are only where players start: a bot
//!   holds the stretch of ring nearer its HQ than any other living HQ, so
//!   when a neighbour falls, its land grows into the gap, and it claims the
//!   new ground with economy buildings.
//! - **Invaded, it fights with zeal.** Any enemy on its land, or near one of
//!   its buildings, brings the whole home army down on it, calls an attack
//!   back if the invasion is serious, and turns production to counters of
//!   what came in. Defenders chase invaders out to the border, then go home.
//! - **It watches its neighbours.** It keeps a running measure of how hard
//!   each one presses into its land and what their armies are made of, and
//!   answers: counters in its production, extra turrets under pressure, and
//!   for the vengeful, a counterattack on whoever came last.
//! - **It attacks in waves** when its home army is big enough, leaving a
//!   garrison behind, and goes for the nearest enemy building first, so
//!   fights start at the borders and push in. A beaten attack turns back.
//! - **It raids** with packs of raiders that go for economy and flee when
//!   hurt.
//!
//! The economy is the same for all: keep the HQ busy, turn utilities into
//! economy buildings at the nearest free sites, and keep one utility back to
//! build turrets and repair.

pub mod personality;

pub use personality::Personality;

use cake_core::geom::{Angle, R_INNER, R_MID, R_OUTER, UNIT, arc_len, arc_to_angle};
use cake_core::stats::{self, Kind, TICK_HZ};
use cake_core::{Command, Entity, EntityId, Order, Pos, Seat, Sim};

/// A bot thinks once a second.
const THINK_EVERY: u32 = TICK_HZ;
/// Enemies this close to one of my buildings are invaders too.
const NEAR_BUILDING: i64 = 110 * UNIT;
/// My land reaches this far past the halfway marks to my neighbours.
const BORDER_MARGIN: i64 = 40 * UNIT;
/// Economy buildings go this far past the halfway marks: border sites are
/// worth contesting.
const CLAIM_MARGIN: i64 = 60 * UNIT;
/// A bot wants at most this many times its temperament's economy, however
/// much land it holds.
const MAX_ECON_FACTOR: usize = 3;
/// Defenders chase this far out of home before turning back.
const CHASE: i64 = 80 * UNIT;
/// Utilities the bot keeps rather than deploying.
const KEEP_UTILITIES: usize = 1;
/// How fast remembered pressure fades, per think.
const PRESSURE_DECAY: f32 = 0.85;
/// Pressure above this gets extra turrets.
const UNDER_PRESSURE: f32 = 6.0;
/// A grudge lasts this many ticks.
const GRUDGE_TICKS: u32 = 90 * TICK_HZ;
const MAX_WAVE: usize = 30;
/// A home army that has been big enough for a first wave this long, without
/// reaching the wave it waits for, goes anyway: losses can keep a bot from
/// ever growing its army further.
const PATIENCE: u32 = 60 * TICK_HZ;

#[derive(Clone, Debug)]
pub struct Bot {
    seat: Seat,
    personality: Personality,
    /// Attacks launched so far; each wave waits for a bigger army.
    waves: u32,
    /// Which side the next turret goes on.
    turret_side: i64,
    attack: Option<Sortie>,
    raid: Option<Sortie>,
    /// How much of each seat's army has been on my land lately.
    pressure: Vec<f32>,
    /// The land I hold, as of my last think.
    land: Land,
    /// Who invaded me last, and when.
    grudge: Option<(Seat, u32)>,
    /// Since when the home army has been big enough for a first wave but
    /// short of the wave it waits for.
    waiting: Option<u32>,
}

/// Units sent out together: an attack or a raid.
#[derive(Clone, Debug)]
struct Sortie {
    units: Vec<EntityId>,
    target: Seat,
    /// Their total HP when they set out.
    start: i64,
}

impl Bot {
    /// A bot for `seat`, with that seat's personality.
    pub fn new(seat: Seat) -> Bot {
        Bot::with_personality(seat, personality::for_seat(seat))
    }

    pub fn with_personality(seat: Seat, personality: Personality) -> Bot {
        Bot {
            seat,
            personality,
            waves: 0,
            turret_side: 1,
            attack: None,
            raid: None,
            pressure: Vec::new(),
            grudge: None,
            waiting: None,
            land: Land::default(),
        }
    }

    pub fn seat(&self) -> Seat {
        self.seat
    }

    pub fn personality(&self) -> &Personality {
        &self.personality
    }

    /// Commands for the coming tick. Mostly empty: bots think once a second,
    /// staggered by seat so they don't all think on the same tick.
    pub fn think(&mut self, sim: &Sim) -> Vec<Command> {
        let seat = self.seat;
        if !sim.is_alive(seat) || sim.outcome.is_some() {
            return Vec::new();
        }
        if !(sim.tick + seat as u32 * 3).is_multiple_of(THINK_EVERY) {
            return Vec::new();
        }
        let Some(land) = Land::of(sim, seat) else {
            return Vec::new();
        };
        let hq = land.hq;
        self.land = land;
        self.observe(sim);
        let invaders = self.invaders(sim);
        let mut out = Vec::new();
        self.produce(sim, &invaders, &mut out);
        self.utilities(sim, hq, !invaders.is_empty(), &mut out);
        if invaders.is_empty() {
            self.keep_attacking(sim, &mut out);
            self.maybe_attack(sim, hq, &mut out);
        } else {
            self.defend(sim, hq, &invaders, &mut out);
        }
        self.raid(sim, hq, &mut out);
        self.send_home(sim, &mut out);
        out
    }

    // ---- What I know -------------------------------------------------------

    fn mine<'a>(&self, sim: &'a Sim) -> impl Iterator<Item = &'a Entity> {
        let seat = self.seat;
        sim.entities.iter().filter(move |e| e.owner == seat)
    }

    /// My army: armed mobile units.
    fn army<'a>(&self, sim: &'a Sim) -> impl Iterator<Item = &'a Entity> {
        self.mine(sim).filter(|e| is_fighter(e.kind))
    }

    fn turrets(&self, sim: &Sim) -> usize {
        self.mine(sim).filter(|e| e.kind.is_turret()).count()
    }

    /// The turret to build next: the personality's defences, in turn.
    fn next_turret(&self, sim: &Sim) -> Kind {
        let d = self.personality.defences;
        d[self.turrets(sim) % d.len()]
    }

    fn count(&self, sim: &Sim, kind: Kind) -> usize {
        let queued = sim.players[self.seat as usize]
            .queue
            .iter()
            .filter(|k| **k == kind)
            .count();
        self.mine(sim).filter(|e| e.kind == kind).count() + queued
    }

    /// Is `p` at home, give or take the border margin and `extra`? Home is
    /// my land as far out as a starting sector reaches: that much I defend
    /// with everything. Ground claimed beyond it is defended where it has
    /// buildings (see [`NEAR_BUILDING`]), so that two survivors sharing the
    /// ring don't each call every attack home the moment it crosses the
    /// middle.
    fn at_home(&self, sim: &Sim, p: Pos, extra: i64) -> bool {
        let half_sector = (1i64 << 32) / (2 * sim.seats() as i64);
        let land = Land {
            ccw: self.land.ccw.min(half_sector),
            cw: self.land.cw.min(half_sector),
            ..self.land
        };
        land.holds(p, BORDER_MARGIN + extra)
    }

    /// Economy buildings wanted: my temperament's, for as much land as a
    /// sector, and more as my land grows.
    fn econ_wanted(&self, sim: &Sim) -> usize {
        let sector = (1i64 << 32) / sim.seats() as i64;
        let base = self.personality.econ;
        let grown = base as i64 * self.land.width() / sector;
        (grown as usize).clamp(base, base * MAX_ECON_FACTOR)
    }

    /// Enemy units in my territory, or close to one of my buildings, that I
    /// can see.
    fn invaders<'a>(&self, sim: &'a Sim) -> Vec<&'a Entity> {
        let seat = self.seat;
        let buildings: Vec<Pos> = self
            .mine(sim)
            .filter(|e| e.kind.is_structure())
            .map(|e| e.pos)
            .collect();
        sim.entities
            .iter()
            .filter(|e| e.owner != seat && e.kind.is_mobile() && e.seen_by_seat(seat))
            .filter(|e| {
                self.at_home(sim, e.pos, 0)
                    || buildings.iter().any(|b| b.within(e.pos, NEAR_BUILDING))
            })
            .collect()
    }

    /// Update the running measures of my neighbours: who presses into my
    /// sector, and my grudge against the last to come.
    fn observe(&mut self, sim: &Sim) {
        let n = sim.seats();
        self.pressure.resize(n, 0.0);
        let mut present = vec![0.0f32; n];
        for e in &sim.entities {
            if e.owner != self.seat && e.kind.is_mobile() && self.at_home(sim, e.pos, 0) {
                present[e.owner as usize] += 1.0;
            }
        }
        for (p, now) in self.pressure.iter_mut().zip(present) {
            *p = *p * PRESSURE_DECAY + now;
        }
        if let Some((seat, _)) = self
            .pressure
            .iter()
            .enumerate()
            .filter(|(s, p)| **p >= 3.0 && sim.is_alive(*s as Seat))
            .max_by(|a, b| a.1.total_cmp(b.1))
        {
            self.grudge = Some((seat as Seat, sim.tick));
        }
        if self
            .grudge
            .is_some_and(|(s, at)| !sim.is_alive(s) || sim.tick > at + GRUDGE_TICKS)
        {
            self.grudge = None;
        }
    }

    fn max_pressure(&self) -> f32 {
        self.pressure.iter().copied().fold(0.0, f32::max)
    }

    /// The alive seats either side of me.
    fn neighbours(&self, sim: &Sim) -> Vec<Seat> {
        let n = sim.seats() as i64;
        let mut out: Vec<Seat> = [1, -1]
            .into_iter()
            .filter_map(|dir| {
                (1..n)
                    .map(|k| (self.seat as i64 + dir * k).rem_euclid(n) as Seat)
                    .find(|s| sim.is_alive(*s))
            })
            .collect();
        out.dedup();
        out
    }

    // ---- Production --------------------------------------------------------

    /// Turrets wanted: the personality's, one more under pressure.
    fn turrets_wanted(&self) -> usize {
        self.personality.turrets + usize::from(self.max_pressure() >= UNDER_PRESSURE)
    }

    /// Once the economy is started, fortify.
    fn wants_turret(&self, sim: &Sim) -> bool {
        self.count(sim, Kind::Econ) >= 2
            && self.turrets(sim) < self.turrets_wanted()
            && self.mine(sim).any(|e| e.kind == Kind::Utility)
    }

    /// How much to want each army kind: the personality's mix, tilted toward
    /// counters of what my neighbours field (weighted by how hard they press)
    /// and, most of all, of what is invading right now.
    fn army_weights(&self, sim: &Sim, invaders: &[&Entity]) -> [f32; 3] {
        let mut w = self.personality.mix.map(|m| m as f32);
        let mut enemy = [0.0f32; 3];
        for s in self.neighbours(sim) {
            let heat = 1.0 + self.pressure.get(s as usize).copied().unwrap_or(0.0) / 4.0;
            for e in sim.entities.iter().filter(|e| e.owner == s) {
                if let Some(i) = army_index(e.kind) {
                    enemy[i] += heat;
                }
            }
        }
        for e in invaders {
            if let Some(i) = army_index(e.kind) {
                enemy[i] += 4.0;
            }
        }
        let total: f32 = enemy.iter().sum();
        if total > 0.0 {
            let base: f32 = w.iter().sum();
            for (i, share) in enemy.iter().enumerate() {
                // Up to as much again as the whole mix, toward the counter.
                w[army_index(counter(ARMY[i])).expect("army")] += base * share / total;
            }
        }
        w
    }

    fn produce(&self, sim: &Sim, invaders: &[&Entity], out: &mut Vec<Command>) {
        let p = &sim.players[self.seat as usize];
        let emergency = !invaders.is_empty();
        let queue_to = if emergency { 3 } else { 2 };
        if p.queue.len() >= queue_to || sim.unit_count(self.seat) >= stats::UNIT_CAP {
            return;
        }
        let econ = self.count(sim, Kind::Econ);
        let wanted_utilities = KEEP_UTILITIES + usize::from(econ < self.econ_wanted(sim));
        let kind = if !emergency && self.count(sim, Kind::Utility) < wanted_utilities {
            Kind::Utility
        } else {
            // The army kind furthest below its weighted share.
            let w = self.army_weights(sim, invaders);
            ARMY.into_iter()
                .zip(w)
                .min_by(|(a, wa), (b, wb)| {
                    let fa = self.count(sim, *a) as f32 / wa.max(0.1);
                    let fb = self.count(sim, *b) as f32 / wb.max(0.1);
                    fa.total_cmp(&fb).then(a.cmp(b))
                })
                .map(|(k, _)| k)
                .expect("three kinds")
        };
        // Saving for a turret, unless there's a fight on.
        let reserve = if !emergency && self.wants_turret(sim) {
            self.next_turret(sim).stats().cost
        } else {
            0
        };
        if p.supply >= kind.stats().cost + reserve {
            out.push(Command::Produce(kind));
        }
    }

    // ---- Utilities ---------------------------------------------------------

    fn utilities(&mut self, sim: &Sim, hq: Pos, fighting: bool, out: &mut Vec<Command>) {
        let idle: Vec<&Entity> = self
            .mine(sim)
            .filter(|e| e.kind == Kind::Utility && e.order == Order::Idle)
            .collect();
        let total = self.mine(sim).filter(|e| e.kind == Kind::Utility).count();
        let mut spare = total.saturating_sub(KEEP_UTILITIES);
        let supply = sim.players[self.seat as usize].supply;

        for u in idle {
            if spare > 0
                && !fighting
                && let Some(site) = self.econ_site(sim, hq, u.pos)
            {
                out.push(Command::Deploy {
                    unit: u.id,
                    at: site,
                });
                spare -= 1;
                continue;
            }
            // The keeper: repair first, then fortify.
            if let Some(hurt) = self
                .mine(sim)
                .filter(|e| e.kind.is_structure() && (e.hp < e.max_hp() || !e.complete))
                .min_by_key(|e| (e.pos.dist2(u.pos), e.id))
            {
                out.push(Command::Repair {
                    units: vec![u.id],
                    target: hurt.id,
                });
                continue;
            }
            let kind = self.next_turret(sim);
            if self.wants_turret(sim)
                && supply >= kind.stats().cost
                && let Some(at) = self.turret_site(sim, hq, kind)
            {
                out.push(Command::Build {
                    unit: u.id,
                    at,
                    kind,
                });
            }
        }
    }

    /// Where the next turret goes: on the front that presses hardest, or
    /// alternating when neither does, staggered inward and outward as more
    /// go up on the same side. Where that is taken, the nearest free spot
    /// around it, then on the other side.
    fn turret_site(&mut self, sim: &Sim, hq: Pos, kind: Kind) -> Option<Pos> {
        let neighbours = self.neighbours(sim);
        let pressed = neighbours
            .iter()
            .copied()
            .max_by(|a, b| {
                let pa = self.pressure.get(*a as usize).copied().unwrap_or(0.0);
                let pb = self.pressure.get(*b as usize).copied().unwrap_or(0.0);
                pa.total_cmp(&pb)
            })
            .filter(|s| self.pressure.get(*s as usize).copied().unwrap_or(0.0) >= 1.0);
        let side = match pressed.and_then(|s| sim.get(sim.players[s as usize].hq)) {
            Some(enemy) => {
                if hq.offset_to(enemy.pos).0 >= 0 {
                    1
                } else {
                    -1
                }
            }
            None => {
                self.turret_side = -self.turret_side;
                self.turret_side
            }
        };
        let half_sector = arc_len((1i64 << 32) / sim.seats() as i64 / 2, R_INNER);
        let along = (half_sector * 55 / 100).clamp(70 * UNIT, 130 * UNIT);
        let first = self.turrets(sim);
        let mut spots = Vec::new();
        for side in [side, -side] {
            for shift in [0, -24, 24, -48, 48] {
                for k in 0..3 {
                    let stagger = [0, 28, -28][(first + k) % 3];
                    let at = Pos::new(hq.a, R_MID + stagger * UNIT)
                        .displaced(side * (along + shift * UNIT), 0);
                    spots.push(at);
                }
            }
        }
        spots
            .into_iter()
            .find_map(|at| sim.turret_site(self.seat, at, kind).ok())
    }

    /// The free economy site nearest `from`, searching outward from the HQ
    /// along the band to a little past the edges of my land. Sites another of
    /// my utilities is already heading for are skipped.
    fn econ_site(&self, sim: &Sim, hq: Pos, from: Pos) -> Option<Pos> {
        let claimed: Vec<Pos> = self
            .mine(sim)
            .filter_map(|e| match e.order {
                Order::Deploy(at) => Some(at),
                _ => None,
            })
            .collect();
        let reach = [
            (-1, arc_len(self.land.cw, R_INNER) + CLAIM_MARGIN),
            (1, arc_len(self.land.ccw, R_INNER) + CLAIM_MARGIN),
        ];
        let furthest = reach.iter().map(|(_, r)| *r).max().unwrap_or(0);
        let mut best: Option<(i64, Pos)> = None;
        let mut along = 150 * UNIT;
        while along <= furthest {
            for (side, limit) in reach {
                if along > limit {
                    continue;
                }
                for r in [
                    R_INNER + 25 * UNIT,
                    (R_INNER + R_OUTER) / 2,
                    R_OUTER - 25 * UNIT,
                ] {
                    let probe = Pos::new(hq.a, r).displaced(side * along, 0);
                    if claimed.iter().any(|c| c.within(probe, stats::ECON_SPACING)) {
                        continue;
                    }
                    if let Ok(site) = sim.econ_site(self.seat, probe) {
                        let d = site.dist2(from);
                        if best.is_none_or(|(bd, _)| d < bd) {
                            best = Some((d, site));
                        }
                    }
                }
            }
            along += 10 * UNIT;
        }
        best.map(|(_, p)| p)
    }

    // ---- Fighting ----------------------------------------------------------

    /// Units of mine that are not away on a sortie.
    fn home_army<'a>(&self, sim: &'a Sim) -> Vec<&'a Entity> {
        let away = |id: EntityId| {
            [&self.attack, &self.raid]
                .into_iter()
                .flatten()
                .any(|s| s.units.contains(&id))
        };
        self.army(sim).filter(|e| !away(e.id)).collect()
    }

    /// Everything at home goes for the invaders, and a serious invasion
    /// brings the attack home too.
    fn defend(&mut self, sim: &Sim, hq: Pos, invaders: &[&Entity], out: &mut Vec<Command>) {
        let invasion: i64 = invaders.iter().map(|e| e.hp as i64).sum();
        let home: Vec<&Entity> = self.home_army(sim);
        let home_hp: i64 = home.iter().map(|e| e.hp as i64).sum();
        let mut defenders: Vec<EntityId> = home.iter().map(|e| e.id).collect();
        if invasion * 5 >= home_hp * 3
            && let Some(attack) = self.attack.take()
        {
            defenders.extend(alive(sim, &attack.units));
        }
        // Meet them at the one closest to home.
        let Some(front) = invaders.iter().min_by_key(|e| (e.pos.dist2(hq), e.id)) else {
            return;
        };
        if !defenders.is_empty() {
            out.push(Command::AttackMove {
                units: defenders,
                to: front.pos,
            });
        }
    }

    /// Follow up a running attack: turn back if beaten, move on to the next
    /// building if this one fell.
    fn keep_attacking(&mut self, sim: &Sim, out: &mut Vec<Command>) {
        let Some(attack) = self.attack.as_mut() else {
            return;
        };
        attack.units = alive(sim, &attack.units);
        let strength = hp(sim, &attack.units);
        let rally = sim.players[self.seat as usize].rally;
        if attack.units.is_empty() || !sim.is_alive(attack.target) {
            self.attack = None;
            return;
        }
        if (strength as f32) < attack.start as f32 * self.personality.retreat_at {
            out.push(Command::Move {
                units: attack.units.clone(),
                to: rally,
            });
            self.attack = None;
            return;
        }
        let idle = attack
            .units
            .iter()
            .filter_map(|id| sim.get(*id))
            .all(|e| e.order == Order::Idle && e.target.is_none());
        if idle && let Some(next) = objective(sim, attack.target, centroid(sim, &attack.units)) {
            out.push(Command::AttackMove {
                units: attack.units.clone(),
                to: next,
            });
        }
    }

    /// Launch a wave when the home army is big enough, or sooner to pay back
    /// an invader.
    fn maybe_attack(&mut self, sim: &Sim, hq: Pos, out: &mut Vec<Command>) {
        let p = self.personality;
        let raiders_home = |e: &&Entity| e.kind == Kind::Raider && p.mix[2] > p.mix[0];
        let home: Vec<&Entity> = self
            .home_army(sim)
            .into_iter()
            .filter(|e| !raiders_home(e))
            .collect();
        let target = self.prey(sim);
        let wave = (p.first_wave + p.wave_growth * self.waves as usize).min(MAX_WAVE);
        let revenge = p.vengeful && self.grudge.is_some_and(|(s, _)| Some(s) == target);
        let enough = home.len() >= p.first_wave;
        if !enough {
            self.waiting = None;
        } else if home.len() < wave {
            self.waiting.get_or_insert(sim.tick);
        }
        let tired = self
            .waiting
            .is_some_and(|since| sim.tick >= since + PATIENCE);
        let ready = if revenge {
            home.len() * 3 >= wave * 2
        } else {
            home.len() >= wave || (enough && tired)
        };

        // Mass on the side facing the prey meanwhile.
        if let Some(prey) = target.and_then(|s| sim.get(sim.players[s as usize].hq)) {
            let toward = if hq.offset_to(prey.pos).0 >= 0 { 1 } else { -1 };
            let rally = hq.displaced(toward * 60 * UNIT, 0);
            if sim.players[self.seat as usize].rally != rally {
                out.push(Command::SetRally(rally));
            }
        }
        if self.attack.is_some() || !ready {
            return;
        }
        let Some(target) = target else {
            return;
        };
        let Some(first) = objective(sim, target, hq) else {
            return;
        };
        // Leave a garrison: the weakest stay.
        let mut sorted = home;
        sorted.sort_by_key(|e| (std::cmp::Reverse(e.hp), e.id));
        let go = ((sorted.len() as f32) * (1.0 - p.garrison)).ceil() as usize;
        let units: Vec<EntityId> = sorted.iter().take(go.max(1)).map(|e| e.id).collect();
        self.waves += 1;
        self.waiting = None;
        self.attack = Some(Sortie {
            start: hp(sim, &units),
            units: units.clone(),
            target,
        });
        out.push(Command::AttackMove { units, to: first });
    }

    /// Who to attack: whoever I hold a grudge against if I'm vengeful,
    /// otherwise the weaker neighbour by army.
    fn prey(&self, sim: &Sim) -> Option<Seat> {
        let neighbours = self.neighbours(sim);
        if self.personality.vengeful
            && let Some((s, _)) = self.grudge
            && neighbours.contains(&s)
        {
            return Some(s);
        }
        neighbours.into_iter().min_by_key(|s| {
            let army: i64 = sim
                .entities
                .iter()
                .filter(|e| e.owner == *s && is_fighter(e.kind))
                .map(|e| e.hp as i64)
                .sum();
            (army, *s)
        })
    }

    /// Raiders at home go out in a pack for the nearest enemy economy; a
    /// hurt raid flees home.
    fn raid(&mut self, sim: &Sim, hq: Pos, out: &mut Vec<Command>) {
        let rally = sim.players[self.seat as usize].rally;
        if let Some(raid) = self.raid.as_mut() {
            raid.units = alive(sim, &raid.units);
            let strength = hp(sim, &raid.units);
            if raid.units.is_empty() {
                self.raid = None;
            } else if (strength as f32) < raid.start as f32 * self.personality.retreat_at {
                out.push(Command::Move {
                    units: raid.units.clone(),
                    to: rally,
                });
                self.raid = None;
            } else {
                let idle = raid
                    .units
                    .iter()
                    .filter_map(|id| sim.get(*id))
                    .all(|e| e.order == Order::Idle && e.target.is_none());
                if idle {
                    match economy_target(sim, self.seat, centroid(sim, &raid.units)) {
                        Some((_, at)) => out.push(Command::AttackMove {
                            units: raid.units.clone(),
                            to: at,
                        }),
                        None => self.raid = None,
                    }
                }
            }
            return;
        }
        let raiders: Vec<EntityId> = self
            .home_army(sim)
            .iter()
            .filter(|e| e.kind == Kind::Raider && e.target.is_none())
            .map(|e| e.id)
            .collect();
        if raiders.len() < self.personality.raid_pack {
            return;
        }
        if let Some((target, at)) = economy_target(sim, self.seat, hq) {
            self.raid = Some(Sortie {
                start: hp(sim, &raiders),
                units: raiders.clone(),
                target,
            });
            out.push(Command::AttackMove {
                units: raiders,
                to: at,
            });
        }
    }

    /// Defenders who chased invaders out, with no one left to fight, come
    /// back to the rally point.
    fn send_home(&self, sim: &Sim, out: &mut Vec<Command>) {
        let rally = sim.players[self.seat as usize].rally;
        let strays: Vec<EntityId> = self
            .home_army(sim)
            .iter()
            .filter(|e| e.target.is_none() && e.order == Order::Idle)
            .filter(|e| !self.at_home(sim, e.pos, CHASE))
            .map(|e| e.id)
            .collect();
        if !strays.is_empty() {
            out.push(Command::Move {
                units: strays,
                to: rally,
            });
        }
    }
}

/// The stretch of ring a seat holds: from its HQ halfway to the nearest
/// living HQ each way. The rules know nothing of it, and neither sectors nor
/// land bind anyone: sectors are only where players start, and when one
/// falls, the ground between its neighbours is split afresh between them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Land {
    /// Where the HQ stands.
    pub hq: Pos,
    /// How far the land reaches counter-clockwise and clockwise of the HQ, in
    /// angle units (a full turn is 2³²).
    pub ccw: i64,
    pub cw: i64,
}

impl Default for Land {
    fn default() -> Self {
        Land {
            hq: Pos::new(Angle(0), R_MID),
            ccw: 0,
            cw: 0,
        }
    }
}

impl Land {
    /// `seat`'s land, or `None` once its HQ is gone.
    pub fn of(sim: &Sim, seat: Seat) -> Option<Land> {
        const TURN: i64 = 1 << 32;
        let hq = sim.get(sim.players.get(seat as usize)?.hq)?.pos;
        let me = hq.a.0 as i64;
        let others: Vec<i64> = (0..sim.seats() as Seat)
            .filter(|s| *s != seat && sim.is_alive(*s))
            .filter_map(|s| sim.get(sim.players[s as usize].hq))
            .map(|e| e.pos.a.0 as i64)
            .collect();
        // Alone, it is the whole ring.
        let nearest =
            |gap: &dyn Fn(i64) -> i64| others.iter().map(|a| gap(*a)).min().unwrap_or(TURN);
        let ccw = nearest(&|a| (a - me).rem_euclid(TURN));
        let cw = nearest(&|a| (me - a).rem_euclid(TURN));
        Some(Land {
            hq,
            ccw: ccw / 2,
            cw: cw / 2,
        })
    }

    /// How wide it is, as an angle.
    pub fn width(&self) -> i64 {
        self.ccw + self.cw
    }

    /// Is `p` on it, or no further than `margin` beyond its edges?
    pub fn holds(&self, p: Pos, margin: i64) -> bool {
        let slack = arc_to_angle(margin, R_INNER);
        let d = p.a.delta(self.hq.a);
        if d >= 0 {
            d <= self.ccw + slack
        } else {
            -d <= self.cw + slack
        }
    }
}

const ARMY: [Kind; 3] = [Kind::Brawler, Kind::Skirmisher, Kind::Raider];

fn army_index(kind: Kind) -> Option<usize> {
    ARMY.iter().position(|k| *k == kind)
}

fn is_fighter(kind: Kind) -> bool {
    army_index(kind).is_some()
}

/// What beats `kind`: brawlers beat raiders, raiders beat skirmishers,
/// skirmishers beat brawlers.
fn counter(kind: Kind) -> Kind {
    match kind {
        Kind::Brawler => Kind::Skirmisher,
        Kind::Skirmisher => Kind::Raider,
        _ => Kind::Brawler,
    }
}

fn alive(sim: &Sim, ids: &[EntityId]) -> Vec<EntityId> {
    ids.iter()
        .copied()
        .filter(|id| sim.get(*id).is_some())
        .collect()
}

fn hp(sim: &Sim, ids: &[EntityId]) -> i64 {
    ids.iter()
        .filter_map(|id| sim.get(*id))
        .map(|e| e.hp as i64)
        .sum()
}

/// Roughly where a group is: the position of its first unit, nudged by the
/// average offset of the rest.
fn centroid(sim: &Sim, ids: &[EntityId]) -> Pos {
    let mut units = ids.iter().filter_map(|id| sim.get(*id));
    let Some(first) = units.next() else {
        return Pos::new(Angle(0), R_MID);
    };
    let (mut t, mut r, mut n) = (0i64, 0i64, 1i64);
    for e in units {
        let (dt, dr) = first.pos.offset_to(e.pos);
        t += dt;
        r += dr;
        n += 1;
    }
    first.pos.displaced(t / n, r / n)
}

/// The next thing to attack of `seat`'s: its building nearest `from`, so an
/// attack takes the border first.
fn objective(sim: &Sim, seat: Seat, from: Pos) -> Option<Pos> {
    sim.entities
        .iter()
        .filter(|e| e.owner == seat && e.kind.is_structure())
        .min_by_key(|e| (e.pos.dist2(from), e.id))
        .map(|e| e.pos)
}

/// The nearest enemy economy (or, failing that, utility) to `from`, and whose
/// it is.
fn economy_target(sim: &Sim, me: Seat, from: Pos) -> Option<(Seat, Pos)> {
    let nearest = |kind: Kind| {
        sim.entities
            .iter()
            .filter(|e| e.owner != me && e.kind == kind)
            .min_by_key(|e| (e.pos.dist2(from), e.id))
            .map(|e| (e.owner, e.pos))
    };
    nearest(Kind::Econ).or_else(|| nearest(Kind::Utility))
}
