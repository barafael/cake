//! Four cameras, drawn in this order into one shared texture.
//!
//! The **backdrop** camera is fixed: the circle's dark disk, which fills the
//! round viewport whatever the view does.
//!
//! The **cake** camera draws what belongs to the circle besides the map: the
//! nebula, the players' names on the ring beyond the map, and everything
//! inside the ring: the readouts, the menus, the recap. It zooms and pans
//! with the map but does not turn: the names follow the view's rotation by
//! their own layout (see [`crate::arctext`]), and the sky and the words stay
//! upright.
//!
//! The **main** camera looks at the map: the whole ring fits the window, the
//! view is turned so my HQ sits at the bottom with my neighbours to the left
//! and right, and it zooms and pans.
//!
//! The **overlay** camera is fixed too, and draws on top: the mask outside the
//! circle and the window chrome. It is also the UI camera; UI nodes are laid
//! out in their own pixels, so the readouts in the middle, which are UI,
//! follow the cake camera by other means: [`UiScale`] zooms them and the
//! [`CircleBox`] they sit in is moved with the pan (see [`follow_ui`]).
//!
//! The fixed cameras measure in "circle units": the circle (radius 600)
//! always fits the window. At zoom 1 the cake and map cameras agree, so the
//! map, its names, its sky and its UI sit exactly inside the frame; zoomed,
//! they grow together behind the round viewport.
//!
//! When the app opens, everything grows out of the centre: see [`Opening`].

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{CameraOutputMode, ScalingMode};
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::render::render_resource::BlendState;
use bevy::window::PrimaryWindow;

use crate::chrome::{PointerBlocked, PointerSet, RADIUS};
use crate::settings::{DisplayMode, Settings};
use crate::{AppState, Match, palette};

/// Units that always fit the window: the whole circle, frame included.
const FRAME: f32 = 2.0 * RADIUS;
/// The render layer of the chrome and anything else fixed to the window.
pub const OVERLAY_LAYER: usize = 1;
/// The render layer of the circle's backdrop.
pub const BACKDROP_LAYER: usize = 2;
/// The render layer of what zooms with the map but is not drawn by it.
pub const CAKE_LAYER: usize = 3;
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

    /// Play it again from the start.
    pub fn restart(&mut self) {
        self.t = 0.0;
    }
}

const MIN_ZOOM: f32 = 0.15;
const MAX_ZOOM: f32 = 1.5;
/// Keyboard pan speed, in screen-heights per second.
const PAN_SPEED: f32 = 0.8;

/// Where the map camera looks, and the cake camera with it: pan (world
/// units), zoom (scale; 1 fits the ring) and rotation (radians).
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
pub struct CakeCamera;

#[derive(Component)]
pub struct OverlayCamera;

#[derive(Component)]
pub struct BackdropCamera;

/// A UI box the size of the circle, in circle units, whose top-left corner
/// is laid out at the window's centre: [`follow_ui`] moves it so that its
/// middle is where the cake camera shows the circle's centre, and so its
/// contents zoom and pan with the map. Spawn it with a [`UiTransform`].
#[derive(Component, Default, Clone)]
pub struct CircleBox;

/// The cursor: in viewport pixels; in world units, as the map sees it; in
/// cake units, zoomed and panned with the map but not turned, where the
/// menus and the recap are; and in circle units, fixed to the window, where
/// the chrome is.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct Cursor {
    pub viewport: Option<Vec2>,
    pub world: Option<Vec2>,
    pub cake: Option<Vec2>,
    pub ui: Option<Vec2>,
}

pub fn plugin(app: &mut App) {
    app.init_resource::<Rig>()
        .init_resource::<Cursor>()
        .init_resource::<Opening>()
        .add_systems(Startup, spawn)
        .add_systems(OnEnter(AppState::Game), face_home)
        .add_systems(OnEnter(AppState::Lobby), |mut rig: ResMut<Rig>| {
            *rig = Rig::default()
        })
        .add_systems(Update, track_cursor.in_set(PointerSet))
        .add_systems(
            Update,
            (
                open,
                controls.run_if(in_state(AppState::Game)),
                apply,
                follow_ui,
            )
                .chain()
                .after(PointerSet),
        );
}

/// Screen pixels per circle unit at the fitted view, where the circle fills
/// the window's shorter side.
pub fn px_per_unit(window: &Window) -> f32 {
    window.width().min(window.height()).max(1.0) / FRAME
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
    // All four render into the window's shared intermediate texture, and
    // only the overlay copies it to the window, replacing what is there. That
    // keeps the transparent pixels that make cake mode's corners
    // see-through.
    commands.spawn((
        Camera2d,
        Camera {
            order: -2,
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
            order: -1,
            clear_color: ClearColorConfig::None,
            output_mode: CameraOutputMode::Skip,
            ..default()
        },
        CakeCamera,
        RenderLayers::layer(CAKE_LAYER),
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
            // Replace, not blend: as a later camera on the window it would
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
    let units_per_px = rig.zoom / px_per_unit(&window);
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

#[allow(clippy::type_complexity)]
fn apply(
    rig: Res<Rig>,
    opening: Res<Opening>,
    mut main: Query<(&mut Transform, &mut Projection), With<MainCamera>>,
    mut cake: Query<(&mut Transform, &mut Projection), (With<CakeCamera>, Without<MainCamera>)>,
    // The overlay and the backdrop: every other camera is fixed.
    mut fixed: Query<&mut Projection, (With<Camera>, Without<MainCamera>, Without<CakeCamera>)>,
) {
    let grow = opening.scale();
    if let Ok((mut tf, mut projection)) = main.single_mut() {
        tf.translation = rig.pan.extend(tf.translation.z);
        tf.rotation = Quat::from_rotation_z(rig.rotation);
        set_scale(&mut projection, rig.zoom / grow);
    }
    if let Ok((mut tf, mut projection)) = cake.single_mut() {
        // The map camera's view without its turn: the pan as the screen sees
        // it.
        let pan = Mat2::from_angle(-rig.rotation) * rig.pan;
        tf.translation = pan.extend(tf.translation.z);
        set_scale(&mut projection, rig.zoom / grow);
    }
    for mut projection in &mut fixed {
        set_scale(&mut projection, 1.0 / grow);
    }
}

/// Take the UI along with the cake camera: scale it so that one UI pixel is
/// one unit of that camera's view, zoom and opening included, and move each
/// [`CircleBox`] with the pan, as the screen sees it.
fn follow_ui(
    window: Single<&Window, With<PrimaryWindow>>,
    rig: Res<Rig>,
    opening: Res<Opening>,
    mut scale: ResMut<UiScale>,
    mut boxes: Query<&mut UiTransform, With<CircleBox>>,
) {
    let s = px_per_unit(&window) * opening.scale() / rig.zoom;
    if scale.0 != s {
        scale.0 = s;
    }
    // The pan moves the world the other way on screen, and UI y runs down.
    let pan = Mat2::from_angle(-rig.rotation) * rig.pan;
    let translation = Val2::px(-RADIUS - pan.x, -RADIUS + pan.y);
    for mut tf in &mut boxes {
        if tf.translation != translation {
            tf.translation = translation;
        }
    }
}

fn set_scale(projection: &mut Projection, scale: f32) {
    if let Projection::Orthographic(ortho) = projection
        && ortho.scale != scale
    {
        ortho.scale = scale;
    }
}

pub fn track_cursor(
    window: Single<&Window, With<PrimaryWindow>>,
    main: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    cake: Query<(&Camera, &GlobalTransform), With<CakeCamera>>,
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
        cake: through(cake.single()),
        ui: through(overlay.single()),
    };
}
