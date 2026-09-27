//! The rules, exercised through commands the way a match drives them.

use cake_core::geom::{R_MID, UNIT, sector_center};
use cake_core::stats::{self, TICK_HZ};
use cake_core::{Angle, Command, EntityId, Event, Kind, Order, Outcome, Pos, Seat, Sim};

fn run(sim: &mut Sim, ticks: u32) {
    for _ in 0..ticks {
        sim.step(&[]);
    }
}

fn units_of(sim: &Sim, seat: Seat, kind: Kind) -> Vec<EntityId> {
    sim.entities
        .iter()
        .filter(|e| e.owner == seat && e.kind == kind)
        .map(|e| e.id)
        .collect()
}

fn first_utility(sim: &Sim, seat: Seat) -> EntityId {
    units_of(sim, seat, Kind::Utility)[0]
}

/// A point `units` map units counter-clockwise of seat `seat`'s HQ, mid-band.
fn beside_hq(sim: &Sim, seat: Seat, units: i64) -> Pos {
    let hq = sim.get(sim.players[seat as usize].hq).unwrap().pos;
    Pos::new(hq.a, R_MID).displaced(units * UNIT, 0)
}

#[test]
fn a_match_starts_with_an_hq_and_a_utility_per_seat_at_its_sector() {
    let sim = Sim::new(6);
    for seat in 0..6u8 {
        let hq = sim.get(sim.players[seat as usize].hq).unwrap();
        assert_eq!(hq.kind, Kind::Hq);
        assert_eq!(hq.owner, seat);
        assert_eq!(hq.pos.a, sector_center(seat as usize, 6));
        assert_eq!(units_of(&sim, seat, Kind::Utility).len(), 1);
        assert_eq!(sim.players[seat as usize].supply, stats::STARTING_SUPPLY);
    }
}

#[test]
fn the_hq_pays_its_income_every_tick() {
    let mut sim = Sim::new(2);
    run(&mut sim, TICK_HZ);
    assert_eq!(
        sim.players[0].supply,
        stats::STARTING_SUPPLY + 5 * stats::SUPPLY,
        "5 supply per second"
    );
}

#[test]
fn production_charges_up_front_and_delivers_after_its_build_time() {
    let mut sim = Sim::new(2);
    sim.step(&[(0, Command::Produce(Kind::Raider))]);
    let paid = stats::STARTING_SUPPLY + stats::HQ_INCOME - stats::RAIDER.cost;
    assert_eq!(sim.players[0].supply, paid);
    assert!(units_of(&sim, 0, Kind::Raider).is_empty());

    run(&mut sim, stats::RAIDER.build_ticks);
    assert_eq!(units_of(&sim, 0, Kind::Raider).len(), 1);
    assert!(sim.players[0].queue.is_empty());
}

#[test]
fn production_refuses_what_it_cannot_afford_and_cancelling_refunds() {
    let mut sim = Sim::new(2);
    // 100 supply buys one brawler (60) but not a second.
    sim.step(&[
        (0, Command::Produce(Kind::Brawler)),
        (0, Command::Produce(Kind::Brawler)),
    ]);
    assert_eq!(sim.players[0].queue.len(), 1);
    let before = sim.players[0].supply;
    sim.step(&[(0, Command::CancelProduce)]);
    assert!(sim.players[0].queue.is_empty());
    assert_eq!(
        sim.players[0].supply,
        before + stats::BRAWLER.cost + stats::HQ_INCOME
    );
}

#[test]
fn a_seat_cannot_command_another_seats_units() {
    let mut sim = Sim::new(2);
    let theirs = first_utility(&sim, 1);
    let before = sim.get(theirs).unwrap().pos;
    let to = beside_hq(&sim, 1, 80);
    sim.step(&[(
        0,
        Command::Move {
            units: vec![theirs],
            to,
        },
    )]);
    run(&mut sim, 40);
    let after = sim.get(theirs).unwrap();
    assert_eq!(after.order, Order::Idle);
    assert!(after.pos.dist(before) < 2 * UNIT);
}

#[test]
fn units_walk_to_where_they_are_sent() {
    let mut sim = Sim::new(2);
    let u = first_utility(&sim, 0);
    let to = beside_hq(&sim, 0, 120);
    sim.step(&[(0, Command::Move { units: vec![u], to })]);
    // 30 units/s covers ~130 units in under five seconds.
    run(&mut sim, 5 * TICK_HZ);
    let e = sim.get(u).unwrap();
    assert!(e.pos.dist(to) < UNIT, "{:?} vs {:?}", e.pos, to);
    assert_eq!(e.order, Order::Idle);
}

#[test]
fn economy_buildings_respect_the_spacing_rule() {
    let sim = Sim::new(6);
    assert!(sim.econ_site(0, beside_hq(&sim, 0, 100)).is_err(), "too close to the HQ");
    assert!(sim.econ_site(0, beside_hq(&sim, 0, 155)).is_ok());
}

#[test]
fn a_utility_deploys_into_an_economy_building_that_raises_income() {
    let mut sim = Sim::new(6);
    let u = first_utility(&sim, 0);
    let site = beside_hq(&sim, 0, 155);
    sim.step(&[(0, Command::Deploy { unit: u, at: site })]);
    // About 160 units at 30/s, then five seconds deploying.
    run(&mut sim, 12 * TICK_HZ);
    assert!(sim.get(u).is_none(), "the utility is consumed");
    let econ = units_of(&sim, 0, Kind::Econ);
    assert_eq!(econ.len(), 1);
    assert_eq!(sim.income(0), stats::HQ_INCOME + stats::ECON_INCOME);

    // A second economy building next to the first is refused.
    assert!(sim.econ_site(0, beside_hq(&sim, 0, 200)).is_err());
}

#[test]
fn deploying_too_close_to_another_economy_building_fails() {
    let mut sim = Sim::new(6);
    let site = beside_hq(&sim, 0, 160);
    sim.place(1, Kind::Econ, beside_hq(&sim, 0, 260));
    let u = first_utility(&sim, 0);
    sim.step(&[(0, Command::Deploy { unit: u, at: site })]);
    run(&mut sim, 10 * TICK_HZ);
    assert!(sim.get(u).is_some(), "the utility must not be consumed");
    assert_eq!(units_of(&sim, 0, Kind::Econ).len(), 0);
}

#[test]
fn a_utility_builds_a_turret_that_only_shoots_once_complete() {
    let mut sim = Sim::new(6);
    let u = first_utility(&sim, 0);
    let site = beside_hq(&sim, 0, 80);
    sim.step(&[(0, Command::Build { unit: u, at: site })]);
    run(&mut sim, 4 * TICK_HZ);
    let turret = units_of(&sim, 0, Kind::Turret);
    assert_eq!(turret.len(), 1, "the frame is laid once the utility arrives");
    let t = sim.get(turret[0]).unwrap();
    assert!(!t.complete);
    assert!(!t.armed());

    run(&mut sim, stats::TURRET.build_ticks + TICK_HZ);
    let t = sim.get(turret[0]).unwrap();
    assert!(t.complete);
    assert_eq!(t.hp, t.max_hp());
    assert_eq!(sim.get(u).unwrap().order, Order::Idle, "the builder is released");
}

#[test]
fn repair_restores_hp_to_the_maximum() {
    let mut sim = Sim::new(2);
    let hq = sim.players[0].hq;
    let u = first_utility(&sim, 0);
    let i = sim.entities.iter().position(|e| e.id == hq).unwrap();
    sim.entities[i].hp -= 100;
    sim.step(&[(
        0,
        Command::Repair {
            units: vec![u],
            target: hq,
        },
    )]);
    run(&mut sim, 10 * TICK_HZ);
    let e = sim.get(hq).unwrap();
    assert_eq!(e.hp, e.max_hp());
}

/// Two armies meet at a quarter turn, far from both HQs. Returns the
/// survivors on each side.
fn fight(a: Kind, b: Kind, budget: i64) -> (usize, usize) {
    let mut sim = Sim::new(2);
    let mid = Pos::new(Angle::from_turns(1, 4), R_MID);
    let mut place = |seat: Seat, kind: Kind, side: i64| {
        let n = budget / kind.stats().cost;
        (0..n)
            .map(|k| {
                let row = k / 5;
                let col = k % 5;
                let at = mid.displaced(side * (80 + 16 * row) * UNIT, (col - 2) * 16 * UNIT);
                sim.place(seat, kind, at)
            })
            .collect::<Vec<_>>()
    };
    let ours = place(0, a, -1);
    let theirs = place(1, b, 1);
    let west = mid.displaced(-100 * UNIT, 0);
    let east = mid.displaced(100 * UNIT, 0);
    sim.step(&[
        (0, Command::AttackMove { units: ours.clone(), to: east }),
        (1, Command::AttackMove { units: theirs.clone(), to: west }),
    ]);
    for _ in 0..90 * TICK_HZ {
        sim.step(&[]);
        let left = |ids: &[EntityId]| ids.iter().filter(|id| sim.get(**id).is_some()).count();
        if left(&ours) == 0 || left(&theirs) == 0 {
            break;
        }
    }
    let left = |ids: &[EntityId]| ids.iter().filter(|id| sim.get(**id).is_some()).count();
    (left(&ours), left(&theirs))
}

const BUDGET: i64 = 360 * stats::SUPPLY;

#[test]
fn skirmishers_beat_brawlers_at_equal_cost() {
    let (s, b) = fight(Kind::Skirmisher, Kind::Brawler, BUDGET);
    assert!(s > 0 && b == 0, "skirmishers {s} vs brawlers {b}");
}

#[test]
fn brawlers_beat_raiders_at_equal_cost() {
    let (b, r) = fight(Kind::Brawler, Kind::Raider, BUDGET);
    assert!(b > 0 && r == 0, "brawlers {b} vs raiders {r}");
}

#[test]
fn raiders_beat_skirmishers_at_equal_cost() {
    let (r, s) = fight(Kind::Raider, Kind::Skirmisher, BUDGET);
    assert!(r > 0 && s == 0, "raiders {r} vs skirmishers {s}");
}

#[test]
fn losing_the_hq_eliminates_a_seat_and_the_last_one_standing_wins() {
    let mut sim = Sim::new(2);
    let hq1 = sim.get(sim.players[1].hq).unwrap().pos;
    let raiders: Vec<EntityId> = (0..12)
        .map(|k| sim.place(0, Kind::Raider, hq1.displaced((30 + 3 * k) * UNIT, 0)))
        .collect();
    let target = sim.players[1].hq;
    sim.step(&[(
        0,
        Command::Attack {
            units: raiders,
            target,
        },
    )]);
    let mut eliminated = false;
    for _ in 0..120 * TICK_HZ {
        sim.step(&[]);
        eliminated |= sim.events.contains(&Event::Eliminated(1));
        if sim.outcome.is_some() {
            break;
        }
    }
    assert!(eliminated);
    assert_eq!(sim.outcome, Some(Outcome::Winner(0)));
    assert!(sim.entities.iter().all(|e| e.owner == 0), "the loser's pieces are removed");
}

#[test]
fn identical_inputs_give_identical_checksums() {
    let script = |sim: &mut Sim| {
        for t in 0..600u32 {
            let mut cmds = Vec::new();
            if t % 50 == 0 {
                for seat in 0..sim.seats() as Seat {
                    cmds.push((seat, Command::Produce(Kind::UNITS[(t / 50) as usize % 4])));
                }
            }
            if t % 90 == 0 && t > 0 {
                for seat in 0..sim.seats() as Seat {
                    let units: Vec<EntityId> = sim
                        .entities
                        .iter()
                        .filter(|e| e.owner == seat && e.kind.is_mobile())
                        .map(|e| e.id)
                        .collect();
                    let to = sim.get(sim.players[((seat as usize) + 1) % sim.seats()].hq).unwrap().pos;
                    cmds.push((seat, Command::AttackMove { units, to }));
                }
            }
            sim.step(&cmds);
        }
    };
    let mut a = Sim::new(4);
    let mut b = Sim::new(4);
    script(&mut a);
    script(&mut b);
    assert_eq!(a.checksum(), b.checksum());
    b.step(&[]);
    assert_ne!(a.checksum(), b.checksum(), "the tick is part of the state");
}

#[test]
fn a_utility_deploys_even_when_its_site_is_occupied() {
    let mut sim = Sim::new(6);
    let site = beside_hq(&sim, 0, 160);
    // A friendly brawler parked exactly on the site.
    sim.place(0, Kind::Brawler, site);
    let u = first_utility(&sim, 0);
    sim.step(&[(0, Command::Deploy { unit: u, at: site })]);
    run(&mut sim, 15 * TICK_HZ);
    assert!(sim.get(u).is_none(), "the utility should settle beside the brawler");
    assert_eq!(units_of(&sim, 0, Kind::Econ).len(), 1);
}
