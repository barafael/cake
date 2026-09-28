//! Three cameras, drawn in this order into one shared texture.
//!
//! The **backdrop** camera is fixed: the circle's dark disk and the nebula.
//!
//! The **main** camera looks at the map: the whole ring fits the window, the
//! view is turned so my HQ sits at the bottom with my neighbours to the left
//! and right, and it zooms and pans.
//!
//! The **overlay** camera is fixed too, and draws on top: the ring beyond the
//! map, the window chrome, players' names and the UI.
//!
//! The fixed cameras measure in "circle units": the circle (radius 600)
//! always fits the window. At zoom 1 the map camera agrees, so the map's ring
//! sits exactly inside the frame.
//!
//! When the app opens, everything grows out of the centre: see [`Opening`].

use bevy::camera::{CameraOutputMode, ScalingMode};
use bevy::camera::visibility::RenderLayers;
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::render::render_resource::BlendState;
use bevy::ui::IsDefaultUiCamera;
use bevy::window::PrimaryWindow;

use crate::chrome::{PointerBlocked, PointerSet, RADIUS};
use crate::settings::{Settings, DisplayMode};
use crate::{AppState, Match, palette};

/// Units that always fit the window: the whole circle, frame included.
const FRAME: f32 = 2.0 * RADIUS;
/// The render layer of the chrome and anything else fixed to the window.
pub const OVERLAY_LAYER: usize = 1;
/// The render layer of the circle's backdrop.
pub const BACKDROP_LAYER: usize = 2;
const OPENING_SECS: f32 = 1.0;

/// The opening animation: the circle and all it holds grow from a point in
/// the centre to full size.
#[derive(Resource, Debug, Default)]
pub struct Opening {
    t: f32,
}

impl Opening {
    /// How big everything is: 0 at first, 1 when open. Eased out, so it
    /// starts quickly and settles.
    pub fn scale(&self) -> f32 {
        let t = self.t.clamp(0.0, 1.0);
        (1.0 - (1.0 - t).powi(3)).max(0.01)
    }

    pub fn done(&self) -> bool {
        self.t >= 1.0
    }
}

const MIN_ZOOM: f32 = 0.15;
const MAX_ZOOM: f32 = 1.5;
/// Keyboard pan speed, in screen-heights per second.
const PAN_SPEED: f32 = 0.8;

/// Where the camera looks: pan (world units), zoom (scale; 1 fits the ring)
/// and rotation (radians).
#[derive(Resource, Clone, Copy, Debug)]
pub struct Rig {
    pub pan: Vec2,
    pub zoom: f32,
    pub rotation: f32,
}

impl Default for Rig {
    fn default() -> Self {
        Rig {
            pan: Vec2::ZERO,
            zoom: 1.0,
            rotation: 0.0,
        }
    }
}

#[derive(Component)]
pub struct MainCamera;

#[derive(Component)]
pub struct OverlayCamera;

#[derive(Component)]
pub struct BackdropCamera;

/// The cursor: in viewport pixels, in world units, and in circle units.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct Cursor {
    pub viewport: Option<Vec2>,
    pub world: Option<Vec2>,
    pub ui: Option<Vec2>,
}

pub fn plugin(app: &mut App) {
    app.init_resource::<Rig>()
        .init_resource::<Cursor>()
        .init_resource::<Opening>()
        .add_systems(Startup, spawn)
        .add_systems(OnEnter(AppState::Game), face_home)
        .add_systems(OnEnter(AppState::Lobby), |mut rig: ResMut<Rig>| *rig = Rig::default())
        .add_systems(Update, track_cursor.in_set(PointerSet))
        .add_systems(
            Update,
            (open, controls.run_if(in_state(AppState::Game)), apply)
                .chain()
                .after(PointerSet),
        );
}

fn fitted() -> Projection {
    Projection::Orthographic(OrthographicProjection {
        scaling_mode: ScalingMode::AutoMin {
            min_width: FRAME,
            min_height: FRAME,
        },
        ..OrthographicProjection::default_2d()
    })
}

/// What the backdrop clears to: nothing around the circle in cake mode, the
/// background colour in window mode.
pub fn backdrop_clear(mode: DisplayMode) -> ClearColorConfig {
    ClearColorConfig::Custom(match mode {
        DisplayMode::Cake => Color::NONE,
        DisplayMode::Window => palette::BACKGROUND,
    })
}

fn spawn(mut commands: Commands, settings: Res<Settings>) {
    // All three render into the window's shared intermediate texture, and
    // only the overlay copies it to the window, replacing what is there. That
    // keeps the transparent pixels that make cake mode's corners
    // see-through.
    commands.spawn((
        Camera2d,
        Camera {
            order: -1,
            clear_color: backdrop_clear(settings.mode),
            output_mode: CameraOutputMode::Skip,
            ..default()
        },
        BackdropCamera,
        RenderLayers::layer(BACKDROP_LAYER),
        fitted(),
    ));
    commands.spawn((
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::None,
            output_mode: CameraOutputMode::Skip,
            ..default()
        },
        MainCamera,
        fitted(),
    ));
    commands.spawn((
        Camera2d,
        Camera {
            order: 1,
            clear_color: ClearColorConfig::None,
            // Replace, not blend: as the second camera on the window it would
            // otherwise be blended over by default, and the chrome's
            // transparent pixels would never reach the window.
            output_mode: CameraOutputMode::Write {
                blend_state: Some(BlendState::REPLACE),
                clear_color: ClearColorConfig::None,
            },
            ..default()
        },
        OverlayCamera,
        IsDefaultUiCamera,
        RenderLayers::layer(OVERLAY_LAYER),
        fitted(),
    ));
}

/// Turn the view so my sector is at 6 o'clock. Watchers get the plain view.
pub fn face_home(mut rig: ResMut<Rig>, m: Option<Res<Match>>) {
    *rig = Rig::default();
    let Some(m) = m else {
        return;
    };
    if let Some(me) = m.me {
        let angle = cake_core::geom::sector_center(me as usize, m.sim.seats()).to_radians() as f32;
        // A world point at angle θ appears at θ − rotation; put mine at −90°.
        rig.rotation = angle + std::f32::consts::FRAC_PI_2;
    }
}

#[allow(clippy::too_many_arguments)]
fn controls(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    motion: Res<bevy::input::mouse::AccumulatedMouseMotion>,
    cursor: Res<Cursor>,
    blocked: Res<PointerBlocked>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut rig: ResMut<Rig>,
) {
    let units_per_px = FRAME * rig.zoom / window.height().min(window.width()).max(1.0);
    let rot = Mat2::from_angle(rig.rotation);

    // Wheel: zoom about the point under the cursor.
    let lines = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
    };
    if lines != 0.0 && !blocked.0 {
        let old = rig.zoom;
        let new = (old * 0.88f32.powf(lines)).clamp(MIN_ZOOM, MAX_ZOOM);
        if let Some(at) = cursor.world {
            rig.pan = at + (rig.pan - at) * (new / old);
        }
        rig.zoom = new;
    }

    // Middle-drag pans.
    if buttons.pressed(MouseButton::Middle) && motion.delta != Vec2::ZERO {
        let d = Vec2::new(-motion.delta.x, motion.delta.y) * units_per_px;
        rig.pan += rot * d;
    }

    // Arrow keys pan.
    let mut dir = Vec2::ZERO;
    for (key, v) in [
        (KeyCode::ArrowLeft, Vec2::NEG_X),
        (KeyCode::ArrowRight, Vec2::X),
        (KeyCode::ArrowUp, Vec2::Y),
        (KeyCode::ArrowDown, Vec2::NEG_Y),
    ] {
        if keys.pressed(key) {
            dir += v;
        }
    }
    if dir != Vec2::ZERO {
        let step = PAN_SPEED * FRAME * rig.zoom * time.delta_secs();
        rig.pan += rot * dir.normalize() * step;
    }

    // Home: back to the fitted view.
    if keys.just_pressed(KeyCode::KeyH) {
        let rotation = rig.rotation;
        *rig = Rig {
            rotation,
            ..Rig::default()
        };
    }
}

/// Advance the opening. Each frame counts at most a thirtieth of a second,
/// so the slow first frames (shaders compiling) cannot swallow it.
fn open(time: Res<Time>, mut opening: ResMut<Opening>) {
    if !opening.done() {
        opening.t = (opening.t + time.delta_secs().min(1.0 / 30.0) / OPENING_SECS).min(1.0);
    }
}

fn apply(
    rig: Res<Rig>,
    opening: Res<Opening>,
    mut main: Query<(&mut Transform, &mut Projection), With<MainCamera>>,
    // The overlay and the backdrop: every other camera is fixed.
    mut fixed: Query<&mut Projection, (With<Camera>, Without<MainCamera>)>,
) {
    let grow = opening.scale();
    if let Ok((mut tf, mut projection)) = main.single_mut() {
        tf.translation = rig.pan.extend(tf.translation.z);
        tf.rotation = Quat::from_rotation_z(rig.rotation);
        set_scale(&mut projection, rig.zoom / grow);
    }
    for mut projection in &mut fixed {
        set_scale(&mut projection, 1.0 / grow);
    }
}

fn set_scale(projection: &mut Projection, scale: f32) {
    if let Projection::Orthographic(ortho) = projection
        && ortho.scale != scale
    {
        ortho.scale = scale;
    }
}

fn track_cursor(
    window: Single<&Window, With<PrimaryWindow>>,
    main: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    overlay: Query<(&Camera, &GlobalTransform), With<OverlayCamera>>,
    mut cursor: ResMut<Cursor>,
) {
    let viewport = window.cursor_position();
    let through = |camera: Result<(&Camera, &GlobalTransform), _>| {
        let (camera, tf) = camera.ok()?;
        camera.viewport_to_world_2d(tf, viewport?).ok()
    };
    *cursor = Cursor {
        viewport,
        world: through(main.single()),
        ui: through(overlay.single()),
    };
}
