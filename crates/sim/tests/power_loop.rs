//! The power grid, against the rules the game ships.
//!
//! Power is only interesting through its consequences: a base short of it
//! should build slowly and stop defending itself, without stopping outright.
//! These tests check the consequences rather than the totals, because a grid
//! that adds up correctly and changes nothing would pass a unit test and be
//! useless in a match.

use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};
use redshift_sim::{EntityId, Rules};

fn shipped_rules() -> Rules {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rules");
    Rules::load_from(&root).expect("the shipped rules should load")
}

fn base(buildings: &[(&str, i32, i32)]) -> (Sim, Vec<EntityId>) {
    let rules = shipped_rules();
    let spawns: Vec<Spawn> = buildings
        .iter()
        .map(|(id, x, y)| Spawn {
            owner: PlayerId(0),
            kind: rules
                .kind_of(id)
                .unwrap_or_else(|| panic!("no entity {id:?}")),
            pos: Cell::new(*x, *y).centre(),
        })
        .collect();

    let sim = Sim::new(MatchSetup {
        seed: 0x9014_5E12,
        map: Map::new(48, 48),
        rules,
        players: vec![
            PlayerSetup {
                id: PlayerId(0),
                faction: None,
            },
            PlayerSetup {
                id: PlayerId(1),
                faction: None,
            },
        ],
        spawns,
    });
    let ids = sim.units().ids();
    (sim, ids)
}

fn produce(sim: &Sim, building: EntityId, kind: &str) -> Command {
    Command::new(
        PlayerId(0),
        0,
        CommandKind::Produce {
            building,
            kind: sim.rules().kind_of(kind).expect("kind"),
        },
    )
}

/// Ticks until a war factory finishes one tank, or gives up.
fn ticks_to_build(sim: &mut Sim, factory: EntityId, limit: u32) -> Option<u32> {
    let tank = sim.rules().kind_of("grizzly_tank").expect("tank");
    let order = produce(sim, factory, "grizzly_tank");
    sim.tick(&[order]);
    for elapsed in 1..limit {
        sim.tick(&[]);
        if sim.units().iter().any(|(_, u)| u.kind == tank) {
            return Some(elapsed);
        }
    }
    None
}

#[test]
fn a_base_with_a_plant_is_powered_and_one_without_is_not() {
    let (powered, _) = base(&[("power_plant", 10, 10), ("war_factory", 20, 20)]);
    assert!(powered.power().is_satisfied(PlayerId(0)));
    assert!(powered.power().supply(PlayerId(0)) > 0);
    assert!(powered.power().draw(PlayerId(0)) > 0);

    let (dark, _) = base(&[("war_factory", 20, 20)]);
    assert!(!dark.power().is_satisfied(PlayerId(0)));
    assert_eq!(dark.power().supply(PlayerId(0)), 0);
    assert!(dark.power().shortfall(PlayerId(0)) > 0);
}

#[test]
fn a_base_short_of_power_builds_slower_but_still_builds() {
    // The distinction that matters. A base that froze outright would end the
    // match on the spot; one that slows gives the player a chance to notice.
    let (mut powered, ids) = base(&[("power_plant", 10, 10), ("war_factory", 20, 20)]);
    let fast = ticks_to_build(&mut powered, ids[1], 4_000).expect("a powered base should build");

    let (mut dark, ids) = base(&[("war_factory", 20, 20)]);
    let slow = ticks_to_build(&mut dark, ids[0], 8_000);

    let slow = slow.expect("an unpowered base should still build, only slowly");
    assert!(
        slow > fast * 2,
        "unpowered took {slow} ticks against {fast} powered, which is barely a penalty"
    );
}

#[test]
fn losing_the_plant_takes_the_base_offline_mid_match() {
    // The grid is rebuilt every tick precisely so this works without anyone
    // remembering to update a running total.
    let (mut sim, ids) = base(&[("power_plant", 10, 10), ("war_factory", 20, 20)]);
    let plant = ids[0];
    sim.tick(&[]);
    assert!(sim.power().is_satisfied(PlayerId(0)));

    // Destroy the plant the way a match would.
    let tank = sim.rules().kind_of("grizzly_tank").expect("tank");
    for i in 0..8i32 {
        sim.spawn_unit(
            PlayerId(1),
            tank,
            Cell::new(14 + i % 4, 10 + i / 4).centre(),
        );
    }
    let mut destroyed = false;
    for _ in 0..8_000 {
        sim.tick(&[]);
        if sim.units().get(plant).is_none() {
            destroyed = true;
            break;
        }
    }
    assert!(
        destroyed,
        "the plant was never destroyed, so this proves nothing"
    );
    assert!(
        !sim.power().is_satisfied(PlayerId(0)),
        "the base still reported power on the tick its only plant was destroyed — \
         the grid is being computed before deaths are resolved"
    );
}

#[test]
fn power_is_not_shared_between_players() {
    let rules = shipped_rules();
    let sim = Sim::new(MatchSetup {
        seed: 1,
        map: Map::new(48, 48),
        players: vec![
            PlayerSetup {
                id: PlayerId(0),
                faction: None,
            },
            PlayerSetup {
                id: PlayerId(1),
                faction: None,
            },
        ],
        spawns: vec![
            Spawn {
                owner: PlayerId(0),
                kind: rules.kind_of("power_plant").unwrap(),
                pos: Cell::new(10, 10).centre(),
            },
            Spawn {
                owner: PlayerId(1),
                kind: rules.kind_of("war_factory").unwrap(),
                pos: Cell::new(30, 30).centre(),
            },
        ],
        rules,
    });

    assert!(sim.power().is_satisfied(PlayerId(0)));
    assert!(
        !sim.power().is_satisfied(PlayerId(1)),
        "one player's plant powered another's factory"
    );
}

#[test]
fn a_building_that_draws_no_power_is_unaffected_by_a_shortage() {
    // What makes a power plant worth attacking rather than merely worth owning:
    // some things carry on regardless, so cutting power is a tactic and not an
    // instant win.
    let (sim, ids) = base(&[("refinery", 20, 20)]);
    let refinery = sim.units().get(ids[0]).expect("refinery");
    let draw = sim.stats().get(PlayerId(0), refinery.kind).power_draw;

    if draw == 0 {
        assert!(
            !sim.is_unpowered(refinery),
            "a building drawing nothing is never unpowered"
        );
    } else {
        // The shipped rules give it a draw; then it should be affected, and
        // this test is documenting the opposite case for whenever they change.
        assert!(sim.is_unpowered(refinery));
    }
}

#[test]
fn power_state_is_deterministic() {
    let run = || {
        let (mut sim, ids) = base(&[
            ("power_plant", 10, 10),
            ("war_factory", 20, 20),
            ("barracks", 28, 28),
        ]);
        let order = produce(&sim, ids[1], "grizzly_tank");
        sim.tick(&[order]);
        let mut hashes = Vec::new();
        for _ in 0..800 {
            sim.tick(&[]);
            hashes.push(sim.state_hash());
        }
        (
            hashes,
            sim.power().supply(PlayerId(0)),
            sim.power().draw(PlayerId(0)),
        )
    };
    let (first, fs, fd) = run();
    let (second, ss, sd) = run();
    assert_eq!(first, second, "two identical bases diverged");
    assert_eq!((fs, fd), (ss, sd));
    assert!(fd > 0, "nothing drew power, so this proves nothing");
}
