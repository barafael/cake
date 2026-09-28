//! The lobby's ring: who would sit where, animated.
//!
//! Each player owns a slice of the ring in proportion to a weight that eases
//! from 0 to 1 when they arrive and back to 0 when they leave, so a new player
//! opens a sector that pushes the others aside, and a departing one closes
//! theirs. Names ride along their sectors and fade with them.

use std::f32::consts::{PI, TAU};

use bevy::prelude::*;
use cake_net::NetState;

use crate::arctext::ArcText;
use crate::lobby::{Lobby, display_name, my_key};
use crate::render::{Sector, draw_ring};
use crate::{AppState, palette};

/// Seconds for a sector to open or close.
const GROW_SECS: f32 = 0.45;

#[derive(Resource, Default)]
pub struct LobbyRing {
    slots: Vec<Slot>,
}

struct Slot {
    key: String,
    name: String,
    colour: usize,
    weight: f32,
    present: bool,
    label: Option<Entity>,
}

pub fn plugin(app: &mut App) {
    app.init_resource::<LobbyRing>()
        .add_systems(OnExit(AppState::Lobby), |mut ring: ResMut<LobbyRing>| {
            ring.slots.clear();
        })
        .add_systems(
            Update,
            (sync, animate, draw).chain().run_if(in_state(AppState::Lobby)),
        );
}

/// Smooth in and out.
fn ease(w: f32) -> f32 {
    let w = w.clamp(0.0, 1.0);
    w * w * (3.0 - 2.0 * w)
}

/// Match the slots to the roster: arrivals get a slot (at weight 0) in
/// roster order, departures are marked to close. Only when the roster (or
/// my identity in it) changed, or the ring is empty after a match.
fn sync(lobby: Res<Lobby>, net: Res<NetState>, mut ring: ResMut<LobbyRing>) {
    if !lobby.is_changed() && !net.is_changed() && !ring.slots.is_empty() {
        return;
    }
    let me = my_key(&net);
    let roster: Vec<(String, String)> = lobby
        .players()
        .map(|m| {
            let key = m.peer.clone().unwrap_or_else(|| format!("bot:{}", m.name));
            (key, display_name(m, &me))
        })
        .collect();
    for slot in &mut ring.slots {
        slot.present = roster.iter().any(|(k, _)| *k == slot.key);
    }
    for (i, (key, name)) in roster.iter().enumerate() {
        if let Some(slot) = ring.slots.iter_mut().find(|s| s.key == *key) {
            slot.name.clone_from(name);
            continue;
        }
        // After the slot of whoever precedes it in the roster.
        let at = roster[..i]
            .iter()
            .rev()
            .find_map(|(k, _)| ring.slots.iter().position(|s| s.key == *k))
            .map_or(0, |p| p + 1);
        let colour = (0..palette::SEAT_COUNT)
            .find(|c| !ring.slots.iter().any(|s| s.colour == *c))
            .unwrap_or(ring.slots.len() % palette::SEAT_COUNT);
        ring.slots.insert(
            at,
            Slot {
                key: key.clone(),
                name: name.clone(),
                colour,
                weight: 0.0,
                present: true,
                label: None,
            },
        );
    }
}

/// Where each slot's sector runs, in world radians: `(from, to)`.
fn sectors(ring: &LobbyRing) -> Vec<(f32, f32)> {
    let total: f32 = ring.slots.iter().map(|s| ease(s.weight)).sum();
    if total <= f32::EPSILON {
        return vec![(0.0, 0.0); ring.slots.len()];
    }
    // The first sector is centred on angle 0, as seat 0 is in a match.
    let first = ease(ring.slots.first().map_or(0.0, |s| s.weight));
    let mut at = -first / total * PI;
    ring.slots
        .iter()
        .map(|s| {
            let span = ease(s.weight) / total * TAU;
            let sector = (at, at + span);
            at += span;
            sector
        })
        .collect()
}

fn animate(
    mut commands: Commands,
    time: Res<Time>,
    mut ring: ResMut<LobbyRing>,
    mut labels: Query<&mut ArcText>,
) {
    let step = time.delta_secs() / GROW_SECS;
    for slot in &mut ring.slots {
        let target = if slot.present { 1.0 } else { 0.0 };
        slot.weight += (target - slot.weight).clamp(-step, step);
    }
    // Closed sectors go, with their names.
    ring.slots.retain(|s| {
        let gone = !s.present && s.weight <= 0.0;
        if gone && let Some(label) = s.label {
            commands.entity(label).despawn();
        }
        !gone
    });

    let sectors = sectors(&ring);
    for (slot, (from, to)) in ring.slots.iter_mut().zip(sectors) {
        let angle = (from + to) / 2.0;
        let color = palette::seat(slot.colour).with_alpha(ease(slot.weight));
        match slot.label.and_then(|e| labels.get_mut(e).ok()) {
            Some(mut arc) => {
                if arc.angle != angle || arc.color != color {
                    arc.angle = angle;
                    arc.color = color;
                }
                if arc.text != slot.name {
                    arc.text.clone_from(&slot.name);
                }
            }
            None => {
                let label = ArcText::name(slot.name.clone(), angle, color);
                slot.label = Some(commands.spawn((label, DespawnOnExit(AppState::Lobby))).id());
            }
        }
    }
}

fn draw(mut gizmos: Gizmos, ring: Res<LobbyRing>) {
    let sectors: Vec<Sector> = ring
        .slots
        .iter()
        .zip(sectors(&ring))
        .map(|(slot, (from, to))| {
            let fade = ease(slot.weight);
            Sector {
                from,
                to,
                color: palette::seat(slot.colour).with_alpha(fade),
                fade,
            }
        })
        .collect();
    draw_ring(&mut gizmos, &sectors);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(key: &str, weight: f32) -> Slot {
        Slot {
            key: key.into(),
            name: key.into(),
            colour: 0,
            weight,
            present: true,
            label: None,
        }
    }

    #[test]
    fn sectors_share_the_ring_by_weight() {
        let ring = LobbyRing {
            slots: vec![slot("a", 1.0), slot("b", 1.0), slot("c", 0.0)],
        };
        let s = sectors(&ring);
        let span = |i: usize| s[i].1 - s[i].0;
        assert!((span(0) - PI).abs() < 1e-5);
        assert!((span(1) - PI).abs() < 1e-5);
        assert!(span(2).abs() < 1e-6, "a slot still at weight 0 has no room yet");
        assert!((s[0].0 + s[0].1).abs() < 1e-5, "the first is centred on 0");
    }

    #[test]
    fn a_growing_slot_takes_room_gradually() {
        let half = LobbyRing {
            slots: vec![slot("a", 1.0), slot("b", 0.5)],
        };
        let s = sectors(&half);
        let span_b = s[1].1 - s[1].0;
        assert!(span_b > 0.0 && span_b < PI);
    }
}
