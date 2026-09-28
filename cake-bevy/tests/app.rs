//! The app as it boots: `logic_plugin` alone, with nothing pre-inserted, and
//! a signaling server that is not there. A solo player must still get a
//! match against bots.

use std::time::Duration;

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use cake_bevy::lobby::{Lobby, LobbyAction, LobbyInput};
use cake_bevy::{AppState, Match, logic_plugin};
use cake_net::{RoomId, Signaling};

#[test]
fn offline_solo_play_boots_into_a_match_against_bots() {
    let mut app = App::new();
    // Port 9 (discard) on loopback: nothing listens, so the socket never
    // connects, which is what being offline looks like.
    app.add_plugins((MinimalPlugins, StatesPlugin))
        .insert_resource(Signaling("ws://127.0.0.1:9".into()))
        .add_plugins(logic_plugin);
    assert!(
        app.world().contains_resource::<RoomId>(),
        "a room is chosen at build time"
    );

    for _ in 0..5 {
        app.update();
    }
    assert_eq!(app.world().resource::<Lobby>().members.len(), 1, "just me");

    app.world_mut().resource_mut::<LobbyInput>().0.extend([
        LobbyAction::AddBot,
        LobbyAction::AddBot,
        LobbyAction::Start,
    ]);
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(
        *app.world().resource::<State<AppState>>().get(),
        AppState::Game
    );

    for _ in 0..30 {
        app.update();
        std::thread::sleep(Duration::from_millis(10));
    }
    let m = app.world().resource::<Match>();
    assert_eq!(m.players.len(), 3);
    assert!(m.me.is_some());
    assert!(m.is_host(), "alone, I am my own host");
    assert!(m.sim.tick > 0, "the match advances");
}

#[test]
fn a_host_who_watches_is_not_seated() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin))
        .insert_resource(Signaling("ws://127.0.0.1:9".into()))
        .add_plugins(logic_plugin);
    app.update();
    // All in one frame, as the CAKE_WATCH quick start does it.
    app.world_mut().resource_mut::<LobbyInput>().0.extend([
        LobbyAction::ToggleWatch,
        LobbyAction::AddBot,
        LobbyAction::AddBot,
        LobbyAction::Start,
    ]);
    for _ in 0..3 {
        app.update();
    }
    let m = app.world().resource::<Match>();
    assert_eq!(m.players.len(), 2, "two bots, and I watch");
    assert_eq!(m.me, None);
}
