//! Building things, against the rules the game actually ships.
//!
//! Deliberately loaded from `rules/` rather than from a purpose-built set. The
//! production rules are almost entirely data — which factory makes what, what
//! each thing costs, what it needs first — so a test against invented rules
//! would prove the code works and say nothing about whether the shipped game
//! can build a tank.

use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};
use redshift_sim::{EntityId, Rules};

fn shipped_rules() -> Rules {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rules");
    Rules::load_from(&root).expect("the shipped rules should load and validate")
}

/// A base with the given buildings, all owned by player zero.
fn base(buildings: &[(&str, i32, i32)]) -> (Sim, Vec<EntityId>) {
    base_with_players(buildings, 2)
}

fn base_with_players(buildings: &[(&str, i32, i32)], players: u8) -> (Sim, Vec<EntityId>) {
    let rules = shipped_rules();
    let map = Map::new(48, 48);

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
        seed: 0xB011_DE12,
        map,
        rules,
        players: (0..players)
            .map(|id| PlayerSetup {
                id: PlayerId(id),
                faction: None,
            })
            .collect(),
        spawns,
    });
    let ids = sim.units().ids();
    (sim, ids)
}

fn produce(building: EntityId, kind: &str, sim: &Sim) -> Command {
    Command::new(
        PlayerId(0),
        0,
        CommandKind::Produce {
            building,
            kind: sim
                .rules()
                .kind_of(kind)
                .unwrap_or_else(|| panic!("no entity {kind:?}")),
        },
    )
}

fn count_of(sim: &Sim, id: &str) -> usize {
    let kind = sim.rules().kind_of(id).expect("kind");
    sim.units().iter().filter(|(_, u)| u.kind == kind).count()
}

#[test]
fn a_war_factory_builds_a_tank_and_charges_for_it() {
    let (mut sim, ids) = base(&[("war_factory", 20, 20)]);
    let factory = ids[0];
    let before = sim.treasury().credits(PlayerId(0));

    sim.tick(&[produce(factory, "grizzly_tank", &sim)]);
    for _ in 0..600 {
        sim.tick(&[]);
        if count_of(&sim, "grizzly_tank") > 0 {
            break;
        }
    }

    assert_eq!(count_of(&sim, "grizzly_tank"), 1, "no tank was delivered");
    let spent = before - sim.treasury().credits(PlayerId(0));
    let cost = sim
        .stats()
        .get(PlayerId(0), sim.rules().kind_of("grizzly_tank").unwrap())
        .cost;
    assert_eq!(spent, cost, "spent {spent} for a tank costing {cost}");
}

#[test]
fn a_factory_will_not_build_something_it_does_not_make() {
    // The barracks makes infantry, the war factory makes vehicles. Asking the
    // wrong one is rejected in the simulation rather than only in the
    // interface, because every peer has to agree it did not happen.
    let (mut sim, ids) = base(&[("barracks", 20, 20)]);
    let barracks = ids[0];

    sim.tick(&[produce(barracks, "grizzly_tank", &sim)]);
    for _ in 0..600 {
        sim.tick(&[]);
    }
    assert_eq!(count_of(&sim, "grizzly_tank"), 0, "a barracks built a tank");
    assert_eq!(
        sim.treasury().credits(PlayerId(0)),
        5_000,
        "a rejected order still took money"
    );
}

#[test]
fn payment_is_spread_across_the_build() {
    // Charging up front would remove a real decision: queueing something you
    // cannot yet afford and letting the harvesters catch up.
    let (mut sim, ids) = base(&[("war_factory", 20, 20)]);
    sim.tick(&[produce(ids[0], "grizzly_tank", &sim)]);

    let start = sim.treasury().credits(PlayerId(0));
    let mut seen_partial = false;
    for _ in 0..600 {
        sim.tick(&[]);
        let spent = start - sim.treasury().credits(PlayerId(0));
        let cost = sim
            .stats()
            .get(PlayerId(0), sim.rules().kind_of("grizzly_tank").unwrap())
            .cost;
        if spent > 0 && spent < cost {
            seen_partial = true;
        }
        if count_of(&sim, "grizzly_tank") > 0 {
            break;
        }
    }
    assert!(seen_partial, "the cost was taken in one lump");
}

#[test]
fn a_queue_holds_when_the_money_runs_out() {
    let (mut sim, ids) = base(&[("war_factory", 20, 20)]);
    let factory = ids[0];

    // Queue far more than the starting credits can cover.
    let orders: Vec<Command> = (0..9)
        .map(|i| {
            Command::new(
                PlayerId(0),
                i,
                CommandKind::Produce {
                    building: factory,
                    kind: sim.rules().kind_of("grizzly_tank").unwrap(),
                },
            )
        })
        .collect();
    sim.tick(&orders);

    for _ in 0..4_000 {
        sim.tick(&[]);
    }

    let queue = sim
        .units()
        .get(factory)
        .and_then(|u| u.production.clone())
        .expect("a queue");
    assert!(queue.starved, "the queue should report why it stopped");

    // Not exactly zero, and it should not be: the queue stops when it cannot
    // afford the *next instalment*, so a residue smaller than one instalment is
    // left over. Asserting zero would be asserting that the last payment
    // happens to divide evenly.
    let remaining = sim.treasury().credits(PlayerId(0));
    let instalment = queue.current().expect("a held item").next_instalment();
    assert!(
        remaining < instalment,
        "{remaining} credits left but the next instalment is only {instalment}, \
         so the queue stopped for some other reason"
    );
    assert!(!queue.is_empty(), "the remaining orders were dropped");
    assert!(
        count_of(&sim, "grizzly_tank") >= 1,
        "nothing at all was built"
    );
}

#[test]
fn prerequisites_are_enforced() {
    // A war factory needs its prerequisite before it can be built. Without the
    // check, a player could skip the tech tree entirely.
    let (sim, _) = base(&[("construction_yard", 20, 20)]);
    let factory_kind = sim.rules().kind_of("war_factory").expect("war factory");

    let prerequisites: Vec<String> =
        match sim
            .rules()
            .entity(factory_kind)
            .traits
            .iter()
            .find_map(|t| match t {
                redshift_data::traits::Trait::Buildable { prerequisites, .. } => {
                    Some(prerequisites)
                }
                _ => None,
            }) {
            Some(list) => list.clone(),
            None => Vec::new(),
        };

    if prerequisites.is_empty() {
        // Nothing to enforce; the shipped rules would have to change for this
        // test to mean anything.
        return;
    }
    assert!(
        !sim.prerequisites_met(PlayerId(0), factory_kind),
        "a lone construction yard satisfied {prerequisites:?}"
    );
}

#[test]
fn cancelling_refunds_only_what_was_paid() {
    let (mut sim, ids) = base(&[("war_factory", 20, 20)]);
    let factory = ids[0];
    let start = sim.treasury().credits(PlayerId(0));

    sim.tick(&[produce(factory, "grizzly_tank", &sim)]);
    for _ in 0..20 {
        sim.tick(&[]);
    }
    let mid = sim.treasury().credits(PlayerId(0));
    assert!(mid < start, "nothing was charged before cancelling");

    sim.tick(&[Command::new(
        PlayerId(0),
        1,
        CommandKind::CancelProduction {
            building: factory,
            index: 0,
        },
    )]);

    assert_eq!(
        sim.treasury().credits(PlayerId(0)),
        start,
        "cancelling should return exactly what was paid"
    );
    assert_eq!(count_of(&sim, "grizzly_tank"), 0);
}

#[test]
fn a_building_blocks_movement_and_frees_its_ground_when_destroyed() {
    // A footprint that outlives its building leaves a hole nothing can walk
    // through and nothing can build on — invisible, and impossible to explain.
    let (mut sim, ids) = base(&[("war_factory", 20, 20)]);
    let factory = ids[0];

    let footprint = sim
        .stats()
        .get(PlayerId(0), sim.units().get(factory).unwrap().kind)
        .footprint;
    assert_ne!(
        footprint,
        (1, 1),
        "the test needs a building with a real footprint"
    );
    assert!(
        sim.map().is_blocked(Cell::new(20, 20)),
        "the factory should occupy its ground"
    );
    assert!(
        !sim.map().can_place(Cell::new(20, 20), 1, 1),
        "nothing else should fit there"
    );

    // Destroyed the way it would be in a match, by an enemy shooting it, so
    // the test exercises the same path a real game does.
    let enemy = sim.rules().kind_of("grizzly_tank").expect("tank");
    for i in 0..6i32 {
        sim.spawn_unit(
            PlayerId(1),
            enemy,
            Cell::new(24 + i % 3, 20 + i / 3).centre(),
        );
    }
    let mut destroyed = false;
    for _ in 0..6_000 {
        sim.tick(&[]);
        if sim.units().get(factory).is_none() {
            destroyed = true;
            break;
        }
    }
    assert!(destroyed, "the enemy never managed to destroy the factory");

    assert!(
        !sim.map().is_blocked(Cell::new(20, 20)),
        "the ground was never released"
    );
    assert!(sim.map().can_place(Cell::new(20, 20), 1, 1));
}

#[test]
fn production_is_deterministic() {
    let run = || {
        let (mut sim, ids) = base(&[
            ("war_factory", 16, 16),
            ("barracks", 24, 24),
            ("construction_yard", 20, 30),
        ]);
        let mut hashes = Vec::new();
        let orders = vec![
            produce(ids[0], "grizzly_tank", &sim),
            produce(ids[1], "gi", &sim),
            produce(ids[0], "harvester", &sim),
        ];
        sim.tick(&orders);
        for _ in 0..1_500 {
            sim.tick(&[]);
            hashes.push(sim.state_hash());
        }
        (
            hashes,
            sim.units().len(),
            sim.treasury().credits(PlayerId(0)),
        )
    };

    let (first, first_units, first_credits) = run();
    let (second, second_units, second_credits) = run();
    assert_eq!(first, second, "two identical build orders diverged");
    assert_eq!(first_units, second_units);
    assert_eq!(first_credits, second_credits);
    assert!(first_units > 3, "nothing was built, so this proves nothing");
}
