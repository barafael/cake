//! Classic RTS controls. Everything here ends in a [`Command`] pushed to the
//! match's outbox; nothing touches the simulation directly.
//!
//! | Input | Effect |
//! |---|---|
//! | Left click / drag | Select (Shift adds) |
//! | Right click | Move; attack an enemy; utilities repair a friendly; with only the HQ selected, set the rally point |
//! | A, then left click | Attack-move |
//! | S | Stop |
//! | B, then left click | A utility builds a turret |
//! | D, then left click | A utility deploys into an economy building |
//! | Q W E R | Queue a brawler, skirmisher, raider or utility |
//! | X | Cancel the last queued unit |
//! | Ctrl+A | Select the whole army |
//! | Space | Select the HQ |
//! | Esc | Leave targeting, then clear the selection |

use bevy::prelude::*;
use cake_core::geom::UNIT;
use cake_core::stats::Kind;
use cake_core::{Command, EntityId, Pos};

use crate::camera::{Cursor, MainCamera};
use crate::{AppState, Match};

/// Pixels the pointer must travel before a click becomes a box.
const DRAG_THRESHOLD: f32 = 6.0;

#[derive(Resource, Default, Debug)]
pub struct Selection {
    /// Selected entities, mine or not (others only for inspection).
    pub ids: Vec<EntityId>,
    /// Is my HQ selected?
    pub hq: bool,
}

/// What the next left click means.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Normal,
    AttackMove,
    Build,
    Deploy,
}

impl Mode {
    pub fn hint(self) -> &'static str {
        match self {
            Mode::Normal => "",
            Mode::AttackMove => "Attack-move: left-click a destination",
            Mode::Build => "Build turret: left-click a site",
            Mode::Deploy => "Deploy economy building: left-click a site",
        }
    }
}

/// A left-button press in progress, in viewport pixels.
#[derive(Resource, Default, Debug)]
pub struct Drag {
    pub start: Option<Vec2>,
}

impl Drag {
    pub fn is_box(&self, now: Vec2) -> bool {
        self.start
            .is_some_and(|s| s.distance(now) > DRAG_THRESHOLD)
    }
}

pub fn plugin(app: &mut App) {
    app.init_resource::<Selection>()
        .init_resource::<Mode>()
        .init_resource::<Drag>()
        .add_systems(
            OnEnter(AppState::Game),
            |mut sel: ResMut<Selection>, mut mode: ResMut<Mode>| {
                *sel = Selection::default();
                *mode = Mode::Normal;
            },
        )
        .add_systems(
            Update,
            (prune, keys, mouse)
                .chain()
                .run_if(in_state(AppState::Game).and_then(resource_exists::<Match>)),
        );
}

/// Forget selected entities that died.
fn prune(mut sel: ResMut<Selection>, m: Res<Match>) {
    let before = sel.ids.len();
    sel.ids.retain(|id| m.sim.get(*id).is_some());
    if sel.ids.len() != before && sel.ids.is_empty() && !sel.hq {
        sel.ids.clear();
    }
}

/// My selected mobile units.
fn my_units(m: &Match, sel: &Selection) -> Vec<EntityId> {
    sel.ids
        .iter()
        .filter_map(|id| m.sim.get(*id))
        .filter(|e| Some(e.owner) == m.me && e.kind.is_mobile())
        .map(|e| e.id)
        .collect()
}

fn my_utilities(m: &Match, sel: &Selection) -> Vec<EntityId> {
    my_units(m, sel)
        .into_iter()
        .filter(|id| m.sim.get(*id).is_some_and(|e| e.kind == Kind::Utility))
        .collect()
}

fn to_pos(world: Vec2) -> Pos {
    Pos::from_xy(world.x as f64, world.y as f64)
}

/// The visible entity under `world`, nearest first, among those `filter`
/// accepts.
fn pick(m: &Match, world: Vec2, filter: impl Fn(&cake_core::Entity) -> bool) -> Option<EntityId> {
    m.sim
        .entities
        .iter()
        .filter(|e| m.sees(e) && filter(e))
        .map(|e| (e, m.draw_pos(e).distance(world)))
        .filter(|(e, d)| *d <= (e.radius() / UNIT) as f32 + 4.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(e, _)| e.id)
}

fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut m: ResMut<Match>,
    mut sel: ResMut<Selection>,
    mut mode: ResMut<Mode>,
) {
    let Some(me) = m.me else {
        return;
    };
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);

    for (key, kind) in [
        (KeyCode::KeyQ, Kind::Brawler),
        (KeyCode::KeyW, Kind::Skirmisher),
        (KeyCode::KeyE, Kind::Raider),
        (KeyCode::KeyR, Kind::Utility),
    ] {
        if keys.just_pressed(key) {
            m.outbox.push(Command::Produce(kind));
        }
    }
    if keys.just_pressed(KeyCode::KeyX) {
        m.outbox.push(Command::CancelProduce);
    }

    if ctrl && keys.just_pressed(KeyCode::KeyA) {
        sel.hq = false;
        sel.ids = m
            .sim
            .entities
            .iter()
            .filter(|e| e.owner == me && e.kind.is_mobile() && e.kind != Kind::Utility)
            .map(|e| e.id)
            .collect();
        return;
    }
    if keys.just_pressed(KeyCode::Space) {
        if let Some(p) = m.sim.player(me) {
            sel.ids = vec![p.hq];
            sel.hq = true;
        }
        return;
    }

    let units = my_units(&m, &sel);
    if keys.just_pressed(KeyCode::KeyA) && !units.is_empty() {
        *mode = Mode::AttackMove;
    }
    if keys.just_pressed(KeyCode::KeyS) && !units.is_empty() {
        m.outbox.push(Command::Stop { units });
    }
    let utilities = !my_utilities(&m, &sel).is_empty();
    if keys.just_pressed(KeyCode::KeyB) && utilities {
        *mode = Mode::Build;
    }
    if keys.just_pressed(KeyCode::KeyD) && utilities {
        *mode = Mode::Deploy;
    }
    if keys.just_pressed(KeyCode::Escape) {
        if *mode != Mode::Normal {
            *mode = Mode::Normal;
        } else {
            *sel = Selection::default();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn mouse(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    cursor: Res<Cursor>,
    camera: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    mut m: ResMut<Match>,
    mut sel: ResMut<Selection>,
    mut mode: ResMut<Mode>,
    mut drag: ResMut<Drag>,
) {
    let (Some(world), Some(viewport)) = (cursor.world, cursor.viewport) else {
        return;
    };
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let me = m.me;

    // Left button: targeting modes act on press; otherwise it selects.
    if buttons.just_pressed(MouseButton::Left) {
        let units = my_units(&m, &sel);
        let at = to_pos(world);
        let nearest_utility = |m: &Match| {
            my_utilities(m, &sel)
                .into_iter()
                .filter_map(|id| m.sim.get(id))
                .min_by_key(|e| (e.pos.dist2(at), e.id))
                .map(|e| e.id)
        };
        let acted = match *mode {
            Mode::Normal => false,
            Mode::AttackMove => {
                m.outbox.push(Command::AttackMove { units, to: at });
                true
            }
            Mode::Build => {
                if let Some(unit) = nearest_utility(&m) {
                    m.outbox.push(Command::Build { unit, at });
                }
                true
            }
            Mode::Deploy => {
                if let Some(unit) = nearest_utility(&m) {
                    m.outbox.push(Command::Deploy { unit, at });
                }
                true
            }
        };
        if acted {
            if !shift {
                *mode = Mode::Normal;
            }
        } else {
            drag.start = Some(viewport);
        }
    }

    if buttons.just_released(MouseButton::Left)
        && let Some(start) = drag.start.take()
    {
        if drag_is_box(start, viewport) {
            let Ok((camera, tf)) = camera.single() else {
                return;
            };
            let lo = start.min(viewport);
            let hi = start.max(viewport);
            let boxed: Vec<EntityId> = m
                .sim
                .entities
                .iter()
                .filter(|e| Some(e.owner) == me && e.kind.is_mobile())
                .filter(|e| {
                    camera
                        .world_to_viewport(tf, m.draw_pos(e).extend(0.0))
                        .is_ok_and(|p| p.cmpge(lo).all() && p.cmple(hi).all())
                })
                .map(|e| e.id)
                .collect();
            if !shift {
                *sel = Selection::default();
            }
            for id in boxed {
                if !sel.ids.contains(&id) {
                    sel.ids.push(id);
                }
            }
        } else {
            let hit = pick(&m, world, |_| true);
            match hit {
                Some(id) if shift => {
                    if let Some(i) = sel.ids.iter().position(|s| *s == id) {
                        sel.ids.remove(i);
                    } else {
                        sel.ids.push(id);
                    }
                }
                Some(id) => {
                    sel.ids = vec![id];
                }
                None if !shift => *sel = Selection::default(),
                None => {}
            }
            sel.hq = me.is_some_and(|me| {
                m.sim
                    .player(me)
                    .is_some_and(|p| sel.ids.contains(&p.hq))
            });
        }
    }

    if buttons.just_pressed(MouseButton::Right) {
        *mode = Mode::Normal;
        let Some(me) = me else {
            return;
        };
        let units = my_units(&m, &sel);
        let at = to_pos(world);
        if units.is_empty() {
            if sel.hq {
                m.outbox.push(Command::SetRally(at));
            }
            return;
        }
        if let Some(target) = pick(&m, world, |e| e.owner != me) {
            m.outbox.push(Command::Attack { units, target });
            return;
        }
        let utilities = my_utilities(&m, &sel);
        if !utilities.is_empty()
            && let Some(friend) = pick(&m, world, |e| {
                e.owner == me && (e.hp < e.max_hp() || !e.complete)
            })
        {
            m.outbox.push(Command::Repair {
                units: utilities.clone(),
                target: friend,
            });
            let rest: Vec<EntityId> = units.into_iter().filter(|u| !utilities.contains(u)).collect();
            if !rest.is_empty() {
                m.outbox.push(Command::Move { units: rest, to: at });
            }
            return;
        }
        m.outbox.push(Command::Move { units, to: at });
    }
}

fn drag_is_box(start: Vec2, end: Vec2) -> bool {
    start.distance(end) > DRAG_THRESHOLD
}
