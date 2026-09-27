//! Cake's rules, with no engine attached.
//!
//! The ring is an annulus (inner radius 400, outer 500) shared by up to eight
//! players, each with an HQ at the centre of an equal sector. [`sim::Sim`] is
//! the whole game state and [`sim::Sim::step`] advances it one tick; the only
//! way to affect it is a [`command::Command`]. The simulation is deterministic,
//! so networked peers stay in step by exchanging commands alone.

pub mod command;
pub mod geom;
pub mod hash;
pub mod sim;
pub mod stats;
pub mod vision;

pub use command::{Command, PlaceError};
pub use geom::{Angle, Pos, UNIT};
pub use sim::{Entity, EntityId, Event, Order, Outcome, Player, Seat, Sim};
pub use stats::Kind;
