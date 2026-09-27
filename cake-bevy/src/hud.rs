//! Text on screen: the lobby panel, and in a match the economy, the
//! selection, the players around the ring, and banners.

use bevy::prelude::*;
use bevy_matchbox::prelude::MatchboxSocket;
use cake_core::stats::{self, Kind, SUPPLY, TICK_HZ};
use cake_core::{Outcome, Seat};
use cake_net::{NetState, RoomId};

use crate::input::{Mode, Selection};
use crate::lobby::{Lobby, LobbyAction, LobbyInput, my_key};
use crate::{AppState, Match, palette};

pub fn plugin(app: &mut App) {
    app.add_systems(OnEnter(AppState::Lobby), spawn_lobby)
        .add_systems(OnEnter(AppState::Game), spawn_game)
        .add_systems(
            Update,
            (lobby_keys, update_lobby).run_if(in_state(AppState::Lobby)),
        )
        .add_systems(
            Update,
            (game_keys, update_game)
                .run_if(in_state(AppState::Game).and_then(resource_exists::<Match>)),
        );
}

#[derive(Component)]
struct LobbyText;

#[derive(Component)]
struct LobbyRoster;

#[derive(Component)]
struct EconomyText;

#[derive(Component)]
struct SelectionText;

#[derive(Component)]
struct HintText;

#[derive(Component)]
struct BannerText;

#[derive(Component)]
struct PlayerRow(Seat);

/// Bevy's built-in font covers ASCII only, so everything shown here is ASCII.
fn font(size: f32) -> TextFont {
    TextFont {
        font_size: FontSize::Px(size),
        ..default()
    }
}

fn panel(top: Option<f32>, bottom: Option<f32>, left: Option<f32>, right: Option<f32>) -> Node {
    let px = |v: Option<f32>| v.map_or(Val::Auto, Val::Px);
    Node {
        position_type: PositionType::Absolute,
        top: px(top),
        bottom: px(bottom),
        left: px(left),
        right: px(right),
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(4.0),
        ..default()
    }
}

// ---- Lobby -----------------------------------------------------------------

fn spawn_lobby(mut commands: Commands) {
    commands
        .spawn((
            panel(Some(16.0), None, Some(16.0), None),
            DespawnOnExit(AppState::Lobby),
        ))
        .with_children(|p| {
            p.spawn((
                Text::new("CAKE"),
                font(28.0),
                TextColor(palette::TEXT),
            ));
            p.spawn((
                Text::new("A ring, some neighbours, and promises."),
                font(14.0),
                TextColor(palette::DIM_TEXT),
            ));
            p.spawn((Text::new(""), font(15.0), TextColor(palette::TEXT), LobbyText));
            p.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(2.0),
                    margin: UiRect::top(Val::Px(8.0)),
                    ..default()
                },
                LobbyRoster,
            ));
        });
    commands.spawn((
        panel(None, Some(16.0), Some(16.0), None),
        Text::new("[B] add bot   [X] remove bot   [W] watch / play   [Enter] start"),
        font(14.0),
        TextColor(palette::DIM_TEXT),
        DespawnOnExit(AppState::Lobby),
    ));
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

fn update_lobby(
    mut commands: Commands,
    net: Res<NetState>,
    room: Option<Res<RoomId>>,
    lobby: Res<Lobby>,
    socket: Option<Res<MatchboxSocket>>,
    mut text: Single<&mut Text, With<LobbyText>>,
    roster: Single<(Entity, Option<&Children>), With<LobbyRoster>>,
) {
    let room = room.map_or_else(|| "...".to_string(), |r| r.0.clone());
    let connection = match (socket.is_some(), net.my_id.is_some()) {
        (false, _) => "offline",
        (true, false) => "connecting to the signaling server...",
        (true, true) => "online",
    };
    let role = if net.sequences() {
        "you host"
    } else {
        "waiting for the host to start"
    };
    let body = format!(
        "Room: {room}   ({connection})\n\
         Share it: CAKE_ROOM={room} cargo run   or   #room={room}\n\
         You are {}, {role}\n{}",
        net.name, lobby.status
    );
    if text.0 != body {
        text.0 = body;
    }

    if !lobby.is_changed() && !net.is_changed() {
        return;
    }
    let (list, children) = *roster;
    if let Some(children) = children {
        for c in children.iter() {
            commands.entity(c).despawn();
        }
    }
    let me = my_key(&net);
    let mut seat = 0;
    commands.entity(list).with_children(|p| {
        for m in &lobby.members {
            let you = if m.peer.as_deref() == Some(me.as_str()) {
                " (you)"
            } else {
                ""
            };
            let (color, label) = if m.watching {
                (palette::DIM_TEXT, format!("   watching   {}{you}", m.name))
            } else {
                seat += 1;
                (palette::seat(seat - 1), format!("[{}] {}{you}", seat, m.name))
            };
            p.spawn((Text::new(label), font(16.0), TextColor(color)));
        }
    });
}

// ---- Match -----------------------------------------------------------------

fn spawn_game(mut commands: Commands, m: Res<Match>) {
    let scope = DespawnOnExit(AppState::Game);
    commands.spawn((
        panel(Some(12.0), None, Some(14.0), None),
        Text::new(""),
        font(16.0),
        TextColor(palette::TEXT),
        EconomyText,
        scope.clone(),
    ));
    commands.spawn((
        panel(None, Some(40.0), Some(14.0), None),
        Text::new(""),
        font(15.0),
        TextColor(palette::TEXT),
        SelectionText,
        scope.clone(),
    ));
    commands.spawn((
        panel(None, Some(12.0), Some(14.0), None),
        Text::new(""),
        font(13.0),
        TextColor(palette::DIM_TEXT),
        HintText,
        scope.clone(),
    ));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(60.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        scope.clone(),
    ))
    .with_child((Text::new(""), font(26.0), TextColor(palette::TEXT), BannerText));
    commands
        .spawn((panel(Some(12.0), None, None, Some(14.0)), scope))
        .with_children(|p| {
            for seat in 0..m.players.len() {
                p.spawn((
                    Text::new(""),
                    font(15.0),
                    TextColor(palette::seat(seat)),
                    PlayerRow(seat as Seat),
                ));
            }
        });
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
        return "idle".into();
    };
    let pct = p.progress * 100 / front.stats().build_ticks.max(1);
    let rest: Vec<&str> = p.queue.iter().skip(1).map(|k| k.name()).collect();
    if rest.is_empty() {
        format!("{} {pct}%", front.name())
    } else {
        format!("{} {pct}%, then {}", front.name(), rest.join(", "))
    }
}

#[allow(clippy::type_complexity)]
fn update_game(
    m: Res<Match>,
    sel: Res<Selection>,
    mode: Res<Mode>,
    mut texts: ParamSet<(
        Single<&mut Text, With<EconomyText>>,
        Single<&mut Text, With<SelectionText>>,
        Single<&mut Text, With<HintText>>,
        Single<&mut Text, With<BannerText>>,
        Query<(&mut Text, &PlayerRow)>,
    )>,
) {
    let sim = &m.sim;
    let set = |text: &mut Text, s: String| {
        if text.0 != s {
            text.0 = s;
        }
    };

    let economy = match m.me.and_then(|me| sim.player(me).map(|p| (me, p))) {
        Some((me, p)) if p.alive => {
            let income = sim.income(me) * TICK_HZ as i64;
            format!(
                "Supply {}   +{}.{}/s   Units {}/{}\nBuilding: {}",
                p.supply / SUPPLY,
                income / SUPPLY,
                (income % SUPPLY) / 100,
                sim.unit_count(me),
                stats::UNIT_CAP,
                queue_line(&m, me),
            )
        }
        Some(_) => "Eliminated. You can keep watching.".into(),
        None => "Watching".into(),
    };
    set(&mut texts.p0(), economy);

    let selection = if sel.ids.is_empty() {
        String::new()
    } else {
        let mut counts: Vec<(Kind, usize)> = Vec::new();
        for e in sel.ids.iter().filter_map(|id| sim.get(*id)) {
            match counts.iter_mut().find(|(k, _)| *k == e.kind) {
                Some((_, n)) => *n += 1,
                None => counts.push((e.kind, 1)),
            }
        }
        counts.sort();
        let parts: Vec<String> = counts
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
            .collect();
        parts.join("   ")
    };
    set(&mut texts.p1(), selection);

    let hint = if *mode != Mode::Normal {
        format!("{}   [Esc] cancel", mode.hint())
    } else {
        "[Q/W/E/R] brawler/skirmisher/raider/utility  [X] cancel  [A] attack-move  [S] stop  \
         [B] turret  [D] deploy  [Space] HQ  [Ctrl+A] army  [H] home view  wheel zoom, middle-drag pan"
            .into()
    };
    set(&mut texts.p2(), hint);

    let banner = if let Some(tick) = m.desync {
        format!("DESYNC at tick {tick}: this match no longer agrees between peers")
    } else if m.host_left {
        "The host left. [Enter] back to the lobby".into()
    } else {
        match sim.outcome {
            Some(Outcome::Winner(w)) if Some(w) == m.me => "Victory! [Enter] back to the lobby".into(),
            Some(Outcome::Winner(w)) => format!(
                "{} holds the ring. [Enter] back to the lobby",
                m.players[w as usize].name
            ),
            Some(Outcome::Draw) => "Nobody is left. [Enter] back to the lobby".into(),
            None => String::new(),
        }
    };
    set(&mut texts.p3(), banner);

    for (mut text, row) in &mut texts.p4() {
        let seat = row.0;
        let Some(member) = m.players.get(seat as usize) else {
            continue;
        };
        let you = if Some(seat) == m.me { " (you)" } else { "" };
        let bot = if member.is_bot() { " [bot]" } else { "" };
        let status = if sim.is_alive(seat) { "" } else { "  (out)" };
        set(&mut text, format!("[{}] {}{you}{bot}{status}", seat + 1, member.name));
    }
}
