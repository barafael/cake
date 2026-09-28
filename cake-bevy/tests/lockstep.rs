//! Lockstep between a host and guests, with the network replaced by direct
//! hand-offs: the host's turns go into each guest's buffer, and the guests'
//! commands go into the host's sequencer, all through the same [`Match`] API
//! the systems use.

use cake_bevy::{Match, TICK_SECS};
use cake_core::Command;
use cake_core::stats::Kind;
use cake_net::Member;

fn member(peer: &str) -> Member {
    Member {
        peer: Some(peer.into()),
        name: peer.into(),
        watching: false,
    }
}

fn bot(n: u32) -> Member {
    Member {
        peer: None,
        name: format!("bot {n}"),
        watching: false,
    }
}

#[test]
fn guests_that_apply_the_hosts_turns_stay_identical() {
    let players = vec![member("a"), bot(1), member("b"), member("c"), bot(2)];
    let mut host = Match::new(players.clone(), "a", true, None);
    let mut guests = [
        Match::new(players.clone(), "b", false, None),
        Match::new(players.clone(), "c", false, None),
    ];
    assert_eq!(host.me, Some(0));
    assert_eq!(guests[0].me, Some(2));
    assert_eq!(guests[1].me, Some(3));

    let mut compared = 0;
    // The host's checksums by tick: guests report the same ticks later.
    let mut host_hashes: Vec<(u32, u64)> = Vec::new();
    for step in 0..2000u32 {
        // Each guest asks for something now and then; it reaches the host
        // like a NetMsg::Cmd would, tagged with the guest's roster seat.
        for (g, peer) in guests.iter().zip(["b", "c"]) {
            if step % 97 == 0 {
                let seat = host.seat_of(peer).expect("seated");
                assert_eq!(Some(seat), g.me);
                host.host
                    .as_mut()
                    .unwrap()
                    .seq
                    .submit(seat, Command::Produce(Kind::Raider));
            }
        }
        let (tick, cmds) = host.cut_turn().expect("host cuts turns");
        host_hashes.extend(host.advance(TICK_SECS));
        for g in &mut guests {
            g.turns.push(tick, cmds.clone());
            // Guests run on their own clocks: sometimes a little behind,
            // sometimes catching up.
            let dt = if step % 5 == 0 { 0.0 } else { TICK_SECS * 1.2 };
            let reports = g.advance(dt);
            for (t, h) in reports {
                let mine = host_hashes
                    .iter()
                    .find(|(ht, _)| *ht == t)
                    .map(|(_, hh)| *hh);
                if let Some(mine) = mine {
                    assert_eq!(mine, h, "guest diverged at tick {t}");
                    compared += 1;
                }
            }
        }
    }
    // Drain the guests to the host's tick and compare the whole state.
    for g in &mut guests {
        while g.turns.ready() > 0 {
            g.advance(TICK_SECS);
        }
        assert_eq!(g.sim.tick, host.sim.tick);
        assert_eq!(g.sim.checksum(), host.sim.checksum());
    }
    assert!(
        compared >= 150,
        "both guests report every 20 ticks: {compared}"
    );
    assert!(
        host.sim.entities.iter().any(|e| e.kind == Kind::Raider),
        "the guests' commands took effect"
    );
}

#[test]
fn a_watcher_has_no_seat_and_the_host_runs_the_bots() {
    let players = vec![bot(1), bot(2), bot(3)];
    let mut host = Match::new(players, "watcher", true, None);
    assert_eq!(host.me, None);
    for _ in 0..(20 * 60) {
        host.cut_turn();
        host.advance(TICK_SECS);
    }
    assert!(
        host.sim
            .entities
            .iter()
            .filter(|e| e.kind.is_mobile())
            .count()
            > 3,
        "bots produce without anyone at the keyboard"
    );
}
