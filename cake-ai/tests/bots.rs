//! Bots-only matches: the whole simulation under realistic load.

use cake_ai::Bot;
use cake_core::stats::{Kind, TICK_HZ};
use cake_core::{Event, Seat, Sim};

/// Play `seats` bots against each other for `ticks`, the way the host does:
/// every bot thinks against the state as it stands, and the commands become
/// the next tick's turn.
fn play(seats: usize, ticks: u32) -> (Sim, Stats) {
    let mut sim = Sim::new(seats);
    let mut bots: Vec<Bot> = (0..seats as Seat).map(Bot::new).collect();
    let mut stats = Stats::default();
    for _ in 0..ticks {
        let mut turn = Vec::new();
        for bot in &mut bots {
            for cmd in bot.think(&sim) {
                turn.push((bot.seat(), cmd));
            }
        }
        sim.step(&turn);
        for e in &sim.events {
            match e {
                Event::Died { .. } => stats.deaths += 1,
                Event::Completed { kind: Kind::Econ, .. } => stats.econ += 1,
                Event::Completed { kind: Kind::Turret, .. } => stats.turrets += 1,
                Event::Eliminated(_) => stats.eliminated += 1,
                _ => {}
            }
        }
        if sim.outcome.is_some() {
            break;
        }
    }
    (sim, stats)
}

#[derive(Default, Debug)]
struct Stats {
    deaths: usize,
    econ: usize,
    turrets: usize,
    eliminated: usize,
}

const FIVE_MINUTES: u32 = 5 * 60 * TICK_HZ;

#[test]
fn a_bot_match_replays_identically() {
    let (a, stats) = play(6, FIVE_MINUTES);
    let (b, _) = play(6, FIVE_MINUTES);
    assert_eq!(a.tick, b.tick);
    assert_eq!(a.checksum(), b.checksum());
    eprintln!("after {} ticks: {stats:?}, checksum {:#x}", a.tick, a.checksum());
}

#[test]
fn bots_build_an_economy_and_fight() {
    let (sim, stats) = play(6, FIVE_MINUTES);
    assert!(stats.econ >= 6, "every bot should deploy economy: {stats:?}");
    assert!(stats.turrets >= 3, "bots fortify: {stats:?}");
    assert!(stats.deaths >= 20, "five minutes in, there should be fighting: {stats:?}");
    assert!(sim.entities.len() > 20);
}

#[test]
#[ignore = "slow: plays bots to the end"]
fn a_bot_match_ends() {
    let (sim, stats) = play(4, 60 * 60 * TICK_HZ);
    eprintln!("ended at {} s: {:?} {stats:?}", sim.tick / TICK_HZ, sim.outcome);
    assert!(sim.outcome.is_some(), "an hour of bots should finish: {stats:?}");
}

/// A tripwire for accidental nondeterminism, or a rules change that was not
/// meant to change play: this exact match must end in this exact state on
/// every platform. When a change *should* alter play (tuning, new rules),
/// update the constant from the failure message.
#[test]
fn a_bot_match_ends_in_the_known_state() {
    const GOLDEN: u64 = 0x4cdc_1a87_58a4_11e6;
    let (sim, _) = play(6, FIVE_MINUTES);
    assert_eq!(
        sim.checksum(),
        GOLDEN,
        "the reference match now ends at {:#x}; update GOLDEN if this change was meant to alter play",
        sim.checksum()
    );
}
