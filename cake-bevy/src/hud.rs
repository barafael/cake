//! Everything you read or click lives inside the circle.
//!
//! The UI is laid out on a box the size of the circle, centred in the window,
//! in circle units (the chrome scales [`UiScale`] so one UI pixel is one
//! unit). The map's band runs from radius 400 to 500, so the lobby and the
//! match HUD fill the inner disk, inside radius ~390. Players' names run along
//! the ring beyond the map, each in their own sector, as [`ArcText`]; the
//! selected unit's menu is in [`crate::menu`].

use bevy::ecs::system::EntityCommands;
use bevy::prelude::*;
use bevy_matchbox::prelude::MatchboxSocket;
use cake_core::geom::sector_center;
use cake_core::stats::{self, Kind, SUPPLY, TICK_HZ};
use cake_core::{Outcome, Seat};
use cake_net::{NetState, RoomId};

use crate::arctext::ArcText;
use crate::chrome::RADIUS;
use crate::input::{Mode, Selection};
use crate::lobby::{Lobby, LobbyAction, LobbyInput, display_name, my_key};
use crate::settings::{self, Settings};
use crate::{AppState, Match, palette};

pub fn plugin(app: &mut App) {
    app.add_systems(OnEnter(AppState::Lobby), spawn_lobby)
        .add_systems(OnEnter(AppState::Game), (spawn_game, spawn_labels))
        .add_systems(Update, (press_buttons, style_buttons))
        .add_systems(
            Update,
            (lobby_keys, update_lobby).run_if(in_state(AppState::Lobby)),
        )
        .add_systems(
            Update,
            (game_keys, update_game, update_labels)
                .run_if(in_state(AppState::Game).and_then(resource_exists::<Match>)),
        );
}

/// What a HUD button does.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiAction {
    Lobby(LobbyAction),
    ToggleWindow,
    BackToLobby,
}

#[derive(Component)]
struct LobbyInfo;

#[derive(Component)]
struct LobbyRoster;

#[derive(Component)]
struct LobbyStatus;

/// The label of the window-style button, which names what it switches to.
#[derive(Component)]
struct StyleLabel;

#[derive(Component)]
struct EconomyText;

#[derive(Component)]
struct QueueText;

#[derive(Component)]
struct SelectionText;

#[derive(Component)]
struct HintText;

#[derive(Component)]
struct BannerText;

#[derive(Component)]
struct BackButton;

/// A player's name on the map, along their sector.
#[derive(Component)]
struct SeatLabel(Seat);

/// Bevy's built-in font covers ASCII only, so everything shown here is ASCII.
fn font(size: f32) -> TextFont {
    TextFont {
        font_size: FontSize::Px(size),
        ..default()
    }
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|first| first.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn centered() -> TextLayout {
    TextLayout {
        justify: Justify::Center,
        ..default()
    }
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

/// The box the circle occupies, centred in the window, despawned when
/// `state` ends.
fn circle_box(commands: &mut Commands, state: AppState) -> Entity {
    let root = commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            DespawnOnExit(state),
        ))
        .id();
    commands
        .spawn((
            Node {
                width: Val::Px(2.0 * RADIUS),
                height: Val::Px(2.0 * RADIUS),
                flex_shrink: 0.0,
                ..default()
            },
            ChildOf(root),
        ))
        .id()
}

fn text<'a>(
    commands: &'a mut Commands,
    parent: Entity,
    node: Node,
    content: &str,
    size: f32,
    color: Color,
) -> EntityCommands<'a> {
    commands.spawn((
        node,
        Text::new(content),
        font(size),
        TextColor(color),
        centered(),
        ChildOf(parent),
    ))
}

/// A button, and its label.
fn button(
    commands: &mut Commands,
    parent: Entity,
    node: Node,
    label: &str,
    action: UiAction,
) -> (Entity, Entity) {
    let b = commands
        .spawn((
            Button,
            Node {
                border_radius: BorderRadius::all(Val::Px(7.0)),
                ..node
            },
            BackgroundColor(palette::CONTROL_IDLE),
            action,
            ChildOf(parent),
        ))
        .id();
    let l = commands
        .spawn((
            // As wide as the button, so the text is centred on it whatever
            // its measured width.
            Node {
                width: Val::Percent(100.0),
                ..default()
            },
            Text::new(label),
            font(15.0),
            TextColor(palette::TEXT),
            centered(),
            ChildOf(b),
        ))
        .id();
    (b, l)
}

// ---- Buttons ---------------------------------------------------------------

fn press_buttons(
    buttons: Query<(&Interaction, &UiAction), Changed<Interaction>>,
    mut input: ResMut<LobbyInput>,
    mut settings: ResMut<Settings>,
    mut next: ResMut<NextState<AppState>>,
) {
    for (interaction, action) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match *action {
            UiAction::Lobby(a) => input.0.push(a),
            UiAction::ToggleWindow => settings.mode = settings.mode.toggled(),
            UiAction::BackToLobby => next.set(AppState::Lobby),
        }
    }
}

fn style_buttons(
    mut buttons: Query<(&Interaction, &UiAction, &mut BackgroundColor, &Children)>,
    mut labels: Query<&mut TextColor>,
    net: Res<NetState>,
) {
    for (interaction, action, mut bg, children) in &mut buttons {
        // Only the host adds and removes bots, or starts.
        let on = match action {
            UiAction::Lobby(LobbyAction::ToggleWatch)
            | UiAction::ToggleWindow
            | UiAction::BackToLobby => true,
            UiAction::Lobby(_) => net.sequences(),
        };
        let fill = match (on, interaction) {
            (false, _) => palette::CONTROL_OFF,
            (true, Interaction::Pressed) => palette::CONTROL_PRESSED,
            (true, Interaction::Hovered) => palette::CONTROL_HOVER,
            (true, Interaction::None) => palette::CONTROL_IDLE,
        };
        bg.set_if_neq(BackgroundColor(fill));
        let ink = if on { palette::TEXT } else { palette::FAINT };
        for c in children.iter() {
            if let Ok(mut color) = labels.get_mut(c) {
                color.set_if_neq(TextColor(ink));
            }
        }
    }
}

// ---- Lobby -----------------------------------------------------------------

fn spawn_lobby(mut commands: Commands) {
    let c = &mut commands;
    let b = circle_box(c, AppState::Lobby);
    text(c, b, place(0.0, 255.0, 400.0, 44.0), "CAKE", 38.0, palette::TEXT);
    let tagline = "A ring, some neighbours, and promises.";
    text(c, b, place(0.0, 214.0, 520.0, 22.0), tagline, 15.0, palette::DIM_TEXT);
    text(c, b, place(0.0, 160.0, 600.0, 56.0), "", 15.0, palette::TEXT).insert(LobbyInfo);
    text(c, b, place(0.0, 40.0, 460.0, 180.0), "", 16.0, palette::DIM_TEXT).insert(LobbyRoster);
    text(c, b, place(0.0, -72.0, 560.0, 22.0), "", 14.0, palette::BAD).insert(LobbyStatus);
    for (i, (label, action)) in [
        ("Add bot", LobbyAction::AddBot),
        ("Remove bot", LobbyAction::RemoveBot),
        ("Watch/Play", LobbyAction::ToggleWatch),
        ("Start", LobbyAction::Start),
    ]
    .into_iter()
    .enumerate()
    {
        let x = -189.0 + 126.0 * i as f32;
        button(c, b, place(x, -125.0, 118.0, 42.0), label, UiAction::Lobby(action));
    }
    if settings::cake_supported() {
        let node = place(0.0, -185.0, 400.0, 36.0);
        let (_, label) = button(c, b, node, "", UiAction::ToggleWindow);
        c.entity(label).insert(StyleLabel);
    }
    let keys = "Keys: B add bot   X remove   W watch/play   Enter start";
    text(c, b, place(0.0, -240.0, 520.0, 20.0), keys, 13.0, palette::DIM_TEXT);
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

#[allow(clippy::type_complexity)]
fn update_lobby(
    net: Res<NetState>,
    room: Option<Res<RoomId>>,
    lobby: Res<Lobby>,
    settings: Res<Settings>,
    socket: Option<Res<MatchboxSocket>>,
    mut texts: ParamSet<(
        Single<&mut Text, With<LobbyInfo>>,
        Single<&mut Text, With<LobbyStatus>>,
        Single<&mut Text, With<LobbyRoster>>,
        Option<Single<&mut Text, With<StyleLabel>>>,
    )>,
) {
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
    if let Some(mut label) = texts.p3() {
        let now = settings.mode;
        let text = format!("{}  (switch to {})", capitalised(now.name()), now.toggled().name());
        label.set_if_neq(Text(text));
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
            lines.push(format!("{seats} on the ring - seats are shuffled at the start"));
        }
        texts.p2().set_if_neq(Text(lines.join("\n")));
    }
}

// ---- Match -----------------------------------------------------------------

fn spawn_game(mut commands: Commands) {
    let c = &mut commands;
    let b = circle_box(c, AppState::Game);
    text(c, b, place(0.0, 185.0, 620.0, 64.0), "", 24.0, palette::TEXT).insert(BannerText);
    let back = place(0.0, 125.0, 210.0, 38.0);
    let (back, _) = button(c, b, back, "Back to the lobby", UiAction::BackToLobby);
    c.entity(back).insert((BackButton, Visibility::Hidden));
    text(c, b, place(0.0, 62.0, 600.0, 26.0), "", 20.0, palette::TEXT).insert(EconomyText);
    text(c, b, place(0.0, 32.0, 600.0, 20.0), "", 14.0, palette::DIM_TEXT).insert(QueueText);
    text(c, b, place(0.0, -18.0, 620.0, 22.0), "", 15.0, palette::TEXT).insert(SelectionText);
    text(c, b, place(0.0, -198.0, 560.0, 40.0), "", 13.0, palette::DIM_TEXT).insert(HintText);
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
    let my_key = m
        .me
        .and_then(|s| m.players.get(s as usize))
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

fn game_keys(keys: Res<ButtonInput<KeyCode>>, m: Res<Match>, mut next: ResMut<NextState<AppState>>) {
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
    let pct = p.progress * 100 / front.stats().build_ticks.max(1);
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

#[allow(clippy::type_complexity)]
fn update_game(
    m: Res<Match>,
    sel: Res<Selection>,
    mode: Res<Mode>,
    mut back: Single<&mut Visibility, With<BackButton>>,
    mut texts: ParamSet<(
        Single<&mut Text, With<EconomyText>>,
        Single<&mut Text, With<QueueText>>,
        Single<&mut Text, With<SelectionText>>,
        Single<&mut Text, With<HintText>>,
        Single<&mut Text, With<BannerText>>,
    )>,
) {
    let sim = &m.sim;
    let mine = m.me.and_then(|me| sim.player(me).map(|p| (me, p)));
    let (economy, queue) = match mine {
        Some((me, p)) if p.alive => {
            let income = sim.income(me) * TICK_HZ as i64;
            (
                format!(
                    "Supply {}    +{}.{}/s    Units {}/{}",
                    p.supply / SUPPLY,
                    income / SUPPLY,
                    (income % SUPPLY) / 100,
                    sim.unit_count(me),
                    stats::UNIT_CAP,
                ),
                queue_line(&m, me),
            )
        }
        Some(_) => ("Eliminated".into(), "You can keep watching.".into()),
        None => ("Watching".into(), String::new()),
    };
    texts.p0().set_if_neq(Text(economy));
    texts.p1().set_if_neq(Text(queue));
    texts.p2().set_if_neq(Text(selection_line(&m, &sel)));

    let hint = if *mode != Mode::Normal {
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
    } else if m.host_left {
        "The host left.".into()
    } else {
        match sim.outcome {
            Some(Outcome::Winner(w)) if Some(w) == m.me => "Victory!".into(),
            Some(Outcome::Winner(w)) => format!("{} holds the ring.", m.players[w as usize].name),
            Some(Outcome::Draw) => "Nobody is left.".into(),
            None => String::new(),
        }
    };
    texts.p4().set_if_neq(Text(banner));

    back.set_if_neq(if over {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
}
