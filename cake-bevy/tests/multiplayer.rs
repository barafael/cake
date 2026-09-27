//! Live multiplayer: three real instances in one room, over real WebRTC.
//!
//! Each instance is its own headless Bevy app running cake's own
//! [`logic_plugin`]: host election, greetings, roster, bots, the `Start`, and
//! then lockstep. Peers are introduced by an in-process full-mesh signaling
//! server (the harness is chinese-checke.rs's) and talk over real data
//! channels. The host verifies the guests' checksums as the match runs, and
//! the test checks that it did, and that none diverged.
//!
//! Slow (it waits for real handshakes), so it is opt-in:
//!
//! ```sh
//! cargo test -p cake-bevy --test multiplayer -- --ignored --nocapture
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use cake_bevy::lobby::{Lobby, LobbyAction, LobbyInput};
use cake_bevy::{AppState, Match, logic_plugin};
use cake_net::{NetState, RoomId, Signaling};

fn start_signaling_server() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("a free loopback port");
    let port = probe.local_addr().expect("probe address").port();
    drop(probe);
    std::thread::Builder::new()
        .name("signaling-server".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("a tokio runtime for the signaling server");
            let server = matchbox_signaling::SignalingServer::full_mesh_builder(
                std::net::SocketAddr::new(
                    std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)),
                    port,
                ),
            )
            .build();
            runtime
                .block_on(server.serve())
                .expect("the signaling server ran");
        })
        .expect("spawn the signaling server thread");
    port
}

static ROOM_SEQ: AtomicU64 = AtomicU64::new(0);

fn fresh_room(label: &str) -> RoomId {
    let n = ROOM_SEQ.fetch_add(1, Ordering::Relaxed);
    RoomId::parse(&format!("mp-{label}-{n}-{:x}", std::process::id())).expect("valid room")
}

fn instance(name: &str, room: &RoomId, port: u16) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin))
        .insert_resource(room.clone())
        .insert_resource(Signaling(format!("ws://127.0.0.1:{port}")))
        .add_plugins(logic_plugin);
    app.world_mut().resource_mut::<NetState>().name = name.into();
    app
}

fn wait_for(apps: &mut [App], timeout: Duration, mut ok: impl FnMut(&[App]) -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if ok(apps) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        for app in apps.iter_mut() {
            app.update();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn state(app: &App) -> AppState {
    *app.world().resource::<State<AppState>>().get()
}

fn describe(apps: &[App]) -> String {
    apps.iter()
        .map(|a| {
            let n = a.world().resource::<NetState>();
            let lobby = a.world().resource::<Lobby>();
            let tick = a.world().get_resource::<Match>().map(|m| m.sim.tick);
            format!(
                "{}: peers={} host={} members={} state={:?} tick={tick:?} status={}",
                n.name,
                n.peers.len(),
                n.is_host,
                lobby.members.len(),
                state(a),
                lobby.status
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
#[ignore = "real WebRTC handshakes: slow"]
fn three_peers_and_a_bot_play_in_lockstep() {
    let port = start_signaling_server();
    let room = fresh_room("ring");
    let mut apps: Vec<App> = ["ada", "bea", "cyd"]
        .iter()
        .map(|n| instance(n, &room, port))
        .collect();

    assert!(
        wait_for(&mut apps, Duration::from_secs(30), |apps| {
            apps.iter().all(|a| {
                a.world().resource::<NetState>().peers.len() == 2
                    && a.world().resource::<Lobby>().members.len() == 3
            })
        }),
        "peers never all met:\n{}",
        describe(&apps)
    );

    let host = apps
        .iter()
        .position(|a| a.world().resource::<NetState>().is_host)
        .expect("somebody hosts");
    apps[host]
        .world_mut()
        .resource_mut::<LobbyInput>()
        .0
        .extend([LobbyAction::AddBot, LobbyAction::Start]);

    assert!(
        wait_for(&mut apps, Duration::from_secs(10), |apps| {
            apps.iter().all(|a| state(a) == AppState::Game)
        }),
        "the match never started everywhere:\n{}",
        describe(&apps)
    );
    for a in &apps {
        let m = a.world().resource::<Match>();
        assert_eq!(m.players.len(), 4, "three peers and a bot");
        assert!(m.me.is_some(), "every peer plays");
    }

    // Twenty seconds of match time.
    assert!(
        wait_for(&mut apps, Duration::from_secs(60), |apps| {
            apps.iter()
                .all(|a| a.world().resource::<Match>().sim.tick >= 400)
        }),
        "the match did not advance everywhere:\n{}",
        describe(&apps)
    );

    let m = apps[host].world().resource::<Match>();
    let verified = m.host.as_ref().expect("the host hosts").verified;
    assert_eq!(m.desync, None, "no peer may diverge");
    assert!(
        verified >= 30,
        "both guests should have reported matching checksums, got {verified}"
    );
    for a in &apps {
        assert_eq!(a.world().resource::<Match>().desync, None);
    }
}
