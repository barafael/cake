//! The background: a procedural nebula that slowly wavers.
//!
//! The algorithm is btl's (github.com/barafael/btl, `btl-shared/src/nebula.rs`),
//! itself after randomfart. Three math expressions, one per colour channel,
//! are grown at random from a weighted grammar, compiled to stack-machine
//! bytecode, and evaluated per pixel over `(x, y, t)` in `[-1, 1]`. Animating
//! `t` along a slow sine makes the whole field drift back and forth.
//!
//! The seed comes from the room name, so everyone in a room shares one sky,
//! as btl's server hands every client the same seed.
//!
//! The nebula fills the whole circle and stays put while the map zooms: a
//! dark disk with the nebula over it on the backdrop layer, and the same
//! texture again on the ring beyond the map (overlay layer), mapped so the two
//! meet without a seam. The ring copy also hides the map where it would
//! spill past its edge when zoomed in.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::sprite_render::{AlphaMode2d, ColorMaterial, MeshMaterial2d};
use cake_net::RoomId;

use crate::camera::{BACKDROP_LAYER, OVERLAY_LAYER};
use crate::chrome::{PLAY_RADIUS, RADIUS};
use crate::{palette, ringmesh};

/// Texture resolution. It is stretched over the whole circle, and a nebula
/// is soft anyway, so low resolution is fine (and cheap to re-render).
const SIZE: u32 = 128;
/// One full back-and-forth of the waver, in seconds.
const WAVER_PERIOD_SECS: f32 = 60.0;
/// Re-render at most this often. Over a one-minute cycle `t` moves at most
/// ~0.01 between renders, below what the eye catches at this opacity.
const RENDER_INTERVAL_SECS: f32 = 0.1;
/// btl's tint: faint violet, so the polylines stay the brightest thing.
const TINT: Color = Color::srgba(0.5, 0.4, 0.8, 0.14);
/// Expression tree depth.
const DEPTH: u32 = 5;

pub fn plugin(app: &mut App) {
    app.add_systems(Startup, spawn).add_systems(Update, waver);
}

#[derive(Resource)]
struct Nebula {
    programs: Programs,
    image: Handle<Image>,
    /// When the texture was last rendered, in elapsed seconds.
    rendered_at: f32,
}

fn spawn(
    mut commands: Commands,
    room: Option<Res<RoomId>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let seed = room.map_or(0x5EED, |r| cake_core::hash::fnv1a(r.0.as_bytes()));
    let programs = Programs::generate(seed);
    let image = images.add(Image::new(
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        render(&programs, 0.0),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    ));
    // The backdrop: the dark disk, and the nebula over it.
    let backdrop = RenderLayers::layer(BACKDROP_LAYER);
    commands.spawn((
        Mesh2d(meshes.add(Circle::new(RADIUS).mesh().resolution(256))),
        MeshMaterial2d(materials.add(ColorMaterial::from_color(palette::BACKGROUND))),
        Transform::from_xyz(0.0, 0.0, -20.0),
        backdrop.clone(),
    ));
    commands.spawn((
        Sprite {
            image: image.clone(),
            custom_size: Some(Vec2::splat(2.0 * RADIUS)),
            color: TINT,
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, -10.0),
        backdrop,
    ));
    // The same again on the ring beyond the map, over whatever the map drew.
    let overlay = RenderLayers::layer(OVERLAY_LAYER);
    let ring = meshes.add(ringmesh::ring(PLAY_RADIUS, RADIUS, RADIUS));
    commands.spawn((
        Mesh2d(ring.clone()),
        MeshMaterial2d(materials.add(ColorMaterial::from_color(palette::BACKGROUND))),
        Transform::from_xyz(0.0, 0.0, -30.0),
        overlay.clone(),
    ));
    commands.spawn((
        Mesh2d(ring),
        MeshMaterial2d(materials.add(ColorMaterial {
            color: TINT,
            alpha_mode: AlphaMode2d::Blend,
            texture: Some(image.clone()),
            ..default()
        })),
        Transform::from_xyz(0.0, 0.0, -29.0),
        overlay,
    ));
    commands.insert_resource(Nebula {
        programs,
        image,
        rendered_at: 0.0,
    });
}

/// Re-render the texture as `t` moves. Both the backdrop sprite and the ring
/// material sample the same image, which is rewritten in place on the GPU,
/// so both follow it.
fn waver(time: Res<Time>, mut nebula: ResMut<Nebula>, mut images: ResMut<Assets<Image>>) {
    let now = time.elapsed_secs();
    if now - nebula.rendered_at < RENDER_INTERVAL_SECS {
        return;
    }
    nebula.rendered_at = now;
    let t = (now * std::f32::consts::TAU / WAVER_PERIOD_SECS).sin();
    let pixels = render(&nebula.programs, t);
    if let Some(mut image) = images.get_mut(&nebula.image) {
        match &mut image.data {
            Some(data) if data.len() == pixels.len() => data.copy_from_slice(&pixels),
            slot => *slot = Some(pixels),
        }
    }
}

/// The RGBA texture at time `t`: opaque over the circle, clear in the
/// corners outside it (which window mode shows).
fn render(programs: &Programs, t: f32) -> Vec<u8> {
    let mut pixels = vec![0u8; (SIZE * SIZE * 4) as usize];
    // One texel of soft edge.
    let texel = 2.0 / SIZE as f32;
    for (i, px) in pixels.chunks_mut(4).enumerate() {
        let i = i as u32;
        // Sample texel centres. Image rows run top to bottom; y runs up.
        let y = 1.0 - ((i / SIZE) as f32 + 0.5) / SIZE as f32 * 2.0;
        let x = ((i % SIZE) as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0;
        let dist = (x * x + y * y).sqrt();
        let alpha = ((1.0 - dist) / texel + 0.5).clamp(0.0, 1.0);
        if alpha == 0.0 {
            // Outside the circle: never seen, so not worth evaluating.
            continue;
        }
        px[0] = channel(eval(&programs.r, x, y, t));
        px[1] = channel(eval(&programs.g, x, y, t));
        px[2] = channel(eval(&programs.b, x, y, t));
        px[3] = (alpha * 255.0) as u8;
    }
    pixels
}

/// Map `[-1, 1]` to a byte.
fn channel(v: f32) -> u8 {
    (((v + 1.0) * 0.5).clamp(0.0, 1.0) * 255.0) as u8
}

// ---- Bytecode --------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    X,
    Y,
    T,
    Num(f32),
    Add,
    Mult,
    Sqrt,
    Abs,
    Sin,
    Mix,
}

/// Evaluate a compiled program at `(x, y, t)`.
fn eval(ops: &[Op], x: f32, y: f32, t: f32) -> f32 {
    let mut stack = [0f32; 32];
    let mut sp = 0usize;
    for &op in ops {
        match op {
            Op::X => {
                stack[sp] = x;
                sp += 1;
            }
            Op::Y => {
                stack[sp] = y;
                sp += 1;
            }
            Op::T => {
                stack[sp] = t;
                sp += 1;
            }
            Op::Num(n) => {
                stack[sp] = n;
                sp += 1;
            }
            Op::Abs => stack[sp - 1] = stack[sp - 1].abs(),
            Op::Sqrt => stack[sp - 1] = stack[sp - 1].abs().sqrt(),
            Op::Sin => stack[sp - 1] = (stack[sp - 1] * std::f32::consts::PI).sin(),
            Op::Add => {
                sp -= 1;
                stack[sp - 1] += stack[sp];
            }
            Op::Mult => {
                sp -= 1;
                stack[sp - 1] *= stack[sp];
            }
            Op::Mix => {
                sp -= 2;
                let w = ((stack[sp + 1] + 1.0) * 0.5).clamp(0.0, 1.0);
                stack[sp - 1] = stack[sp - 1] * (1.0 - w) + stack[sp] * w;
            }
        }
    }
    stack[0]
}

/// One compiled program per colour channel.
#[derive(Debug, PartialEq)]
struct Programs {
    r: Vec<Op>,
    g: Vec<Op>,
    b: Vec<Op>,
}

impl Programs {
    /// Same seed, same nebula.
    fn generate(seed: u64) -> Programs {
        // xorshift gets stuck at zero.
        let mut rng = Rng(seed.max(1));
        let compile = |rng: &mut Rng| {
            let mut ops = Vec::new();
            Expr::generate(rng, DEPTH).compile(&mut ops);
            ops
        };
        let r = compile(&mut rng);
        let g = compile(&mut rng);
        let b = compile(&mut rng);
        Programs { r, g, b }
    }
}

// ---- Expression grammar ----------------------------------------------------

enum Expr {
    X,
    Y,
    T,
    Num(f32),
    Add(Box<Expr>, Box<Expr>),
    Mult(Box<Expr>, Box<Expr>),
    Sqrt(Box<Expr>),
    Abs(Box<Expr>),
    Sin(Box<Expr>),
    Mix(Box<Expr>, Box<Expr>, Box<Expr>),
}

impl Expr {
    fn compile(&self, ops: &mut Vec<Op>) {
        match self {
            Expr::X => ops.push(Op::X),
            Expr::Y => ops.push(Op::Y),
            Expr::T => ops.push(Op::T),
            Expr::Num(n) => ops.push(Op::Num(*n)),
            Expr::Abs(e) => {
                e.compile(ops);
                ops.push(Op::Abs);
            }
            Expr::Sqrt(e) => {
                e.compile(ops);
                ops.push(Op::Sqrt);
            }
            Expr::Sin(e) => {
                e.compile(ops);
                ops.push(Op::Sin);
            }
            Expr::Add(a, b) => {
                a.compile(ops);
                b.compile(ops);
                ops.push(Op::Add);
            }
            Expr::Mult(a, b) => {
                a.compile(ops);
                b.compile(ops);
                ops.push(Op::Mult);
            }
            Expr::Mix(a, b, c) => {
                a.compile(ops);
                b.compile(ops);
                c.compile(ops);
                ops.push(Op::Mix);
            }
        }
    }

    /// btl's weights, tuned for smooth, flowing fields: plenty of sin, add
    /// and mix for undulation and blending, and no modulo, so no tiling.
    fn generate(rng: &mut Rng, depth: u32) -> Expr {
        const W_TERMINAL: u32 = 2;
        const W_ADD: u32 = 3;
        const W_MULT: u32 = 2;
        const W_SQRT: u32 = 1;
        const W_SIN: u32 = 3;
        const W_MIX: u32 = 2;
        const TOTAL: u32 = W_TERMINAL + W_ADD + W_MULT + W_SQRT + W_SIN + W_MIX;

        let roll = rng.next_u32() % TOTAL;
        if depth == 0 || roll < W_TERMINAL {
            return Self::terminal(rng);
        }
        let mut cursor = W_TERMINAL + W_ADD;
        if roll < cursor {
            return Expr::Add(
                Box::new(Self::generate(rng, depth - 1)),
                Box::new(Self::generate(rng, depth - 1)),
            );
        }
        cursor += W_MULT;
        if roll < cursor {
            return Expr::Mult(
                Box::new(Self::generate(rng, depth - 1)),
                Box::new(Self::generate(rng, depth - 1)),
            );
        }
        cursor += W_SQRT;
        if roll < cursor {
            return Expr::Sqrt(Box::new(Expr::Abs(Box::new(Self::generate(
                rng,
                depth - 1,
            )))));
        }
        cursor += W_SIN;
        if roll < cursor {
            return Expr::Sin(Box::new(Self::generate(rng, depth - 1)));
        }
        Expr::Mix(
            Box::new(Self::generate(rng, depth - 1)),
            Box::new(Self::generate(rng, depth - 1)),
            Box::new(Self::generate(rng, depth - 1)),
        )
    }

    fn terminal(rng: &mut Rng) -> Expr {
        match rng.next_u32() % 7 {
            0 => Expr::Num(rng.next_f32() * 2.0 - 1.0),
            1 => Expr::X,
            2 => Expr::Y,
            3 => Expr::Abs(Box::new(Expr::X)),
            4 => Expr::Abs(Box::new(Expr::Y)),
            5 => Expr::Sqrt(Box::new(Expr::Add(
                Box::new(Expr::Mult(Box::new(Expr::X), Box::new(Expr::X))),
                Box::new(Expr::Mult(Box::new(Expr::Y), Box::new(Expr::Y))),
            ))),
            _ => Expr::T,
        }
    }
}

/// btl's xorshift64.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 16) as u32
    }

    /// In `[0, 1)`.
    fn next_f32(&mut self) -> f32 {
        (self.next_u32() & 0x00FF_FFFF) as f32 / 16_777_216.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_grows_the_same_nebula() {
        assert_eq!(Programs::generate(42), Programs::generate(42));
        assert_ne!(Programs::generate(42), Programs::generate(43));
    }

    /// Depth 5 bounds the stack well inside the evaluator's 32 slots, for
    /// every seed; a program that overflowed would panic mid-frame.
    #[test]
    fn every_program_fits_the_stack_and_leaves_one_value() {
        for seed in 1..2000u64 {
            let p = Programs::generate(seed);
            for ops in [&p.r, &p.g, &p.b] {
                let mut depth: i32 = 0;
                let mut max = 0;
                for op in ops.iter() {
                    depth += match op {
                        Op::X | Op::Y | Op::T | Op::Num(_) => 1,
                        Op::Abs | Op::Sqrt | Op::Sin => 0,
                        Op::Add | Op::Mult => -1,
                        Op::Mix => -2,
                    };
                    assert!(depth >= 1, "seed {seed}: stack underflow");
                    max = max.max(depth);
                }
                assert_eq!(depth, 1, "seed {seed}");
                assert!(max <= 32, "seed {seed}: needs {max} slots");
            }
        }
    }

    #[test]
    fn the_texture_is_opaque_over_the_circle_and_clear_at_the_corners() {
        let pixels = render(&Programs::generate(7), 0.3);
        assert_eq!(pixels.len(), (SIZE * SIZE * 4) as usize);
        let alpha = |x: u32, y: u32| pixels[((y * SIZE + x) * 4 + 3) as usize];
        assert_eq!(alpha(SIZE / 2, SIZE / 2), 255);
        assert_eq!(alpha(0, 0), 0);
        assert_eq!(alpha(SIZE - 1, SIZE - 1), 0);
    }

    /// Not an assertion: how long one re-render takes in this build.
    #[test]
    fn render_cost() {
        let p = Programs::generate(fnv(b"cost"));
        let start = std::time::Instant::now();
        for k in 0..10 {
            std::hint::black_box(render(&p, k as f32 / 10.0));
        }
        eprintln!("one render: {:?}", start.elapsed() / 10);
    }

    fn fnv(b: &[u8]) -> u64 {
        cake_core::hash::fnv1a(b)
    }

    #[test]
    fn time_moves_the_field() {
        // Some seed among the first few must use `t`; each one that does
        // must change with it.
        let moved = (1..20u64).any(|seed| {
            let p = Programs::generate(seed);
            render(&p, -0.5) != render(&p, 0.5)
        });
        assert!(moved);
    }
}
