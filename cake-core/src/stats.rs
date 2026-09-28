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
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum Kind {
    Hq,
    Econ,
    /// The gun turret: rapid bullets.
    Turret,
    /// Slow plasma balls that splash where they land.
    PlasmaTurret,
    /// Homing missiles at long range.
    MissileTurret,
    Brawler,
    Skirmisher,
    Raider,
    Utility,
}

impl Kind {
    /// What an HQ can produce, in hotkey order.
    pub const UNITS: [Kind; 4] = [Kind::Brawler, Kind::Skirmisher, Kind::Raider, Kind::Utility];

    /// What a utility can build.
    pub const TURRETS: [Kind; 3] = [Kind::Turret, Kind::PlasmaTurret, Kind::MissileTurret];

    pub const ALL: [Kind; 9] = [
        Kind::Hq,
        Kind::Econ,
        Kind::Turret,
        Kind::PlasmaTurret,
        Kind::MissileTurret,
        Kind::Brawler,
        Kind::Skirmisher,
        Kind::Raider,
        Kind::Utility,
    ];

    pub fn is_structure(self) -> bool {
        matches!(self, Kind::Hq | Kind::Econ) || self.is_turret()
    }

    pub fn is_turret(self) -> bool {
        matches!(
            self,
            Kind::Turret | Kind::PlasmaTurret | Kind::MissileTurret
        )
    }

    pub fn is_mobile(self) -> bool {
        !self.is_structure()
    }

    pub fn stats(self) -> &'static Stats {
        match self {
            Kind::Hq => &HQ,
            Kind::Econ => &ECON,
            Kind::Turret => &TURRET,
            Kind::PlasmaTurret => &PLASMA_TURRET,
            Kind::MissileTurret => &MISSILE_TURRET,
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
            Kind::Turret => "Gun turret",
            Kind::PlasmaTurret => "Plasma turret",
            Kind::MissileTurret => "Missile turret",
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
    pub shot: Shot,
}

/// How a weapon's damage gets there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shot {
    /// At once, hand to hand.
    Melee,
    /// A fast projectile that follows its target and always hits.
    Bullet { speed: i64 },
    /// A slow ball that flies to where the target was and bursts there,
    /// hurting everything of the enemy's within `splash`. It can be dodged.
    Plasma { speed: i64, splash: i64 },
    /// Launched sideways, it steers toward its target: `turn` is how much of
    /// the way to the wanted heading it turns each tick, in thousandths.
    Missile { speed: i64, turn: i64 },
}

/// A building's explosion: units within `radius` are thrown back, up to
/// `force` milli-units per tick at the centre, divided by their mass.
#[derive(Debug)]
pub struct Blast {
    pub radius: i64,
    pub force: i64,
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
    /// How hard to throw: blasts are divided by it. Zero for what doesn't
    /// move.
    pub mass: i64,
    /// What happens when it is destroyed, for buildings.
    pub blast: Option<Blast>,
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
        shot: Shot::Melee,
    }),
    cost: 60 * SUPPLY,
    build_ticks: secs(6),
    mass: 6,
    blast: None,
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
        shot: Shot::Bullet {
            speed: per_sec(500 * UNIT),
        },
    }),
    cost: 50 * SUPPLY,
    build_ticks: secs(5),
    mass: 3,
    blast: None,
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
        shot: Shot::Bullet {
            speed: per_sec(420 * UNIT),
        },
    }),
    cost: 45 * SUPPLY,
    build_ticks: secs(4),
    mass: 2,
    blast: None,
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
    mass: 3,
    blast: None,
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
        shot: Shot::Missile {
            speed: per_sec(200 * UNIT),
            turn: 180,
        },
    }),
    cost: 0,
    build_ticks: 0,
    mass: 0,
    blast: Some(Blast {
        radius: 110 * UNIT,
        force: 30 * UNIT,
    }),
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
    mass: 0,
    blast: Some(Blast {
        radius: 70 * UNIT,
        force: 18 * UNIT,
    }),
};

pub const TURRET: Stats = Stats {
    hp: 500,
    speed: 0,
    radius: 10 * UNIT,
    vision: 70 * UNIT,
    radar: 0,
    weapon: Some(Weapon {
        range: 55 * UNIT,
        damage: 5,
        cooldown: 7,
        shot: Shot::Bullet {
            speed: per_sec(600 * UNIT),
        },
    }),
    cost: 75 * SUPPLY,
    build_ticks: secs(8),
    mass: 0,
    blast: Some(Blast {
        radius: 60 * UNIT,
        force: 14 * UNIT,
    }),
};

pub const PLASMA_TURRET: Stats = Stats {
    hp: 450,
    speed: 0,
    radius: 11 * UNIT,
    vision: 90 * UNIT,
    radar: 0,
    weapon: Some(Weapon {
        range: 70 * UNIT,
        damage: 36,
        cooldown: 50,
        shot: Shot::Plasma {
            speed: per_sec(60 * UNIT),
            splash: 28 * UNIT,
        },
    }),
    cost: 100 * SUPPLY,
    build_ticks: secs(10),
    mass: 0,
    blast: Some(Blast {
        radius: 70 * UNIT,
        force: 18 * UNIT,
    }),
};

pub const MISSILE_TURRET: Stats = Stats {
    hp: 400,
    speed: 0,
    radius: 10 * UNIT,
    vision: 110 * UNIT,
    radar: 0,
    weapon: Some(Weapon {
        range: 95 * UNIT,
        damage: 24,
        cooldown: 36,
        shot: Shot::Missile {
            speed: per_sec(220 * UNIT),
            turn: 160,
        },
    }),
    cost: 110 * SUPPLY,
    build_ticks: secs(10),
    mass: 0,
    blast: Some(Blast {
        radius: 60 * UNIT,
        force: 14 * UNIT,
    }),
};

/// Damage multiplier, in percent: the counter triangle (brawler > raider >
/// skirmisher > brawler), and raiders' bonus against economy targets.
pub fn damage_pct(attacker: Kind, target: Kind) -> i32 {
    use Kind::*;
    match (attacker, target) {
        (Brawler, Raider) => 200,
        (Skirmisher, Brawler) => 300,
        (Raider, Skirmisher) => 150,
        (Raider, Utility) => 300,
        (Raider, t) if t.is_structure() => 300,
        _ => 100,
    }
}

// Economy.
pub const STARTING_SUPPLY: i64 = 100 * SUPPLY;
pub const HQ_INCOME: i64 = per_sec(5 * SUPPLY);
pub const ECON_INCOME: i64 = per_sec(2 * SUPPLY);
/// Each finished economy building also makes the HQ build this many percent
/// faster, so that a bigger economy can be spent: one production queue alone
/// can't use much more than the HQ's own income.
pub const ECON_PRODUCTION_PCT: u32 = 15;
pub const QUEUE_MAX: usize = 5;
pub const UNIT_CAP: usize = 60;

/// Economy buildings keep this far from every other economy building (any
/// owner) and from every HQ.
pub const ECON_SPACING: i64 = 150 * UNIT;
/// No structure may be placed this close to an enemy HQ.
pub const ENEMY_HQ_CLEARANCE: i64 = 100 * UNIT;
/// Structures keep this much clear space between their edges.
pub const STRUCTURE_GAP: i64 = 4 * UNIT;

/// A missile that has flown this long without reaching its target bursts
/// where it is.
pub const MISSILE_FUEL: u32 = secs(4);

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
        for kind in Kind::ALL {
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
