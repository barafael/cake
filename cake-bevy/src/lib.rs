//! Bevy front-end for cake.
//!
//! [`logic_plugin`] is everything that works without a window: the room, the
//! lobby conversation, and lockstep. [`view_plugin`] is the window side:
//! camera, drawing, input and HUD. Headless tests run the first alone.

use bevy::prelude::*;
use bevy_matchbox::prelude::PeerId;
use cake_ai::Bot;
use cake_core::geom::Pos;
use cake_core::history::History;
use cake_core::stats::TICK_HZ;
use cake_core::{Command, Entity, EntityId, Event, Seat, Sim};
use cake_net::{HASH_INTERVAL, HashLog, Member, NetState, Sequencer, Signaling, TurnBuffer};

pub mod arctext;
pub mod camera;
pub mod chrome;
#[cfg(not(target_family = "wasm"))]
pub mod demo;
pub mod fx;
pub mod hud;
pub mod input;
pub mod lobby;
pub mod lobby_ring;
pub mod lockstep;
pub mod menu;
pub mod nebula;
pub mod palette;
pub mod recap;
pub mod render;
pub mod ringmesh;
pub mod segments;
pub mod settings;
pub mod settings_window;
pub mod web;

/// Seconds per simulation tick.
pub const TICK_SECS: f32 = 1.0 / TICK_HZ as f32;

#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AppState {
    #[default]
    Lobby,
    Game,
}

/// A match in progress, on this peer.
#[derive(Resource)]
pub struct Match {
    pub sim: Sim,
    /// Positions before the last applied tick, in id order, for smoothing
    /// motion between ticks.
    pub prev: Vec<(EntityId, Pos)>,
    /// Where the screen is between `prev` (0) and the current state (1).
    pub alpha: f32,
    /// My seat, or `None` when watching.
    pub me: Option<Seat>,
    /// Everyone playing, in ring order: `players[i]` owns seat `i`.
    pub players: Vec<Member>,
    pub turns: TurnBuffer,
    /// Commands issued here, not yet handed to the host.
    pub outbox: Vec<Command>,
    /// Set on the host only.
    pub host: Option<HostSide>,
    /// Guests: the peer whose turns this match follows.
    pub host_peer: Option<PeerId>,
    /// Guests: time banked toward the next tick.
    pub accum: f32,
    /// The first tick at which some peer's state was seen to diverge.
    pub desync: Option<u32>,
    pub host_left: bool,
    /// Events from recently applied ticks, for the renderer to consume.
    pub events: Vec<Event>,
    /// Everyone's standing over time, for the charts at the end.
    pub history: History,
}

/// What only the host keeps: the clock, the sequencer and the bots.
pub struct HostSide {
    pub seq: Sequencer,
    pub hashes: HashLog,
    pub bots: Vec<Bot>,
    pub accum: f32,
    /// Guest checksums that matched mine.
    pub verified: u32,
}

/// Turns a peer lets pile up before it runs extra ticks to catch up.
const TARGET_BACKLOG: usize = 2;

impl Match {
    /// Set up a match from the host's `Start`. `my_key` is how I appear in
    /// `players` (see [`lobby::my_key`]); `hosting` makes this peer the clock.
    pub fn new(
        players: Vec<Member>,
        my_key: &str,
        hosting: bool,
        host_peer: Option<PeerId>,
    ) -> Match {
        let me = players
            .iter()
            .position(|m| m.peer.as_deref() == Some(my_key))
            .map(|i| i as Seat);
        let host = hosting.then(|| HostSide {
            seq: Sequencer::default(),
            hashes: HashLog::default(),
            bots: players
                .iter()
                .enumerate()
                .filter(|(_, m)| m.is_bot())
                .map(|(i, _)| Bot::with_personality(i as Seat, lobby::temperament(i as Seat)))
                .collect(),
            accum: 0.0,
            verified: 0,
        });
        let sim = Sim::new(players.len());
        Match {
            history: History::new(&sim),
            sim,
            prev: Vec::new(),
            alpha: 1.0,
            me,
            players,
            turns: TurnBuffer::default(),
            outbox: Vec::new(),
            host,
            host_peer,
            accum: 0.0,
            desync: None,
            host_left: false,
            events: Vec::new(),
        }
    }

    pub fn is_host(&self) -> bool {
        self.host.is_some()
    }

    /// The seat `peer` plays, if any.
    pub fn seat_of(&self, peer: &str) -> Option<Seat> {
        self.players
            .iter()
            .position(|m| m.peer.as_deref() == Some(peer))
            .map(|i| i as Seat)
    }

    /// Host: close the current tick. Bots think against the state as it
    /// stands, then everything submitted becomes the turn, which goes into my
    /// own buffer too. Returns the turn so it can be broadcast.
    pub fn cut_turn(&mut self) -> Option<(u32, Vec<(Seat, Command)>)> {
        let host = self.host.as_mut()?;
        for bot in &mut host.bots {
            for cmd in bot.think(&self.sim) {
                host.seq.submit(bot.seat(), cmd);
            }
        }
        let (tick, cmds) = host.seq.cut();
        self.turns.push(tick, cmds.clone());
        Some((tick, cmds))
    }

    /// Apply the next turn if it has arrived. Returns `(tick, checksum)` when
    /// this tick is one peers compare.
    pub fn apply_next(&mut self) -> Option<Option<(u32, u64)>> {
        let (_, cmds) = self.turns.pop()?;
        self.prev.clear();
        self.prev
            .extend(self.sim.entities.iter().map(|e| (e.id, e.pos)));
        self.sim.step(&cmds);
        self.history.observe(&self.sim);
        self.events.extend(self.sim.events.iter().cloned());
        // A renderer that isn't running (headless, minimised) must not let
        // this grow without bound.
        if self.events.len() > 4096 {
            self.events.drain(..2048);
        }
        let tick = self.sim.tick;
        Some(
            tick.is_multiple_of(HASH_INTERVAL)
                .then(|| (tick, self.sim.checksum())),
        )
    }

    /// Advance by up to one tick's worth of banked time, plus catch-up ticks
    /// if turns have piled up. Returns the checksums due for comparison.
    pub fn advance(&mut self, dt: f32) -> Vec<(u32, u64)> {
        let mut reports = Vec::new();
        if self.is_host() {
            // The host is the clock, and applies its turns as it cuts them
            // (see `lockstep`); anything still buffered goes now.
            while let Some(report) = self.apply_next() {
                reports.extend(report);
            }
            self.alpha = self.host.as_ref().map_or(1.0, |h| h.accum / TICK_SECS);
            return reports;
        }
        self.accum += dt;
        while self.accum >= TICK_SECS {
            match self.apply_next() {
                Some(report) => {
                    reports.extend(report);
                    self.accum -= TICK_SECS;
                }
                None => {
                    // Starved: bank no more than one tick, or a burst of
                    // arrivals would be played back in fast-forward.
                    self.accum = self.accum.min(TICK_SECS);
                    break;
                }
            }
        }
        while self.turns.ready() > TARGET_BACKLOG {
            if let Some(report) = self.apply_next() {
                reports.extend(report);
            }
        }
        self.alpha = (self.accum / TICK_SECS).clamp(0.0, 1.0);
        reports
    }

    /// Where `e` is drawn: between its previous and current position.
    pub fn draw_pos(&self, e: &Entity) -> Vec2 {
        match self.prev.binary_search_by_key(&e.id, |(id, _)| *id) {
            Ok(i) => self.between(self.prev[i].1, e.pos),
            Err(_) => render::to_vec2(e.pos),
        }
    }

    /// Where something that moved from `before` to `now` in the last tick is
    /// drawn, following the ring.
    pub fn between(&self, before: Pos, now: Pos) -> Vec2 {
        let da = now.a.delta(before.a) as f64 * self.alpha as f64;
        let r = before.r as f64 + (now.r - before.r) as f64 * self.alpha as f64;
        render::to_vec2(Pos {
            a: before.a.turned(da as i64),
            r: r as i64,
        })
    }

    /// Can I see `e`? Watchers see everything.
    pub fn sees(&self, e: &Entity) -> bool {
        cake_core::vision::visible(e, self.me)
    }
}

/// Networking, lobby and lockstep: everything that runs without a window.
pub fn logic_plugin(app: &mut App) {
    app.init_resource::<NetState>()
        .init_resource::<Signaling>()
        .init_resource::<lobby::Lobby>()
        .init_resource::<lobby::LobbyInput>()
        .add_plugins((lobby::plugin, lockstep::plugin));
}

/// The window side: camera, chrome, drawing, input and HUD. Expects a
/// [`settings::Settings`] resource (see `main`).
pub fn view_plugin(app: &mut App) {
    app.add_plugins((
        arctext::plugin,
        camera::plugin,
        chrome::plugin,
        lobby_ring::plugin,
        menu::plugin,
        nebula::plugin,
        segments::plugin,
        settings_window::plugin,
        render::plugin,
        fx::plugin,
        input::plugin,
        hud::plugin,
        recap::plugin,
    ));
}
