//! The circle window's chrome.
//!
//! In [`WindowStyle::Circle`] the OS window is frameless and transparent, and
//! only a disk of radius [`RADIUS`] shows. The ring between [`PLAY_RADIUS`]
//! and [`RADIUS`] continues the nebula (see [`crate::nebula`]) and is the
//! window frame:
//!
//! - drag it to move the window;
//! - drag its outermost band to resize, in the direction of the edge grabbed;
//! - slices of the ring on its top arc, each with an icon, close, maximise,
//!   minimise, pin always-on-top, and switch to a normal window.
//!
//! Everything here is drawn on the overlay layer, whose camera is fixed: one
//! unit is one "circle unit", and the circle is always 1200 across however
//! the map is zoomed. Outside the circle a [`Punch`] mesh writes fully
//! transparent pixels, clearing whatever the map drew there.
//!
//! In [`WindowStyle::Windowed`] (and always on the web) the buttons and the
//! mask are hidden, and the OS draws the usual decorations.
//!
//! This module also owns the pointer's arbitration: [`PointerSet`] decides
//! where the pointer is and whether the map may have it ([`PointerBlocked`]),
//! and everything that reacts to the pointer runs after it.

use bevy::app::AppExit;
use bevy::asset::embedded_asset;
use bevy::camera::visibility::RenderLayers;
use bevy::math::CompassOctant;
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use bevy::sprite_render::{
    AlphaMode2d, ColorMaterial, Material2d, Material2dPlugin, MeshMaterial2d,
};
use bevy::window::{CursorEntered, PrimaryWindow, WindowLevel};

use crate::arctext::{Reserved, ReservedArcs};
use crate::camera::{BackdropCamera, Cursor, OVERLAY_LAYER, Opening, backdrop_clear};
use crate::ringmesh::{self, Slots};
use crate::settings::{self, Settings, WindowStyle};
use crate::palette;

/// The whole circle, in circle units.
pub const RADIUS: f32 = 600.0;
/// Inside this, the game; beyond it, the frame. The map's outer edge is at
/// 500, and the player-colour arcs just outside it stay visible.
pub const PLAY_RADIUS: f32 = 512.0;
/// The outermost band of the frame resizes instead of moving.
const RESIZE_BAND: f32 = 14.0;
/// The window buttons: slices of the whole ring across the top, left to
/// right, with a hairline between.
const BUTTONS: Slots = Slots {
    inner: PLAY_RADIUS,
    outer: RADIUS,
    centre: 90.0,
    width: 6.0,
    gap: 0.6,
    clockwise: true,
};
/// Half the size of a button's icon.
const SYMBOL: f32 = 12.0;

#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct ChromeGizmos;

/// A material that punches holes: it writes `(0, 0, 0, 0)` without blending.
/// `ColorMaterial` can't, because opaque mode forces alpha to 1, which
/// painted the corners black.
#[derive(Asset, TypePath, AsBindGroup, Clone, Default)]
pub struct Punch {}

impl Material2d for Punch {
    fn fragment_shader() -> ShaderRef {
        "embedded://cake_bevy/punch.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Opaque
    }
}

/// Shown in circle style only: the outside mask and the button slices.
#[derive(Component)]
struct ChromePiece;

/// A button's slice, whose fill shows hover and state.
#[derive(Component)]
struct Segment(ChromeButton);

/// The window buttons, left to right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChromeButton {
    Style,
    Pin,
    Minimize,
    Maximize,
    Close,
}

impl ChromeButton {
    const ALL: [ChromeButton; 5] = [
        ChromeButton::Style,
        ChromeButton::Pin,
        ChromeButton::Minimize,
        ChromeButton::Maximize,
        ChromeButton::Close,
    ];

    fn centre(self) -> Vec2 {
        BUTTONS.centre_of(self as usize, Self::ALL.len())
    }

    fn at(p: Vec2) -> Option<ChromeButton> {
        BUTTONS.at(p, Self::ALL.len()).map(|i| Self::ALL[i])
    }
}

/// Where the pointer is, and whether the map may have it. Everything that
/// reacts to the pointer runs after this set.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct PointerSet;

/// Is the pointer somewhere the map should not react to: a UI button, a
/// menu slot, the frame, or anywhere while the window is still opening?
#[derive(Resource, Default, Debug, PartialEq)]
pub struct PointerBlocked(pub bool);

/// Something other than the map claims the pointer: set by hover systems in
/// [`PointerSet`] before [`block_pointer`] reads it, and cleared after.
#[derive(Resource, Default, Debug, PartialEq)]
pub struct PointerClaimed(pub bool);

/// What the chrome asked of the window. The platform may disagree (the user
/// can maximise with a keyboard shortcut), so this is our best knowledge.
#[derive(Resource, Default, Debug)]
struct WindowFlags {
    maximized: bool,
    pinned: bool,
    /// Can this platform pin at all? Decided once.
    can_pin: bool,
}

pub fn plugin(app: &mut App) {
    embedded_asset!(app, "punch.wgsl");
    app.add_plugins(Material2dPlugin::<Punch>::default())
        .init_gizmo_group::<ChromeGizmos>()
        .init_resource::<PointerBlocked>()
        .init_resource::<PointerClaimed>()
        .init_resource::<WindowFlags>()
        .add_systems(Startup, (spawn, configure_gizmos))
        .add_systems(Update, forget_buttons_on_enter.before(PointerSet))
        .add_systems(Update, block_pointer.in_set(PointerSet))
        .add_systems(
            Update,
            (
                scale_ui,
                (press, draw, shade).run_if(circle_style),
                apply_style,
            )
                .chain()
                .after(PointerSet),
        );
}

pub fn circle_style(settings: Res<Settings>) -> bool {
    settings.window == WindowStyle::Circle
}

fn configure_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<ChromeGizmos>();
    config.render_layers = RenderLayers::layer(OVERLAY_LAYER);
    config.line.width = 3.0;
}

fn visibility(settings: &Settings) -> Visibility {
    if settings.window == WindowStyle::Circle {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    }
}

fn spawn(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut punches: ResMut<Assets<Punch>>,
    mut flags: ResMut<WindowFlags>,
    settings: Res<Settings>,
) {
    flags.can_pin = settings::pin_supported();
    let layer = RenderLayers::layer(OVERLAY_LAYER);
    // Everything the map drew outside the circle becomes see-through. Far
    // enough out to cover the window while the opening shrinks the circle.
    commands.spawn((
        Mesh2d(meshes.add(ringmesh::ring(RADIUS, 200_000.0, RADIUS))),
        MeshMaterial2d(punches.add(Punch {})),
        Transform::from_xyz(0.0, 0.0, -10.0),
        layer.clone(),
        ChromePiece,
        visibility(&settings),
    ));
    let n = ChromeButton::ALL.len();
    for b in ChromeButton::ALL {
        commands.spawn((
            Mesh2d(meshes.add(BUTTONS.mesh(b as usize, n, RADIUS))),
            MeshMaterial2d(materials.add(ColorMaterial::from_color(palette::CONTROL_IDLE))),
            Transform::from_xyz(0.0, 0.0, -5.0),
            layer.clone(),
            Segment(b),
            ChromePiece,
            visibility(&settings),
        ));
    }
}

/// Scale the UI with the window, so one UI pixel is one circle unit and
/// the circle's contents always fit it; and with the opening.
fn scale_ui(
    window: Single<&Window, With<PrimaryWindow>>,
    opening: Res<Opening>,
    mut scale: ResMut<UiScale>,
) {
    let s = window.width().min(window.height()).max(1.0) / (2.0 * RADIUS) * opening.scale();
    if scale.0 != s {
        scale.0 = s;
    }
}

pub fn block_pointer(
    cursor: Res<Cursor>,
    settings: Res<Settings>,
    opening: Res<Opening>,
    interactions: Query<&Interaction>,
    mut claimed: ResMut<PointerClaimed>,
    mut blocked: ResMut<PointerBlocked>,
) {
    let over_ui = interactions.iter().any(|i| *i != Interaction::None);
    let on_frame = settings.window == WindowStyle::Circle
        && cursor.ui.is_some_and(|p| p.length() > PLAY_RADIUS);
    blocked.set_if_neq(PointerBlocked(
        over_ui || claimed.0 || on_frame || !opening.done(),
    ));
    claimed.set_if_neq(PointerClaimed(false));
}

/// The edge a point on the frame belongs to, as a resize direction.
fn octant(p: Vec2) -> CompassOctant {
    let degrees = ringmesh::angle_of(p).to_degrees().rem_euclid(360.0);
    match ((degrees + 22.5) / 45.0) as u32 % 8 {
        0 => CompassOctant::East,
        1 => CompassOctant::NorthEast,
        2 => CompassOctant::North,
        3 => CompassOctant::NorthWest,
        4 => CompassOctant::West,
        5 => CompassOctant::SouthWest,
        6 => CompassOctant::South,
        _ => CompassOctant::SouthEast,
    }
}

fn request(window: &mut Window, resize: Option<CompassOctant>) {
    match resize {
        Some(edge) => window.start_drag_resize(edge),
        None => window.start_drag_move(),
    }
}

/// A press on the frame: a button, a resize, or a move.
///
/// A move or resize hands the pointer to the compositor, which then keeps
/// the button's release to itself. So the button is let go of here, at once:
/// left held, the next real press would not count as a press, and every
/// action on the frame would take two clicks.
#[allow(clippy::too_many_arguments)]
fn press(
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    cursor: Res<Cursor>,
    opening: Res<Opening>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
    mut flags: ResMut<WindowFlags>,
    mut settings: ResMut<Settings>,
    mut exit: MessageWriter<AppExit>,
) {
    if !buttons.just_pressed(MouseButton::Left) || !opening.done() {
        return;
    }
    let Some(p) = cursor.ui else {
        return;
    };
    let r = p.length();
    if !(PLAY_RADIUS..=RADIUS).contains(&r) {
        return;
    }
    match ChromeButton::at(p) {
        Some(ChromeButton::Close) => {
            exit.write(AppExit::Success);
        }
        Some(ChromeButton::Maximize) => {
            flags.maximized = !flags.maximized;
            window.set_maximized(flags.maximized);
        }
        Some(ChromeButton::Minimize) => window.set_minimized(true),
        Some(ChromeButton::Pin) => {
            if flags.can_pin {
                flags.pinned = !flags.pinned;
                pin(&mut window, flags.pinned);
            }
        }
        Some(ChromeButton::Style) => settings.window = settings.window.toggled(),
        None => {
            let resize = (r >= RADIUS - RESIZE_BAND).then(|| octant(p));
            request(&mut window, resize);
            buttons.reset(MouseButton::Left);
        }
    }
}

/// The pointer coming back into the window, as it does when the compositor
/// finishes a move or resize: whatever buttons we think are held were let go
/// of where we could not see it.
fn forget_buttons_on_enter(
    mut entered: MessageReader<CursorEntered>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
) {
    if entered.read().count() > 0 {
        buttons.reset_all();
    }
}

/// Keep the window above others. winit can do it everywhere but Wayland; on
/// KDE Plasma under Wayland, KWin can, if asked through its scripting
/// interface.
fn pin(window: &mut Window, on: bool) {
    if settings::wayland() {
        if settings::kde() {
            kwin_keep_above(on);
        }
    } else {
        window.window_level = if on {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        };
    }
}

/// Run a one-line KWin script that sets `keepAbove` on this process's
/// windows. Off the main thread: it talks to D-Bus.
#[cfg(not(target_family = "wasm"))]
fn kwin_keep_above(on: bool) {
    use std::process::Command;
    let pid = std::process::id();
    std::thread::spawn(move || {
        let name = format!("cake-pin-{pid}");
        let path = std::env::temp_dir().join(format!("{name}.js"));
        let script = format!(
            "const windows = workspace.windowList ? workspace.windowList() : workspace.clientList();\n\
             for (const w of windows) {{ if (w.pid === {pid}) {{ w.keepAbove = {on}; }} }}\n"
        );
        if let Err(error) = std::fs::write(&path, script) {
            warn!(%error, "could not write the KWin script");
            return;
        }
        let dbus = |args: &[&str]| {
            Command::new("dbus-send")
                .args(["--session", "--print-reply", "--dest=org.kde.KWin"])
                .args(args)
                .output()
        };
        let unload = || dbus(&["/Scripting", "org.kde.kwin.Scripting.unloadScript", &format!("string:{name}")]);
        let _ = unload();
        let loaded = dbus(&[
            "/Scripting",
            "org.kde.kwin.Scripting.loadScript",
            &format!("string:{}", path.display()),
            &format!("string:{name}"),
        ]);
        let id = loaded.ok().and_then(|out| {
            String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .skip_while(|w| *w != "int32")
                .nth(1)
                .and_then(|n| n.parse::<i32>().ok())
        });
        match id {
            Some(id) if id >= 0 => {
                let _ = dbus(&[&format!("/Scripting/Script{id}"), "org.kde.kwin.Script.run"]);
            }
            _ => warn!("KWin did not load the pin script"),
        }
        let _ = unload();
        let _ = std::fs::remove_file(&path);
    });
}

#[cfg(target_family = "wasm")]
fn kwin_keep_above(_on: bool) {}

/// Slice fills: hover, and the pin while it is on.
fn shade(
    cursor: Res<Cursor>,
    flags: Res<WindowFlags>,
    segments: Query<(&Segment, &MeshMaterial2d<ColorMaterial>)>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let hovered = cursor.ui.and_then(ChromeButton::at);
    for (segment, material) in &segments {
        let b = segment.0;
        let fill = match (hovered == Some(b), b) {
            (true, ChromeButton::Close) => palette::CLOSE_HOVER,
            (true, _) => palette::CONTROL_HOVER,
            (false, ChromeButton::Pin) if flags.pinned => palette::CONTROL_LIT,
            (false, _) => palette::CONTROL_IDLE,
        };
        palette::tint(&mut materials, &material.0, fill);
    }
}

fn draw(mut gizmos: Gizmos<ChromeGizmos>, cursor: Res<Cursor>, flags: Res<WindowFlags>) {
    let hover = cursor.ui;
    // The resize band shows itself while the pointer is on it.
    if hover.is_some_and(|p| (RADIUS - RESIZE_BAND..=RADIUS).contains(&p.length())) {
        gizmos
            .circle_2d(Isometry2d::IDENTITY, RADIUS - RESIZE_BAND, palette::FAINT)
            .resolution(256);
        gizmos
            .circle_2d(Isometry2d::IDENTITY, RADIUS - 1.5, palette::FAINT)
            .resolution(256);
    }
    let hovered = hover.and_then(ChromeButton::at);

    for b in ChromeButton::ALL {
        let c = b.centre();
        let lit = hovered == Some(b) || (b == ChromeButton::Pin && flags.pinned);
        let color = if b == ChromeButton::Pin && !flags.can_pin {
            palette::FAINT
        } else if lit {
            palette::TEXT
        } else {
            palette::DIM_TEXT
        };
        let s = SYMBOL;
        match b {
            ChromeButton::Close => {
                gizmos.line_2d(c + Vec2::new(-s, -s), c + Vec2::new(s, s), color);
                gizmos.line_2d(c + Vec2::new(-s, s), c + Vec2::new(s, -s), color);
            }
            ChromeButton::Maximize => {
                gizmos.rect_2d(Isometry2d::from_translation(c), Vec2::splat(1.8 * s), color);
            }
            ChromeButton::Minimize => {
                gizmos.line_2d(c + Vec2::new(-s, -0.7 * s), c + Vec2::new(s, -0.7 * s), color);
            }
            ChromeButton::Pin => {
                // A drawing pin: head, and needle.
                gizmos
                    .circle_2d(Isometry2d::from_translation(c + Vec2::Y * 0.35 * s), 0.55 * s, color)
                    .resolution(16);
                gizmos.line_2d(c + Vec2::new(0.0, -0.2 * s), c + Vec2::new(0.0, -s), color);
            }
            ChromeButton::Style => {
                // A little ordinary window: what this button switches to.
                gizmos.rect_2d(
                    Isometry2d::from_translation(c),
                    Vec2::new(2.0 * s, 1.6 * s),
                    color,
                );
                gizmos.line_2d(c + Vec2::new(-s, 0.4 * s), c + Vec2::new(s, 0.4 * s), color);
            }
        }
    }
}

/// Follow the style setting: decorations, the buttons and the mask, the
/// backdrop behind the circle, the arc the buttons reserve from labels, and
/// the saved settings file.
fn apply_style(
    settings: Res<Settings>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
    mut pieces: Query<&mut Visibility, With<ChromePiece>>,
    mut backdrop: Query<&mut Camera, With<BackdropCamera>>,
    mut reserved: ResMut<ReservedArcs>,
) {
    if !settings.is_changed() {
        return;
    }
    let circle = settings.window == WindowStyle::Circle;
    if window.decorations == circle {
        window.decorations = !circle;
    }
    if circle {
        // Square, so the circle fills the window.
        let side = window.resolution.width().min(window.resolution.height());
        if window.resolution.width() != window.resolution.height() {
            window.resolution.set(side, side);
        }
    }
    for mut v in &mut pieces {
        v.set_if_neq(visibility(&settings));
    }
    for mut camera in &mut backdrop {
        camera.clear_color = backdrop_clear(settings.window);
    }
    reserved.set_if_neq(ReservedArcs(if circle {
        vec![Reserved {
            span: BUTTONS.extent(ChromeButton::ALL.len()),
            inner: PLAY_RADIUS,
            outer: RADIUS,
        }]
    } else {
        Vec::new()
    }));
    if !settings.is_added() {
        settings.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_run_left_to_right_with_close_last() {
        for (i, a) in ChromeButton::ALL.iter().enumerate() {
            assert_eq!(*a as usize, i);
            assert_eq!(ChromeButton::at(a.centre()), Some(*a));
            if let Some(b) = ChromeButton::ALL.get(i + 1) {
                assert!(a.centre().x < b.centre().x, "{a:?} is left of {b:?}");
            }
        }
        assert_eq!(ChromeButton::ALL.last(), Some(&ChromeButton::Close));
    }

    #[test]
    fn edges_resize_toward_themselves() {
        let at = |deg: f32| octant(Vec2::from_angle(deg.to_radians()) * RADIUS);
        assert_eq!(at(0.0), CompassOctant::East);
        assert_eq!(at(44.0), CompassOctant::NorthEast);
        assert_eq!(at(90.0), CompassOctant::North);
        assert_eq!(at(180.0), CompassOctant::West);
        assert_eq!(at(-90.0), CompassOctant::South);
        assert_eq!(at(-30.0), CompassOctant::SouthEast);
        assert_eq!(at(350.0), CompassOctant::East);
    }
}
