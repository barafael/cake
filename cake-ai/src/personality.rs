//! Bot temperaments. Each bot plays one, so a table of bots is a table of
//! characters: some turtle, some raid, some throw waves at their neighbours.

use cake_core::{Kind, Seat};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Personality {
    pub name: &'static str,
    /// Brawlers, skirmishers and raiders, as weights.
    pub mix: [u32; 3],
    /// Army units at home before the first attack; each attack waits for
    /// `wave_growth` more.
    pub first_wave: usize,
    pub wave_growth: usize,
    /// The share of the home army left behind when an attack sets out.
    pub garrison: f32,
    /// An attack or raid turns back once its strength falls below this share
    /// of what it set out with.
    pub retreat_at: f32,
    /// Raiders go out in packs this big.
    pub raid_pack: usize,
    pub turrets: usize,
    /// Which turrets to build, in turn.
    pub defences: &'static [Kind],
    pub econ: usize,
    /// Answers an invasion with a counterattack on the invader, sooner than
    /// its next wave would go.
    pub vengeful: bool,
}

pub const WARLORD: Personality = Personality {
    name: "Warlord",
    mix: [5, 3, 1],
    first_wave: 6,
    wave_growth: 2,
    garrison: 0.1,
    retreat_at: 0.25,
    raid_pack: 5,
    turrets: 1,
    defences: &[Kind::Turret],
    econ: 3,
    vengeful: true,
};

pub const RAIDER: Personality = Personality {
    name: "Raider",
    mix: [1, 2, 5],
    first_wave: 8,
    wave_growth: 3,
    garrison: 0.2,
    retreat_at: 0.45,
    raid_pack: 3,
    turrets: 1,
    defences: &[Kind::MissileTurret],
    econ: 3,
    vengeful: false,
};

pub const TURTLE: Personality = Personality {
    name: "Turtle",
    mix: [3, 4, 1],
    first_wave: 14,
    wave_growth: 4,
    garrison: 0.5,
    retreat_at: 0.5,
    raid_pack: 6,
    turrets: 4,
    defences: &[Kind::PlasmaTurret, Kind::MissileTurret, Kind::Turret],
    econ: 4,
    vengeful: true,
};

pub const BALANCED: Personality = Personality {
    name: "Balanced",
    mix: [3, 3, 2],
    first_wave: 9,
    wave_growth: 3,
    garrison: 0.3,
    retreat_at: 0.35,
    raid_pack: 4,
    turrets: 2,
    defences: &[Kind::Turret, Kind::PlasmaTurret],
    econ: 4,
    vengeful: false,
};

pub const ALL: [Personality; 4] = [WARLORD, TURTLE, RAIDER, BALANCED];

/// The temperament called `name` (any case), if there is one.
pub fn by_name(name: &str) -> Option<Personality> {
    ALL.into_iter()
        .find(|p| p.name.eq_ignore_ascii_case(name.trim()))
}

/// The temperament of the bot in `seat`. Fixed by the seat, so a match
/// replays the same, and spread so neighbours differ.
pub fn for_seat(seat: Seat) -> Personality {
    ALL[seat as usize % ALL.len()]
}
