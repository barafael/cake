//! Everything on the map is a polyline, drawn with gizmos each frame.
//!
//! Units face where they are going (or what they are shooting), in their local
//! frame along the ring. The simulation's integer polar positions become
//! Cartesian map units only here.

use bevy::prelude::*;
use cake_core::geom::{R_INNER, R_OUTER, UNIT, sector_edge};
use cake_core::stats::{self, Kind};
use cake_core::{Entity, Event, Order, Pos, Seat};

use crate::camera::{Cursor, Rig};
use crate::input::{Drag, Mode, Selection};
use crate::lobby::Lobby;
use crate::palette;
use crate::{AppState, Match};

const INNER: f32 = (R_INNER / UNIT) as f32;
const OUTER: f32 = (R_OUTER / UNIT) as f32;

pub fn plugin(app: &mut App) {
    app.init_resource::<Effects>()
        .init_resource::<Shapes>()
        .add_systems(OnEnter(AppState::Game), |mut fx: ResMut<Effects>| fx.0.clear())
        .add_systems(
            Update,
            (
                draw_lobby.run_if(in_state(AppState::Lobby)),
                (collect_effects, draw_game)
                    .chain()
                    .run_if(in_state(AppState::Game).and_then(resource_exists::<Match>)),
            ),
        );
}

/// Presentation-only flashes: shots, deaths, completions.
#[derive(Resource, Default)]
pub struct Effects(Vec<Effect>);

struct Effect {
    kind: EffectKind,
    age: f32,
    ttl: f32,
}

enum EffectKind {
    Tracer { from: Vec2, to: Vec2, color: Color },
    Burst { at: Vec2, color: Color, size: f32 },
    Done { at: Vec2, color: Color, size: f32 },
}

/// Local-frame outlines per kind: forward is +x, left is +y, in map units.
#[derive(Resource)]
pub struct Shapes(Vec<(Kind, Vec<Vec<Vec2>>)>);

fn ngon(n: usize, r: f32, phase: f32) -> Vec<Vec2> {
    (0..=n)
        .map(|k| Vec2::from_angle(phase + k as f32 * std::f32::consts::TAU / n as f32) * r)
        .collect()
}

fn line(points: &[(f32, f32)]) -> Vec<Vec2> {
    points.iter().map(|&(x, y)| Vec2::new(x, y)).collect()
}

impl Default for Shapes {
    fn default() -> Self {
        let eighth = std::f32::consts::PI / 8.0;
        Shapes(vec![
            (
                Kind::Brawler,
                vec![
                    line(&[
                        (7.0, 0.0),
                        (3.5, 6.0),
                        (-4.0, 6.0),
                        (-7.0, 0.0),
                        (-4.0, -6.0),
                        (3.5, -6.0),
                        (7.0, 0.0),
                    ]),
                    line(&[(0.5, -3.0), (4.0, 0.0), (0.5, 3.0)]),
                ],
            ),
            (
                Kind::Skirmisher,
                vec![
                    line(&[(6.0, 0.0), (-5.0, 5.0), (-2.0, 0.0), (-5.0, -5.0), (6.0, 0.0)]),
                    line(&[(6.0, 0.0), (11.0, 0.0)]),
                ],
            ),
            (
                Kind::Raider,
                vec![
                    line(&[(8.0, 0.0), (-5.0, 3.5), (-2.5, 0.0), (-5.0, -3.5), (8.0, 0.0)]),
                    line(&[(-5.0, 3.5), (-7.5, 5.0)]),
                    line(&[(-5.0, -3.5), (-7.5, -5.0)]),
                ],
            ),
            (
                Kind::Utility,
                vec![
                    ngon(8, 6.0, eighth),
                    line(&[(0.0, 0.0), (9.0, 0.0)]),
                    line(&[(9.0, -3.0), (9.0, 3.0)]),
                ],
            ),
            (
                Kind::Hq,
                vec![
                    ngon(8, 22.0, eighth),
                    ngon(8, 15.0, eighth),
                    ngon(4, 6.0, std::f32::consts::FRAC_PI_4),
                    line(&[(15.0, 0.0), (22.0, 0.0)]),
                    line(&[(-15.0, 0.0), (-22.0, 0.0)]),
                    line(&[(0.0, 15.0), (0.0, 22.0)]),
                    line(&[(0.0, -15.0), (0.0, -22.0)]),
                ],
            ),
            (
                Kind::Econ,
                vec![
                    line(&[(12.0, 0.0), (0.0, 12.0), (-12.0, 0.0), (0.0, -12.0), (12.0, 0.0)]),
                    line(&[(-6.0, 0.0), (6.0, 0.0)]),
                    line(&[(0.0, -6.0), (0.0, 6.0)]),
                ],
            ),
            (
                Kind::Turret,
                vec![
                    line(&[(7.0, 7.0), (-7.0, 7.0), (-7.0, -7.0), (7.0, -7.0), (7.0, 7.0)]),
                    line(&[(0.0, 0.0), (13.0, 0.0)]),
                ],
            ),
        ])
    }
}

impl Shapes {
    fn of(&self, kind: Kind) -> &[Vec<Vec2>] {
        self.0
            .iter()
            .find(|(k, _)| *k == kind)
            .map_or(&[], |(_, s)| s.as_slice())
    }
}

pub fn to_vec2(p: Pos) -> Vec2 {
    let (x, y) = p.to_xy();
    Vec2::new(x as f32, y as f32)
}

/// The world direction an entity faces, from its local `(tangential,
/// radial)` facing at world point `at`.
fn facing_dir(e: &Entity, at: Vec2) -> Vec2 {
    let radial = at.normalize_or(Vec2::X);
    let tangent = radial.perp();
    let (t, r) = e.facing;
    // Structures other than turrets always face outward, so they read the same
    // all round the ring.
    let (t, r) = if e.kind.is_structure() && e.kind != Kind::Turret {
        (0, 1000)
    } else {
        (t, r)
    };
    (tangent * t as f32 + radial * r as f32).normalize_or(radial)
}

fn draw_shape(gizmos: &mut Gizmos, shapes: &Shapes, kind: Kind, at: Vec2, dir: Vec2, color: Color) {
    let left = dir.perp();
    for strip in shapes.of(kind) {
        gizmos.linestrip_2d(strip.iter().map(|p| at + dir * p.x + left * p.y), color);
    }
}

/// An arc of the ring between two angles (radians, counter-clockwise).
fn arc_points(radius: f32, from: f32, to: f32) -> impl Iterator<Item = Vec2> {
    let span = (to - from).rem_euclid(std::f32::consts::TAU);
    let n = ((span / std::f32::consts::TAU) * 256.0).ceil().max(2.0) as usize;
    (0..=n).map(move |k| Vec2::from_angle(from + span * k as f32 / n as f32) * radius)
}

fn draw_ring(gizmos: &mut Gizmos, seats: usize, color_of: impl Fn(usize) -> Color) {
    gizmos
        .circle_2d(Isometry2d::IDENTITY, INNER, palette::RING)
        .resolution(512);
    gizmos
        .circle_2d(Isometry2d::IDENTITY, OUTER, palette::RING)
        .resolution(512);
    if seats == 0 {
        return;
    }
    for i in 0..seats {
        let edge = sector_edge(i, seats).to_radians() as f32;
        let prev = sector_edge((i + seats - 1) % seats, seats).to_radians() as f32;
        let dir = Vec2::from_angle(edge);
        if seats > 1 {
            gizmos.line_2d(dir * INNER, dir * OUTER, palette::FAINT);
        }
        // Each sector's owner, as a coloured arc just outside the band.
        let (from, to) = if seats == 1 {
            (0.0, std::f32::consts::TAU - 0.001)
        } else {
            (prev + 0.01, edge - 0.01)
        };
        gizmos.linestrip_2d(arc_points(OUTER + 7.0, from, to), color_of(i));
    }
}

fn draw_lobby(mut gizmos: Gizmos, lobby: Res<Lobby>) {
    let n = lobby.players().count();
    draw_ring(&mut gizmos, n, palette::seat);
}

fn collect_effects(time: Res<Time>, mut fx: ResMut<Effects>, mut m: ResMut<Match>) {
    let dt = time.delta_secs();
    fx.0.retain_mut(|e| {
        e.age += dt;
        e.age < e.ttl
    });
    let events = std::mem::take(&mut m.events);
    let me = m.me;
    // A flash is shown when I could see where it happened.
    let seen = |p: Pos| {
        me.is_none_or(|s| {
            m.sim
                .entities
                .iter()
                .any(|e| e.owner == s && e.pos.within(p, e.stats().vision))
        })
    };
    for event in events {
        match event {
            Event::Shot {
                from,
                to,
                owner,
                kind,
            } if seen(from) || seen(to) => {
                let color = palette::seat(owner as usize).lighter(0.2);
                let ttl = if kind == Kind::Brawler { 0.08 } else { 0.12 };
                fx.0.push(Effect {
                    kind: EffectKind::Tracer {
                        from: to_vec2(from),
                        to: to_vec2(to),
                        color,
                    },
                    age: 0.0,
                    ttl,
                });
            }
            Event::Died { owner, kind, pos } if seen(pos) => fx.0.push(Effect {
                kind: EffectKind::Burst {
                    at: to_vec2(pos),
                    color: palette::seat(owner as usize),
                    size: (kind.stats().radius / UNIT) as f32 * 1.6,
                },
                age: 0.0,
                ttl: 0.45,
            }),
            Event::Completed { owner, kind, pos } if seen(pos) => fx.0.push(Effect {
                kind: EffectKind::Done {
                    at: to_vec2(pos),
                    color: palette::seat(owner as usize),
                    size: (kind.stats().radius / UNIT) as f32 + 4.0,
                },
                age: 0.0,
                ttl: 0.6,
            }),
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_game(
    mut gizmos: Gizmos,
    m: Res<Match>,
    shapes: Res<Shapes>,
    fx: Res<Effects>,
    rig: Res<Rig>,
    selection: Res<Selection>,
    mode: Res<Mode>,
    cursor: Res<Cursor>,
    drag: Res<Drag>,
    camera: Query<(&Camera, &GlobalTransform), With<crate::camera::MainCamera>>,
) {
    let sim = &m.sim;
    draw_ring(&mut gizmos, sim.seats(), |i| {
        let c = palette::seat(i);
        if sim.is_alive(i as Seat) {
            c
        } else {
            c.with_alpha(0.2)
        }
    });

    let screen_right = Vec2::from_angle(rig.rotation);
    let screen_up = screen_right.perp();

    for e in sim.entities.iter().filter(|e| m.sees(e)) {
        let at = m.draw_pos(e);
        let dir = facing_dir(e, at);
        let base = palette::seat(e.owner as usize);
        let color = if e.complete { base } else { base.with_alpha(0.45) };
        draw_shape(&mut gizmos, &shapes, e.kind, at, dir, color);

        let radius = (e.radius() / UNIT) as f32;
        let selected = selection.ids.contains(&e.id);
        if selected {
            gizmos
                .circle_2d(Isometry2d::from_translation(at), radius + 4.0, palette::SELECTED)
                .resolution(24);
        }
        if e.hp < e.max_hp() || selected {
            let width = (radius * 2.0).max(10.0);
            let frac = (e.hp as f32 / e.max_hp() as f32).clamp(0.0, 1.0);
            let start = at - screen_up * (radius + 5.0) - screen_right * width / 2.0;
            gizmos.line_2d(start, start + screen_right * width, palette::FAINT);
            let health = palette::BAD.mix(&palette::GOOD, frac);
            gizmos.line_2d(start, start + screen_right * width * frac, health);
        }
        // A deploying utility shows its progress as a closing ring.
        if e.kind == Kind::Utility && e.progress > 0 {
            let frac = e.progress as f32 / stats::ECON.build_ticks as f32;
            gizmos
                .arc_2d(
                    Isometry2d::new(at, Rot2::radians(0.0)),
                    std::f32::consts::TAU * frac,
                    12.0,
                    color,
                )
                .resolution(24);
        }
    }

    // Radar: something is there, but not what.
    if let Some(me) = m.me {
        for p in cake_core::vision::radar_blips(sim, me) {
            let at = to_vec2(p);
            gizmos.line_2d(at - Vec2::X * 4.0, at + Vec2::X * 4.0, palette::BLIP);
            gizmos.line_2d(at - Vec2::Y * 4.0, at + Vec2::Y * 4.0, palette::BLIP);
        }
    }

    // Where my selection is headed.
    for e in selection.ids.iter().filter_map(|id| sim.get(*id)) {
        if Some(e.owner) != m.me {
            continue;
        }
        let (to, color) = match e.order {
            Order::Move(p) | Order::Deploy(p) | Order::Build(p) => (p, palette::GOOD),
            Order::AttackMove(p) => (p, palette::BAD),
            _ => continue,
        };
        gizmos.line_2d(m.draw_pos(e), to_vec2(to), color.with_alpha(0.35));
    }
    if selection.hq
        && let Some(me) = m.me
        && let Some(p) = sim.player(me)
    {
        let at = to_vec2(p.rally);
        gizmos
            .circle_2d(Isometry2d::from_translation(at), 4.0, palette::GOOD)
            .resolution(12);
        if let Some(hq) = sim.get(p.hq) {
            gizmos.line_2d(to_vec2(hq.pos), at, palette::GOOD.with_alpha(0.3));
        }
    }

    // Placement preview.
    if let (Some(me), Some(world)) = (m.me, cursor.world) {
        let at = Pos::from_xy(world.x as f64, world.y as f64);
        let site = match *mode {
            Mode::Build => Some((Kind::Turret, sim.turret_site(me, at))),
            Mode::Deploy => Some((Kind::Econ, sim.econ_site(me, at))),
            _ => None,
        };
        if let Some((kind, site)) = site {
            if kind == Kind::Econ {
                // The spacing rule, around every economy building and HQ I
                // know of.
                let spacing = (stats::ECON_SPACING / UNIT) as f32;
                for e in sim
                    .entities
                    .iter()
                    .filter(|e| (e.kind == Kind::Econ || e.kind == Kind::Hq) && m.sees(e))
                {
                    gizmos
                        .circle_2d(Isometry2d::from_translation(m.draw_pos(e)), spacing, palette::FAINT)
                        .resolution(64);
                }
            }
            let (pos, color) = match site {
                Ok(p) => (p, palette::GOOD),
                Err(_) => (at, palette::BAD),
            };
            let at = to_vec2(pos);
            draw_shape(&mut gizmos, &shapes, kind, at, at.normalize_or(Vec2::X), color);
            if kind == Kind::Turret {
                let reach = (stats::TURRET.weapon.as_ref().map_or(0, |w| w.range) / UNIT) as f32;
                gizmos
                    .circle_2d(Isometry2d::from_translation(at), reach, color.with_alpha(0.3))
                    .resolution(48);
            }
        }
    }

    // Effects.
    for e in &fx.0 {
        let t = e.age / e.ttl;
        match e.kind {
            EffectKind::Tracer { from, to, color } => {
                gizmos.line_2d(from, to, color.with_alpha(1.0 - t));
            }
            EffectKind::Burst { at, color, size } => {
                gizmos
                    .circle_2d(Isometry2d::from_translation(at), size * (0.4 + t), color.with_alpha(1.0 - t))
                    .resolution(16);
            }
            EffectKind::Done { at, color, size } => {
                gizmos
                    .circle_2d(Isometry2d::from_translation(at), size + 10.0 * t, color.with_alpha(1.0 - t))
                    .resolution(24);
            }
        }
    }

    // The selection box, in screen space.
    if let (Some(start), Some(end)) = (drag.start, cursor.viewport)
        && drag.is_box(end)
        && let Ok((camera, tf)) = camera.single()
    {
        let corners = [
            start,
            Vec2::new(end.x, start.y),
            end,
            Vec2::new(start.x, end.y),
        ];
        let world: Vec<Vec2> = corners
            .iter()
            .filter_map(|c| camera.viewport_to_world_2d(tf, *c).ok())
            .collect();
        if world.len() == 4 {
            gizmos.lineloop_2d(world, palette::SELECTED.with_alpha(0.6));
        }
    }
}
