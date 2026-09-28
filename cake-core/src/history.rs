//! A match's story in numbers: every player's standing, sampled about once a
//! second, for the charts shown when it ends.
//!
//! It watches the simulation from outside and is no part of its state, so it
//! stays out of the checksum. It is deterministic all the same: peers that
//! agree on the match agree on its history.

use crate::geom::arc_to_angle;
use crate::sim::{Event, Seat, Sim};
use crate::stats::{SUPPLY, TICK_HZ};

/// Samples kept before the oldest are thinned out.
const MAX_SAMPLES: usize = 720;

/// What the charts can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    /// Mobile units on the ring.
    Units,
    /// What the fighting units cost, in supply.
    Army,
    /// Supply per second.
    Income,
    /// The share of the ring in sight, in per-mille.
    Reach,
    /// Supply's worth of units and buildings lost so far.
    Losses,
}

impl Metric {
    pub const ALL: [Metric; 5] = [
        Metric::Units,
        Metric::Army,
        Metric::Income,
        Metric::Reach,
        Metric::Losses,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Metric::Units => "Units",
            Metric::Army => "Army",
            Metric::Income => "Income",
            Metric::Reach => "Reach",
            Metric::Losses => "Losses",
        }
    }

    /// What its numbers mean, in a few words.
    pub fn caption(self) -> &'static str {
        match self {
            Metric::Units => "units on the ring",
            Metric::Army => "supply's worth of army",
            Metric::Income => "supply per second",
            Metric::Reach => "share of the ring in sight",
            Metric::Losses => "supply's worth lost",
        }
    }

    /// `v` as shown: reach as a percentage, the rest as they are.
    pub fn format(self, v: u32) -> String {
        match self {
            Metric::Reach => format!("{}%", (v + 5) / 10),
            _ => v.to_string(),
        }
    }
}

/// One player at one moment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Standing {
    pub units: u32,
    pub army: u32,
    pub income: u32,
    /// Per-mille of the ring.
    pub reach: u32,
    pub losses: u32,
}

impl Standing {
    pub fn get(&self, metric: Metric) -> u32 {
        match metric {
            Metric::Units => self.units,
            Metric::Army => self.army,
            Metric::Income => self.income,
            Metric::Reach => self.reach,
            Metric::Losses => self.losses,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
    pub tick: u32,
    /// By seat.
    pub players: Vec<Standing>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct History {
    /// Ticks between samples. It doubles whenever the record fills up, so a
    /// long match keeps half the detail over twice the time.
    pub every: u32,
    /// Oldest first. The last is the final standing once the match is over.
    pub samples: Vec<Sample>,
    /// When each seat was knocked out, in the order it happened.
    pub eliminated: Vec<(Seat, u32)>,
    /// Supply's worth each seat has lost so far.
    losses: Vec<u32>,
    ended: bool,
}

impl History {
    /// An empty record, with a sample of the start.
    pub fn new(sim: &Sim) -> History {
        let mut history = History {
            every: TICK_HZ,
            samples: Vec::new(),
            eliminated: Vec::new(),
            losses: vec![0; sim.seats()],
            ended: false,
        };
        history.sample(sim);
        history
    }

    /// Take in the tick just stepped: count its losses, and sample it if a
    /// sample is due or the match has just ended.
    pub fn observe(&mut self, sim: &Sim) {
        if self.ended {
            return;
        }
        for event in &sim.events {
            match *event {
                Event::Died { owner, kind, .. } => {
                    self.losses[owner as usize] += (kind.stats().cost / SUPPLY) as u32;
                }
                Event::Eliminated(seat) => self.eliminated.push((seat, sim.tick)),
                _ => {}
            }
        }
        self.ended = sim.outcome.is_some();
        if self.ended || sim.tick.is_multiple_of(self.every) {
            self.sample(sim);
        }
        if self.samples.len() > MAX_SAMPLES {
            self.every *= 2;
            let every = self.every;
            self.samples.retain(|s| s.tick.is_multiple_of(every));
        }
    }

    fn sample(&mut self, sim: &Sim) {
        let players = (0..sim.seats() as Seat)
            .map(|seat| Standing {
                units: sim.unit_count(seat) as u32,
                army: sim
                    .entities
                    .iter()
                    .filter(|e| e.owner == seat && e.kind.is_mobile() && e.armed())
                    .map(|e| (e.stats().cost / SUPPLY) as u32)
                    .sum(),
                income: (sim.income(seat) * TICK_HZ as i64 / SUPPLY) as u32,
                reach: reach(sim, seat),
                losses: self.losses[seat as usize],
            })
            .collect();
        self.samples.push(Sample {
            tick: sim.tick,
            players,
        });
    }

    /// The last tick recorded.
    pub fn end(&self) -> u32 {
        self.samples.last().map_or(0, |s| s.tick)
    }

    /// The highest value anyone reached.
    pub fn peak(&self, metric: Metric) -> u32 {
        self.samples
            .iter()
            .flat_map(|s| s.players.iter().map(|p| p.get(metric)))
            .max()
            .unwrap_or(0)
    }
}

/// The share of the ring, in per-mille, that `seat`'s units and buildings
/// see: the union of the arcs their vision spans.
pub fn reach(sim: &Sim, seat: Seat) -> u32 {
    const TURN: i64 = 1 << 32;
    let mut arcs: Vec<(i64, i64)> = Vec::new();
    for e in sim.entities.iter().filter(|e| e.owner == seat) {
        let half = arc_to_angle(e.stats().vision, e.pos.r).min(TURN / 2);
        let from = e.pos.a.0 as i64 - half;
        let to = e.pos.a.0 as i64 + half;
        // Split an arc across the zero angle in two.
        if from < 0 {
            arcs.push((from + TURN, TURN));
            arcs.push((0, to));
        } else if to > TURN {
            arcs.push((from, TURN));
            arcs.push((0, to - TURN));
        } else {
            arcs.push((from, to));
        }
    }
    arcs.sort_unstable();
    let mut covered = 0;
    let mut reached = 0;
    for (from, to) in arcs {
        let from = from.max(reached);
        if to > from {
            covered += to - from;
            reached = to;
        }
    }
    (covered * 1000 / TURN) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;
    use crate::stats::Kind;

    #[test]
    fn reach_counts_overlap_once_and_grows_with_units() {
        let mut sim = Sim::new(2);
        let start = reach(&sim, 0);
        assert!(start > 0 && start < 500, "a base sees a slice: {start}");
        // A unit made at home stands inside what the base already sees.
        let units = sim.unit_count(0);
        sim.step(&[(0, Command::Produce(Kind::Raider))]);
        for _ in 0..200 {
            sim.step(&[]);
        }
        assert_eq!(sim.unit_count(0), units + 1);
        let with_raider = reach(&sim, 0);
        assert!(with_raider >= start);
        assert!(with_raider < start * 2, "overlap is counted once");
    }

    #[test]
    fn history_samples_each_second_and_thins_out() {
        let mut sim = Sim::new(2);
        let mut history = History::new(&sim);
        for _ in 0..(MAX_SAMPLES as u32 + 10) * TICK_HZ {
            sim.step(&[]);
            history.observe(&sim);
        }
        assert_eq!(history.every, TICK_HZ * 2);
        assert!(history.samples.len() <= MAX_SAMPLES);
        assert!(
            history
                .samples
                .windows(2)
                .all(|w| w[1].tick - w[0].tick == history.every)
        );
        assert_eq!(history.samples[0].tick, 0);
        assert_eq!(history.peak(Metric::Income), 5);
    }
}
