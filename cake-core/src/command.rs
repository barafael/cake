//! What a player can ask of the simulation.
//!
//! Commands are intents, not effects. The host puts them in order, and every
//! peer applies them inside [`crate::sim::Sim::step`], which checks each one
//! against the state at that tick. A command that no longer makes sense is
//! ignored the same way on every peer, whether it is late, stale, or forged:
//! the unit is dead, it isn't yours, you can't afford it, or the site is taken.

use serde::{Deserialize, Serialize};

use crate::geom::Pos;
use crate::sim::EntityId;
use crate::stats::Kind;

/// Longest unit list a single command may carry. Anything beyond is dropped.
pub const MAX_UNITS_PER_COMMAND: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    /// Queue a unit at the HQ. Supply is paid on queueing.
    Produce(Kind),
    /// Drop the last queued unit and refund it.
    CancelProduce,
    /// Where new units walk after they spawn.
    SetRally(Pos),
    /// Walk there, ignoring enemies.
    Move { units: Vec<EntityId>, to: Pos },
    /// Walk there, fighting anything met on the way.
    AttackMove { units: Vec<EntityId>, to: Pos },
    /// Chase and attack one target while it stays visible.
    Attack {
        units: Vec<EntityId>,
        target: EntityId,
    },
    Stop { units: Vec<EntityId> },
    /// A utility walks to `at` and lays down a turret, which it then builds.
    Build { unit: EntityId, at: Pos },
    /// A utility walks to `at` and becomes an economy building.
    Deploy { unit: EntityId, at: Pos },
    /// Utilities repair a friendly entity, or finish building it.
    Repair {
        units: Vec<EntityId>,
        target: EntityId,
    },
}

/// Why a structure cannot go where it was asked to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceError {
    /// Another economy building, or an HQ, is closer than the spacing rule.
    EconSpacing,
    /// Too close to an enemy HQ.
    EnemyHq,
    /// Overlaps an existing structure.
    Overlaps,
}

impl core::fmt::Display for PlaceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            PlaceError::EconSpacing => "too close to another economy building or an HQ",
            PlaceError::EnemyHq => "too close to an enemy HQ",
            PlaceError::Overlaps => "overlaps a structure",
        })
    }
}
