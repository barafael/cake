//! What a seat may see. Fog of war is presentation: every peer holds the whole
//! state, and these filters decide what its screen shows.

use crate::geom::Pos;
use crate::sim::{Entity, Seat, Sim};
use crate::stats::Kind;

/// Does `viewer` see `e`? `None` is a spectator, who sees everything.
pub fn visible(e: &Entity, viewer: Option<Seat>) -> bool {
    viewer.is_none_or(|s| e.seen_by_seat(s))
}

/// Positions of enemies that `seat` does not see but its utilities' radar
/// picks up: a blip says *something* is there, not what.
pub fn radar_blips(sim: &Sim, seat: Seat) -> Vec<Pos> {
    let radars: Vec<&Entity> = sim
        .entities
        .iter()
        .filter(|e| e.owner == seat && e.kind == Kind::Utility)
        .collect();
    if radars.is_empty() {
        return Vec::new();
    }
    sim.entities
        .iter()
        .filter(|e| e.owner != seat && !e.seen_by_seat(seat))
        .filter(|e| radars.iter().any(|r| r.pos.within(e.pos, r.stats().radar)))
        .map(|e| e.pos)
        .collect()
}
