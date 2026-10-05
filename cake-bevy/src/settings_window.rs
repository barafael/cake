//! The settings dial: a round sub-window that unfolds out of the gear in
//! the frame.
//!
//! Every effect has its own segment, laid out as two arcs of buttons the
//! way every other control in the game is: Sparks, Smoke, Trails, Stains,
//! Wind and Sky, lit when on, each saying on or off along its arc, Mode
//! sitting at the bottom. The middle belongs to the frame rate and to two
//! wide panes rendering the same little scene - a plasma turret picking on
//! a brawler - the left under the settings as they now stand, the right
//! with everything off. The scene is drawn with the game's own shapes, as
//! meshes. The dial grows out of the gear that opened it, dims the game
//! behind it (which stays visible through the disc), claims the whole
//! pointer while open, and folds back when dismissed - by the gear, G, or
//! a click on the dark.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use bevy::sprite_render::{ColorMaterial, MeshMaterial2d};
use cake_core::Kind;

use crate::arctext::ArcText;
use crate::camera::{Cursor, OVERLAY_LAYER};
use crate::chrome::{self, ChromeButton, PointerClaimed, PointerSet};
use crate::render::Shapes;
use crate::ringmesh::{self, Slots};
use crate::segments::{self, Segment, SegmentFills, SegmentLabel, SegmentPressed};
use crate::settings::Settings;
use crate::{AppState, palette};

/// The dial's radius, in circle units: it fills the inner disk, over the
/// HUD (and under the segment ring), deep enough to cover the lobby title
/// that lives at radius 300.
const PANEL: f32 = 330.0;
/// How long it takes to unfold, and to fold away.
const OPEN_SECS: f32 = 0.4;
const CLOSE_SECS: f32 = 0.22;
/// Clicks this far outside the dial put it away.
const DISMISS_MARGIN: f32 = 20.0;
/// The dial's height: above the map's gizmos and the recap's chart (both at
/// z 0), so the rings, labels and panes draw over them, while the darkening
/// behind stays under.
const DIAL_Z: f32 = 6.5;

const PANEL_COLOUR: Color = Color::srgba(0.10, 0.105, 0.13, 0.94);

/// The two preview panes, under the effect segments. Their radius,
/// centres, and the little scene inside: a plasma turret picking on a
/// brawler.
const LENS: f32 = 62.0;
const LENS_ON: Vec2 = Vec2::new(-72.0, -42.0);
const LENS_OFF: Vec2 = Vec2::new(72.0, -42.0);
/// Seconds for one round of the preview scene.
const SCENE_PERIOD: f32 = 1.5;

const FLAME: Color = Color::srgb(1.0, 0.72, 0.35);
const SMOKE: Color = Color::srgb(0.62, 0.64, 0.70);

/// One segment of the dial, and what it toggles.
#[derive(Component, Clone, Copy, PartialEq)]
pub enum SettingRow {
    Sparks,
    Smoke,
    Trails,
    Stains,
    Wind,
    Sky,
    Mode,
}

impl SettingRow {
    /// Its title, and its state as words.
    fn describe(&self, settings: &Settings) -> (String, String) {
        match self {
            SettingRow::Sparks => ("Sparks".into(), bool_word(settings.sparks).into()),
            SettingRow::Smoke => ("Smoke".into(), bool_word(settings.smoke).into()),
            SettingRow::Trails => ("Trails".into(), bool_word(settings.trails).into()),
            SettingRow::Stains => ("Stains".into(), bool_word(settings.stains).into()),
            SettingRow::Wind => ("Wind".into(), bool_word(settings.wind).into()),
            SettingRow::Sky => ("Sky".into(), bool_word(settings.sky).into()),
            SettingRow::Mode => ("Mode".into(), settings.mode.name().into()),
        }
    }

    /// Whether the setting is on (a mode has no on).
    fn is_on(&self, settings: &Settings) -> bool {
        match self {
            SettingRow::Sparks => settings.sparks,
            SettingRow::Smoke => settings.smoke,
            SettingRow::Trails => settings.trails,
            SettingRow::Stains => settings.stains,
            SettingRow::Wind => settings.wind,
            SettingRow::Sky => settings.sky,
            SettingRow::Mode => false,
        }
    }

    fn set(&self, settings: &mut Settings) {
        match self {
            SettingRow::Sparks => settings.sparks = !settings.sparks,
            SettingRow::Smoke => settings.smoke = !settings.smoke,
            SettingRow::Trails => settings.trails = !settings.trails,
            SettingRow::Stains => settings.stains = !settings.stains,
            SettingRow::Wind => settings.wind = !settings.wind,
            SettingRow::Sky => settings.sky = !settings.sky,
            SettingRow::Mode => settings.mode = settings.mode.toggled(),
        }
    }
}

fn bool_word(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

/// The dial, and where it is between folded and unfolded.
#[derive(Resource)]
pub struct SettingsWindow {
    root: Entity,
    backdrop: Handle<ColorMaterial>,
    from: Vec2,
    /// 0 closed to 1 open.
    t: f32,
    closing: bool,
}

/// Frames counted over the last beat, for the readout in the dial.
#[derive(Resource, Default)]
struct Fps {
    frames: u32,
    window: f32,
    value: u32,
}

/// The frame-rate text in the dial.
#[derive(Component)]
struct DialFps;

/// A moving piece of the effects preview, driven by the scene's clock.
#[derive(Component)]
enum PreviewRole {
    /// The plasma ball, and the faint disc behind it.
    Ball,
    Glow,
    /// The burst's expanding ring.
    Wave,
    Flash,
    /// A spark, `k` of six around the impact.
    Spark(u8),
    /// A smoke puff, `k` of three.
    Smoke(u8),
    /// The turret's charging core.
    Core,
}

pub fn plugin(app: &mut App) {
    app.init_resource::<Fps>()
        .add_systems(Startup, open_from_env)
        .add_systems(OnEnter(AppState::Game), discard)
        .add_systems(OnEnter(AppState::Lobby), discard)
        .add_systems(
            Update,
            (
                measure_fps,
                toggle_key,
                animate,
                fps_text,
                preview,
                claim
                    .in_set(PointerSet)
                    .before(chrome::block_pointer),
                click_outside.after(PointerSet),
                sync_rows,
                act.after(segments::press),
            ),
        );
}

/// Open the dial (growing out of `from`), or start closing it.
#[allow(clippy::too_many_arguments)]
pub fn toggle(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    fills: &SegmentFills,
    shapes: &Shapes,
    settings: &Settings,
    from: Vec2,
    state: AppState,
    existing: Option<&mut SettingsWindow>,
) {
    match existing {
        Some(w) => w.closing = true,
        None => spawn(
            commands, meshes, materials, fills, shapes, settings, from, state,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    fills: &SegmentFills,
    shapes: &Shapes,
    settings: &Settings,
    from: Vec2,
    state: AppState,
) {
    let root = commands
        .spawn((
            Transform::from_translation(from.extend(DIAL_Z)).with_scale(Vec3::splat(0.04)),
            Visibility::Inherited,
        ))
        .id();

    // The game behind, dimmed but still visible: the dial's own disc is
    // translucent, so a live match reads through it.
    let backdrop = materials.add(ColorMaterial::from_color(Color::BLACK.with_alpha(0.0)));
    commands.spawn((
        Mesh2d(meshes.add(Circle::new(chrome::PLAY_RADIUS).mesh().resolution(64))),
        MeshMaterial2d(backdrop.clone()),
        Transform::from_xyz(0.0, 0.0, -7.0),
        RenderLayers::layer(OVERLAY_LAYER),
        ChildOf(root),
    ));

    // The dial: a disc with a rim. All a step above the recap's chart, so
    // the two can share the ring when a match is over.
    commands.spawn((
        Mesh2d(meshes.add(Circle::new(PANEL).mesh().resolution(64))),
        MeshMaterial2d(materials.add(ColorMaterial::from_color(PANEL_COLOUR))),
        Transform::from_xyz(0.0, 0.0, -5.0),
        RenderLayers::layer(OVERLAY_LAYER),
        ChildOf(root),
    ));
    commands.spawn((
        Mesh2d(meshes.add(ringmesh::ring(PANEL - 1.5, PANEL, chrome::RADIUS))),
        MeshMaterial2d(materials.add(ColorMaterial::from_color(palette::FAINT))),
        Transform::from_xyz(0.0, 0.0, -4.5),
        RenderLayers::layer(OVERLAY_LAYER),
        ChildOf(root),
    ));

    // Two arcs of effect segments, and Mode at the bottom.
    for (row, index) in [
        (SettingRow::Sparks, 0usize),
        (SettingRow::Smoke, 1),
        (SettingRow::Trails, 2),
        (SettingRow::Stains, 0),
        (SettingRow::Wind, 1),
        (SettingRow::Sky, 2),
        (SettingRow::Mode, 0),
    ] {
        let count = if row == SettingRow::Mode { 1 } else { 3 };
        let (inner, outer) = match row {
            SettingRow::Stains | SettingRow::Wind | SettingRow::Sky => (165.0, 225.0),
            SettingRow::Mode => (95.0, 155.0),
            _ => (235.0, 295.0),
        };
        let (centre, width, gap, clockwise) = match row {
            SettingRow::Mode => (270.0, 40.0, 0.0, false),
            _ => (90.0, 58.0, 6.0, true),
        };
        let slots = Slots {
            inner,
            outer,
            centre,
            width,
            gap,
            clockwise,
        };
        let (title, detail) = row.describe(settings);
        let e = segments::spawn(
            commands,
            meshes,
            fills,
            slots,
            index,
            count,
            &title,
            &detail,
            state,
            row,
        );
        commands.entity(e).insert(Segment {
            row: slots,
            index,
            count,
            enabled: true,
            lit: row.is_on(settings),
        });
        commands.entity(root).add_child(e);
    }

    // The two panes: what the settings buy. Same outlines as the game's,
    // the left scene playing under the settings as they stand, the right
    // one with everything off.
    for centre in [LENS_ON, LENS_OFF] {
        commands.spawn((
            Mesh2d(meshes.add(Circle::new(LENS).mesh().resolution(48))),
            MeshMaterial2d(materials.add(ColorMaterial::from_color(Color::srgba(
                1.0, 1.0, 1.0, 0.04,
            )))),
            Transform::from_translation(centre.extend(-2.0)),
            RenderLayers::layer(OVERLAY_LAYER),
            ChildOf(root),
        ));
        commands.spawn((
            Mesh2d(meshes.add(ringmesh::ring(LENS - 1.2, LENS, chrome::RADIUS))),
            MeshMaterial2d(materials.add(ColorMaterial::from_color(palette::FAINT))),
            Transform::from_translation(centre.extend(-2.0)),
            RenderLayers::layer(OVERLAY_LAYER),
            ChildOf(root),
        ));
    }
    for (centre, lit) in [(LENS_ON, true), (LENS_OFF, false)] {
        let tint = if lit {
            palette::seat(1)
        } else {
            palette::seat(1).with_alpha(0.35)
        };
        for (kind, at) in [
            (Kind::PlasmaTurret, centre + Vec2::new(-24.0, -8.0)),
            (Kind::Brawler, centre + Vec2::new(26.0, 5.0)),
        ] {
            commands.spawn((
                Mesh2d(meshes.add(outline_mesh(shapes.of(kind)))),
                MeshMaterial2d(materials.add(ColorMaterial::from_color(tint))),
                Transform::from_translation(at.extend(-2.5)),
                RenderLayers::layer(OVERLAY_LAYER),
                ChildOf(root),
            ));
        }
    }

    // The scene's movers, one mesh each, positioned by `preview`.
    let unit_circle = meshes.add(Circle::new(1.0).mesh().resolution(24));
    let unit_ring = meshes.add(ringmesh::ring(0.82, 1.0, 1.0));
    let unit_stroke = meshes.add(Rectangle::new(1.0, 1.0).mesh());
    for (role, mesh, colour) in [
        (PreviewRole::Ball, unit_circle.clone(), palette::TEXT),
        (PreviewRole::Glow, unit_circle.clone(), palette::seat(1)),
        (PreviewRole::Wave, unit_ring.clone(), palette::seat(1)),
        (PreviewRole::Flash, unit_circle.clone(), FLAME),
        (PreviewRole::Core, unit_circle.clone(), palette::seat(1)),
    ] {
        commands.spawn((
            Mesh2d(mesh),
            MeshMaterial2d(materials.add(ColorMaterial::from_color(colour))),
            Transform::from_xyz(0.0, 0.0, -1.5).with_scale(Vec3::ZERO),
            RenderLayers::layer(OVERLAY_LAYER),
            role,
            ChildOf(root),
        ));
    }
    for k in 0..6u8 {
        commands.spawn((
            Mesh2d(unit_stroke.clone()),
            MeshMaterial2d(materials.add(ColorMaterial::from_color(FLAME))),
            Transform::from_xyz(0.0, 0.0, -1.5).with_scale(Vec3::ZERO),
            RenderLayers::layer(OVERLAY_LAYER),
            PreviewRole::Spark(k),
            ChildOf(root),
        ));
    }
    for k in 0..3u8 {
        commands.spawn((
            Mesh2d(unit_circle.clone()),
            MeshMaterial2d(materials.add(ColorMaterial::from_color(SMOKE))),
            Transform::from_xyz(0.0, 0.0, -1.5).with_scale(Vec3::ZERO),
            RenderLayers::layer(OVERLAY_LAYER),
            PreviewRole::Smoke(k),
            ChildOf(root),
        ));
    }

    // The frame rate, since these settings move it, and the panes' names.
    commands.spawn((
        Text2d::new(""),
        TextFont {
            font_size: bevy::text::FontSize::Px(14.0),
            ..default()
        },
        TextColor(palette::TEXT),
        Transform::from_xyz(0.0, 118.0, -3.0),
        RenderLayers::layer(OVERLAY_LAYER),
        DialFps,
        ChildOf(root),
    ));
    for (at, word) in [
        (LENS_ON + Vec2::new(0.0, -78.0), "now"),
        (LENS_OFF + Vec2::new(0.0, -78.0), "all off"),
    ] {
        commands.spawn((
            Text2d::new(word),
            TextFont {
                font_size: bevy::text::FontSize::Px(9.0),
                ..default()
            },
            TextColor(palette::DIM_TEXT),
            Transform::from_translation(at.extend(-1.2)),
            RenderLayers::layer(OVERLAY_LAYER),
            ChildOf(root),
        ));
    }

    commands.insert_resource(SettingsWindow {
        root,
        backdrop,
        from,
        t: 0.0,
        closing: false,
    });
}

/// The game's own outlines, as a drawable mesh: each polyline folded into
/// line segments.
fn outline_mesh(strips: &[Vec<Vec2>]) -> Mesh {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    for strip in strips {
        for w in strip.windows(2) {
            positions.push([w[0].x, w[0].y, 0.0]);
            positions.push([w[1].x, w[1].y, 0.0]);
        }
    }
    let normals = vec![[0.0, 0.0, 1.0]; positions.len()];
    let uvs = vec![[0.0, 0.0]; positions.len()];
    Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
}

/// Development aid (and a way in for window mode, which has no frame):
/// `CAKE_SETTINGS=1` opens the dial as the app starts.
fn open_from_env(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    fills: Res<SegmentFills>,
    shapes: Res<Shapes>,
    settings: Res<Settings>,
    state: Res<State<AppState>>,
) {
    if std::env::var("CAKE_SETTINGS").ok().as_deref() != Some("1") {
        return;
    }
    spawn(
        &mut commands,
        &mut meshes,
        &mut materials,
        &fills,
        &shapes,
        &settings,
        ChromeButton::Settings.centre(),
        *state.get(),
    );
}

/// `G` toggles the dial, in either state and either mode.
#[allow(clippy::too_many_arguments)]
fn toggle_key(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    fills: Res<SegmentFills>,
    shapes: Res<Shapes>,
    settings: Res<Settings>,
    state: Res<State<AppState>>,
    mut win: Option<ResMut<SettingsWindow>>,
) {
    if keys.just_pressed(KeyCode::KeyG) {
        toggle(
            &mut commands,
            &mut meshes,
            &mut materials,
            &fills,
            &shapes,
            &settings,
            ChromeButton::Settings.centre(),
            *state.get(),
            win.as_deref_mut(),
        );
    }
}

/// Fold and unfold: the dial grows out of the gear that opened it, and the
/// game behind darkens as it comes.
fn animate(
    time: Res<Time>,
    mut win: Option<ResMut<SettingsWindow>>,
    mut commands: Commands,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut roots: Query<&mut Transform>,
) {
    let Some(w) = win.as_deref_mut() else {
        return;
    };
    let secs = if w.closing { CLOSE_SECS } else { OPEN_SECS };
    let dir = if w.closing { -1.0 } else { 1.0 };
    w.t = (w.t + dir * time.delta_secs() / secs).clamp(0.0, 1.0);
    let e = ease_out(w.t);
    if let Ok(mut tf) = roots.get_mut(w.root) {
        tf.translation = w.from.lerp(Vec2::ZERO, e).extend(DIAL_Z);
        tf.scale = Vec3::splat(0.04 + 0.96 * e);
    }
    palette::tint(&mut materials, &w.backdrop, Color::BLACK.with_alpha(0.5 * e));
    if w.closing && w.t <= 0.0 {
        if let Ok(mut entity) = commands.get_entity(w.root) {
            entity.despawn();
        }
        commands.remove_resource::<SettingsWindow>();
    }
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

/// The dial has the pointer, wherever it is.
fn claim(win: Option<Res<SettingsWindow>>, mut claimed: ResMut<PointerClaimed>) {
    if win.is_some() {
        claimed.0 = true;
    }
}

/// A click on the dark puts the dial away. (The gear handles its own
/// stretch of the frame, and the other window buttons keep working.)
fn click_outside(
    buttons: Res<ButtonInput<MouseButton>>,
    cursor: Res<Cursor>,
    mut win: Option<ResMut<SettingsWindow>>,
) {
    if buttons.just_pressed(MouseButton::Left)
        && let (Some(w), Some(p)) = (win.as_deref_mut(), cursor.ui)
        && p.length() > PANEL + DISMISS_MARGIN
        && !(chrome::PLAY_RADIUS..=chrome::RADIUS).contains(&p.length())
    {
        w.closing = true;
    }
}

/// Keep the segments saying what the settings now say, and lit when on.
fn sync_rows(
    settings: Res<Settings>,
    win: Option<Res<SettingsWindow>>,
    mut rows: Query<(Entity, &SettingRow, &mut Segment)>,
    mut labels: Query<(&SegmentLabel, &mut ArcText)>,
) {
    let Some(_) = win else {
        return;
    };
    if !settings.is_changed() {
        return;
    }
    for (e, row, mut segment) in &mut rows {
        let (title, detail) = row.describe(&settings);
        segments::relabel(&mut labels, e, &title, &detail);
        let lit = row.is_on(&settings);
        if segment.lit != lit {
            segment.lit = lit;
        }
    }
}

/// Clicking a segment turns its setting.
fn act(
    mut pressed: MessageReader<SegmentPressed>,
    rows: Query<&SettingRow>,
    mut settings: ResMut<Settings>,
) {
    for SegmentPressed(e) in pressed.read() {
        if let Ok(row) = rows.get(*e) {
            row.set(&mut settings);
        }
    }
}

/// A state change takes the dial with it.
fn discard(mut commands: Commands, win: Option<Res<SettingsWindow>>) {
    if let Some(w) = win {
        if let Ok(mut entity) = commands.get_entity(w.root) {
            entity.despawn();
        }
        commands.remove_resource::<SettingsWindow>();
    }
}

/// Count frames over half-second beats.
fn measure_fps(time: Res<Time>, mut fps: ResMut<Fps>) {
    fps.frames += 1;
    fps.window += time.delta_secs();
    if fps.window >= 0.5 {
        fps.value = (fps.frames as f32 / fps.window).round() as u32;
        fps.frames = 0;
        fps.window = 0.0;
    }
}

/// The readout, only while the dial is out.
fn fps_text(
    win: Option<Res<SettingsWindow>>,
    fps: Res<Fps>,
    mut texts: Query<&mut Text2d, With<DialFps>>,
) {
    if win.is_none() {
        return;
    }
    let line = format!("{} fps", fps.value);
    for mut text in &mut texts {
        if text.0 != line {
            text.0 = line.clone();
        }
    }
}

/// The little scene in the left pane: a plasma turret peppering a brawler,
/// under the settings as they now stand. The right pane holds the same
/// outlines with everything off. Both are drawn with the game's own shapes
/// and colours.
fn preview(
    time: Res<Time>,
    settings: Res<Settings>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut things: Query<(&PreviewRole, &mut Transform, &MeshMaterial2d<ColorMaterial>)>,
) {
    let muzzle = LENS_ON + Vec2::new(-18.0, -4.0);
    let target = LENS_ON + Vec2::new(24.0, 4.0);
    let now = time.elapsed_secs() % SCENE_PERIOD;
    let fly = 0.33;
    let settle = |after: f32, secs: f32| ((now - after) / secs).clamp(0.0, 1.0);

    for (role, mut tf, material) in &mut things {
        let theatre = settings.sparks;
        let (at, size, colour) = match *role {
            PreviewRole::Ball => {
                let p = (now / fly).clamp(0.0, 1.0);
                (
                    muzzle.lerp(target, p),
                    1.4 + 2.2 * p,
                    palette::TEXT.with_alpha(0.9),
                )
            }
            PreviewRole::Glow => {
                let p = (now / fly).clamp(0.0, 1.0);
                (
                    muzzle.lerp(target, p),
                    2.6 + 4.2 * p,
                    palette::seat(1).with_alpha(0.3),
                )
            }
            PreviewRole::Wave => {
                let u = settle(fly, 0.3);
                (
                    target,
                    2.0 + 20.0 * u,
                    palette::seat(1).with_alpha(0.4 * (1.0 - u)),
                )
            }
            PreviewRole::Flash => {
                let u = settle(fly, 0.14);
                (
                    target,
                    2.0 + 10.0 * u,
                    FLAME.with_alpha(0.8 * (1.0 - u)),
                )
            }
            PreviewRole::Spark(k) => {
                let u = settle(fly, 0.3);
                let dir = Vec2::from_angle(
                    std::f32::consts::TAU * (k as f32 / 6.0) + 0.35 + (k as f32 * 1.7).sin() * 0.2,
                );
                (
                    target + dir * (3.0 + 14.0 * u),
                    1.4 + 5.5 * (1.0 - u),
                    FLAME.with_alpha(0.8 * (1.0 - u)),
                )
            }
            PreviewRole::Smoke(k) => {
                let u = settle(fly + 0.05, 0.9);
                let drift = Vec2::new(-6.0 - k as f32 * 4.0, 7.0 + k as f32 * 2.5);
                (
                    target + drift * u,
                    2.5 + 6.5 * u,
                    SMOKE.with_alpha(0.22 * (1.0 - u)),
                )
            }
            PreviewRole::Core => {
                let charge = ((now - fly) / (SCENE_PERIOD - fly)).clamp(0.0, 1.0);
                (
                    LENS_ON + Vec2::new(-24.0, -8.0),
                    1.8 + 2.2 * charge,
                    palette::seat(1).with_alpha(0.15 + 0.5 * charge),
                )
            }
        };
        let on = match *role {
            PreviewRole::Smoke(_) => settings.smoke,
            _ => theatre,
        };
        if on {
            tf.translation = at.extend(-1.5);
            tf.scale = Vec3::splat(size);
            palette::tint(&mut materials, &material.0, colour);
        } else {
            tf.scale = Vec3::ZERO;
            palette::tint(&mut materials, &material.0, Color::NONE);
        }
    }
}
