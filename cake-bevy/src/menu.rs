//! The selected unit's menu: segments of the ring just inside the map's band.
//!
//! With my HQ selected the segments produce units; with my units selected they
//! give orders. Each is a [`segments`](crate::segments) button on the lower
//! arc, nearest my HQ, with its key and price, on the segment ring: clearly
//! inside the inner circle, not touching the map's band. They do what the
//! keyboard does, through [`Act`].

use bevy::prelude::*;
use cake_core::geom::{R_INNER, UNIT};
use cake_core::stats::Kind;

use crate::camera::CAKE_LAYER;
use crate::input::{Act, Mode, Selection};
use crate::ringmesh::Slots;
use crate::segments::{self, Segment, SegmentFills, SegmentPressed};
use crate::{AppState, Match};

/// The segment ring, where every button inside the circle sits: 50 deep,
/// with a gap to the map's band so it reads as a ring of its own.
pub const MENU_OUTER: f32 = (R_INNER / UNIT) as f32 - 15.0;
pub const MENU_INNER: f32 = MENU_OUTER - 50.0;
/// Side by side along the bottom, by my HQ, left to right.
const SLOTS: Slots = Slots {
    inner: MENU_INNER,
    outer: MENU_OUTER,
    centre: 270.0,
    width: 15.0,
    gap: 1.0,
    clockwise: false,
};

/// The menu on screen now, and the tick it was worked out at.
#[derive(Resource, Default, Debug)]
struct Menu {
    acts: Vec<Act>,
    tick: u32,
}

/// What a menu segment does.
#[derive(Component)]
struct MenuSlot(Act);

pub fn plugin(app: &mut App) {
    app.init_resource::<Menu>()
        .add_systems(OnExit(AppState::Game), |mut menu: ResMut<Menu>| {
            *menu = Menu::default();
        })
        .add_systems(
            Update,
            (rebuild, enable, act)
                .chain()
                .after(segments::press)
                .run_if(in_state(AppState::Game).and_then(resource_exists::<Match>)),
        );
}

/// What the current selection offers.
fn offered(m: &Match, sel: &Selection) -> Vec<Act> {
    // Once the match is over, the ring's centre belongs to the recap.
    let Some(me) =
        m.me.filter(|s| m.sim.is_alive(*s) && m.sim.outcome.is_none())
    else {
        return Vec::new();
    };
    let mine = || {
        sel.ids
            .iter()
            .filter_map(|id| m.sim.get(*id))
            .filter(move |e| e.owner == me)
    };
    if mine().any(|e| e.kind.is_mobile()) {
        let mut acts = vec![Act::AttackMove, Act::Stop];
        if mine().any(|e| e.kind == Kind::Utility) {
            acts.extend(Kind::TURRETS.map(Act::Build));
            acts.push(Act::Deploy);
        }
        acts
    } else if mine().any(|e| e.kind == Kind::Hq) {
        Kind::UNITS
            .into_iter()
            .map(Act::Produce)
            .chain([Act::Cancel])
            .collect()
    } else {
        Vec::new()
    }
}

#[allow(clippy::too_many_arguments)]
fn rebuild(
    mut commands: Commands,
    m: Res<Match>,
    sel: Res<Selection>,
    fills: Res<SegmentFills>,
    mut menu: ResMut<Menu>,
    slots: Query<Entity, With<MenuSlot>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    // The offer depends on the selection and on who is still alive.
    if !sel.is_changed() && menu.tick == m.sim.tick {
        return;
    }
    menu.tick = m.sim.tick;
    let acts = offered(&m, &sel);
    if acts == menu.acts {
        return;
    }
    for e in &slots {
        commands.entity(e).despawn();
    }
    let n = acts.len();
    for (i, act) in acts.iter().enumerate() {
        segments::spawn(
            &mut commands,
            &mut meshes,
            &fills,
            CAKE_LAYER,
            SLOTS,
            i,
            n,
            act.title(),
            &act.detail(),
            AppState::Game,
            MenuSlot(*act),
        );
    }
    menu.acts = acts;
}

/// Grey out what can't be done right now.
fn enable(m: Res<Match>, sel: Res<Selection>, mut slots: Query<(&MenuSlot, &mut Segment)>) {
    for (slot, mut segment) in &mut slots {
        let on = slot.0.available(&m, &sel);
        if segment.enabled != on {
            segment.enabled = on;
        }
    }
}

fn act(
    mut pressed: MessageReader<SegmentPressed>,
    slots: Query<&MenuSlot>,
    sel: Res<Selection>,
    mut m: ResMut<Match>,
    mut mode: ResMut<Mode>,
) {
    for SegmentPressed(e) in pressed.read() {
        if let Ok(slot) = slots.get(*e) {
            slot.0.perform(&mut m, &sel, &mut mode);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_sit_side_by_side_on_the_lower_arc() {
        for n in 1..=5 {
            for i in 0..n {
                let p = SLOTS.centre_of(i, n);
                assert!(p.y < 0.0, "slot {i} of {n} is on the lower half");
                assert_eq!(SLOTS.at(p, n), Some(i));
                if i + 1 < n {
                    assert!(p.x < SLOTS.centre_of(i + 1, n).x, "left to right");
                }
            }
        }
    }
}
