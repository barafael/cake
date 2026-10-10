//! Everything you read or click lives inside the circle.
//!
//! The UI is laid out on a [`CircleBox`], a box the size of the circle, in
//! circle units, which zooms and pans with the map (see
//! [`crate::camera`]). The map's band runs from radius 400 to 500, so the
//! lobby and the match HUD fill the inner disk, inside radius ~390. Players'
//! names run along the ring beyond the map, each in their own sector, as
//! [`ArcText`] on the circle; the selected unit's menu is in
//! [`crate::menu`].

use bevy::prelude::*;
use bevy_matchbox::prelude::MatchboxSocket;
use cake_core::Seat;
use cake_core::geom::sector_center;
use cake_core::stats::{self, Kind, SUPPLY, TICK_HZ};
use cake_net::{NetState, RoomId};

use crate::arctext::{ArcText, Frame};
use crate::camera::{CAKE_LAYER, CircleBox};
use crate::chrome::{ChromeButton, RADIUS};
use crate::input::{Mode, Selection};
use crate::lobby::{Lobby, LobbyAction, LobbyInput, display_name, my_key};
use crate::menu::{MENU_INNER, MENU_OUTER};
use crate::render::Shapes;
use crate::ringmesh::Slots;
use crate::segments::{self, Segment, SegmentFills, SegmentLabel, SegmentPressed};
use crate::settings::{self, Settings};
use crate::settings_window::{self, SettingsWindow};
use crate::{AppState, Match, palette};

pub fn plugin(app: &mut App) {
    app.add_systems(OnEnter(AppState::Lobby), spawn_lobby)
        .add_systems(OnEnter(AppState::Game), (spawn_game, spawn_labels))
        .add_systems(Update, press_segments.after(segments::press))
        .add_systems(
            Update,
            (lobby_keys, enable_lobby, update_lobby).run_if(in_state(AppState::Lobby)),
        )
        .add_systems(
            Update,
            (game_keys, update_game, update_labels)
                .run_if(in_state(AppState::Game).and_then(resource_exists::<Match>)),
        );
}

/// What a lobby or match segment does.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiAction {
    Lobby(LobbyAction),
    ToggleMode,
    Settings,
    BackToLobby,
}

/// The lobby's buttons, along the bottom of the inner ring.
const LOBBY_ROW: Slots = Slots {
    inner: MENU_INNER,
    outer: MENU_OUTER,
    centre: 270.0,
    width: 19.0,
    gap: 1.0,
    clockwise: false,
};

/// One wide segment at the top of the inner ring: the display mode in the
/// lobby, the way back to it when a match is over.
const TOP_SLOT: Slots = Slots {
    inner: MENU_INNER,
    outer: MENU_OUTER,
    centre: 90.0,
    width: 40.0,
    gap: 0.0,
    clockwise: true,
};

#[derive(Component, Default, Clone)]
struct LobbyInfo;

#[derive(Component, Default, Clone)]
struct LobbyRoster;

#[derive(Component, Default, Clone)]
struct LobbyStatus;

#[derive(Component, Default, Clone)]
struct LobbyKeys;

#[derive(Component, Default, Clone)]
struct EconomyText;

#[derive(Component, Default, Clone)]
struct QueueText;

#[derive(Component, Default, Clone)]
struct SelectionText;

#[derive(Component, Default, Clone)]
struct HintText;

#[derive(Component, Default, Clone)]
struct BannerText;

/// What the middle says to a watcher instead of "Watching", with the
/// controls hint gone: a demo's title.
#[derive(Resource, Clone, Debug)]
pub struct Caption(pub String);

/// A player's name on the map, along their sector.
#[derive(Component)]
struct SeatLabel(Seat);

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|first| first.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// A box of `w` x `h` centred on `(x, y)` in circle units (y up), inside the
/// circle box.
fn place(x: f32, y: f32, w: f32, h: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(RADIUS + x - w / 2.0),
        top: Val::Px(RADIUS - y - h / 2.0),
        width: Val::Px(w),
        height: Val::Px(h),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..default()
    }
}

/// The box the circle occupies, holding `contents` and despawned when
/// `state` ends. It starts where the fitted view has the circle, centred in
/// the window; [`CircleBox`] takes it from there.
fn circle_box(state: AppState, contents: impl SceneList) -> impl Scene {
    let scoped = DespawnOnExit(state);
    bsn! {
        Node {
            position_type: PositionType::Absolute,
            left: percent(50),
            top: percent(50),
            width: px(2.0 * RADIUS),
            height: px(2.0 * RADIUS),
        }
        UiTransform { translation: Val2::px(-RADIUS, -RADIUS) }
        CircleBox
        scoped
        Children [ {contents} ]
    }
}

/// Centred text in `node`'s box, `size` circle units high. Bevy's built-in
/// font covers ASCII only, so everything shown here is ASCII.
fn readout(node: Node, content: &str, size: f32, color: Color) -> impl Scene {
    let content = content.to_string();
    bsn! {
        node
        Text(content)
        TextFont { font_size: FontSize::Px(size) }
        TextColor(color)
        TextLayout { justify: Justify::Center }
    }
}

// ---- Segments --------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn press_segments(
    mut pressed: MessageReader<SegmentPressed>,
    actions: Query<&UiAction>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    fills: Res<SegmentFills>,
    shapes: Res<Shapes>,
    mut input: ResMut<LobbyInput>,
    mut settings: ResMut<Settings>,
    mut next: ResMut<NextState<AppState>>,
    state: Res<State<AppState>>,
    mut win: Option<ResMut<SettingsWindow>>,
) {
    for SegmentPressed(e) in pressed.read() {
        match actions.get(*e) {
            Ok(UiAction::Lobby(a)) => input.0.push(*a),
            Ok(UiAction::ToggleMode) => settings.mode = settings.mode.toggled(),
            Ok(UiAction::Settings) => settings_window::toggle(
                &mut commands,
                &mut meshes,
                &mut materials,
                &fills,
                &shapes,
                &settings,
                ChromeButton::Settings.centre(),
                *state.get(),
                win.as_deref_mut(),
            ),
            Ok(UiAction::BackToLobby) => next.set(AppState::Lobby),
            Err(_) => {}
        }
    }
}

/// The mode segment's title and detail: the mode now, and what it switches
/// to.
fn mode_labels(settings: &Settings) -> (String, String) {
    let now = settings.mode;
    (
        capitalised(now.name()),
        format!("switch to {}", now.toggled().name()),
    )
}

// ---- Lobby -----------------------------------------------------------------

fn spawn_lobby(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    fills: Res<SegmentFills>,
    settings: Res<Settings>,
) {
    let c = &mut commands;
    // The title and the tagline follow the top of the circle.
    for (text, radius, size, color) in [
        ("CAKE", 300.0, 40.0, palette::TEXT),
        (
            "A ring, some neighbours, and promises.",
            262.0,
            15.0,
            palette::DIM_TEXT,
        ),
    ] {
        c.spawn((
            ArcText {
                text: text.into(),
                angle: std::f32::consts::FRAC_PI_2,
                frame: Frame::Screen,
                radius,
                size,
                color,
            },
            DespawnOnExit(AppState::Lobby),
        ));
    }
    let keys = "Keys: B add bot   X remove   W watch/play   Enter start";
    c.spawn_scene(circle_box(
        AppState::Lobby,
        bsn_list! {
            @readout(place(0.0, 150.0, 600.0, 56.0), "", 15.0, palette::TEXT) LobbyInfo
            --
            @readout(place(0.0, 30.0, 460.0, 150.0), "", 16.0, palette::DIM_TEXT) LobbyRoster
            --
            @readout(place(0.0, -80.0, 560.0, 22.0), "", 14.0, palette::BAD) LobbyStatus
            --
            @readout(place(0.0, -215.0, 520.0, 20.0), keys, 13.0, palette::DIM_TEXT) LobbyKeys
        },
    ));

    let row = [
        ("Add bot", "B", LobbyAction::AddBot),
        ("Remove bot", "X", LobbyAction::RemoveBot),
        ("Watch/Play", "W", LobbyAction::ToggleWatch),
        ("Start", "Enter", LobbyAction::Start),
    ];
    for (i, (title, key, action)) in row.iter().enumerate() {
        segments::spawn(
            c,
            &mut meshes,
            &fills,
            CAKE_LAYER,
            LOBBY_ROW,
            i,
            row.len(),
            title,
            key,
            AppState::Lobby,
            UiAction::Lobby(*action),
        );
    }
    // The mode toggle only where cake mode exists; the settings dial is
    // always reachable, which in window mode is here and only here.
    let slots = if settings::cake_supported() { 2 } else { 1 };
    if let Some(i) = settings::cake_supported().then_some(0) {
        let (title, detail) = mode_labels(&settings);
        segments::spawn(
            c,
            &mut meshes,
            &fills,
            CAKE_LAYER,
            TOP_SLOT,
            i,
            slots,
            &title,
            &detail,
            AppState::Lobby,
            UiAction::ToggleMode,
        );
    }
    segments::spawn(
        c,
        &mut meshes,
        &fills,
        CAKE_LAYER,
        TOP_SLOT,
        slots - 1,
        slots,
        "Settings",
        "G",
        AppState::Lobby,
        UiAction::Settings,
    );
}

fn lobby_keys(keys: Res<ButtonInput<KeyCode>>, mut input: ResMut<LobbyInput>) {
    for (key, action) in [
        (KeyCode::KeyB, LobbyAction::AddBot),
        (KeyCode::KeyX, LobbyAction::RemoveBot),
        (KeyCode::KeyW, LobbyAction::ToggleWatch),
        (KeyCode::Enter, LobbyAction::Start),
        (KeyCode::NumpadEnter, LobbyAction::Start),
    ] {
        if keys.just_pressed(key) {
            input.0.push(action);
        }
    }
}

/// Only the host adds and removes bots, or starts; and the mode segment
/// names the mode.
fn enable_lobby(
    net: Res<NetState>,
    settings: Res<Settings>,
    mut buttons: Query<(Entity, &UiAction, &mut Segment)>,
    mut labels: Query<(&SegmentLabel, &mut ArcText)>,
) {
    for (e, action, mut segment) in &mut buttons {
        let on = match action {
            UiAction::Lobby(LobbyAction::AddBot | LobbyAction::RemoveBot | LobbyAction::Start) => {
                net.sequences()
            }
            _ => true,
        };
        if segment.enabled != on {
            segment.enabled = on;
        }
        if *action == UiAction::ToggleMode && settings.is_changed() {
            let (title, detail) = mode_labels(&settings);
            segments::relabel(&mut labels, e, &title, &detail);
        }
    }
}

#[allow(clippy::type_complexity)]
fn update_lobby(
    net: Res<NetState>,
    room: Option<Res<RoomId>>,
    lobby: Res<Lobby>,
    socket: Option<Res<MatchboxSocket>>,
    win: Option<Res<SettingsWindow>>,
    mut texts: ParamSet<(
        Single<&mut Text, With<LobbyInfo>>,
        Single<&mut Text, With<LobbyStatus>>,
        Single<&mut Text, With<LobbyRoster>>,
        Single<&mut Text, With<LobbyKeys>>,
    )>,
) {
    // While the settings dial is open, the dial speaks.
    if win.is_some() {
        texts.p0().set_if_neq(Text(String::new()));
        texts.p1().set_if_neq(Text(String::new()));
        texts.p2().set_if_neq(Text(String::new()));
        texts.p3().set_if_neq(Text(String::new()));
        return;
    }
    let room = room.map_or_else(|| "...".to_string(), |r| r.0.clone());
    let connection = match (socket.is_some(), net.my_id.is_some()) {
        (false, _) => "offline",
        (true, false) => "connecting...",
        (true, true) => "online",
    };
    let role = if net.sequences() {
        "you host"
    } else {
        "waiting for the host to start"
    };
    texts.p0().set_if_neq(Text(format!(
        "Room {room} ({connection})  -  you are {}, {role}\n\
         share: CAKE_ROOM={room}  or  #room={room}",
        net.name
    )));
    if texts.p1().0 != lobby.status {
        texts.p1().0.clone_from(&lobby.status);
    }

    // Players are shown on the ring (see `lobby_ring`); watchers are listed
    // here.
    if lobby.is_changed() || net.is_changed() {
        let me = my_key(&net);
        let mut lines: Vec<String> = lobby
            .members
            .iter()
            .filter(|m| m.watching)
            .map(|m| format!("watching: {}", display_name(m, &me)))
            .collect();
        let seats = lobby.players().count();
        if seats > 0 {
            lines.push(format!(
                "{seats} on the ring - seats are shuffled at the start"
            ));
        }
        texts.p2().set_if_neq(Text(lines.join("\n")));
    }
}

// ---- Match -----------------------------------------------------------------

fn spawn_game(mut commands: Commands) {
    commands.spawn_scene(circle_box(
        AppState::Game,
        bsn_list! {
            @readout(place(0.0, 185.0, 620.0, 64.0), "", 24.0, palette::TEXT) BannerText
            --
            @readout(place(0.0, 62.0, 600.0, 26.0), "", 20.0, palette::TEXT) EconomyText
            --
            @readout(place(0.0, 32.0, 600.0, 20.0), "", 14.0, palette::DIM_TEXT) QueueText
            --
            @readout(place(0.0, -18.0, 620.0, 22.0), "", 15.0, palette::TEXT) SelectionText
            --
            @readout(place(0.0, -198.0, 560.0, 40.0), "", 13.0, palette::DIM_TEXT) HintText
        },
    ));
}

fn spawn_labels(mut commands: Commands, m: Res<Match>) {
    let n = m.players.len();
    for seat in 0..n {
        let angle = sector_center(seat, n).to_radians() as f32;
        commands.spawn((
            ArcText::name(String::new(), angle, palette::seat(seat)),
            SeatLabel(seat as Seat),
            DespawnOnExit(AppState::Game),
        ));
    }
}

/// Names along their sectors, dimmed once a player is out.
fn update_labels(m: Res<Match>, mut labels: Query<(&SeatLabel, &mut ArcText)>) {
    let my_key =
        m.me.and_then(|s| m.players.get(s as usize))
            .and_then(|p| p.peer.clone())
            .unwrap_or_default();
    for (label, mut arc) in &mut labels {
        let seat = label.0;
        let Some(member) = m.players.get(seat as usize) else {
            continue;
        };
        let alive = m.sim.is_alive(seat);
        let name = display_name(member, &my_key);
        let text = if alive { name } else { format!("{name} - out") };
        let color = palette::seat_status(seat as usize, alive);
        // Only touch it when it differs: a changed text respawns its glyphs.
        if arc.text != text || arc.color != color {
            arc.text = text;
            arc.color = color;
        }
    }
}

fn game_keys(
    keys: Res<ButtonInput<KeyCode>>,
    m: Res<Match>,
    mut next: ResMut<NextState<AppState>>,
) {
    let over = m.sim.outcome.is_some() || m.host_left;
    if over && (keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter)) {
        next.set(AppState::Lobby);
    }
}

fn queue_line(m: &Match, seat: Seat) -> String {
    let Some(p) = m.sim.player(seat) else {
        return String::new();
    };
    let Some(front) = p.queue.front() else {
        return "HQ idle".into();
    };
    let pct = (p.progress / front.stats().build_ticks.max(1)).min(100);
    let rest: Vec<&str> = p.queue.iter().skip(1).map(|k| k.name()).collect();
    if rest.is_empty() {
        format!("Building {} {pct}%", front.name())
    } else {
        format!("Building {} {pct}%, then {}", front.name(), rest.join(", "))
    }
}

fn selection_line(m: &Match, sel: &Selection) -> String {
    let sim = &m.sim;
    let mut counts: Vec<(Kind, usize)> = Vec::new();
    for e in sel.ids.iter().filter_map(|id| sim.get(*id)) {
        match counts.iter_mut().find(|(k, _)| *k == e.kind) {
            Some((_, n)) => *n += 1,
            None => counts.push((e.kind, 1)),
        }
    }
    counts.sort();
    counts
        .iter()
        .map(|(k, n)| {
            if *n == 1 {
                let e = sel
                    .ids
                    .iter()
                    .filter_map(|id| sim.get(*id))
                    .find(|e| e.kind == *k)
                    .expect("counted");
                format!("{} {}/{}", k.name(), e.hp.max(0), e.max_hp())
            } else {
                format!("{n} x {}", k.name())
            }
        })
        .collect::<Vec<_>>()
        .join("   ")
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn update_game(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    fills: Res<SegmentFills>,
    m: Res<Match>,
    sel: Res<Selection>,
    mode: Res<Mode>,
    caption: Option<Res<Caption>>,
    win: Option<Res<SettingsWindow>>,
    back: Query<(), With<UiAction>>,
    mut texts: ParamSet<(
        Single<&mut Text, With<EconomyText>>,
        Single<&mut Text, With<QueueText>>,
        Single<&mut Text, With<SelectionText>>,
        Single<&mut Text, With<HintText>>,
        Single<&mut Text, With<BannerText>>,
    )>,
) {
    let sim = &m.sim;
    // Once it is decided, the middle belongs to the recap. While the
    // settings dial is open, it belongs to the dial.
    let recap = sim.outcome.is_some();
    let covered = win.is_some();
    let mine = m.me.and_then(|me| sim.player(me).map(|p| (me, p)));
    let (economy, queue) = match mine {
        _ if covered => (String::new(), String::new()),
        Some((me, p)) if p.alive => {
            let income = sim.income(me) * TICK_HZ as i64;
            (
                format!(
                    "Supply {}    +{}.{}/s    Pace {}%    Units {}/{}",
                    p.supply / SUPPLY,
                    income / SUPPLY,
                    (income % SUPPLY) / 100,
                    sim.production_pct(me),
                    sim.unit_count(me),
                    stats::UNIT_CAP,
                ),
                queue_line(&m, me),
            )
        }
        Some(_) => ("Eliminated".into(), "You can keep watching.".into()),
        None => match &caption {
            Some(c) => (c.0.clone(), String::new()),
            None => ("Watching".into(), String::new()),
        },
    };
    texts.p0().set_if_neq(Text(economy));
    texts.p1().set_if_neq(Text(queue));
    let selection = if covered {
        String::new()
    } else {
        selection_line(&m, &sel)
    };
    texts.p2().set_if_neq(Text(selection));

    let hint = if covered || caption.is_some() {
        String::new()
    } else if *mode != Mode::Normal {
        format!("{}   [Esc] cancel", mode.hint())
    } else {
        "Drag or click to select. Right-click: move, attack, repair, rally.\n\
         Space: HQ   Ctrl+A: army   Wheel: zoom   Middle-drag: pan   H: home"
            .into()
    };
    texts.p3().set_if_neq(Text(hint));

    let over = sim.outcome.is_some() || m.host_left;
    let banner = if let Some(tick) = m.desync {
        format!("DESYNC at tick {tick}\nthis match no longer agrees between peers")
    } else if m.host_left && !recap {
        "The host left.".into()
    } else {
        String::new()
    };
    texts.p4().set_if_neq(Text(banner));

    // Once it is over, the way back: at the top of the inner ring.
    if over && back.is_empty() {
        segments::spawn(
            &mut commands,
            &mut meshes,
            &fills,
            CAKE_LAYER,
            TOP_SLOT,
            0,
            1,
            "Back to the lobby",
            "Enter",
            AppState::Game,
            UiAction::BackToLobby,
        );
    }
}
