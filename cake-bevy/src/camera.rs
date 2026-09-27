//! The camera: the whole ring fits the window, and the view is turned so my
//! HQ sits at the bottom, with my two neighbours to the left and right.

use bevy::camera::ScalingMode;
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::{AppState, Match};

/// World units that always fit the window: the ring plus a margin.
const FRAME: f32 = 1080.0;
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

/// The cursor, in viewport pixels and in world units.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct Cursor {
    pub viewport: Option<Vec2>,
    pub world: Option<Vec2>,
}

pub fn plugin(app: &mut App) {
    app.init_resource::<Rig>()
        .init_resource::<Cursor>()
        .add_systems(Startup, spawn)
        .add_systems(OnEnter(AppState::Game), face_home)
        .add_systems(OnEnter(AppState::Lobby), |mut rig: ResMut<Rig>| *rig = Rig::default())
        .add_systems(
            Update,
            (controls.run_if(in_state(AppState::Game)), apply, track_cursor).chain(),
        );
}

fn spawn(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        MainCamera,
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::AutoMin {
                min_width: FRAME,
                min_height: FRAME,
            },
            ..OrthographicProjection::default_2d()
        }),
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
    if lines != 0.0 {
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

fn apply(rig: Res<Rig>, mut camera: Query<(&mut Transform, &mut Projection), With<MainCamera>>) {
    let Ok((mut tf, mut projection)) = camera.single_mut() else {
        return;
    };
    tf.translation = rig.pan.extend(tf.translation.z);
    tf.rotation = Quat::from_rotation_z(rig.rotation);
    if let Projection::Orthographic(ortho) = projection.as_mut()
        && ortho.scale != rig.zoom
    {
        ortho.scale = rig.zoom;
    }
}

fn track_cursor(
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    mut cursor: ResMut<Cursor>,
) {
    let viewport = window.cursor_position();
    let world = viewport.and_then(|p| {
        let (camera, tf) = camera.single().ok()?;
        camera.viewport_to_world_2d(tf, p).ok()
    });
    *cursor = Cursor { viewport, world };
}
