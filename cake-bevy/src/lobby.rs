//! The lobby conversation: who is here, who plays, and when the match starts.
//!
//! As in chinese-checke.rs, the host is the peer with the smallest id,
//! recomputed every frame so a departed host is replaced. The host owns the
//! roster and broadcasts it whenever it changes; guests take it verbatim.
//! Every connected peer plays unless they choose to watch, and the host adds
//! bots to fill the ring. On Start the host shuffles the players into ring
//! order (who neighbours whom is a strategic fact, so nobody picks it) and
//! tells everyone.

use bevy::prelude::*;
use bevy_matchbox::prelude::*;
use cake_core::Seat;
use cake_core::sim::MAX_SEATS;
use cake_net::{Member, NetMsg, NetState, RoomId, broadcast, decode, open_socket, send_to};

use crate::{AppState, Match, web};

/// The lobby as this peer sees it.
#[derive(Resource, Default)]
pub struct Lobby {
    /// Everyone in the room, and bots. The host's copy is authoritative.
    pub members: Vec<Member>,
    /// Do I want to watch rather than play?
    pub watching: bool,
    /// Feedback for the last thing that was refused.
    pub status: String,
}

/// A `Start` that arrived while this peer was still in the last match: the
/// players, and the host that sent it. Kept apart from [`Lobby`], whose
/// changes redraw the lobby.
#[derive(Resource, Default)]
pub struct PendingStart(pub Option<(Vec<Member>, PeerId)>);

impl Lobby {
    pub fn players(&self) -> impl Iterator<Item = &Member> {
        self.members.iter().filter(|m| !m.watching)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LobbyAction {
    AddBot,
    RemoveBot,
    ToggleWatch,
    Start,
}

/// Actions requested this frame, by the keyboard or by a test.
#[derive(Resource, Default)]
pub struct LobbyInput(pub Vec<LobbyAction>);

/// How I appear in the roster: my peer id, or `local` before the signaling
/// server has given me one (offline solo play).
pub fn my_key(net: &NetState) -> String {
    net.my_id
        .map_or_else(|| "local".to_string(), |id| id.to_string())
}

/// A member's name as shown to me: marked when it is me or a bot.
pub fn display_name(m: &Member, my_key: &str) -> String {
    let you = if m.peer.as_deref() == Some(my_key) {
        " (you)"
    } else {
        ""
    };
    let bot = if m.is_bot() { " [bot]" } else { "" };
    format!("{}{you}{bot}", m.name)
}

pub fn plugin(app: &mut App) {
    // The room must exist before anything runs: the initial state's OnEnter,
    // which opens the socket, runs ahead of Startup.
    if !app.world().contains_resource::<RoomId>() {
        let room = web::room_from_url().unwrap_or_else(web::random_room);
        web::share_room(&room);
        info!(room = %room.0, "joining room");
        app.insert_resource(room);
    }
    let mut net = app.world_mut().get_resource_or_init::<NetState>();
    if net.name.is_empty() {
        net.name = web::player_name();
    }
    app.init_state::<AppState>()
        .init_resource::<PendingStart>()
        .add_systems(Startup, quick_start)
        .add_systems(OnEnter(AppState::Lobby), open_socket)
        .add_systems(
            Update,
            (
                elect_host,
                resume_pending,
                greet,
                pump_lobby,
                host_roster,
                apply_input,
            )
                .chain()
                .run_if(in_state(AppState::Lobby)),
        );
}

/// Development shortcut (native only): `CAKE_BOTS=n` fills the ring with `n`
/// bots and starts straight away; `CAKE_WATCH=1` watches instead of playing.
fn quick_start(mut input: ResMut<LobbyInput>) {
    #[cfg(not(target_family = "wasm"))]
    {
        let Some(bots) = std::env::var("CAKE_BOTS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
        else {
            return;
        };
        if std::env::var("CAKE_WATCH").is_ok_and(|v| v == "1") {
            input.0.push(LobbyAction::ToggleWatch);
        }
        input.0.extend(std::iter::repeat_n(
            LobbyAction::AddBot,
            bots.min(MAX_SEATS),
        ));
        input.0.push(LobbyAction::Start);
    }
    #[cfg(target_family = "wasm")]
    let _ = &mut input;
}

/// Fold the socket's peer changes into [`NetState`].
pub fn sync_peers(socket: &mut MatchboxSocket, net: &mut ResMut<NetState>) {
    for (peer, state) in socket.update_peers() {
        match state {
            PeerState::Connected => {
                if !net.peers.contains(&peer) {
                    info!(%peer, "peer connected");
                    net.peers.push(peer);
                }
            }
            PeerState::Disconnected => {
                if net.peers.contains(&peer) {
                    info!(%peer, "peer disconnected");
                    net.peers.retain(|p| *p != peer);
                    net.greeted.retain(|p| *p != peer);
                }
            }
        }
    }
}

/// Smallest peer id hosts. Recomputed every frame, so host loss self-heals.
pub fn elect_host(socket: Option<ResMut<MatchboxSocket>>, mut net: ResMut<NetState>) {
    let Some(mut socket) = socket else {
        return;
    };
    sync_peers(&mut socket, &mut net);
    if net.my_id.is_none()
        && let Some(id) = socket.id()
    {
        net.my_id = Some(id);
    }
    let Some(me) = net.my_id else {
        return;
    };
    let is_host = net.peers.iter().all(|p| me.to_string() < p.to_string());
    if is_host != net.is_host {
        net.is_host = is_host;
        info!(name = %net.name, is_host, "host election");
    }
}

/// Begin a match whose `Start` arrived while the previous one was on screen.
fn resume_pending(
    mut commands: Commands,
    net: Res<NetState>,
    mut pending: ResMut<PendingStart>,
    mut next: ResMut<NextState<AppState>>,
) {
    if let Some((players, host)) = pending.0.take() {
        begin(&mut commands, &net, &mut next, players, false, Some(host));
    }
}

/// Introduce myself to every peer once.
fn greet(socket: Option<ResMut<MatchboxSocket>>, mut net: ResMut<NetState>) {
    let Some(mut socket) = socket else {
        return;
    };
    let fresh: Vec<PeerId> = net
        .peers
        .iter()
        .copied()
        .filter(|p| !net.greeted.contains(p))
        .collect();
    for peer in fresh {
        send_to(
            &mut socket,
            peer,
            &NetMsg::Hello {
                name: net.name.clone(),
            },
        );
        net.greeted.push(peer);
    }
}

fn pump_lobby(
    mut commands: Commands,
    socket: Option<ResMut<MatchboxSocket>>,
    mut net: ResMut<NetState>,
    mut lobby: ResMut<Lobby>,
    mut next: ResMut<NextState<AppState>>,
) {
    let Some(mut socket) = socket else {
        return;
    };
    let inbox: Vec<(PeerId, Box<[u8]>)> = socket.channel_mut(cake_net::CH_RELIABLE).receive();
    for (from, raw) in inbox {
        let Some(msg) = decode(&raw) else {
            continue;
        };
        match msg {
            NetMsg::Hello { name } => {
                net.names.retain(|(p, _)| *p != from);
                net.names.push((from, name));
            }
            NetMsg::Roster(members) if !net.sequences() => {
                // Keep my own watch preference in step with what the host
                // recorded.
                let me = my_key(&net);
                if let Some(mine) = members.iter().find(|m| m.peer.as_deref() == Some(&me)) {
                    lobby.watching = mine.watching;
                }
                lobby.members = members;
            }
            NetMsg::Watch(watching) if net.sequences() => {
                let key = from.to_string();
                if let Some(m) = lobby
                    .members
                    .iter_mut()
                    .find(|m| m.peer.as_deref() == Some(&key))
                {
                    m.watching = watching;
                }
            }
            NetMsg::Start { players } => {
                begin(&mut commands, &net, &mut next, players, false, Some(from));
            }
            // Stale match traffic, or messages only the host acts on.
            NetMsg::Roster(_)
            | NetMsg::Watch(_)
            | NetMsg::Cmd(_)
            | NetMsg::Turn { .. }
            | NetMsg::Hash { .. }
            | NetMsg::Desync { .. } => {}
        }
    }
}

/// The host keeps the roster: me first, then every connected peer, then the
/// bots. Broadcast whenever it changes.
fn host_roster(
    socket: Option<ResMut<MatchboxSocket>>,
    net: Res<NetState>,
    mut lobby: ResMut<Lobby>,
) {
    if !net.sequences() {
        return;
    }
    let watching_of = |key: &str, lobby: &Lobby| {
        lobby
            .members
            .iter()
            .find(|m| m.peer.as_deref() == Some(key))
            .is_some_and(|m| m.watching)
    };
    let mut roster = vec![Member {
        peer: Some(my_key(&net)),
        name: net.name.clone(),
        watching: lobby.watching,
    }];
    for peer in &net.peers {
        let key = peer.to_string();
        roster.push(Member {
            name: net
                .name_of(*peer)
                .map_or_else(|| key.chars().take(6).collect(), str::to_string),
            watching: watching_of(&key, &lobby),
            peer: Some(key),
        });
    }
    roster.extend(lobby.members.iter().filter(|m| m.is_bot()).cloned());
    if roster != lobby.members {
        lobby.members = roster;
        if let Some(mut socket) = socket {
            broadcast(
                &mut socket,
                &net.peers,
                &NetMsg::Roster(lobby.members.clone()),
            );
        }
    }
}

fn apply_input(
    mut commands: Commands,
    mut input: ResMut<LobbyInput>,
    socket: Option<ResMut<MatchboxSocket>>,
    net: Res<NetState>,
    mut lobby: ResMut<Lobby>,
    mut next: ResMut<NextState<AppState>>,
) {
    let mut socket = socket;
    for action in std::mem::take(&mut input.0) {
        let hosting = net.sequences();
        match action {
            LobbyAction::AddBot | LobbyAction::RemoveBot if !hosting => {
                lobby.status = "Only the host adds and removes bots.".into();
            }
            LobbyAction::AddBot => {
                if lobby.players().count() >= MAX_SEATS {
                    lobby.status = format!("The ring seats at most {MAX_SEATS}.");
                    continue;
                }
                let n = lobby.members.iter().filter(|m| m.is_bot()).count() + 1;
                lobby.members.push(Member {
                    peer: None,
                    name: format!("bot {n}"),
                    watching: false,
                });
                lobby.status.clear();
            }
            LobbyAction::RemoveBot => {
                if let Some(i) = lobby.members.iter().rposition(Member::is_bot) {
                    lobby.members.remove(i);
                }
            }
            LobbyAction::ToggleWatch => {
                lobby.watching = !lobby.watching;
                let watching = lobby.watching;
                // My own entry follows at once, so a Start later this frame
                // already sees it.
                let me = my_key(&net);
                if let Some(mine) = lobby
                    .members
                    .iter_mut()
                    .find(|m| m.peer.as_deref() == Some(me.as_str()))
                {
                    mine.watching = watching;
                }
                if !hosting
                    && let Some(socket) = socket.as_mut()
                    && let Some(host) = net.host()
                {
                    send_to(socket, host, &NetMsg::Watch(watching));
                }
            }
            LobbyAction::Start if !hosting => {
                lobby.status = "Only the host can start. Waiting for them.".into();
            }
            LobbyAction::Start => {
                let mut players: Vec<Member> = lobby.players().cloned().collect();
                if players.is_empty() {
                    lobby.status = "Nobody is playing: add a bot, or stop watching.".into();
                    continue;
                }
                if players.len() > MAX_SEATS {
                    lobby.status = format!("The ring seats at most {MAX_SEATS}.");
                    continue;
                }
                shuffle(&mut players, web::fresh_seed());
                name_bots(&mut players);
                if let Some(socket) = socket.as_mut() {
                    broadcast(
                        socket,
                        &net.peers,
                        &NetMsg::Start {
                            players: players.clone(),
                        },
                    );
                }
                lobby.status.clear();
                begin(&mut commands, &net, &mut next, players, true, None);
            }
        }
    }
}

/// Bots are named for their temperament, which their seat decides ("Turtle",
/// "Raider 2"), so everyone watching can tell who is who. Done by the host
/// once the seats are dealt, before the `Start` goes out, so every peer sees
/// the same names.
fn name_bots(players: &mut [Member]) {
    let names: Vec<&str> = (0..players.len())
        .map(|seat| temperament(seat as Seat).name)
        .collect();
    for (seat, m) in players.iter_mut().enumerate() {
        if !m.is_bot() {
            continue;
        }
        let name = names[seat];
        let before = (0..seat).filter(|&s| names[s] == name).count();
        m.name = if before == 0 {
            name.to_string()
        } else {
            format!("{name} {}", before + 1)
        };
    }
}

/// The temperament of the bot in `seat`: set by the seat, or (native, for
/// recording demos) listed in `CAKE_TEMPERS`, as in `turtle,warlord,raider`.
pub fn temperament(seat: Seat) -> cake_ai::Personality {
    #[cfg(not(target_family = "wasm"))]
    if let Some(p) = std::env::var("CAKE_TEMPERS").ok().and_then(|list| {
        list.split(',')
            .nth(seat as usize)
            .and_then(cake_ai::personality::by_name)
    }) {
        return p;
    }
    cake_ai::personality::for_seat(seat)
}

/// Fisher–Yates with a small xorshift. Only the host shuffles, and it sends
/// the result, so this needs no determinism across peers.
fn shuffle<T>(items: &mut [T], mut seed: u64) {
    seed |= 1;
    for i in (1..items.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        items.swap(i, (seed % (i as u64 + 1)) as usize);
    }
}

/// Start the match on this peer.
fn begin(
    commands: &mut Commands,
    net: &NetState,
    next: &mut NextState<AppState>,
    players: Vec<Member>,
    hosting: bool,
    host_peer: Option<PeerId>,
) {
    info!(
        players = ?players.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
        hosting,
        "match starts"
    );
    commands.insert_resource(Match::new(players, &my_key(net), hosting, host_peer));
    next.set(AppState::Game);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bots_are_named_for_their_seat_and_people_keep_theirs() {
        let bot = |n: u32| Member {
            peer: None,
            name: format!("bot {n}"),
            watching: false,
        };
        let person = Member {
            peer: Some("p".into()),
            name: "ada".into(),
            watching: false,
        };
        let mut players = vec![bot(1), person.clone(), bot(2), bot(3), bot(4), bot(5)];
        name_bots(&mut players);
        assert_eq!(players[1], person, "a person keeps their name");
        for (seat, m) in players.iter().enumerate().filter(|(_, m)| m.is_bot()) {
            let temperament = cake_ai::personality::for_seat(seat as Seat).name;
            assert!(m.name.starts_with(temperament), "{} in seat {seat}", m.name);
        }
        // Seats 0 and 4 share a temperament: the second is numbered.
        assert_ne!(players[0].name, players[4].name);
        assert!(players[4].name.ends_with(" 2"), "{}", players[4].name);
    }

    #[test]
    fn shuffling_keeps_every_item() {
        let mut v: Vec<u32> = (0..8).collect();
        shuffle(&mut v, 12345);
        let mut sorted = v.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..8).collect::<Vec<_>>());
    }

    #[test]
    fn shuffling_depends_on_the_seed() {
        let orders: std::collections::BTreeSet<Vec<u32>> = (0..20u64)
            .map(|seed| {
                let mut v: Vec<u32> = (0..6).collect();
                shuffle(&mut v, seed * 7919);
                v
            })
            .collect();
        assert!(orders.len() > 5);
    }
}
