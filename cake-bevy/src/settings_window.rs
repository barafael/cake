//! The settings dial: a round sub-window that unfolds out of the gear in
//! the frame.
//!
//! One ring per setting, labelled along its arc in the design language of
//! every other control: click a ring to turn it, the lit ones are on. The
//! dial grows out of the gear that opened it, dims the game behind it, and
//! shrinks back when it closes. It claims the whole pointer while open, so
//! the map underneath is safe from stray orders; the gear (or G, or a
//! click on the dark) puts it away.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::sprite_render::{ColorMaterial, MeshMaterial2d};

use crate::camera::{Cursor, OVERLAY_LAYER};
use crate::arctext::{ArcText, Frame};
use crate::chrome::{self, ChromeButton, PointerClaimed, PointerSet};
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

const PANEL_COLOUR: Color = Color::srgb(0.10, 0.105, 0.13);

/// The dial's height: above the map's gizmos and the recap's chart (both at
/// z 0), so the rings and their labels draw over them, while the darkening
/// behind stays under.
const DIAL_Z: f32 = 6.5;

/// One ring of the dial, and what it toggles.
#[derive(Component, Clone, Copy, PartialEq)]
pub enum SettingRow {
    Effects,
    Sky,
    Mode,
}

impl SettingRow {
    /// Its title, its state as words, and whether the band is lit.
    fn describe(&self, settings: &Settings) -> (String, String, bool) {
        match self {
            SettingRow::Effects => (
                "Effects".into(),
                bool_word(settings.effects).into(),
                settings.effects,
            ),
            SettingRow::Sky => (
                "Sky".into(),
                format!("nebula {}", bool_word(settings.sky)),
                settings.sky,
            ),
            SettingRow::Mode => ("Mode".into(), settings.mode.name().into(), false),
        }
    }
}

fn bool_word(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

/// The open dial, and where it is between folded and unfolded.
#[derive(Resource)]
pub struct SettingsWindow {
    root: Entity,
    backdrop: Handle<ColorMaterial>,
    from: Vec2,
    /// 0 closed to 1 open.
    t: f32,
    closing: bool,
}

pub fn plugin(app: &mut App) {
    app.add_systems(Startup, open_from_env)
        .add_systems(OnEnter(AppState::Game), discard)
        .add_systems(OnEnter(AppState::Lobby), discard)
        .add_systems(
            Update,
            (
                toggle_key,
                animate,
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
    settings: &Settings,
    from: Vec2,
    state: AppState,
    existing: Option<&mut SettingsWindow>,
) {
    match existing {
        Some(w) => w.closing = true,
        None => spawn(commands, meshes, materials, fills, settings, from, state),
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    fills: &SegmentFills,
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

    // The game behind, dimmed. Under the map's lines by z, over everything
    // that matters: the point is the dimming, not the covering.
    let backdrop = materials.add(ColorMaterial::from_color(Color::BLACK.with_alpha(0.0)));
    commands.spawn((
        Mesh2d(meshes.add(Circle::new(chrome::PLAY_RADIUS).mesh().resolution(64))),
        MeshMaterial2d(backdrop.clone()),
        Transform::from_xyz(0.0, 0.0, -7.0),
        RenderLayers::layer(OVERLAY_LAYER),
        ChildOf(root),
    ));

    // The dial: a disc with a rim, one ring per setting, a word in the
    // middle. All a step above the recap's chart, so the two can share the
    // ring when a match is over.
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

    let ring = |inner, outer| Slots {
        inner,
        outer,
        centre: 90.0,
        width: 356.0,
        gap: 8.0,
        clockwise: true,
    };
    for (row, slots) in [
        (SettingRow::Effects, ring(230.0, 290.0)),
        (SettingRow::Sky, ring(160.0, 220.0)),
        (SettingRow::Mode, ring(90.0, 150.0)),
    ]
    .into_iter()
    {
        let (title, detail, lit) = row.describe(settings);
        let e = segments::spawn(
            commands,
            meshes,
            fills,
            slots,
            0,
            1,
            &title,
            &detail,
            state,
            row,
        );
        commands.entity(e).insert(Segment {
            row: slots,
            index: 0,
            count: 1,
            enabled: true,
            lit,
        });
        commands.entity(root).add_child(e);
    }

    // The dial's name, in the hole in the middle, and how to put it away.
    commands.spawn((
        ArcText {
            text: "SETTINGS".into(),
            angle: std::f32::consts::FRAC_PI_2,
            frame: Frame::Screen,
            radius: 64.0,
            size: 10.0,
            color: palette::DIM_TEXT,
        },
        ChildOf(root),
    ));
    commands.spawn((
        Text2d::new("G closes"),
        TextFont {
            font_size: bevy::text::FontSize::Px(10.0),
            ..default()
        },
        TextColor(palette::DIM_TEXT),
        Transform::from_xyz(0.0, -34.0, -4.0),
        RenderLayers::layer(OVERLAY_LAYER),
        ChildOf(root),
    ));

    commands.insert_resource(SettingsWindow {
        root,
        backdrop,
        from,
        t: 0.0,
        closing: false,
    });
}

/// Development aid (and a way in for window mode, which has no frame):
/// `CAKE_SETTINGS=1` opens the dial as the app starts.
fn open_from_env(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    fills: Res<SegmentFills>,
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
    palette::tint(&mut materials, &w.backdrop, Color::BLACK.with_alpha(0.45 * e));
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

/// Keep the rings saying what the settings now say.
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
        let (title, detail, lit) = row.describe(&settings);
        segment.lit = lit;
        segments::relabel(&mut labels, e, &title, &detail);
    }
}

/// Clicking a ring turns its setting.
fn act(
    mut pressed: MessageReader<SegmentPressed>,
    rows: Query<&SettingRow>,
    mut settings: ResMut<Settings>,
) {
    for SegmentPressed(e) in pressed.read() {
        if let Ok(row) = rows.get(*e) {
            match row {
                SettingRow::Effects => settings.effects = !settings.effects,
                SettingRow::Sky => settings.sky = !settings.sky,
                SettingRow::Mode => settings.mode = settings.mode.toggled(),
            }
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
