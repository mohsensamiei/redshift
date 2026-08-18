//! Siting buildings.
//!
//! Placement is a two-step act in this genre — build it, then choose where it
//! goes — and that second step is most of what base layout *is* as a decision.
//! These tests care about the rules around that choice: that a finished
//! structure waits rather than appearing, that the ground has to be clear, and
//! that a player cannot build wherever they like.

use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map, Terrain};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};
use redshift_sim::{EntityId, Rules};

fn shipped_rules() -> Rules {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rules");
    Rules::load_from(&root).expect("the shipped rules should load")
}

/// A construction yard at (20, 20), plus whatever else is asked for.
fn base(extra: &[(&str, i32, i32)]) -> (Sim, EntityId) {
    let rules = shipped_rules();
    let mut spawns = vec![Spawn {
        owner: PlayerId(0),
        kind: rules
            .kind_of("construction_yard")
            .expect("construction yard"),
        pos: Cell::new(20, 20).centre(),
    }];
    for (id, x, y) in extra {
        spawns.push(Spawn {
            owner: PlayerId(0),
            kind: rules
                .kind_of(id)
                .unwrap_or_else(|| panic!("no entity {id:?}")),
            pos: Cell::new(*x, *y).centre(),
        });
    }

    let mut map = Map::new(64, 64);
    // A lake well away from the yard, for the terrain tests.
    map.fill_rect(Cell::new(40, 40), Cell::new(48, 48), Terrain::Water);

    let sim = Sim::new(MatchSetup {
        seed: 0x51_7E,
        map,
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
    let yard = sim.units().ids()[0];
    (sim, yard)
}

/// Queues a structure and runs until it is ready to place.
fn build_until_ready(sim: &mut Sim, yard: EntityId, kind: &str) {
    let order = Command::new(
        PlayerId(0),
        0,
        CommandKind::Produce {
            building: yard,
            kind: sim.rules().kind_of(kind).expect("kind"),
        },
    );
    sim.tick(&[order]);
    for _ in 0..8_000 {
        sim.tick(&[]);
        if sim.ready_to_place(PlayerId(0)).is_some() {
            return;
        }
    }
    panic!("{kind} never finished building");
}

fn place(yard: EntityId, at: Cell) -> Command {
    Command::new(
        PlayerId(0),
        1,
        CommandKind::PlaceBuilding { producer: yard, at },
    )
}

fn count_of(sim: &Sim, id: &str) -> usize {
    let kind = sim.rules().kind_of(id).expect("kind");
    sim.units().iter().filter(|(_, u)| u.kind == kind).count()
}

#[test]
fn a_finished_structure_waits_to_be_placed_rather_than_appearing() {
    // Spawning it beside the construction yard would take base layout away as
    // a decision entirely.
    let (mut sim, yard) = base(&[]);
    build_until_ready(&mut sim, yard, "power_plant");

    assert_eq!(count_of(&sim, "power_plant"), 0, "it placed itself");
    let (producer, kind) = sim.ready_to_place(PlayerId(0)).expect("something is ready");
    assert_eq!(producer, yard);
    assert_eq!(kind, sim.rules().kind_of("power_plant").unwrap());
}

#[test]
fn placing_it_puts_a_building_on_the_map() {
    let (mut sim, yard) = base(&[]);
    build_until_ready(&mut sim, yard, "power_plant");

    sim.tick(&[place(yard, Cell::new(24, 20))]);

    assert_eq!(count_of(&sim, "power_plant"), 1, "the plant never appeared");
    assert!(
        sim.ready_to_place(PlayerId(0)).is_none(),
        "it is still pending"
    );
    assert!(
        sim.map().is_blocked(Cell::new(24, 20)),
        "it did not claim its ground"
    );
    assert!(
        sim.power().supply(PlayerId(0)) > 0,
        "a placed plant should supply power"
    );
}

#[test]
fn a_structure_cannot_be_placed_on_top_of_another() {
    let (mut sim, yard) = base(&[]);
    build_until_ready(&mut sim, yard, "power_plant");

    // Directly on the construction yard.
    sim.tick(&[place(yard, Cell::new(20, 20))]);
    assert_eq!(
        count_of(&sim, "power_plant"),
        0,
        "it was placed inside the yard"
    );
    assert!(
        sim.ready_to_place(PlayerId(0)).is_some(),
        "the order should have been refused"
    );
}

#[test]
fn a_structure_cannot_be_placed_in_water() {
    let (mut sim, yard) = base(&[("barracks", 44, 38)]);
    build_until_ready(&mut sim, yard, "power_plant");

    sim.tick(&[place(yard, Cell::new(43, 42))]);
    assert_eq!(
        count_of(&sim, "power_plant"),
        0,
        "a plant was built in a lake"
    );
}

#[test]
fn a_structure_cannot_be_placed_beyond_the_build_area() {
    // What stops a player dropping a barracks in the enemy's base, and most of
    // what makes expanding a decision rather than a formality.
    let (mut sim, yard) = base(&[]);
    build_until_ready(&mut sim, yard, "power_plant");

    sim.tick(&[place(yard, Cell::new(55, 55))]);
    assert_eq!(
        count_of(&sim, "power_plant"),
        0,
        "a plant was built across the map"
    );
    assert!(sim.ready_to_place(PlayerId(0)).is_some());

    // Close by is fine.
    sim.tick(&[place(yard, Cell::new(24, 24))]);
    assert_eq!(count_of(&sim, "power_plant"), 1);
}

#[test]
fn the_build_area_grows_with_the_base() {
    // Expanding outward step by step is the intended shape: each new building
    // extends the area the next one can go in.
    let (mut sim, yard) = base(&[]);

    let far = Cell::new(34, 20);
    assert!(
        !sim.can_build_at(PlayerId(0), far, (3, 3)),
        "the test needs somewhere initially out of reach"
    );

    // Build a stepping stone.
    build_until_ready(&mut sim, yard, "power_plant");
    sim.tick(&[place(yard, Cell::new(27, 20))]);
    assert_eq!(count_of(&sim, "power_plant"), 1);

    assert!(
        sim.can_build_at(PlayerId(0), far, (3, 3)),
        "the new building should have extended the build area"
    );
}

#[test]
fn a_mobile_unit_does_not_anchor_a_build_area() {
    // A tank parked in the enemy base must not become a foothold.
    let (mut sim, yard) = base(&[]);
    let tank = sim.rules().kind_of("grizzly_tank").expect("tank");
    sim.spawn_unit(PlayerId(0), tank, Cell::new(55, 55).centre());
    sim.tick(&[]);

    assert!(
        !sim.can_build_at(PlayerId(0), Cell::new(55, 57), (3, 3)),
        "a tank extended the build area"
    );
    let _ = yard;
}

#[test]
fn another_player_cannot_place_your_building() {
    let (mut sim, yard) = base(&[]);
    build_until_ready(&mut sim, yard, "power_plant");

    sim.tick(&[Command::new(
        PlayerId(1),
        0,
        CommandKind::PlaceBuilding {
            producer: yard,
            at: Cell::new(24, 20),
        },
    )]);
    assert_eq!(count_of(&sim, "power_plant"), 0, "another player placed it");
    assert!(sim.ready_to_place(PlayerId(0)).is_some());
}

#[test]
fn a_pending_structure_holds_the_queue() {
    // The original built one structure at a time. Letting the next start would
    // leave the player with several finished buildings and no way to tell them
    // apart.
    let (mut sim, yard) = base(&[]);
    let plant = sim.rules().kind_of("power_plant").expect("plant");
    let orders = vec![
        Command::new(
            PlayerId(0),
            0,
            CommandKind::Produce {
                building: yard,
                kind: plant,
            },
        ),
        Command::new(
            PlayerId(0),
            1,
            CommandKind::Produce {
                building: yard,
                kind: plant,
            },
        ),
    ];
    sim.tick(&orders);

    for _ in 0..8_000 {
        sim.tick(&[]);
        if sim.ready_to_place(PlayerId(0)).is_some() {
            break;
        }
    }
    let credits_when_ready = sim.treasury().credits(PlayerId(0));

    // Nothing should progress while the first waits for a site.
    for _ in 0..500 {
        sim.tick(&[]);
    }
    assert_eq!(
        sim.treasury().credits(PlayerId(0)),
        credits_when_ready,
        "the second structure kept building while the first waited to be placed"
    );
}

#[test]
fn placement_is_deterministic() {
    let run = || {
        let (mut sim, yard) = base(&[]);
        build_until_ready(&mut sim, yard, "power_plant");
        sim.tick(&[place(yard, Cell::new(24, 24))]);
        let mut hashes = Vec::new();
        for _ in 0..400 {
            sim.tick(&[]);
            hashes.push(sim.state_hash());
        }
        (hashes, sim.units().len(), sim.power().supply(PlayerId(0)))
    };
    let (a, a_units, a_power) = run();
    let (b, b_units, b_power) = run();
    assert_eq!(a, b, "two identical placements diverged");
    assert_eq!((a_units, a_power), (b_units, b_power));
    assert!(a_power > 0, "nothing was placed, so this proves nothing");
}
