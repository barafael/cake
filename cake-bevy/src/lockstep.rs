//! In-match networking: hand commands to the host, and apply turns.
//!
//! The one rule, as in chinese-checke.rs: **nothing reaches the simulation
//! except a sequenced turn**. The host's own commands and its bots' commands go
//! through its [`cake_net::Sequencer`] exactly like a guest's, and the host
//! applies its turns from the same buffer guests do. Solo play takes the same
//! path, so the networked code is always the code that runs.

use bevy::prelude::*;
use bevy_matchbox::prelude::*;
use cake_net::{CH_RELIABLE, NetMsg, NetState, broadcast, decode, send_to};

use crate::lobby::{Lobby, PendingStart, sync_peers};
use crate::{AppState, Match, TICK_SECS};

pub fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (pump, host_clock, advance)
            .chain()
            .run_if(in_state(AppState::Game).and_then(resource_exists::<Match>)),
    )
    .add_systems(OnExit(AppState::Game), |mut commands: Commands| {
        commands.remove_resource::<Match>();
    });
}

/// Hand my commands on, and take in what arrived.
fn pump(
    mut next: ResMut<NextState<AppState>>,
    socket: Option<ResMut<MatchboxSocket>>,
    mut net: ResMut<NetState>,
    mut m: ResMut<Match>,
    mut lobby: ResMut<Lobby>,
    mut pending: ResMut<PendingStart>,
) {
    let outbox = std::mem::take(&mut m.outbox);
    let Some(mut socket) = socket else {
        // No socket at all: this peer is alone and is its own host.
        if let (Some(me), Some(host)) = (m.me, m.host.as_mut()) {
            for cmd in outbox {
                host.seq.submit(me, cmd);
            }
        }
        return;
    };
    sync_peers(&mut socket, &mut net);

    if let Some(host_peer) = m.host_peer
        && !net.peers.contains(&host_peer)
        && !m.host_left
    {
        warn!("the host left the match");
        m.host_left = true;
    }

    // 1. My commands: straight into the sequencer on the host, to the host
    //    otherwise.
    let me = m.me;
    if let Some(host) = m.host.as_mut() {
        if let Some(me) = me {
            for cmd in outbox {
                host.seq.submit(me, cmd);
            }
        }
    } else if let Some(host_peer) = m.host_peer {
        for cmd in outbox {
            send_to(&mut socket, host_peer, &NetMsg::Cmd(cmd));
        }
    }

    // 2. What arrived.
    let inbox: Vec<(PeerId, Box<[u8]>)> = socket.channel_mut(CH_RELIABLE).receive();
    for (from, raw) in inbox {
        let Some(msg) = decode(&raw) else {
            continue;
        };
        match msg {
            // The sender's seat comes from the roster, never from the
            // message: a peer can only ever command its own seat.
            NetMsg::Cmd(cmd) if m.is_host() => {
                if let Some(seat) = m.seat_of(&from.to_string())
                    && let Some(host) = m.host.as_mut()
                {
                    host.seq.submit(seat, cmd);
                }
            }
            NetMsg::Turn { tick, cmds } if !m.is_host() && Some(from) == m.host_peer => {
                if !m.turns.push(tick, cmds) {
                    warn!(
                        tick,
                        "refused the host's turn: stale, duplicate, or past the horizon"
                    );
                }
            }
            NetMsg::Hash { tick, hash } if m.is_host() => {
                let verdict = m.host.as_ref().and_then(|h| h.hashes.check(tick, hash));
                if verdict == Some(true)
                    && let Some(host) = m.host.as_mut()
                {
                    host.verified += 1;
                }
                if verdict == Some(false) && m.desync.is_none() {
                    error!(tick, peer = %from, "desync: a guest's state diverged");
                    m.desync = Some(tick);
                    broadcast(&mut socket, &net.peers, &NetMsg::Desync { tick });
                }
            }
            NetMsg::Desync { tick } => {
                if m.desync.is_none() {
                    error!(tick, "desync reported by the host");
                    m.desync = Some(tick);
                }
            }
            NetMsg::Hello { name } => {
                net.names.retain(|(p, _)| *p != from);
                net.names.push((from, name));
            }
            // The host dealt a rematch while I was still looking at this one:
            // go back through the lobby, which starts it. Only the peer whose
            // turns I follow may deal one.
            NetMsg::Start { players } if Some(from) == m.host_peer => {
                pending.0 = Some((players, from));
                next.set(AppState::Lobby);
            }
            NetMsg::Roster(members) if Some(from) == m.host_peer => lobby.members = members,
            _ => {}
        }
    }
}

/// Development aid (native): `CAKE_SPEED=8` runs the host's clock eight times
/// as fast, to watch a bot match through to its end.
fn speed() -> f32 {
    static SPEED: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *SPEED.get_or_init(|| {
        std::env::var("CAKE_SPEED")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .map_or(1.0, |s| s.clamp(0.1, 64.0))
    })
}

/// The host's clock: one turn per tick of wall time.
fn host_clock(
    time: Res<Time>,
    socket: Option<ResMut<MatchboxSocket>>,
    net: Res<NetState>,
    mut m: ResMut<Match>,
) {
    let Some(host) = m.host.as_mut() else {
        return;
    };
    // A long hitch (a dragged window, a breakpoint) should not be replayed
    // as a burst of ticks: cap what one frame can owe.
    let speed = speed();
    host.accum = (host.accum + time.delta_secs() * speed).min(5.0 * TICK_SECS * speed);
    let mut socket = socket;
    while m.host.as_ref().is_some_and(|h| h.accum >= TICK_SECS) {
        if let Some(h) = m.host.as_mut() {
            h.accum -= TICK_SECS;
        }
        let Some((tick, cmds)) = m.cut_turn() else {
            break;
        };
        if let Some(socket) = socket.as_mut() {
            broadcast(socket, &net.peers, &NetMsg::Turn { tick, cmds });
        }
        // Apply it before cutting the next, so that bots think against the
        // state as it now stands: several turns can fall due in one frame.
        if let Some(Some((tick, hash))) = m.apply_next()
            && let Some(host) = m.host.as_mut()
        {
            host.hashes.record(tick, hash);
        }
    }
}

/// Apply whatever turns are due, and compare checksums.
fn advance(time: Res<Time>, socket: Option<ResMut<MatchboxSocket>>, mut m: ResMut<Match>) {
    let reports = m.advance(time.delta_secs());
    if reports.is_empty() {
        return;
    }
    if let Some(host) = m.host.as_mut() {
        for (tick, hash) in reports {
            host.hashes.record(tick, hash);
        }
    } else if let (Some(mut socket), Some(host_peer)) = (socket, m.host_peer) {
        for (tick, hash) in reports {
            send_to(&mut socket, host_peer, &NetMsg::Hash { tick, hash });
        }
    }
}
