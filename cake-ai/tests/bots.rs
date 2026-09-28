//! Bots-only matches, and the bots' character: they defend with zeal, they
//! differ, and they answer their neighbours.

use cake_ai::{Bot, Land, personality};
use cake_core::geom::{R_MID, UNIT, sector_center};
use cake_core::stats::{Kind, TICK_HZ};
use cake_core::{Command, EntityId, Event, Pos, Seat, Sim};

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
                Event::Died { .. } => {
                    stats.deaths += 1;
                    stats.first_death.get_or_insert(sim.tick);
                }
                Event::Completed {
                    kind: Kind::Econ, ..
                } => stats.econ += 1,
                Event::Completed { kind, .. } if kind.is_turret() => stats.turrets += 1,
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
    first_death: Option<u32>,
    econ: usize,
    turrets: usize,
    eliminated: usize,
}

const FIVE_MINUTES: u32 = 5 * 60 * TICK_HZ;

/// The tick seat `seat`'s bot next thinks on, at or after `from`.
fn thinking_tick(seat: Seat, from: u32) -> u32 {
    (from..)
        .find(|t| (t + seat as u32 * 3).is_multiple_of(TICK_HZ))
        .unwrap()
}

/// Step with no commands until seat `seat` is about to think.
fn run_until_think(sim: &mut Sim, seat: Seat) {
    let at = thinking_tick(seat, sim.tick);
    while sim.tick < at {
        sim.step(&[]);
    }
}

fn hq(sim: &Sim, seat: Seat) -> Pos {
    sim.get(sim.players[seat as usize].hq).unwrap().pos
}

#[test]
fn a_bot_match_replays_identically() {
    let (a, stats) = play(6, FIVE_MINUTES);
    let (b, _) = play(6, FIVE_MINUTES);
    assert_eq!(a.tick, b.tick);
    assert_eq!(a.checksum(), b.checksum());
    eprintln!(
        "after {} ticks: {stats:?}, checksum {:#x}",
        a.tick,
        a.checksum()
    );
}

#[test]
fn bots_build_an_economy_and_fight() {
    let (sim, stats) = play(6, FIVE_MINUTES);
    assert!(
        stats.econ >= 6,
        "every bot should deploy economy: {stats:?}"
    );
    assert!(stats.turrets >= 3, "bots fortify: {stats:?}");
    assert!(
        stats.deaths >= 40,
        "five minutes in, there should be fighting: {stats:?}"
    );
    assert!(sim.entities.len() > 20);
}

/// A match worth watching: blood is drawn within the first two minutes.
#[test]
fn fighting_starts_early() {
    let (_, stats) = play(6, FIVE_MINUTES);
    let first = stats.first_death.expect("somebody died");
    assert!(
        first < 2 * 60 * TICK_HZ,
        "first death at {} s: {stats:?}",
        first / TICK_HZ
    );
}

#[test]
fn bots_are_named_characters() {
    let names: Vec<&str> = (0..4).map(|s| personality::for_seat(s).name).collect();
    let mut unique = names.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 4, "four different temperaments: {names:?}");
}

/// Enemies in a bot's sector bring its whole home army down on them.
#[test]
fn an_invaded_bot_defends_with_everything() {
    let mut sim = Sim::new(4);
    let home = hq(&sim, 0);
    let army: Vec<EntityId> = (0..6)
        .map(|k| {
            sim.place(
                0,
                Kind::Brawler,
                home.displaced((-60 + 10 * k) * UNIT, -30 * UNIT),
            )
        })
        .collect();
    // Raiders of seat 1 slip into seat 0's sector, well inside its border.
    let inside = home.displaced(120 * UNIT, 0);
    for k in 0..3 {
        sim.place(1, Kind::Raider, inside.displaced(k * 8 * UNIT, 0));
    }
    // Let vision catch up, then let the bot think.
    sim.step(&[]);
    let mut bot = Bot::with_personality(0, personality::TURTLE);
    run_until_think(&mut sim, 0);
    // The raiders have not waited: meet them where they are now.
    let raiders: Vec<Pos> = sim
        .entities
        .iter()
        .filter(|e| e.owner == 1 && e.kind == Kind::Raider)
        .map(|e| e.pos)
        .collect();
    let cmds = bot.think(&sim);
    let sent: Vec<EntityId> = cmds
        .iter()
        .filter_map(|c| match c {
            Command::AttackMove { units, to }
                if raiders.iter().any(|r| r.dist(*to) < 30 * UNIT) =>
            {
                Some(units.clone())
            }
            _ => None,
        })
        .flatten()
        .collect();
    for id in &army {
        assert!(sent.contains(id), "brawler {id} stayed home: {cmds:?}");
    }
}

/// Facing a neighbour of brawlers, a bot builds more of their counter,
/// skirmishers, than its temperament alone would.
#[test]
fn a_bot_counters_what_its_neighbours_field() {
    let make = |enemy: Kind| {
        let mut sim = Sim::new(3);
        let at = Pos::new(sector_center(1, 3), R_MID);
        for k in 0..12 {
            sim.place(1, enemy, at.displaced(k * 9 * UNIT, 0));
        }
        let mut bot = Bot::with_personality(0, personality::BALANCED);
        let mut built = [0usize; 3];
        for _ in 0..3 * 60 * TICK_HZ {
            let cmds = bot.think(&sim);
            for c in &cmds {
                if let Command::Produce(k) = c
                    && let Some(i) = [Kind::Brawler, Kind::Skirmisher, Kind::Raider]
                        .iter()
                        .position(|x| x == k)
                {
                    built[i] += 1;
                }
            }
            let turn: Vec<(Seat, Command)> = cmds.into_iter().map(|c| (0, c)).collect();
            sim.step(&turn);
        }
        built
    };
    let vs_brawlers = make(Kind::Brawler);
    let vs_raiders = make(Kind::Raider);
    assert!(
        vs_brawlers[1] > vs_raiders[1],
        "more skirmishers against brawlers: {vs_brawlers:?} vs {vs_raiders:?}"
    );
    assert!(
        vs_raiders[0] > vs_brawlers[0],
        "more brawlers against raiders: {vs_raiders:?} vs {vs_brawlers:?}"
    );
}

/// Turtles fortify, warlords don't bother.
#[test]
fn a_turtle_builds_more_turrets_than_a_warlord() {
    let turrets_of = |p: cake_ai::Personality| {
        let mut sim = Sim::new(2);
        let mut bot = Bot::with_personality(0, p);
        for _ in 0..FIVE_MINUTES {
            let turn: Vec<(Seat, Command)> = bot.think(&sim).into_iter().map(|c| (0, c)).collect();
            sim.step(&turn);
        }
        sim.entities
            .iter()
            .filter(|e| e.owner == 0 && e.kind.is_turret())
            .count()
    };
    let turtle = turrets_of(personality::TURTLE);
    let warlord = turrets_of(personality::WARLORD);
    assert!(turtle > warlord, "turtle {turtle} vs warlord {warlord}");
}

#[test]
#[ignore = "slow: plays bots to the end"]
fn a_bot_match_ends() {
    let (sim, stats) = play(4, 60 * 60 * TICK_HZ);
    eprintln!(
        "ended at {} s: {:?} {stats:?}",
        sim.tick / TICK_HZ,
        sim.outcome
    );
    assert!(
        sim.outcome.is_some(),
        "an hour of bots should finish: {stats:?}"
    );
}

/// A tripwire for accidental nondeterminism, or a rules change that was not
/// meant to change play: this exact match must end in this exact state on
/// every platform. When a change *should* alter play (tuning, new rules,
/// bot behaviour), update the constant from the failure message.
#[test]
fn a_bot_match_ends_in_the_known_state() {
    const GOLDEN: u64 = 0x1865_26fc_63c8_adf7;
    let (sim, _) = play(6, FIVE_MINUTES);
    assert_eq!(
        sim.checksum(),
        GOLDEN,
        "the reference match now ends at {:#x}; update GOLDEN if this change was meant to alter play",
        sim.checksum()
    );
}

/// Knock `seat` out: its HQ falls, and everything of its goes with it.
fn eliminate(sim: &mut Sim, seat: Seat) {
    let hq = sim.players[seat as usize].hq;
    if let Some(e) = sim.entities.iter_mut().find(|e| e.id == hq) {
        e.hp = 0;
    }
    sim.step(&[]);
    assert!(!sim.is_alive(seat));
}

#[test]
fn land_grows_into_a_fallen_neighbours_sector() {
    let mut sim = Sim::new(4);
    let before = Land::of(&sim, 0).expect("alive");
    let theirs = sim.get(sim.players[1].hq).expect("alive").pos;
    assert!(
        !before.holds(theirs, 0),
        "a neighbour's HQ is on its own land"
    );

    eliminate(&mut sim, 1);
    let after = Land::of(&sim, 0).expect("alive");
    assert!(
        after.holds(theirs, 0),
        "the gap is split between who is left"
    );
    assert_eq!(after.width(), before.width() * 3 / 2);
    // The far side is untouched.
    assert_eq!(after.cw.min(after.ccw), before.cw);
}

#[test]
fn a_bot_claims_a_fallen_neighbours_land() {
    let mut sim = Sim::new(4);
    let hq = sim.get(sim.players[0].hq).expect("alive").pos;
    let fallen = sim.get(sim.players[1].hq).expect("alive").pos;
    eliminate(&mut sim, 1);
    let mut bot = Bot::with_personality(0, personality::TURTLE);
    for _ in 0..6 * 60 * TICK_HZ {
        let cmds: Vec<_> = bot.think(&sim).into_iter().map(|c| (0, c)).collect();
        sim.step(&cmds);
    }
    // Economy beyond where my sector ended, on the side where they were.
    let toward = fallen.a.delta(hq.a).signum();
    let sector_edge = (1i64 << 32) / 8;
    let claimed = sim
        .entities
        .iter()
        .filter(|e| e.owner == 0 && e.kind == Kind::Econ)
        .filter(|e| e.pos.a.delta(hq.a) * toward > sector_edge)
        .count();
    assert!(claimed > 0, "no economy on the fallen player's land");
}
