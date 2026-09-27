//! Wire protocol and host-sequenced lockstep for networked play.
//!
//! Peer-to-peer over WebRTC via `bevy_matchbox`, exactly as in
//! chinese-checke.rs: the signaling server only introduces peers, and one peer
//! (the one with the smallest id) is **host**, the single sequencing authority.
//!
//! Chinese checkers orders moves; cake orders ticks:
//!
//! 1. A guest submits a [`Command`] as [`NetMsg::Cmd`], to the host only.
//! 2. The host is the clock. Every tick it cuts a [`NetMsg::Turn`] holding
//!    every command that arrived since the last one, tagged with the sender's
//!    seat *as the roster says*, and sends it to everyone, itself included.
//!    Turns are cut even when empty.
//! 3. Every peer advances its [`cake_core::Sim`] only by applying turns, in
//!    order. The host applies its own turns through the same buffer.
//!
//! The simulation is deterministic, so identical turns give identical states.
//! Guests report a checksum every [`HASH_INTERVAL`] ticks, and the host flags
//! any mismatch as a desync.

use std::collections::{BTreeMap, VecDeque};

use bevy::prelude::*;
use bevy_matchbox::prelude::*;
use cake_core::{Command, Seat};
use serde::{Deserialize, Serialize};

/// Signaling server used to introduce peers, set at compile time with
/// `MATCHBOX_SERVER`.
///
/// The default is omdurman's shared deployment. Cake's rooms are prefixed with
/// `cake-` (see [`open_socket`]) so they cannot meet another game's peers.
/// Only introductions cross it; commands travel peer-to-peer.
pub const SIGNALING_SERVER: &str = match option_env!("MATCHBOX_SERVER") {
    Some(s) => s,
    None => "wss://omdurman-matchbox.fly.dev",
};

/// Reliable, ordered channel. Lockstep needs every turn, in order.
pub const CH_RELIABLE: usize = 0;

/// Guests send a checksum after every tick that is a multiple of this.
pub const HASH_INTERVAL: u32 = 20;

/// Somebody in the lobby.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Member {
    /// The peer's id as a string, or `None` for a bot. Bots are played by the
    /// host, and their commands are sequenced like anyone else's.
    pub peer: Option<String>,
    pub name: String,
    /// Watching rather than playing.
    pub watching: bool,
}

impl Member {
    pub fn is_bot(&self) -> bool {
        self.peer.is_none()
    }
}

/// Top-level wire envelope.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum NetMsg {
    /// Introduce yourself on join.
    Hello { name: String },
    /// Host -> all: the lobby, whenever it changes.
    Roster(Vec<Member>),
    /// Guest -> host: play (`false`) or watch (`true`).
    Watch(bool),
    /// Host -> all: the match begins. `players` are in ring order: player
    /// `i` sits in sector `i`.
    Start { players: Vec<Member> },
    /// Guest -> host: an unsequenced command. Never applied directly.
    Cmd(Command),
    /// Host -> all: everything that happens at `tick`. The only thing applied.
    Turn { tick: u32, cmds: Vec<(Seat, Command)> },
    /// Guest -> host: my checksum after applying `tick`.
    Hash { tick: u32, hash: u64 },
    /// Host -> all: some peer's state diverged at `tick`.
    Desync { tick: u32 },
}

pub fn encode(msg: &NetMsg) -> Option<Box<[u8]>> {
    match postcard::to_allocvec(msg) {
        // A WebRTC data channel can silently drop a zero-byte payload. Any
        // real NetMsg encodes to at least a variant tag, so this only happens
        // if encoding fails: skip the send rather than send nothing.
        Ok(v) if !v.is_empty() => Some(v.into_boxed_slice()),
        Ok(_) => {
            error!("postcard produced an empty NetMsg encoding; dropping");
            None
        }
        Err(error) => {
            error!(%error, "postcard encode failed");
            None
        }
    }
}

pub fn decode(raw: &[u8]) -> Option<NetMsg> {
    postcard::from_bytes(raw)
        .inspect_err(|error| warn!(%error, "NetMsg decode failed"))
        .ok()
}

/// Everything the front-end needs to know about the connection.
#[derive(Resource, Default)]
pub struct NetState {
    pub peers: Vec<PeerId>,
    pub my_id: Option<PeerId>,
    pub is_host: bool,
    pub name: String,
    /// Peers we have already sent our [`NetMsg::Hello`] to.
    pub greeted: Vec<PeerId>,
    /// Names peers introduced themselves with.
    pub names: Vec<(PeerId, String)>,
}

impl NetState {
    /// Drop everything the current room's socket established, but keep the
    /// player's own name.
    pub fn leave_room(&mut self) {
        let name = std::mem::take(&mut self.name);
        *self = Self {
            name,
            ..Self::default()
        };
    }

    /// Am I the sequencing authority? True for the host, and for a peer that
    /// is alone, so that a single player can start without anyone else.
    pub fn sequences(&self) -> bool {
        self.is_host || self.peers.is_empty()
    }

    /// The host: the smallest id among everyone present, me included.
    pub fn host(&self) -> Option<PeerId> {
        self.peers
            .iter()
            .copied()
            .chain(self.my_id)
            .min_by_key(|p| p.to_string())
    }

    pub fn name_of(&self, peer: PeerId) -> Option<&str> {
        self.names
            .iter()
            .find(|(p, _)| *p == peer)
            .map(|(_, n)| n.as_str())
    }
}

/// Turns received and not yet applied. Applies strictly in tick order, drops
/// duplicates, and holds turns that arrive early until the gap fills.
#[derive(Default, Debug)]
pub struct TurnBuffer {
    next: u32,
    pending: BTreeMap<u32, Vec<(Seat, Command)>>,
}

impl TurnBuffer {
    /// Store a turn. Returns `false` for a turn already applied or held.
    pub fn push(&mut self, tick: u32, cmds: Vec<(Seat, Command)>) -> bool {
        if tick < self.next || self.pending.contains_key(&tick) {
            return false;
        }
        self.pending.insert(tick, cmds);
        true
    }

    /// The next turn in order, if it has arrived.
    pub fn pop(&mut self) -> Option<(u32, Vec<(Seat, Command)>)> {
        let cmds = self.pending.remove(&self.next)?;
        let tick = self.next;
        self.next += 1;
        Some((tick, cmds))
    }

    /// How many turns could be applied right now, in order.
    pub fn ready(&self) -> usize {
        (self.next..)
            .take_while(|t| self.pending.contains_key(t))
            .count()
    }

    /// The tick the next applied turn will carry.
    pub fn next_tick(&self) -> u32 {
        self.next
    }
}

/// The host's side of lockstep: collects commands and cuts them into turns.
#[derive(Default, Debug)]
pub struct Sequencer {
    next: u32,
    pending: Vec<(Seat, Command)>,
}

impl Sequencer {
    /// Queue `cmd` from `seat` for the next turn.
    pub fn submit(&mut self, seat: Seat, cmd: Command) {
        self.pending.push((seat, cmd));
    }

    /// Close the current tick: everything submitted so far becomes its turn.
    pub fn cut(&mut self) -> (u32, Vec<(Seat, Command)>) {
        let tick = self.next;
        self.next += 1;
        (tick, std::mem::take(&mut self.pending))
    }
}

/// The host's recent checksums, to compare guests' reports against.
#[derive(Default, Debug)]
pub struct HashLog {
    recent: VecDeque<(u32, u64)>,
}

impl HashLog {
    const KEEP: usize = 64;

    pub fn record(&mut self, tick: u32, hash: u64) {
        self.recent.push_back((tick, hash));
        while self.recent.len() > Self::KEEP {
            self.recent.pop_front();
        }
    }

    /// `Some(true)` if `hash` matches mine at `tick`, `Some(false)` if it
    /// doesn't, `None` if I no longer (or don't yet) know that tick.
    pub fn check(&self, tick: u32, hash: u64) -> Option<bool> {
        self.recent
            .iter()
            .find(|(t, _)| *t == tick)
            .map(|(_, h)| *h == hash)
    }
}

/// Why a room name was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomIdError {
    Empty,
    TooLong { len: usize },
    /// The offending character, so the message can name it.
    BadChar(char),
}

impl core::fmt::Display for RoomIdError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RoomIdError::Empty => write!(f, "a room name cannot be empty"),
            RoomIdError::TooLong { len } => write!(
                f,
                "a room name may be at most {} characters, got {len}",
                RoomId::MAX_LEN
            ),
            RoomIdError::BadChar(c) => write!(
                f,
                "'{c}' is not allowed in a room name; use letters, digits, '-' or '_'"
            ),
        }
    }
}

impl core::error::Error for RoomIdError {}

/// The room to join. Peers sharing a room id find each other.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct RoomId(pub String);

impl RoomId {
    pub const MAX_LEN: usize = 40;

    /// Validate a room name. It is interpolated into the signaling URL's path,
    /// so only characters that need no escaping are allowed: ASCII letters,
    /// digits, `-` and `_`.
    pub fn parse(name: &str) -> Result<Self, RoomIdError> {
        if name.is_empty() {
            return Err(RoomIdError::Empty);
        }
        if name.chars().count() > Self::MAX_LEN {
            return Err(RoomIdError::TooLong {
                len: name.chars().count(),
            });
        }
        if let Some(c) = name
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_'))
        {
            return Err(RoomIdError::BadChar(c));
        }
        Ok(Self(name.to_string()))
    }
}

/// Where peers find each other: [`SIGNALING_SERVER`] unless something else is
/// inserted. A resource, so tests can point at an in-process server.
#[derive(Resource, Clone, Debug)]
pub struct Signaling(pub String);

impl Default for Signaling {
    fn default() -> Self {
        Self(SIGNALING_SERVER.to_string())
    }
}

/// The signaling URL for `room`: namespaced, so a cake room never meets a
/// chinese-checkers or omdurman room of the same name.
pub fn room_url(signaling: &Signaling, room: &RoomId) -> String {
    format!("{}/cake-{}", signaling.0, room.0)
}

/// Open the room's socket, unless one is already open. The socket lives as
/// long as the room: coming back from a match keeps it, and with it the peer
/// ids and the host.
pub fn open_socket(
    mut commands: Commands,
    room: Res<RoomId>,
    signaling: Res<Signaling>,
    socket: Option<Res<MatchboxSocket>>,
) {
    if socket.is_some() {
        return;
    }
    let url = room_url(&signaling, &room);
    info!(%url, "opening matchbox socket");
    commands.insert_resource(MatchboxSocket::from(
        WebRtcSocketBuilder::new(url)
            .reconnect_attempts(None)
            .add_reliable_channel(),
    ));
}

/// Send to one peer on the reliable channel.
pub fn send_to(socket: &mut MatchboxSocket, peer: PeerId, msg: &NetMsg) {
    let Some(bytes) = encode(msg) else {
        return;
    };
    if let Err(error) = socket.channel_mut(CH_RELIABLE).try_send(bytes, peer) {
        warn!(%error, "send failed");
    }
}

/// Send to every connected peer.
pub fn broadcast(socket: &mut MatchboxSocket, peers: &[PeerId], msg: &NetMsg) {
    let Some(bytes) = encode(msg) else {
        return;
    };
    for &peer in peers {
        if let Err(error) = socket
            .channel_mut(CH_RELIABLE)
            .try_send(bytes.clone(), peer)
        {
            warn!(%error, %peer, "send failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cake_core::{Angle, Kind, Pos};

    #[test]
    fn a_turn_round_trips_through_postcard() {
        let msg = NetMsg::Turn {
            tick: 77,
            cmds: vec![
                (2, Command::Produce(Kind::Raider)),
                (
                    0,
                    Command::Move {
                        units: vec![3, 5, 8],
                        to: Pos::new(Angle(12345), 432_000),
                    },
                ),
            ],
        };
        let bytes = encode(&msg).expect("encodes");
        let Some(NetMsg::Turn { tick, cmds }) = decode(&bytes) else {
            panic!("decoded to the wrong variant");
        };
        assert_eq!(tick, 77);
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0], (2, Command::Produce(Kind::Raider)));
    }

    #[test]
    fn decoding_garbage_yields_none() {
        assert!(decode(&[0xff, 0xff, 0xff, 0xff]).is_none());
    }

    #[test]
    fn turns_apply_in_order_and_wait_for_gaps() {
        let mut buf = TurnBuffer::default();
        assert!(buf.push(1, vec![]));
        assert_eq!(buf.ready(), 0, "turn 0 is missing");
        assert!(buf.pop().is_none());
        assert!(buf.push(0, vec![]));
        assert_eq!(buf.ready(), 2);
        assert_eq!(buf.pop().map(|t| t.0), Some(0));
        assert_eq!(buf.pop().map(|t| t.0), Some(1));
        assert!(buf.pop().is_none());
        assert_eq!(buf.next_tick(), 2);
    }

    #[test]
    fn duplicate_and_stale_turns_are_dropped() {
        let mut buf = TurnBuffer::default();
        assert!(buf.push(0, vec![]));
        assert!(!buf.push(0, vec![(1, Command::CancelProduce)]), "held already");
        buf.pop();
        assert!(!buf.push(0, vec![]), "applied already");
    }

    #[test]
    fn the_sequencer_numbers_turns_and_empties_between_them() {
        let mut seq = Sequencer::default();
        seq.submit(1, Command::CancelProduce);
        seq.submit(0, Command::Produce(Kind::Brawler));
        let (t0, c0) = seq.cut();
        assert_eq!(t0, 0);
        assert_eq!(c0.len(), 2);
        assert_eq!(c0[0].0, 1, "submission order is kept");
        let (t1, c1) = seq.cut();
        assert_eq!(t1, 1);
        assert!(c1.is_empty());
    }

    #[test]
    fn the_hash_log_spots_divergence_and_forgets_old_ticks() {
        let mut log = HashLog::default();
        log.record(20, 0xabc);
        assert_eq!(log.check(20, 0xabc), Some(true));
        assert_eq!(log.check(20, 0xdef), Some(false));
        assert_eq!(log.check(40, 0xabc), None);
        for t in 0..100 {
            log.record(1000 + t, t as u64);
        }
        assert_eq!(log.check(20, 0xabc), None);
    }

    #[test]
    fn a_solo_peer_sequences() {
        let mut net = NetState::default();
        assert!(net.sequences());
        net.is_host = true;
        assert!(net.sequences());
    }

    #[test]
    fn room_urls_are_namespaced() {
        let url = room_url(&Signaling("wss://x".into()), &RoomId::parse("abc").unwrap());
        assert_eq!(url, "wss://x/cake-abc");
    }

    #[test]
    fn room_names_are_validated() {
        for ok in ["a", "game", "Room_7", "my-game-2"] {
            assert!(RoomId::parse(ok).is_ok(), "{ok}");
        }
        assert_eq!(RoomId::parse(""), Err(RoomIdError::Empty));
        assert_eq!(RoomId::parse("a/b"), Err(RoomIdError::BadChar('/')));
        assert_eq!(RoomId::parse("café"), Err(RoomIdError::BadChar('é')));
        let long = "a".repeat(RoomId::MAX_LEN + 1);
        assert!(matches!(RoomId::parse(&long), Err(RoomIdError::TooLong { .. })));
    }
}
