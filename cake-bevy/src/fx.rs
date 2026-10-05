//! Presentation-only effects: shots in flight, and the sparks, flashes,
//! shockwaves and debris of the fighting.
//!
//! The simulation says what happened ([`Event`]s) and where its projectiles
//! are. Everything here is decoration on top of that, with its own randomness,
//! and never feeds back. What should shine is drawn twice: thin and bright
//! with the default gizmos, and wide and faint with [`GlowGizmos`].

use std::f32::consts::TAU;

use bevy::prelude::*;
use cake_core::geom::UNIT;
use cake_core::sim::Projectile;
use cake_core::stats::{Kind, Shot};
use cake_core::{EntityId, Event, Pos};

use crate::render::{self, Shapes, to_vec2};
use crate::settings::Settings;
use crate::{AppState, Match, TICK_SECS, palette};

/// Wide, faint lines under the bright ones: the glow.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct GlowGizmos;

/// Beyond this many particles, the oldest go first.
const MAX_PARTICLES: usize = 4000;
/// Smoke puffs per second behind each missile.
const TRAIL_HZ: f32 = 30.0;
/// Burning buildings smoke this many times a second.
const BURN_HZ: f32 = 6.0;
/// How long a hurt entity flashes, in seconds.
const HURT_SECS: f32 = 0.15;
/// A moving unit's wake: this many points, drawn under the shapes.
const WAKE_POINTS: usize = 8;
/// How far a unit must travel between wake points, in map units.
const WAKE_STEP: f32 = 0.5;
/// A scorched stain lingers where a building fell, this long.
const STAIN_SECS: f32 = 45.0;
/// A slow circulation around the ring carries smoke and embers the way
/// everything on it moves: counter-clockwise. Map units per second.
const RING_WIND: f32 = 5.0;

const FLAME: Color = Color::srgb(1.0, 0.72, 0.35);
const HEAT: Color = Color::srgb(1.0, 0.9, 0.7);
const SMOKE: Color = Color::srgb(0.62, 0.64, 0.70);

pub fn plugin(app: &mut App) {
    app.init_gizmo_group::<GlowGizmos>()
        .init_resource::<Fx>()
        .add_systems(Startup, configure_gizmos)
        .add_systems(OnEnter(AppState::Game), |mut fx: ResMut<Fx>| {
            *fx = Fx::default();
        })
        .add_systems(
            Update,
            (
                collect.before(render::draw_game),
                wakes.before(render::draw_game),
                draw.after(render::draw_game),
            )
                .run_if(in_state(AppState::Game).and_then(resource_exists::<Match>)),
        );
}

fn configure_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (glow, _) = store.config_mut::<GlowGizmos>();
    glow.line.width = 6.0;
}

#[derive(Resource, Default)]
pub struct Fx {
    marks: Vec<Mark>,
    particles: Vec<Particle>,
    /// Where I can see, as of this frame.
    sight: Sight,
    /// Every entity's health last frame, in id order, to notice hits.
    health: Vec<(EntityId, i32)>,
    /// Entities hit lately, and how brightly they still flash (1 to 0).
    hurt: Vec<(EntityId, f32)>,
    /// Moving units' recent positions, in id order, drawn as wakes.
    wakes: Vec<Wake>,
    rng: Rng,
    /// Emission owed to missile trails and burning buildings.
    trail_clock: f32,
    burn_clock: f32,
}

/// A unit's recent positions, oldest kept at `head - len`.
struct Wake {
    id: EntityId,
    points: [Vec2; WAKE_POINTS],
    len: usize,
    head: usize,
}

impl Wake {
    fn point(&self, j: usize) -> Vec2 {
        self.points[(self.head + WAKE_POINTS - self.len + j) % WAKE_POINTS]
    }
}

impl Fx {
    /// How brightly `id` flashes from a recent hit: 0 when it wasn't hit.
    pub fn hurt(&self, id: EntityId) -> f32 {
        self.hurt
            .iter()
            .find(|(h, _)| *h == id)
            .map_or(0.0, |(_, f)| *f)
    }

    fn mark(&mut self, at: Vec2, ttl: f32, color: Color, look: Look) {
        self.marks.push(Mark {
            at,
            age: 0.0,
            ttl,
            color,
            look,
        });
    }

    /// A ring widening from `from` to `to`, starting after `delay`.
    fn wave(&mut self, at: Vec2, from: f32, to: f32, ttl: f32, delay: f32, color: Color) {
        self.marks.push(Mark {
            at,
            age: -delay,
            ttl,
            color,
            look: Look::Wave { from, to },
        });
    }

    fn particle(&mut self, pos: Vec2, vel: Vec2, drag: f32, ttl: f32, color: Color, mote: Mote) {
        self.particles.push(Particle {
            pos,
            vel,
            drag,
            age: 0.0,
            ttl,
            color,
            mote,
        });
    }

    /// `n` sparks thrown from `at`, around `dir` within `spread` (a full
    /// turn is every way).
    fn sparks(
        &mut self,
        at: Vec2,
        n: usize,
        dir: Vec2,
        spread: f32,
        speed: (f32, f32),
        color: Color,
    ) {
        for _ in 0..n {
            let turn = self.rng.range(-spread, spread) / 2.0;
            let vel = Vec2::from_angle(turn).rotate(dir) * self.rng.range(speed.0, speed.1);
            let ttl = self.rng.range(0.15, 0.35);
            self.particle(at, vel, 4.0, ttl, color, Mote::Spark);
        }
    }

    fn smoke(&mut self, at: Vec2, n: usize, size: (f32, f32), drift: f32, ttl: (f32, f32)) {
        for _ in 0..n {
            let pos = at + self.rng.dir() * self.rng.range(0.0, size.0);
            let vel = self.rng.dir() * self.rng.range(0.2, 1.0) * drift;
            let ttl = self.rng.range(ttl.0, ttl.1);
            let mote = Mote::Smoke {
                from: size.0,
                to: size.1 * self.rng.range(0.7, 1.2),
            };
            self.particle(pos, vel, 1.5, ttl, SMOKE, mote);
        }
    }

    fn embers(&mut self, at: Vec2, n: usize, speed: (f32, f32), color: Color) {
        for _ in 0..n {
            let vel = self.rng.dir() * self.rng.range(speed.0, speed.1);
            let ttl = self.rng.range(0.4, 0.9);
            let size = self.rng.range(0.6, 1.4);
            self.particle(at, vel, 3.0, ttl, color, Mote::Ember { size });
        }
    }

    /// Something fired: the muzzle's flash, or a brawler's swipe.
    fn shot(&mut self, from: Vec2, to: Vec2, kind: Kind, color: Color) {
        let Some(weapon) = &kind.stats().weapon else {
            return;
        };
        let dir = (to - from).normalize_or(Vec2::X);
        let reach = (kind.stats().radius / UNIT) as f32;
        let muzzle = from + dir * reach;
        let hot = hot(color);
        match weapon.shot {
            Shot::Melee => {
                self.mark(
                    from,
                    0.2,
                    hot,
                    Look::Slash {
                        dir,
                        reach: reach + 6.0,
                    },
                );
                self.sparks(to - dir * 3.0, 3, -dir, 1.6, (40.0, 90.0), hot);
            }
            Shot::Bullet { .. } => self.mark(muzzle, 0.07, hot, Look::Muzzle { dir, len: 6.0 }),
            Shot::Plasma { .. } => {
                self.wave(muzzle, 2.0, 10.0, 0.3, 0.0, color);
                self.mark(muzzle, 0.15, hot, Look::Flash { size: 6.0 });
            }
            Shot::Missile { .. } => {
                self.mark(muzzle, 0.1, FLAME, Look::Flash { size: 4.0 });
                self.smoke(from, 3, (1.5, 5.0), 10.0, (0.5, 0.9));
            }
        }
    }

    /// A projectile arrived.
    fn impact(&mut self, at: Vec2, shooter: Kind, color: Color) {
        let Some(weapon) = &shooter.stats().weapon else {
            return;
        };
        let hot = hot(color);
        match weapon.shot {
            Shot::Melee => {}
            Shot::Bullet { .. } => {
                self.mark(at, 0.1, hot, Look::Flash { size: 4.0 });
                self.sparks(at, 3, Vec2::X, TAU, (50.0, 110.0), hot);
            }
            Shot::Plasma { splash, .. } => {
                let r = (splash / UNIT) as f32;
                self.wave(at, 3.0, r, 0.45, 0.0, hot);
                self.wave(at, 2.0, r * 0.6, 0.35, 0.06, color);
                self.mark(at, 0.22, hot, Look::Flash { size: 10.0 });
                self.embers(at, 12, (r * 1.5, r * 3.0), hot);
            }
            Shot::Missile { .. } => {
                self.mark(at, 0.14, FLAME, Look::Flash { size: 7.0 });
                self.wave(at, 2.0, 12.0, 0.3, 0.0, FLAME);
                self.sparks(at, 6, Vec2::X, TAU, (50.0, 120.0), FLAME);
                self.smoke(at, 4, (2.0, 6.0), 8.0, (0.6, 1.0));
            }
        }
    }

    /// Something died: its outline breaks apart, and buildings burn.
    fn died(&mut self, shapes: &Shapes, at: Vec2, kind: Kind, color: Color) {
        let radius = (kind.stats().radius / UNIT) as f32;
        let structure = kind.is_structure();
        let facing = if structure {
            at.normalize_or(Vec2::X)
        } else {
            self.rng.dir()
        };
        let throw = if structure { 60.0 } else { 40.0 };
        for strip in shapes.of(kind) {
            for w in strip.windows(2) {
                let (a, b) = (facing.rotate(w[0]), facing.rotate(w[1]));
                let mid = at + (a + b) / 2.0;
                let out = (mid - at).normalize_or(self.rng.dir());
                let vel = out * self.rng.range(throw * 0.3, throw) + self.rng.dir() * 8.0;
                let mote = Mote::Shard {
                    half: (b - a) / 2.0,
                    angle: 0.0,
                    spin: self.rng.range(-9.0, 9.0),
                };
                let ttl = self.rng.range(0.7, 1.3);
                self.particle(mid, vel, 2.2, ttl, color, mote);
            }
        }
        self.mark(at, 0.2, hot(color), Look::Flash { size: radius * 1.6 });
        self.sparks(
            at,
            5 + radius as usize / 2,
            Vec2::X,
            TAU,
            (60.0, 140.0),
            FLAME,
        );
        if structure {
            // The ring keeps a scorched stain where it stood, fading long
            // after the embers are out.
            self.marks.push(Mark {
                at,
                age: -0.3,
                ttl: STAIN_SECS,
                color,
                look: Look::Stain {
                    size: radius * 1.15,
                },
            });
            self.embers(at, 18, (20.0, 70.0), FLAME);
            self.smoke(at, 8, (radius * 0.4, radius * 1.4), 12.0, (1.2, 2.2));
            self.marks.push(Mark {
                at,
                age: -0.12,
                ttl: 0.25,
                color: HEAT,
                look: Look::Flash { size: radius * 2.2 },
            });
        }
    }

    /// A building's blast: a shockwave out to where it throws units.
    fn blast(&mut self, at: Vec2, radius: f32, color: Color) {
        let warm = HEAT.mix(&color, 0.3);
        self.wave(at, 4.0, radius, 0.55, 0.0, warm);
        self.wave(at, 2.0, radius * 0.7, 0.7, 0.08, color);
        self.mark(
            at,
            0.25,
            HEAT,
            Look::Flash {
                size: radius * 0.35,
            },
        );
        // Dust thrown out in a ring.
        for k in 0..16 {
            let dir = Vec2::from_angle(k as f32 * TAU / 16.0 + self.rng.range(0.0, 0.3));
            let mote = Mote::Smoke {
                from: 2.0,
                to: self.rng.range(5.0, 9.0),
            };
            let vel = dir * radius * self.rng.range(1.8, 2.6);
            self.particle(at + dir * radius * 0.2, vel, 3.0, 0.9, SMOKE, mote);
        }
    }
}

/// Where I can see: my entities and how far each sees. Watchers see all.
#[derive(Default)]
struct Sight(Option<Vec<(Pos, i64)>>);

impl Sight {
    fn of(m: &Match) -> Sight {
        Sight(m.me.map(|me| {
            m.sim
                .entities
                .iter()
                .filter(|e| e.owner == me)
                .map(|e| (e.pos, e.stats().vision))
                .collect()
        }))
    }

    fn sees(&self, p: Pos) -> bool {
        self.0
            .as_ref()
            .is_none_or(|eyes| eyes.iter().any(|(at, vision)| at.within(p, *vision)))
    }
}

/// A small, fast generator: effects only need to look random.
struct Rng(u64);

impl Default for Rng {
    fn default() -> Self {
        Rng(0x9e37_79b9_7f4a_7c15)
    }
}

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }

    fn dir(&mut self) -> Vec2 {
        Vec2::from_angle(self.range(0.0, TAU))
    }
}

/// A stationary effect that plays out where it happened.
struct Mark {
    at: Vec2,
    /// Negative while it waits to start.
    age: f32,
    ttl: f32,
    color: Color,
    look: Look,
}

enum Look {
    /// A ring widening from `from` to `to`, fast and then slow.
    Wave { from: f32, to: f32 },
    /// A bright star that shrinks away.
    Flash { size: f32 },
    /// A melee swipe, facing `dir`.
    Slash { dir: Vec2, reach: f32 },
    /// The flame at a gun's mouth.
    Muzzle { dir: Vec2, len: f32 },
    /// A finished structure's outline, rippling out.
    Ripple { size: f32 },
    /// Scorched ground where a building stood, fading slowly.
    Stain { size: f32 },
}

struct Particle {
    pos: Vec2,
    vel: Vec2,
    /// How fast it slows: the share of its speed lost per second, roughly.
    drag: f32,
    age: f32,
    ttl: f32,
    color: Color,
    mote: Mote,
}

enum Mote {
    /// A streak along its motion.
    Spark,
    /// A glowing speck.
    Ember { size: f32 },
    /// A puff that grows as it fades.
    Smoke { from: f32, to: f32 },
    /// A piece of a broken outline, tumbling: the segment is `pos ± half`,
    /// turned by `angle`.
    Shard { half: Vec2, angle: f32, spin: f32 },
}

/// A colour, run hot: halfway to white.
fn hot(color: Color) -> Color {
    color.mix(&Color::WHITE, 0.5)
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

/// A projectile's drawn position and heading, between ticks.
fn flight(m: &Match, p: &Projectile) -> (Vec2, Vec2) {
    let at = m.between(p.prev, p.pos);
    let dir = match p.shot {
        // A missile's heading is its velocity, in its local frame.
        Shot::Missile { .. } => {
            let radial = at.normalize_or(Vec2::X);
            (radial.perp() * p.vel.0 as f32 + radial * p.vel.1 as f32).normalize_or(radial)
        }
        _ => (to_vec2(p.pos) - to_vec2(p.prev)).normalize_or(Vec2::X),
    };
    (at, dir)
}

pub fn collect(
    time: Res<Time>,
    settings: Res<Settings>,
    mut fx: ResMut<Fx>,
    mut m: ResMut<Match>,
    shapes: Res<Shapes>,
) {
    let dt = time.delta_secs();
    let fx = &mut *fx;

    // Age everything, and let it move.
    fx.marks.retain_mut(|mk| {
        mk.age += dt;
        mk.age < mk.ttl
    });
    fx.particles.retain_mut(|p| {
        p.age += dt;
        let slow = (-p.drag * dt).exp();
        p.vel *= slow;
        p.pos += p.vel * dt;
        // The ring's slow circulation, carrying smoke and embers the way
        // everything on it moves.
        let wind = match p.mote {
            Mote::Smoke { .. } => RING_WIND,
            Mote::Ember { .. } => RING_WIND * 0.5,
            _ => 0.0,
        };
        if wind != 0.0 {
            p.pos += p.pos.normalize_or(Vec2::X).perp() * (wind * dt);
        }
        if let Mote::Shard { angle, spin, .. } = &mut p.mote {
            *angle += *spin * dt;
            *spin *= slow;
        }
        p.age < p.ttl
    });
    fx.hurt.retain_mut(|(_, f)| {
        *f -= dt / HURT_SECS;
        *f > 0.0
    });

    fx.sight = Sight::of(&m);
    let me = m.me;
    // With calm effects the events are read and dropped: the hurt flash on
    // the health rings stays, the theatre does not.
    let events = std::mem::take(&mut m.events);
    let theatre = settings.effects;
    if theatre {
        for event in events {
        match event {
            Event::Shot {
                from,
                to,
                owner,
                kind,
            } if fx.sight.sees(from) || fx.sight.sees(to) => {
                fx.shot(
                    to_vec2(from),
                    to_vec2(to),
                    kind,
                    palette::seat(owner as usize),
                );
            }
            Event::Impact {
                at, owner, kind, ..
            } if fx.sight.sees(at) => {
                fx.impact(to_vec2(at), kind, palette::seat(owner as usize));
            }
            // My own losses show even where nothing of mine still sees.
            Event::Died { owner, kind, pos } if fx.sight.sees(pos) || me == Some(owner) => {
                fx.died(&shapes, to_vec2(pos), kind, palette::seat(owner as usize));
            }
            Event::Blast { at, owner, radius } if fx.sight.sees(at) || me == Some(owner) => {
                let r = (radius / UNIT) as f32;
                fx.blast(to_vec2(at), r, palette::seat(owner as usize));
            }
            Event::Completed { owner, kind, pos } if fx.sight.sees(pos) => {
                let size = (kind.stats().radius / UNIT) as f32 + 4.0;
                fx.mark(
                    to_vec2(pos),
                    0.6,
                    palette::seat(owner as usize),
                    Look::Ripple { size },
                );
            }
            _ => {}
        }
        }
    }

    // Whatever lost health since last frame flashes, and moving units lay
    // down a wake.
    let mut health = Vec::with_capacity(m.sim.entities.len());
    for e in &m.sim.entities {
        let before = fx
            .health
            .binary_search_by_key(&e.id, |(id, _)| *id)
            .map(|i| fx.health[i].1);
        if before.is_ok_and(|hp| e.hp < hp) && m.sees(e) {
            match fx.hurt.iter_mut().find(|(id, _)| *id == e.id) {
                Some((_, f)) => *f = 1.0,
                None => fx.hurt.push((e.id, 1.0)),
            }
        }
        health.push((e.id, e.hp));
        if settings.effects && e.kind.is_mobile() && e.hp > 0 && m.sees(e) {
            lay_wake(&mut fx.wakes, e.id, m.draw_pos(e));
        }
    }
    fx.health = health;
    fx.wakes.retain(|w| m.sim.get(w.id).is_some());

    // Missiles leave smoke, plasma sheds sparks.
    if !theatre {
        return;
    }
    fx.trail_clock = (fx.trail_clock + dt).min(3.0 / TRAIL_HZ);
    while fx.trail_clock >= 1.0 / TRAIL_HZ {
        fx.trail_clock -= 1.0 / TRAIL_HZ;
        for p in &m.sim.projectiles {
            if !fx.sight.sees(p.pos) {
                continue;
            }
            let (at, dir) = flight(&m, p);
            match p.shot {
                Shot::Missile { .. } => {
                    let vel = -dir * 8.0 + fx.rng.dir() * 3.0;
                    let mote = Mote::Smoke { from: 1.2, to: 4.5 };
                    let ttl = fx.rng.range(0.6, 1.0);
                    fx.particle(at - dir * 3.0, vel, 1.5, ttl, SMOKE, mote);
                }
                Shot::Plasma { .. } if fx.rng.next() < 0.4 => {
                    let color = hot(palette::seat(p.owner as usize));
                    let vel = fx.rng.dir() * 12.0;
                    fx.particle(at, vel, 2.0, 0.4, color, Mote::Ember { size: 0.8 });
                }
                _ => {}
            }
        }
    }

    // Badly damaged buildings smoke, and the worst of them spark.
    fx.burn_clock = (fx.burn_clock + dt).min(2.0 / BURN_HZ);
    while fx.burn_clock >= 1.0 / BURN_HZ {
        fx.burn_clock -= 1.0 / BURN_HZ;
        for e in &m.sim.entities {
            let share = e.hp as f32 / e.max_hp() as f32;
            if !e.kind.is_structure() || !e.complete || share >= 0.5 || !m.sees(e) {
                continue;
            }
            let at = to_vec2(e.pos);
            let r = (e.radius() / UNIT) as f32;
            if fx.rng.next() < 0.6 {
                fx.smoke(at, 1, (r * 0.3, r * 0.9), 6.0, (1.0, 1.8));
            }
            if share < 0.25 && fx.rng.next() < 0.5 {
                let spot = at + fx.rng.dir() * r * 0.5;
                fx.embers(spot, 1, (10.0, 30.0), FLAME);
            }
        }
    }

    if fx.particles.len() > MAX_PARTICLES {
        let excess = fx.particles.len() - MAX_PARTICLES;
        fx.particles.drain(..excess);
    }
}

/// Record `at` as the newest point of `id`'s wake; a unit standing still
/// retracts its wake from the tail instead.
fn lay_wake(wakes: &mut Vec<Wake>, id: EntityId, at: Vec2) {
    match wakes.binary_search_by_key(&id, |w| w.id) {
        Ok(i) => {
            let w = &mut wakes[i];
            let moving = w.len == 0 || w.point(w.len - 1).distance(at) >= WAKE_STEP;
            if moving {
                w.points[w.head] = at;
                w.head = (w.head + 1) % WAKE_POINTS;
                w.len = (w.len + 1).min(WAKE_POINTS);
            } else {
                // Standing still: the wake retracts behind it.
                w.len -= 1;
            }
        }
        Err(i) => {
            wakes.insert(
                i,
                Wake {
                    id,
                    points: [at; WAKE_POINTS],
                    len: 1,
                    head: 1 % WAKE_POINTS,
                },
            );
        }
    }
}

/// Wakes drawn under the shapes: a fading line through the unit's recent
/// positions, in its own colour.
fn wakes(settings: Res<Settings>, fx: Res<Fx>, m: Res<Match>, mut gizmos: Gizmos) {
    if !settings.effects {
        return;
    }
    for w in &fx.wakes {
        let Some(e) = m.sim.get(w.id) else {
            continue;
        };
        if !m.sees(e) || w.len < 2 {
            continue;
        }
        let color = palette::seat(e.owner as usize);
        for j in 0..w.len - 1 {
            let a = (j + 1) as f32 / w.len as f32;
            gizmos.line_2d(w.point(j), w.point(j + 1), color.with_alpha(0.22 * a));
        }
    }
}

/// Points along an arc around `center`, from angle `from` over `span`.
fn arc(center: Vec2, radius: f32, from: f32, span: f32) -> impl Iterator<Item = Vec2> {
    let n = ((span.abs() * radius / 3.0).ceil() as usize).clamp(4, 48);
    (0..=n).map(move |k| center + Vec2::from_angle(from + span * k as f32 / n as f32) * radius)
}

fn circle(gizmos: &mut Gizmos<impl GizmoConfigGroup>, at: Vec2, r: f32, color: Color) {
    let resolution = (r * 1.5).clamp(8.0, 96.0) as u32;
    gizmos
        .circle_2d(Isometry2d::from_translation(at), r, color)
        .resolution(resolution);
}

fn draw(
    mut gizmos: Gizmos,
    mut glow: Gizmos<GlowGizmos>,
    time: Res<Time>,
    fx: Res<Fx>,
    m: Res<Match>,
) {
    let now = time.elapsed_secs();

    // In flight.
    for (n, p) in m.sim.projectiles.iter().enumerate() {
        if !fx.sight.sees(p.pos) {
            continue;
        }
        let (at, dir) = flight(&m, p);
        let color = palette::seat(p.owner as usize);
        let hot = hot(color);
        // Each shot pulses and flickers out of step with the others.
        let phase = n as f32 * 1.7;
        match p.shot {
            Shot::Melee => {}
            Shot::Bullet { .. } => {
                let step = (to_vec2(p.pos) - to_vec2(p.prev)).length();
                let len = (step * 0.7).clamp(4.0, 12.0);
                glow.line_2d(at - dir * len * 1.4, at, color.with_alpha(0.35));
                gizmos.line_2d(at - dir * len, at, hot);
            }
            Shot::Plasma { .. } => {
                // It swells as it flies, and throbs.
                let flown = (p.age as f32 + m.alpha) * TICK_SECS;
                let grown = (flown / 1.2).min(1.0);
                let r = (2.5 + 4.5 * grown) * (1.0 + 0.12 * (now * 20.0 + phase).sin());
                circle(&mut glow, at, r * 1.9, color.with_alpha(0.18));
                circle(&mut glow, at, r * 1.25, color.with_alpha(0.35));
                circle(&mut gizmos, at, r, hot.with_alpha(0.9));
                circle(&mut gizmos, at, r * 0.45, Color::WHITE.with_alpha(0.9));
                for k in 0..3 {
                    let from = now * 5.0 + phase + k as f32 * TAU / 3.0;
                    gizmos.linestrip_2d(arc(at, r * 1.4, from, 0.9), hot.with_alpha(0.6));
                }
            }
            Shot::Missile { .. } => {
                let side = dir.perp() * 2.2;
                let (nose, tail) = (at + dir * 4.0, at - dir * 3.0);
                gizmos.linestrip_2d([tail + side, nose, tail - side, tail + side], hot);
                let flicker = 0.7 + 0.3 * (now * 40.0 + phase).sin();
                gizmos.line_2d(tail, tail - dir * 5.0 * flicker, FLAME);
                circle(&mut glow, tail, 2.5, FLAME.with_alpha(0.4));
            }
        }
    }

    for p in &fx.particles {
        let t = p.age / p.ttl;
        let fade = 1.0 - t;
        match p.mote {
            Mote::Spark => {
                let len = (p.vel.length() * 0.05).max(1.5);
                let tail = p.pos - p.vel.normalize_or(Vec2::X) * len;
                gizmos.line_2d(tail, p.pos, p.color.with_alpha(fade));
                glow.line_2d(tail, p.pos, p.color.with_alpha(0.3 * fade));
            }
            Mote::Ember { size } => {
                let r = size * (1.0 - 0.5 * t);
                circle(&mut gizmos, p.pos, r, p.color.with_alpha(fade));
                circle(&mut glow, p.pos, r, p.color.with_alpha(0.25 * fade));
            }
            Mote::Smoke { from, to } => {
                let r = from + (to - from) * ease_out(t);
                circle(
                    &mut gizmos,
                    p.pos,
                    r,
                    p.color.with_alpha(0.3 * fade.powf(1.5)),
                );
            }
            Mote::Shard { half, angle, .. } => {
                let half = Vec2::from_angle(angle).rotate(half);
                // Whole for a while, then fading.
                let alpha = (fade / 0.4).min(1.0);
                gizmos.line_2d(p.pos - half, p.pos + half, p.color.with_alpha(alpha));
                glow.line_2d(p.pos - half, p.pos + half, p.color.with_alpha(0.25 * fade));
            }
        }
    }

    for mk in fx.marks.iter().filter(|mk| mk.age >= 0.0) {
        let t = mk.age / mk.ttl;
        let fade = 1.0 - t;
        let at = mk.at;
        match mk.look {
            Look::Wave { from, to } => {
                let r = from + (to - from) * ease_out(t);
                circle(&mut gizmos, at, r, mk.color.with_alpha(fade.powf(1.5)));
                circle(&mut glow, at, r, mk.color.with_alpha(0.3 * fade));
            }
            Look::Flash { size } => {
                let s = size * (1.0 - 0.6 * t);
                for k in 0..8 {
                    let ray = Vec2::from_angle(0.3 + k as f32 * TAU / 8.0);
                    let len = if k % 2 == 0 { s } else { s * 0.5 };
                    gizmos.line_2d(
                        at + ray * s * 0.15,
                        at + ray * len,
                        mk.color.with_alpha(fade),
                    );
                }
                circle(&mut glow, at, s * 0.6, mk.color.with_alpha(0.5 * fade));
                circle(&mut gizmos, at, s * 0.3, Color::WHITE.with_alpha(fade));
            }
            Look::Slash { dir, reach } => {
                // The swipe sweeps across, then fades.
                let span = 1.9 * (t * 3.0).min(1.0);
                let from = dir.to_angle() - 0.95;
                gizmos.linestrip_2d(arc(at, reach, from, span), mk.color.with_alpha(fade));
                glow.linestrip_2d(arc(at, reach, from, span), mk.color.with_alpha(0.3 * fade));
            }
            Look::Muzzle { dir, len } => {
                for (turn, share) in [(0.0, 1.0), (0.4, 0.55), (-0.4, 0.55)] {
                    let ray = Vec2::from_angle(turn).rotate(dir) * len * share;
                    gizmos.line_2d(at, at + ray, mk.color.with_alpha(fade));
                }
                circle(
                    &mut glow,
                    at + dir * len * 0.3,
                    len * 0.5,
                    FLAME.with_alpha(0.5 * fade),
                );
            }
            Look::Ripple { size } => {
                circle(&mut gizmos, at, size + 10.0 * t, mk.color.with_alpha(fade));
            }
            Look::Stain { size } => {
                // Ash in the player's colour, most of the way to black: it
                // dims whatever is drawn over it, like a scorch shadow.
                let ash = mk.color.mix(&Color::BLACK, 0.78);
                let linger = fade * fade;
                circle(&mut gizmos, at, size, ash.with_alpha(0.16 * linger));
                gizmos
                    .circle_2d(Isometry2d::from_translation(at), size, ash.with_alpha(0.1 * linger))
                    .resolution(24);
            }
        }
    }
}
