//! The one structure whose entire effect is on the other side of the table.
//!
//! A Gap Generator reveals nothing for its owner. It *hides* ground from
//! everyone else — the only subtractive operation in a visibility model where
//! everything else adds and explored ground is otherwise cumulative for the
//! whole match.
//!
//! The design turns on one ordering decision: hiding runs before revealing.
//! A player with nothing inside the area is left with black ground; one who
//! walks a scout in has it push the shroud back around itself. That makes
//! scouting the answer rather than a counter-structure, and it falls out of the
//! order rather than needing a rule of its own.

use redshift_data::rules::{ArmourTable, EntityDef, Rules};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn armour() -> ArmourTable {
    ron::from_str(r#"( classes: ["none"], table: { "shot": { "none": 100 } } )"#).unwrap()
}

fn scout() -> EntityDef {
    EntityDef {
        id: "scout".into(),
        name_key: "unit.scout".into(),
        side: None,
        category: "infantry".into(),
        traits: vec![
            Trait::Health {
                max: 200,
                armour: "none".into(),
            },
            Trait::Mobile {
                speed: Hundredths(400),
                turn_rate: 3600,
                locomotor: Locomotor::Foot,
                surfaces: None,
                size: None,
                layer: None,
            },
            Trait::Vision {
                range: Hundredths(400),
            },
        ],
    }
}

/// Eyes from a long way off. The point of a generator is that this cannot see
/// through it, which is only testable with an observer outside the area.
fn watchtower() -> EntityDef {
    EntityDef {
        id: "tower".into(),
        name_key: "structure.tower".into(),
        side: None,
        category: "structure".into(),
        traits: vec![
            Trait::Health {
                max: 400,
                armour: "none".into(),
            },
            Trait::Vision {
                range: Hundredths(2_500),
            },
        ],
    }
}

/// A building worth hiding: it can be seen from a long way off, which is
/// exactly the problem the generator solves.
fn barracks() -> EntityDef {
    EntityDef {
        id: "barracks".into(),
        name_key: "structure.barracks".into(),
        side: None,
        category: "structure".into(),
        traits: vec![
            Trait::Health {
                max: 800,
                armour: "none".into(),
            },
            Trait::Vision {
                range: Hundredths(400),
            },
            Trait::Footprint {
                width: 3,
                height: 3,
            },
        ],
    }
}

fn generator(draws_power: bool) -> EntityDef {
    let mut traits = vec![
        Trait::Health {
            max: 600,
            armour: "none".into(),
        },
        Trait::Vision {
            range: Hundredths(200),
        },
        Trait::HidesGround {
            radius: Hundredths(800),
        },
    ];
    if draws_power {
        traits.push(Trait::PowerDraw {
            amount: 200,
            works_unpowered: false,
        });
    }
    // A price, so it can be sold. Selling refuses anything that cost nothing,
    // which is right — and which makes "cost" the shortest honest route to
    // removing a structure in a test.
    traits.push(Trait::Buildable {
        cost: 1_000,
        build_time: Ticks(30),
        prerequisites: vec![],
        produced_by: "barracks".into(),
    });
    EntityDef {
        id: if draws_power { "hungry_gap" } else { "gap" }.into(),
        name_key: "structure.gap".into(),
        side: None,
        category: "structure".into(),
        traits,
    }
}

fn rules() -> Rules {
    Rules::from_parts(
        vec![
            scout(),
            barracks(),
            watchtower(),
            generator(false),
            generator(true),
        ],
        vec![],
        armour(),
        Vec::new(),
    )
    .expect("valid rules")
}

fn scenario(spawns: Vec<(u8, &str, i32, i32)>) -> Sim {
    let rules = rules();
    let spawns = spawns
        .into_iter()
        .map(|(owner, id, x, y)| Spawn {
            owner: PlayerId(owner),
            kind: rules
                .kind_of(id)
                .unwrap_or_else(|| panic!("no entity {id:?}")),
            pos: Cell::new(x, y).centre(),
        })
        .collect();
    let mut sim = Sim::new(MatchSetup {
        seed: 0x_6A9,
        map: Map::new(64, 64),
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
        rules,
    });
    sim.tick(&[]);
    sim
}

// -- What it does -----------------------------------------------------------

#[test]
fn a_gap_generator_hides_a_base_from_the_enemy() {
    // Player 1 has a watchtower of a scout looking straight at player 0's base.
    // Without the generator it sees the ground; with it, nothing.
    let plain = scenario(vec![(0, "barracks", 20, 20), (1, "tower", 40, 20)]);
    assert!(
        plain
            .visibility()
            .is_visible(PlayerId(1), Cell::new(21, 20)),
        "the enemy should be able to see this ground to begin with"
    );

    let hidden = scenario(vec![
        (0, "barracks", 20, 20),
        (0, "gap", 20, 20),
        (1, "tower", 40, 20),
    ]);
    assert!(
        !hidden
            .visibility()
            .is_visible(PlayerId(1), Cell::new(21, 20)),
        "the generator should have taken that ground away"
    );
}

#[test]
fn it_reveals_nothing_for_its_owner() {
    // A structure whose entire effect is on the other side of the table. If it
    // helped its owner see, it would be a radar with a side effect.
    let sim = scenario(vec![(0, "gap", 20, 20)]);
    // Its own tiny vision reaches two cells; the concealment reaches eight.
    assert!(
        !sim.visibility().is_visible(PlayerId(0), Cell::new(27, 20)),
        "hiding ground must not double as revealing it"
    );
}

#[test]
fn the_owner_still_sees_their_own_ground() {
    let sim = scenario(vec![(0, "barracks", 20, 20), (0, "gap", 20, 20)]);
    assert!(
        sim.visibility().is_visible(PlayerId(0), Cell::new(21, 20)),
        "a player should not blind themselves"
    );
}

#[test]
fn ground_already_explored_is_taken_back() {
    // The part a purely additive model cannot express. Leaving `explored` alone
    // would hide live units and leave the buildings drawn exactly where they
    // were, which is no concealment at all.
    let mut sim = scenario(vec![(0, "barracks", 20, 20), (1, "tower", 40, 20)]);
    for _ in 0..20 {
        sim.tick(&[]);
    }
    assert!(sim.visibility().is_explored(PlayerId(1), Cell::new(21, 20)));

    // The generator goes up next to the barracks, and the scout walks away.
    let gap = sim.rules().kind_of("gap").unwrap();
    sim.spawn_unit(PlayerId(0), gap, Cell::new(20, 20).centre());
    let scout_id = sim.units().ids()[1];
    sim.tick(&[Command::new(
        PlayerId(1),
        0,
        CommandKind::Move {
            units: vec![scout_id],
            target: Cell::new(50, 50),
        },
    )]);
    for _ in 0..400 {
        sim.tick(&[]);
    }

    assert!(
        !sim.visibility().is_explored(PlayerId(1), Cell::new(21, 20)),
        "ground the enemy had explored should have gone black again"
    );
}

// -- Its answer -------------------------------------------------------------

#[test]
fn a_scout_sent_inside_sees_around_itself() {
    // The reason hiding runs before revealing. Scouting is the answer to a Gap
    // Generator, and it should not need a rule of its own.
    let sim = scenario(vec![
        (0, "barracks", 20, 20),
        (0, "gap", 20, 20),
        (1, "scout", 21, 24),
    ]);

    assert!(
        sim.visibility().is_visible(PlayerId(1), Cell::new(21, 24)),
        "a unit inside the area should see the ground it is standing on"
    );
    assert!(
        !sim.visibility().is_visible(PlayerId(1), Cell::new(21, 17)),
        "and no further than it can see"
    );
}

#[test]
fn a_destroyed_generator_gives_the_ground_back() {
    let mut sim = scenario(vec![
        (0, "barracks", 20, 20),
        (0, "gap", 20, 20),
        (1, "tower", 40, 20),
    ]);
    assert!(!sim.visibility().is_visible(PlayerId(1), Cell::new(21, 20)));

    // Sold rather than shot, which is the shortest route to "the structure is
    // gone" that goes through code the game actually uses.
    let gap_id = sim.units().ids()[1];
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Sell { building: gap_id },
    )]);
    for _ in 0..60 {
        sim.tick(&[]);
        if sim.unit(gap_id).is_none() {
            break;
        }
    }
    assert!(sim.unit(gap_id).is_none(), "the generator should be gone");

    assert!(
        sim.visibility().is_visible(PlayerId(1), Cell::new(21, 20)),
        "the concealment should have died with the structure"
    );
}

#[test]
fn a_generator_with_no_power_conceals_nothing() {
    // Like every other structure that draws from the grid. Cutting an enemy's
    // power should peel their base open.
    let sim = scenario(vec![
        (0, "barracks", 20, 20),
        (0, "hungry_gap", 20, 20),
        (1, "tower", 40, 20),
    ]);
    assert!(
        sim.visibility().is_visible(PlayerId(1), Cell::new(21, 20)),
        "an unpowered generator hid something"
    );
}

#[test]
fn concealment_is_deterministic() {
    let run = || {
        let mut sim = scenario(vec![
            (0, "barracks", 20, 20),
            (0, "gap", 20, 20),
            (1, "scout", 30, 20),
        ]);
        let scout_id = sim.units().ids()[2];
        sim.tick(&[Command::new(
            PlayerId(1),
            0,
            CommandKind::Move {
                units: vec![scout_id],
                target: Cell::new(20, 20),
            },
        )]);
        for _ in 0..300 {
            sim.tick(&[]);
        }
        sim.state_hash()
    };
    assert_eq!(run(), run());
}
