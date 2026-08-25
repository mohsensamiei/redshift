//! Townspeople with nothing to do.
//!
//! Deliberately not an AI, and it should not grow into one. A civilian with
//! nothing to do picks a spot near home and walks to it; that is the entire
//! behaviour, and it exists so a town reads as alive rather than as a set of
//! props.
//!
//! The bound to a home is the part that is easy to leave out. An unbounded
//! random walk carries a townsperson across the map over a long match — slowly
//! enough that nobody would call it a bug, and far enough that the town empties.

use redshift_data::rules::{ArmourTable, EntityDef, Rules};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map, Terrain};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

const WANDER_RADIUS: i32 = 4;

fn armour() -> ArmourTable {
    ron::from_str(r#"( classes: ["none"], table: { "shot": { "none": 100 } } )"#).unwrap()
}

fn person(id: &str, wanders: bool) -> EntityDef {
    let mut traits = vec![
        Trait::Health {
            max: 100,
            armour: "none".into(),
        },
        Trait::Mobile {
            speed: Hundredths(200),
            turn_rate: 3600,
            locomotor: Locomotor::Foot,
            surfaces: None,
            size: None,
            layer: None,
        },
        Trait::Vision {
            range: Hundredths(300),
        },
    ];
    if wanders {
        traits.push(Trait::Wanders {
            radius: Hundredths(WANDER_RADIUS * 100),
            interval: Ticks(20),
        });
    }
    EntityDef {
        id: id.into(),
        name_key: format!("unit.{id}"),
        side: None,
        category: "civilian".into(),
        traits,
    }
}

fn rules() -> Rules {
    Rules::from_parts(
        vec![person("townsperson", true), person("statue", false)],
        vec![],
        armour(),
        Vec::new(),
    )
    .expect("valid rules")
}

fn scenario(map: Map, spawns: Vec<(&str, i32, i32)>) -> Sim {
    let rules = rules();
    let spawns = spawns
        .into_iter()
        .map(|(id, x, y)| Spawn {
            owner: PlayerId::NEUTRAL,
            kind: rules
                .kind_of(id)
                .unwrap_or_else(|| panic!("no entity {id:?}")),
            pos: Cell::new(x, y).centre(),
        })
        .collect();
    Sim::new(MatchSetup {
        seed: 0x_70A,
        map,
        players: vec![PlayerSetup {
            id: PlayerId(0),
            faction: None,
        }],
        spawns,
        rules,
    })
}

fn run(sim: &mut Sim, ticks: u32) {
    for _ in 0..ticks {
        sim.tick(&[]);
    }
}

// -- That they move at all --------------------------------------------------

#[test]
fn a_townsperson_does_not_stand_still_forever() {
    let mut sim = scenario(Map::new(64, 64), vec![("townsperson", 30, 30)]);
    let id = sim.units().ids()[0];
    let start = sim.unit(id).unwrap().cell();

    run(&mut sim, 400);

    assert_ne!(
        sim.unit(id).unwrap().cell(),
        start,
        "the town is a set of props"
    );
}

#[test]
fn something_without_the_trait_stays_put() {
    // Wandering is a declared behaviour, not what idle units do. A tank left
    // alone must not go for a walk.
    let mut sim = scenario(Map::new(64, 64), vec![("statue", 30, 30)]);
    let id = sim.units().ids()[0];
    let start = sim.unit(id).unwrap().cell();

    run(&mut sim, 400);

    assert_eq!(sim.unit(id).unwrap().cell(), start);
}

// -- That they stay home ----------------------------------------------------

#[test]
fn a_townsperson_stays_near_where_it_started() {
    // The whole reason wandering is bounded. Over a long match an unbounded
    // walk empties the town, slowly enough that nobody calls it a bug.
    let mut sim = scenario(Map::new(64, 64), vec![("townsperson", 30, 30)]);
    let id = sim.units().ids()[0];

    let mut furthest = 0;
    for _ in 0..8_000 {
        sim.tick(&[]);
        let at = sim.unit(id).unwrap().cell();
        furthest = furthest.max(at.chebyshev_to(Cell::new(30, 30)));
    }

    assert!(
        furthest <= WANDER_RADIUS + 1,
        "strayed {furthest} cells from home, which is a walk rather than a wander"
    );
}

#[test]
fn a_crowd_does_not_move_in_lockstep() {
    // Rolled against rather than counted down. A counter shared by everyone
    // would have the whole town step off together like a chorus line.
    let mut sim = scenario(
        Map::new(64, 64),
        vec![
            ("townsperson", 30, 30),
            ("townsperson", 34, 30),
            ("townsperson", 30, 34),
            ("townsperson", 34, 34),
        ],
    );
    let ids = sim.units().ids();

    let mut moved_together = 0;
    let mut ticks_with_movement = 0;
    for _ in 0..2_000 {
        let before: Vec<Cell> = ids.iter().map(|i| sim.unit(*i).unwrap().cell()).collect();
        sim.tick(&[]);
        let after: Vec<Cell> = ids.iter().map(|i| sim.unit(*i).unwrap().cell()).collect();
        let n = before.iter().zip(&after).filter(|(a, b)| a != b).count();
        if n > 0 {
            ticks_with_movement += 1;
            if n == ids.len() {
                moved_together += 1;
            }
        }
    }

    assert!(ticks_with_movement > 0, "nobody moved at all");
    assert!(
        moved_together * 2 < ticks_with_movement,
        "{moved_together} of {ticks_with_movement} moving ticks had the whole \
         town step at once"
    );
}

#[test]
fn it_will_not_walk_into_a_lake() {
    // A civilian repeatedly ordered into water would stand still and look
    // broken, which is worse than not wandering at all.
    let mut map = Map::new(64, 64);
    map.fill_rect(Cell::new(31, 20), Cell::new(40, 40), Terrain::Water);
    let mut sim = scenario(map, vec![("townsperson", 30, 30)]);
    let id = sim.units().ids()[0];

    for _ in 0..4_000 {
        sim.tick(&[]);
        let at = sim.unit(id).unwrap().cell();
        assert_ne!(
            sim.map().terrain(at),
            Terrain::Water,
            "a townsperson went for a swim at {at:?}"
        );
    }
}

// -- That it stays out of the way -------------------------------------------

#[test]
fn an_order_overrides_the_wandering() {
    // Only *idle* units wander. A civilian a player has told to go somewhere
    // must go there, not drift off along the way.
    let mut sim = scenario(Map::new(64, 64), vec![("townsperson", 30, 30)]);
    let id = sim.units().ids()[0];
    // Owned by nobody, so ordering it takes the neutral player's name — which
    // is exactly the point being made: wandering is not a command, and a real
    // command replaces it.
    sim.tick(&[Command::new(
        PlayerId::NEUTRAL,
        0,
        CommandKind::Move {
            units: vec![id],
            target: Cell::new(50, 30),
        },
    )]);
    for _ in 0..3_000 {
        sim.tick(&[]);
        if sim.unit(id).unwrap().cell().x >= 49 {
            break;
        }
    }

    let at = sim.unit(id).unwrap().cell();
    assert!(
        at.x >= 49,
        "it wandered off instead of going where it was sent, ending at {at:?}"
    );
}

#[test]
fn wandering_is_deterministic() {
    let go = || {
        let mut sim = scenario(
            Map::new(64, 64),
            vec![
                ("townsperson", 30, 30),
                ("townsperson", 34, 30),
                ("townsperson", 30, 34),
            ],
        );
        run(&mut sim, 1_000);
        sim.state_hash()
    };
    assert_eq!(go(), go());
}
