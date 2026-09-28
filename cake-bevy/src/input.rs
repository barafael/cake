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
//!
//! The HUD's buttons do the same through [`Act`], and the map ignores
//! presses over them or over the window frame.

use bevy::prelude::*;
use cake_core::geom::UNIT;
use cake_core::stats::{self, Kind, SUPPLY};
use cake_core::{Command, EntityId, Pos};

use crate::camera::Cursor;
use crate::chrome::PointerBlocked;
use crate::ringmesh;
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
    /// Lay down a turret of this kind.
    Build(Kind),
    Deploy,
}

impl Mode {
    pub fn hint(self) -> String {
        match self {
            Mode::Normal => String::new(),
            Mode::AttackMove => "Attack-move: left-click a destination".into(),
            Mode::Build(kind) => {
                format!("Build a {}: left-click a site", kind.name().to_lowercase())
            }
            Mode::Deploy => "Deploy economy building: left-click a site".into(),
        }
    }
}

/// A left-button press in progress: where it started, in viewport pixels
/// and in world units.
#[derive(Resource, Default, Debug)]
pub struct Drag {
    pub start: Option<(Vec2, Vec2)>,
}

impl Drag {
    /// The selection region from the press to the pointer, once the pointer
    /// has moved far enough for a press to be a drag.
    pub fn region(&self, viewport: Vec2, world: Vec2) -> Option<PolarBox> {
        let (px, w) = self.start?;
        (px.distance(viewport) > DRAG_THRESHOLD).then(|| PolarBox::spanning(w, world))
    }
}

/// The selection region: a rectangle in polar coordinates around the ring's
/// centre. Two sides are radii, and two are arcs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PolarBox {
    pub r_min: f32,
    pub r_max: f32,
    /// The arc runs from `from` through `sweep` radians (either sign), the
    /// short way round.
    pub from: f32,
    pub sweep: f32,
}

impl PolarBox {
    pub fn spanning(a: Vec2, b: Vec2) -> PolarBox {
        let from = ringmesh::angle_of(a);
        PolarBox {
            r_min: a.length().min(b.length()),
            r_max: a.length().max(b.length()),
            from,
            sweep: ringmesh::wrap_pi(ringmesh::angle_of(b) - from),
        }
    }

    pub fn contains(&self, p: Vec2) -> bool {
        let (start, end) = (self.from, self.from + self.sweep);
        (self.r_min..=self.r_max).contains(&p.length())
            && ringmesh::in_span(ringmesh::angle_of(p), (start.min(end), start.max(end)))
    }

    /// The outline, as one closed line: inner arc, then the outer arc back.
    pub fn outline(&self) -> Vec<Vec2> {
        let mut points: Vec<Vec2> = ringmesh::arc(self.r_min, self.from, self.sweep).collect();
        let outer: Vec<Vec2> = ringmesh::arc(self.r_max, self.from, self.sweep).collect();
        points.extend(outer.into_iter().rev());
        points
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

/// Forget selected entities that died. Looks first: a mutable borrow alone
/// would mark the selection changed every frame.
fn prune(mut sel: ResMut<Selection>, m: Res<Match>) {
    if sel.ids.iter().any(|id| m.sim.get(*id).is_none()) {
        sel.ids.retain(|id| m.sim.get(*id).is_some());
    }
}

/// My selected mobile units.
fn mine<'a>(m: &'a Match, sel: &'a Selection) -> impl Iterator<Item = &'a cake_core::Entity> {
    sel.ids
        .iter()
        .filter_map(|id| m.sim.get(*id))
        .filter(|e| Some(e.owner) == m.me && e.kind.is_mobile())
}

fn my_units(m: &Match, sel: &Selection) -> Vec<EntityId> {
    mine(m, sel).map(|e| e.id).collect()
}

fn my_utilities(m: &Match, sel: &Selection) -> Vec<EntityId> {
    mine(m, sel)
        .filter(|e| e.kind == Kind::Utility)
        .map(|e| e.id)
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

/// Something the player can do from the keyboard or a HUD button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Produce(Kind),
    Cancel,
    AttackMove,
    Stop,
    Build(Kind),
    Deploy,
}

impl Act {
    pub fn key(self) -> &'static str {
        match self {
            Act::Produce(Kind::Brawler) => "Q",
            Act::Produce(Kind::Skirmisher) => "W",
            Act::Produce(Kind::Raider) => "E",
            Act::Produce(_) => "R",
            Act::Cancel => "X",
            Act::AttackMove => "A",
            Act::Stop => "S",
            Act::Build(Kind::PlasmaTurret) => "N",
            Act::Build(Kind::MissileTurret) => "M",
            Act::Build(_) => "B",
            Act::Deploy => "D",
        }
    }

    /// What it is, in a word.
    pub fn title(self) -> &'static str {
        match self {
            Act::Produce(kind) => kind.name(),
            Act::Cancel => "Cancel",
            Act::AttackMove => "Attack",
            Act::Stop => "Stop",
            Act::Build(Kind::PlasmaTurret) => "Plasma",
            Act::Build(Kind::MissileTurret) => "Missile",
            Act::Build(_) => "Gun",
            Act::Deploy => "Deploy",
        }
    }

    /// Its key, and its price if it has one.
    pub fn detail(self) -> String {
        match self {
            Act::Produce(kind) | Act::Build(kind) => {
                format!("{}  {}", self.key(), kind.stats().cost / SUPPLY)
            }
            _ => self.key().to_string(),
        }
    }

    /// Would doing this now make sense?
    pub fn available(self, m: &Match, sel: &Selection) -> bool {
        let Some(me) = m.me else {
            return false;
        };
        let Some(p) = m.sim.player(me).filter(|p| p.alive) else {
            return false;
        };
        match self {
            Act::Produce(kind) => p.supply >= kind.stats().cost && p.queue.len() < stats::QUEUE_MAX,
            Act::Cancel => !p.queue.is_empty(),
            Act::AttackMove | Act::Stop => mine(m, sel).next().is_some(),
            Act::Build(_) | Act::Deploy => mine(m, sel).any(|e| e.kind == Kind::Utility),
        }
    }

    /// Do it: queue a command, or enter a targeting mode.
    pub fn perform(self, m: &mut Match, sel: &Selection, mode: &mut Mode) {
        if !self.available(m, sel) {
            return;
        }
        match self {
            Act::Produce(kind) => m.outbox.push(Command::Produce(kind)),
            Act::Cancel => m.outbox.push(Command::CancelProduce),
            Act::AttackMove => *mode = Mode::AttackMove,
            Act::Stop => {
                let units = my_units(m, sel);
                m.outbox.push(Command::Stop { units });
            }
            Act::Build(kind) => *mode = Mode::Build(kind),
            Act::Deploy => *mode = Mode::Deploy,
        }
    }
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

    for (key, act) in [
        (KeyCode::KeyQ, Act::Produce(Kind::Brawler)),
        (KeyCode::KeyW, Act::Produce(Kind::Skirmisher)),
        (KeyCode::KeyE, Act::Produce(Kind::Raider)),
        (KeyCode::KeyR, Act::Produce(Kind::Utility)),
        (KeyCode::KeyX, Act::Cancel),
        (KeyCode::KeyA, Act::AttackMove),
        (KeyCode::KeyS, Act::Stop),
        (KeyCode::KeyB, Act::Build(Kind::Turret)),
        (KeyCode::KeyN, Act::Build(Kind::PlasmaTurret)),
        (KeyCode::KeyM, Act::Build(Kind::MissileTurret)),
        (KeyCode::KeyD, Act::Deploy),
    ] {
        if keys.just_pressed(key) {
            act.perform(&mut m, &sel, &mut mode);
        }
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
    mut m: ResMut<Match>,
    mut sel: ResMut<Selection>,
    mut mode: ResMut<Mode>,
    mut drag: ResMut<Drag>,
    blocked: Res<PointerBlocked>,
) {
    let (Some(world), Some(viewport)) = (cursor.world, cursor.viewport) else {
        return;
    };
    // Over a button or the window frame, presses are not the map's. A drag
    // that started on the map still ends wherever it is released.
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let me = m.me;

    // Left button: targeting modes act on press; otherwise it selects.
    if buttons.just_pressed(MouseButton::Left) && !blocked.0 {
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
            Mode::Build(kind) => {
                if let Some(unit) = nearest_utility(&m) {
                    m.outbox.push(Command::Build { unit, at, kind });
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
            drag.start = Some((viewport, world));
        }
    }

    if buttons.just_released(MouseButton::Left) && drag.start.is_some() {
        let region = drag.region(viewport, world);
        drag.start = None;
        if let Some(region) = region {
            let boxed: Vec<EntityId> = m
                .sim
                .entities
                .iter()
                .filter(|e| Some(e.owner) == me && e.kind.is_mobile())
                .filter(|e| region.contains(m.draw_pos(e)))
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
            sel.hq = me.is_some_and(|me| m.sim.player(me).is_some_and(|p| sel.ids.contains(&p.hq)));
        }
    }

    if buttons.just_pressed(MouseButton::Right) && !blocked.0 {
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
            let rest: Vec<EntityId> = units
                .into_iter()
                .filter(|u| !utilities.contains(u))
                .collect();
            if !rest.is_empty() {
                m.outbox.push(Command::Move {
                    units: rest,
                    to: at,
                });
            }
            return;
        }
        m.outbox.push(Command::Move { units, to: at });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, PI};

    #[test]
    fn a_polar_box_holds_what_lies_between_its_radii_and_arcs() {
        let b = PolarBox::spanning(Vec2::new(400.0, 0.0), Vec2::from_angle(FRAC_PI_2) * 480.0);
        assert!(b.contains(Vec2::from_angle(0.7) * 450.0));
        assert!(
            !b.contains(Vec2::from_angle(0.7) * 350.0),
            "inside the inner arc"
        );
        assert!(
            !b.contains(Vec2::from_angle(-0.2) * 450.0),
            "before the first radius"
        );
        assert!(
            !b.contains(Vec2::from_angle(2.0) * 450.0),
            "past the second radius"
        );
    }

    #[test]
    fn a_polar_box_takes_the_short_way_round_either_way() {
        // Dragged clockwise across the seam at angle pi.
        let b = PolarBox::spanning(
            Vec2::from_angle(PI - 0.2) * 450.0,
            Vec2::from_angle(-PI + 0.2) * 460.0,
        );
        assert!(b.sweep.abs() < 0.5, "{b:?}");
        assert!(b.contains(Vec2::from_angle(PI) * 455.0));
        assert!(!b.contains(Vec2::from_angle(0.0) * 455.0));
        // And the other way.
        let c = PolarBox::spanning(
            Vec2::from_angle(0.3) * 450.0,
            Vec2::from_angle(-0.3) * 460.0,
        );
        assert!(c.sweep < 0.0);
        assert!(c.contains(Vec2::from_angle(0.0) * 455.0));
        assert!(!c.contains(Vec2::from_angle(PI) * 455.0));
    }

    #[test]
    fn the_outline_is_closed_by_its_radii() {
        let b = PolarBox::spanning(Vec2::new(400.0, 0.0), Vec2::new(0.0, 500.0));
        let o = b.outline();
        assert!((o[0].length() - 400.0).abs() < 1e-3);
        assert!((o.last().unwrap().length() - 500.0).abs() < 1e-3);
        // First and last points share the start angle: the closing radius.
        assert!(o[0].normalize().distance(o.last().unwrap().normalize()) < 1e-4);
    }
}
