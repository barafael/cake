//! The selected unit's menu: segments of the ring just inside the map's band.
//!
//! With my HQ selected the segments produce units; with my units selected they
//! give orders. Each segment is a slice of the ring between radius 350 and
//! the band's inner edge on the lower arc, nearest my HQ, and shows what it
//! does along the arc, with its key and price. They do what the keyboard
//! does, through [`Act`].

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::sprite_render::{ColorMaterial, MeshMaterial2d};
use cake_core::geom::{R_INNER, UNIT};
use cake_core::stats::Kind;

use crate::arctext::{ArcText, Frame};
use crate::camera::{Cursor, OVERLAY_LAYER};
use crate::chrome::{PointerClaimed, PointerSet, RADIUS, block_pointer};
use crate::input::{Act, Mode, Selection};
use crate::ringmesh::Slots;
use crate::{AppState, Match, palette};

pub const MENU_INNER: f32 = 350.0;
pub const MENU_OUTER: f32 = (R_INNER / UNIT) as f32;
/// Side by side along the bottom, by my HQ, left to right.
const SLOTS: Slots = Slots {
    inner: MENU_INNER,
    outer: MENU_OUTER,
    centre: 270.0,
    width: 15.0,
    gap: 1.0,
    clockwise: false,
};

/// Which menu slot the pointer is over, if any.
#[derive(Resource, Default, Debug, PartialEq)]
pub struct MenuHover(pub Option<usize>);

/// The menu on screen now, and the tick it was worked out at.
#[derive(Resource, Default, Debug)]
struct Menu {
    acts: Vec<Act>,
    tick: u32,
}

/// Slot fills, shared by every slot.
#[derive(Resource)]
struct Fills {
    idle: Handle<ColorMaterial>,
    hover: Handle<ColorMaterial>,
    off: Handle<ColorMaterial>,
}

impl FromWorld for Fills {
    fn from_world(world: &mut World) -> Self {
        let mut materials = world.resource_mut::<Assets<ColorMaterial>>();
        Fills {
            idle: materials.add(ColorMaterial::from_color(palette::CONTROL_IDLE)),
            hover: materials.add(ColorMaterial::from_color(palette::CONTROL_HOVER)),
            off: materials.add(ColorMaterial::from_color(palette::CONTROL_OFF)),
        }
    }
}

/// A slot's segment.
#[derive(Component)]
struct Slot(usize);

/// A slot's label, and its colour when the slot can be used.
#[derive(Component)]
struct SlotLabel {
    slot: usize,
    ink: Color,
}

pub fn plugin(app: &mut App) {
    app.init_resource::<MenuHover>()
        .init_resource::<Menu>()
        .init_resource::<Fills>()
        .add_systems(
            OnExit(AppState::Game),
            |mut menu: ResMut<Menu>, mut hover: ResMut<MenuHover>| {
                *menu = Menu::default();
                hover.0 = None;
            },
        )
        .add_systems(
            Update,
            (
                (rebuild, hover)
                    .chain()
                    .in_set(PointerSet)
                    .before(block_pointer),
                (press, shade).chain().after(PointerSet),
            )
                .run_if(in_state(AppState::Game).and_then(resource_exists::<Match>)),
        );
}

/// What the current selection offers.
fn offered(m: &Match, sel: &Selection) -> Vec<Act> {
    let Some(me) = m.me.filter(|s| m.sim.is_alive(*s)) else {
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
            acts.extend([Act::Build, Act::Deploy]);
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
    fills: Res<Fills>,
    mut menu: ResMut<Menu>,
    slots: Query<Entity, With<Slot>>,
    labels: Query<Entity, With<SlotLabel>>,
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
    for e in slots.iter().chain(labels.iter()) {
        commands.entity(e).despawn();
    }
    let n = acts.len();
    for (i, act) in acts.iter().enumerate() {
        let (from, to) = SLOTS.span(i, n);
        commands.spawn((
            Mesh2d(meshes.add(SLOTS.mesh(i, n, RADIUS))),
            MeshMaterial2d(fills.idle.clone()),
            Transform::from_xyz(0.0, 0.0, -4.0),
            RenderLayers::layer(OVERLAY_LAYER),
            Slot(i),
            DespawnOnExit(AppState::Game),
        ));
        // At the bottom text is flipped, tops toward the centre: the title
        // sits nearer the centre, so it reads above the detail.
        for (text, radius, size, ink) in [
            (act.title().to_string(), MENU_INNER + 19.0, 15.0, palette::TEXT),
            (act.detail(), MENU_OUTER - 13.0, 12.0, palette::DIM_TEXT),
        ] {
            commands.spawn((
                ArcText {
                    text,
                    angle: (from + to) / 2.0,
                    frame: Frame::Screen,
                    radius,
                    size,
                    color: ink,
                },
                SlotLabel { slot: i, ink },
                DespawnOnExit(AppState::Game),
            ));
        }
    }
    menu.acts = acts;
}

fn hover(
    cursor: Res<Cursor>,
    menu: Res<Menu>,
    mut hover: ResMut<MenuHover>,
    mut claimed: ResMut<PointerClaimed>,
) {
    let h = cursor.ui.and_then(|p| SLOTS.at(p, menu.acts.len()));
    hover.set_if_neq(MenuHover(h));
    if h.is_some() {
        claimed.0 = true;
    }
}

fn press(
    buttons: Res<ButtonInput<MouseButton>>,
    hover: Res<MenuHover>,
    menu: Res<Menu>,
    sel: Res<Selection>,
    mut m: ResMut<Match>,
    mut mode: ResMut<Mode>,
) {
    if buttons.just_pressed(MouseButton::Left)
        && let Some(act) = hover.0.and_then(|i| menu.acts.get(i))
    {
        act.perform(&mut m, &sel, &mut mode);
    }
}

/// Fills for hover and availability; unusable slots' labels fade.
fn shade(
    hover: Res<MenuHover>,
    menu: Res<Menu>,
    m: Res<Match>,
    sel: Res<Selection>,
    fills: Res<Fills>,
    mut slots: Query<(&Slot, &mut MeshMaterial2d<ColorMaterial>)>,
    mut labels: Query<(&SlotLabel, &mut ArcText)>,
) {
    let on: Vec<bool> = menu.acts.iter().map(|a| a.available(&m, &sel)).collect();
    let usable = |i: usize| on.get(i).copied().unwrap_or(false);
    for (slot, mut material) in &mut slots {
        let fill = match (usable(slot.0), hover.0 == Some(slot.0)) {
            (false, _) => &fills.off,
            (true, true) => &fills.hover,
            (true, false) => &fills.idle,
        };
        if material.0 != *fill {
            material.0 = fill.clone();
        }
    }
    for (label, mut arc) in &mut labels {
        let color = if usable(label.slot) {
            label.ink
        } else {
            palette::FAINT
        };
        if arc.color != color {
            arc.color = color;
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

    #[test]
    fn nothing_outside_the_menu_ring_is_a_slot() {
        assert_eq!(SLOTS.at(Vec2::new(0.0, -300.0), 3), None);
        assert_eq!(SLOTS.at(Vec2::new(0.0, -450.0), 3), None);
    }
}
