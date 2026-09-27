//! Scripted bots.
//!
//! A bot is just another seat: it reads the simulation and returns
//! [`Command`]s, which the host sequences like anyone else's. It has no
//! privileged path into the game. It does read the whole state, fog included,
//! which is fine for a sparring partner.
//!
//! The script is a sketch of competent play:
//!
//! - keep the HQ busy with a mixed army;
//! - turn utilities into economy buildings at the nearest free site, and keep
//!   one back to build a turret on each front and repair;
//! - answer threats to the HQ with everything;
//! - send raiders after the nearest enemy economy;
//! - and when the army is big enough, push the weaker neighbour.

use cake_core::geom::{R_INNER, R_OUTER, UNIT, arc_len};
use cake_core::stats::{self, Kind, TICK_HZ};
use cake_core::{Command, Entity, EntityId, Order, Pos, Seat, Sim};

/// A bot thinks once a second.
const THINK_EVERY: u32 = TICK_HZ;
/// Enemies this close to the HQ are a threat to answer.
const THREAT_RADIUS: i64 = 200 * UNIT;
/// Raiders go out in packs at least this big.
const RAID_PACK: usize = 3;
/// Utilities the bot keeps rather than deploying.
const KEEP_UTILITIES: usize = 1;
const MAX_TURRETS: usize = 2;

#[derive(Clone, Debug)]
pub struct Bot {
    seat: Seat,
    /// Attacks launched so far; each wave waits for a bigger army.
    waves: u32,
    /// Which side the next turret goes on.
    turret_side: i64,
}

impl Bot {
    pub fn new(seat: Seat) -> Bot {
        Bot {
            seat,
            waves: 0,
            turret_side: 1,
        }
    }

    pub fn seat(&self) -> Seat {
        self.seat
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
        let Some(hq) = sim.get(sim.players[seat as usize].hq) else {
            return Vec::new();
        };
        let hq = hq.pos;
        let mut out = Vec::new();
        self.produce(sim, &mut out);
        self.utilities(sim, hq, &mut out);
        self.army(sim, hq, &mut out);
        out
    }

    fn mine<'a>(&self, sim: &'a Sim) -> impl Iterator<Item = &'a Entity> {
        let seat = self.seat;
        sim.entities.iter().filter(move |e| e.owner == seat)
    }

    fn count(&self, sim: &Sim, kind: Kind) -> usize {
        let queued = sim.players[self.seat as usize]
            .queue
            .iter()
            .filter(|k| **k == kind)
            .count();
        self.mine(sim).filter(|e| e.kind == kind).count() + queued
    }

    /// Once the economy is started, fortify: a turret on each front.
    fn wants_turret(&self, sim: &Sim) -> bool {
        self.count(sim, Kind::Econ) >= 2
            && self.count(sim, Kind::Turret) < MAX_TURRETS
            && self.mine(sim).any(|e| e.kind == Kind::Utility)
    }

    fn produce(&self, sim: &Sim, out: &mut Vec<Command>) {
        let p = &sim.players[self.seat as usize];
        if p.queue.len() >= 2 || sim.unit_count(self.seat) >= stats::UNIT_CAP {
            return;
        }
        let econ = self.count(sim, Kind::Econ);
        let wanted_utilities = KEEP_UTILITIES + usize::from(econ < 4);
        let kind = if self.count(sim, Kind::Utility) < wanted_utilities {
            Kind::Utility
        } else {
            // The army kind furthest below its share: brawlers, skirmishers
            // and raiders in 3:3:2.
            [(Kind::Brawler, 3), (Kind::Skirmisher, 3), (Kind::Raider, 2)]
                .into_iter()
                .min_by_key(|(k, w)| (self.count(sim, *k) * 6 / w, *k))
                .map(|(k, _)| k)
                .expect("three kinds")
        };
        // Saving for a turret: don't spend below its price.
        let reserve = if self.wants_turret(sim) {
            stats::TURRET.cost
        } else {
            0
        };
        if p.supply >= kind.stats().cost + reserve {
            out.push(Command::Produce(kind));
        }
    }

    fn utilities(&mut self, sim: &Sim, hq: Pos, out: &mut Vec<Command>) {
        let idle: Vec<&Entity> = self
            .mine(sim)
            .filter(|e| e.kind == Kind::Utility && e.order == Order::Idle)
            .collect();
        let total = self.mine(sim).filter(|e| e.kind == Kind::Utility).count();
        let mut spare = total.saturating_sub(KEEP_UTILITIES);
        let supply = sim.players[self.seat as usize].supply;

        for u in idle {
            if spare > 0
                && let Some(site) = self.econ_site(sim, hq, u.pos)
            {
                out.push(Command::Deploy { unit: u.id, at: site });
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
            if self.wants_turret(sim) && supply >= stats::TURRET.cost {
                let at = hq.displaced(self.turret_side * 90 * UNIT, 0);
                self.turret_side = -self.turret_side;
                out.push(Command::Build { unit: u.id, at });
            }
        }
    }

    /// The free economy site nearest `from`, searching outward from the HQ
    /// along the band. Sites another of my utilities is already heading for
    /// are skipped.
    fn econ_site(&self, sim: &Sim, hq: Pos, from: Pos) -> Option<Pos> {
        let claimed: Vec<Pos> = self
            .mine(sim)
            .filter_map(|e| match e.order {
                Order::Deploy(at) => Some(at),
                _ => None,
            })
            .collect();
        let half_sector = arc_len((1i64 << 32) / sim.seats() as i64 / 2, R_INNER);
        let mut best: Option<(i64, Pos)> = None;
        let mut along = 150 * UNIT;
        while along <= half_sector + 60 * UNIT {
            for side in [-1, 1] {
                for r in [R_INNER + 25 * UNIT, (R_INNER + R_OUTER) / 2, R_OUTER - 25 * UNIT] {
                    let probe = Pos::new(hq.a, r).displaced(side * along, 0);
                    if claimed
                        .iter()
                        .any(|c| c.within(probe, stats::ECON_SPACING))
                    {
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

    fn army(&mut self, sim: &Sim, hq: Pos, out: &mut Vec<Command>) {
        let seat = self.seat;
        let army: Vec<&Entity> = self
            .mine(sim)
            .filter(|e| e.kind.is_mobile() && e.kind != Kind::Utility)
            .collect();
        if army.is_empty() {
            return;
        }

        // Defend first.
        if let Some(threat) = sim
            .entities
            .iter()
            .filter(|e| e.owner != seat && e.seen_by_seat(seat))
            .filter(|e| e.pos.within(hq, THREAT_RADIUS))
            .min_by_key(|e| (e.pos.dist2(hq), e.id))
        {
            let units: Vec<EntityId> = army
                .iter()
                .filter(|e| e.target.is_none())
                .map(|e| e.id)
                .collect();
            if !units.is_empty() {
                out.push(Command::AttackMove {
                    units,
                    to: threat.pos,
                });
            }
            return;
        }

        let idle: Vec<&Entity> = army
            .iter()
            .copied()
            .filter(|e| e.order == Order::Idle && e.target.is_none())
            .collect();

        // Raiders hunt economy.
        let raiders: Vec<EntityId> = idle
            .iter()
            .filter(|e| e.kind == Kind::Raider)
            .map(|e| e.id)
            .collect();
        if raiders.len() >= RAID_PACK
            && let Some(econ) = sim
                .entities
                .iter()
                .filter(|e| e.owner != seat && e.kind == Kind::Econ)
                .min_by_key(|e| (e.pos.dist2(hq), e.id))
        {
            out.push(Command::AttackMove {
                units: raiders.clone(),
                to: econ.pos,
            });
        }

        // The main army pushes the weaker neighbour once it is big enough.
        let main: Vec<EntityId> = idle
            .iter()
            .filter(|e| e.kind != Kind::Raider || raiders.len() < RAID_PACK)
            .map(|e| e.id)
            .collect();
        let wave = 6 + 3 * self.waves as usize;
        let Some(prey) = self.weaker_neighbour(sim) else {
            return;
        };
        let prey_hq = sim
            .get(sim.players[prey as usize].hq)
            .map(|e| e.pos)
            .expect("a live seat has an HQ");
        if main.len() >= wave {
            self.waves += 1;
            out.push(Command::AttackMove {
                units: main,
                to: prey_hq,
            });
        } else {
            // Mass on the side facing the prey.
            let toward = if hq.offset_to(prey_hq).0 >= 0 { 1 } else { -1 };
            let rally = hq.displaced(toward * 60 * UNIT, 0);
            if sim.players[seat as usize].rally != rally {
                out.push(Command::SetRally(rally));
            }
        }
    }

    /// The alive neighbour, either way round the ring, with less standing HP.
    fn weaker_neighbour(&self, sim: &Sim) -> Option<Seat> {
        let n = sim.seats() as i64;
        let find = |dir: i64| {
            (1..n)
                .map(|k| (self.seat as i64 + dir * k).rem_euclid(n) as Seat)
                .find(|s| sim.is_alive(*s))
        };
        let strength = |s: Seat| -> i64 {
            sim.entities
                .iter()
                .filter(|e| e.owner == s)
                .map(|e| e.hp as i64)
                .sum()
        };
        let a = find(1)?;
        let b = find(-1)?;
        Some(if strength(a) <= strength(b) { a } else { b })
    }
}
