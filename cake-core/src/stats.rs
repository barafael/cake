//! The tuning table. Every gameplay number lives here.
//!
//! Units of measure:
//! - lengths are milli-units ([`UNIT`] = one map unit);
//! - time is ticks, [`TICK_HZ`] per second;
//! - supply is milli-supply, so income per tick stays an integer.

use serde::{Deserialize, Serialize};

use crate::geom::UNIT;

pub const TICK_HZ: u32 = 20;

/// One supply, in milli-supply.
pub const SUPPLY: i64 = 1000;

const fn secs(s: u32) -> u32 {
    s * TICK_HZ
}

/// Per-second amount of milli-units or milli-supply, as a per-tick amount.
const fn per_sec(v: i64) -> i64 {
    v / TICK_HZ as i64
}

/// What an entity is.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize,
)]
pub enum Kind {
    Hq,
    Econ,
    Turret,
    Brawler,
    Skirmisher,
    Raider,
    Utility,
}

impl Kind {
    /// What an HQ can produce, in hotkey order.
    pub const UNITS: [Kind; 4] = [Kind::Brawler, Kind::Skirmisher, Kind::Raider, Kind::Utility];

    pub fn is_structure(self) -> bool {
        matches!(self, Kind::Hq | Kind::Econ | Kind::Turret)
    }

    pub fn is_mobile(self) -> bool {
        !self.is_structure()
    }

    pub fn stats(self) -> &'static Stats {
        match self {
            Kind::Hq => &HQ,
            Kind::Econ => &ECON,
            Kind::Turret => &TURRET,
            Kind::Brawler => &BRAWLER,
            Kind::Skirmisher => &SKIRMISHER,
            Kind::Raider => &RAIDER,
            Kind::Utility => &UTILITY,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Hq => "HQ",
            Kind::Econ => "Economy",
            Kind::Turret => "Turret",
            Kind::Brawler => "Brawler",
            Kind::Skirmisher => "Skirmisher",
            Kind::Raider => "Raider",
            Kind::Utility => "Utility",
        }
    }
}

#[derive(Debug)]
pub struct Weapon {
    /// Reach from this entity's centre to the target's edge.
    pub range: i64,
    pub damage: i32,
    pub cooldown: u32,
}

#[derive(Debug)]
pub struct Stats {
    pub hp: i32,
    /// Milli-units per tick.
    pub speed: i64,
    /// Body radius, for separation and for melee reach.
    pub radius: i64,
    pub vision: i64,
    /// Radar range: enemies inside it but outside vision show as blips. Zero
    /// for no radar.
    pub radar: i64,
    pub weapon: Option<Weapon>,
    /// Milli-supply.
    pub cost: i64,
    pub build_ticks: u32,
}

pub const BRAWLER: Stats = Stats {
    hp: 240,
    speed: per_sec(28 * UNIT),
    radius: 7 * UNIT,
    vision: 70 * UNIT,
    radar: 0,
    weapon: Some(Weapon {
        range: 10 * UNIT,
        damage: 7,
        cooldown: 10,
    }),
    cost: 60 * SUPPLY,
    build_ticks: secs(6),
};

pub const SKIRMISHER: Stats = Stats {
    hp: 90,
    speed: per_sec(32 * UNIT),
    radius: 6 * UNIT,
    vision: 90 * UNIT,
    radar: 0,
    weapon: Some(Weapon {
        range: 65 * UNIT,
        damage: 10,
        cooldown: 20,
    }),
    cost: 50 * SUPPLY,
    build_ticks: secs(5),
};

pub const RAIDER: Stats = Stats {
    hp: 70,
    speed: per_sec(65 * UNIT),
    radius: 5 * UNIT,
    vision: 100 * UNIT,
    radar: 0,
    weapon: Some(Weapon {
        range: 18 * UNIT,
        damage: 5,
        cooldown: 10,
    }),
    cost: 45 * SUPPLY,
    build_ticks: secs(4),
};

pub const UTILITY: Stats = Stats {
    hp: 80,
    speed: per_sec(30 * UNIT),
    radius: 6 * UNIT,
    vision: 80 * UNIT,
    radar: 220 * UNIT,
    weapon: None,
    cost: 50 * SUPPLY,
    build_ticks: secs(6),
};

pub const HQ: Stats = Stats {
    hp: 2000,
    speed: 0,
    radius: 22 * UNIT,
    vision: 120 * UNIT,
    radar: 0,
    weapon: Some(Weapon {
        range: 60 * UNIT,
        damage: 10,
        cooldown: 20,
    }),
    cost: 0,
    build_ticks: 0,
};

pub const ECON: Stats = Stats {
    hp: 400,
    speed: 0,
    radius: 12 * UNIT,
    vision: 50 * UNIT,
    radar: 0,
    weapon: None,
    cost: 0,
    build_ticks: secs(5),
};

pub const TURRET: Stats = Stats {
    hp: 500,
    speed: 0,
    radius: 10 * UNIT,
    vision: 70 * UNIT,
    radar: 0,
    weapon: Some(Weapon {
        range: 55 * UNIT,
        damage: 15,
        cooldown: 20,
    }),
    cost: 75 * SUPPLY,
    build_ticks: secs(8),
};

/// Damage multiplier, in percent: the counter triangle (brawler > raider >
/// skirmisher > brawler), and raiders' bonus against economy targets.
pub fn damage_pct(attacker: Kind, target: Kind) -> i32 {
    use Kind::*;
    match (attacker, target) {
        (Brawler, Raider) => 200,
        (Skirmisher, Brawler) => 300,
        (Raider, Skirmisher) => 150,
        (Raider, Hq | Econ | Turret | Utility) => 300,
        _ => 100,
    }
}

// Economy.
pub const STARTING_SUPPLY: i64 = 100 * SUPPLY;
pub const HQ_INCOME: i64 = per_sec(5 * SUPPLY);
pub const ECON_INCOME: i64 = per_sec(2 * SUPPLY);
pub const QUEUE_MAX: usize = 5;
pub const UNIT_CAP: usize = 60;

/// Economy buildings keep this far from every other economy building (any
/// owner) and from every HQ.
pub const ECON_SPACING: i64 = 150 * UNIT;
/// No structure may be placed this close to an enemy HQ.
pub const ENEMY_HQ_CLEARANCE: i64 = 100 * UNIT;
/// Structures keep this much clear space between their edges.
pub const STRUCTURE_GAP: i64 = 4 * UNIT;

// Utility abilities.
/// How close a utility must stand to lay a turret down.
pub const BUILD_RANGE: i64 = 25 * UNIT;
pub const REPAIR_RANGE: i64 = 30 * UNIT;
/// Repair: this much HP every [`REPAIR_INTERVAL`] ticks (15 HP/s).
pub const REPAIR_AMOUNT: i32 = 3;
pub const REPAIR_INTERVAL: u32 = 4;
/// A fresh turret frame starts at this share of its HP, in percent.
pub const FRAME_HP_PCT: i32 = 10;

/// Idle and attack-moving units engage enemies this far away: their vision.
pub fn acquire_range(kind: Kind) -> i64 {
    kind.stats().vision
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Weapons must reach no further than their owner sees, or units would
    /// hold targets they cannot see.
    #[test]
    fn every_weapon_is_inside_its_vision() {
        for kind in [
            Kind::Hq,
            Kind::Econ,
            Kind::Turret,
            Kind::Brawler,
            Kind::Skirmisher,
            Kind::Raider,
            Kind::Utility,
        ] {
            let s = kind.stats();
            if let Some(w) = &s.weapon {
                assert!(w.range < s.vision, "{kind:?}");
            }
        }
    }

    /// The design's turret arithmetic: at mid-band a turret reaches both
    /// edges, but only just.
    #[test]
    fn a_turret_at_mid_band_just_spans_the_band() {
        let reach = TURRET.weapon.as_ref().unwrap().range;
        assert!(reach > 50 * UNIT && reach < 60 * UNIT);
    }
}
