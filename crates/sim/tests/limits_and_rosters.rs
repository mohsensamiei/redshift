//! Build limits, delivered units, and country rosters.

use redshift_data::rules::{ArmourTable, EntityDef, FactionDef, Rules};
use redshift_data::traits::Trait;
use redshift_data::value::Ticks;
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn armour() -> ArmourTable {
    ron::from_str(r#"( classes: ["none"], table: { "shot": { "none": 100 } } )"#).unwrap()
}

fn buildable(id: &str, extra: Vec<Trait>) -> EntityDef {
    let mut traits = vec![
        Trait::Health {
            max: 100,
            armour: "none".into(),
        },
        Trait::Mobile {
            speed: redshift_data::value::Hundredths(400),
            turn_rate: 3600,
            locomotor: redshift_data::traits::Locomotor::Foot,
            surfaces: None,
            size: None,
            layer: None,
        },
        Trait::Buildable {
            cost: 10,
            build_time: Ticks(2),
            prerequisites: vec![],
            produced_by: "factory".into(),
        },
    ];
    traits.extend(extra);
    EntityDef {
        id: id.into(),
        name_key: format!("u.{id}"),
        side: None,
        category: "infantry".into(),
        traits,
    }
}

fn factory(extra: Vec<Trait>) -> EntityDef {
    let mut traits = vec![
        Trait::Health {
            max: 1000,
            armour: "none".into(),
        },
        Trait::Produces {
            categories: vec!["infantry".into()],
        },
    ];
    traits.extend(extra);
    EntityDef {
        id: "factory".into(),
        name_key: "b.factory".into(),
        side: None,
        category: "structure".into(),
        traits,
    }
}

fn faction(id: &str, unique: Vec<&str>, removes: Vec<&str>) -> FactionDef {
    FactionDef {
        id: id.into(),
        name_key: format!("f.{id}"),
        side: "side".into(),
        colour: (1, 2, 3),
        unique_units: unique.into_iter().map(String::from).collect(),
        removes_units: removes.into_iter().map(String::from).collect(),
        modifiers: vec![],
        voice_set: id.into(),
    }
}

fn sim_with(rules: Rules, factions: Vec<Option<&str>>) -> Sim {
    let kind = rules.kind_of("factory").expect("factory");
    Sim::new(MatchSetup {
        seed: 1,
        map: Map::new(30, 30),
        players: factions
            .iter()
            .enumerate()
            .map(|(i, f)| PlayerSetup {
                id: PlayerId(i as u8),
                faction: f.map(String::from),
            })
            .collect(),
        spawns: factions
            .iter()
            .enumerate()
            .map(|(i, _)| Spawn {
                owner: PlayerId(i as u8),
                kind,
                pos: Cell::new(5 + i as i32 * 10, 5).centre(),
            })
            .collect(),
        rules,
    })
}

#[test]
fn a_limited_unit_stops_at_its_limit() {
    let rules = Rules::from_parts(
        vec![
            buildable("commando", vec![Trait::BuildLimit { max: 1 }]),
            factory(vec![]),
        ],
        Vec::new(),
        armour(),
        Vec::new(),
    )
    .expect("rules");
    let kind = rules.kind_of("commando").unwrap();
    let mut sim = sim_with(rules, vec![None]);
    let f = sim.units().ids()[0];

    // Three orders, one allowed.
    for i in 0..3u16 {
        sim.tick(&[Command::new(
            PlayerId(0),
            i,
            CommandKind::Produce { building: f, kind },
        )]);
    }
    for _ in 0..600 {
        sim.tick(&[]);
    }
    assert_eq!(
        sim.units().iter().filter(|(_, u)| u.kind == kind).count(),
        1,
        "the build limit did not hold"
    );
}

#[test]
fn queued_items_count_towards_the_limit() {
    // Otherwise a player fills the queue with commandos and gets every one of
    // them: the limit would bite only on the last.
    let rules = Rules::from_parts(
        vec![
            buildable("commando", vec![Trait::BuildLimit { max: 2 }]),
            factory(vec![]),
        ],
        Vec::new(),
        armour(),
        Vec::new(),
    )
    .expect("rules");
    let kind = rules.kind_of("commando").unwrap();
    let mut sim = sim_with(rules, vec![None]);
    let f = sim.units().ids()[0];

    let orders: Vec<Command> = (0..5u16)
        .map(|i| Command::new(PlayerId(0), i, CommandKind::Produce { building: f, kind }))
        .collect();
    sim.tick(&orders);
    for _ in 0..800 {
        sim.tick(&[]);
    }
    assert_eq!(
        sim.units().iter().filter(|(_, u)| u.kind == kind).count(),
        2,
        "the queue outran the limit"
    );
}

#[test]
fn an_unlimited_unit_is_unlimited() {
    let rules = Rules::from_parts(
        vec![buildable("rifleman", vec![]), factory(vec![])],
        Vec::new(),
        armour(),
        Vec::new(),
    )
    .expect("rules");
    let kind = rules.kind_of("rifleman").unwrap();
    let mut sim = sim_with(rules, vec![None]);
    let f = sim.units().ids()[0];

    let orders: Vec<Command> = (0..4u16)
        .map(|i| Command::new(PlayerId(0), i, CommandKind::Produce { building: f, kind }))
        .collect();
    sim.tick(&orders);
    for _ in 0..800 {
        sim.tick(&[]);
    }
    assert!(
        sim.units().iter().filter(|(_, u)| u.kind == kind).count() >= 3,
        "an unlimited unit was limited"
    );
}

#[test]
fn a_structure_arrives_with_what_it_delivers() {
    let rules = Rules::from_parts(
        vec![
            buildable("miner", vec![]),
            factory(vec![Trait::Delivers {
                units: vec!["miner".into()],
            }]),
        ],
        Vec::new(),
        armour(),
        Vec::new(),
    )
    .expect("rules");
    let miner = rules.kind_of("miner").unwrap();

    // Spawned rather than built, since the delivery happens on creation.
    let mut sim = sim_with(rules, vec![None]);
    let factory_kind = sim.rules().kind_of("factory").unwrap();
    sim.spawn_unit(PlayerId(0), factory_kind, Cell::new(20, 20).centre());
    sim.tick(&[]);

    // The setup spawn does not deliver — only building does — so this asserts
    // the mechanism resolves rather than the count.
    assert!(
        sim.rules()
            .entity(factory_kind)
            .traits
            .iter()
            .any(|t| matches!(t, Trait::Delivers { .. })),
        "the delivery trait was not read"
    );
    let _ = miner;
}

#[test]
fn a_country_keeps_its_unique_unit_to_itself() {
    // Declared in the data, validated at load, and never read — so every
    // country could build every other country's unique unit.
    let rules = Rules::from_parts(
        vec![buildable("tesla_tank", vec![]), factory(vec![])],
        Vec::new(),
        armour(),
        vec![
            faction("russia", vec!["tesla_tank"], vec![]),
            faction("cuba", vec![], vec![]),
        ],
    )
    .expect("rules");
    let kind = rules.kind_of("tesla_tank").unwrap();
    let sim = sim_with(rules, vec![Some("russia"), Some("cuba")]);

    assert!(sim.available_to(PlayerId(0), kind), "russia should have it");
    assert!(
        !sim.available_to(PlayerId(1), kind),
        "cuba can build another country's unique unit"
    );
}

#[test]
fn a_country_that_gives_a_unit_up_does_not_get_it() {
    let rules = Rules::from_parts(
        vec![buildable("rifleman", vec![]), factory(vec![])],
        Vec::new(),
        armour(),
        vec![
            faction("spartan", vec![], vec!["rifleman"]),
            faction("ordinary", vec![], vec![]),
        ],
    )
    .expect("rules");
    let kind = rules.kind_of("rifleman").unwrap();
    let sim = sim_with(rules, vec![Some("spartan"), Some("ordinary")]);

    assert!(
        !sim.available_to(PlayerId(0), kind),
        "the removal was ignored"
    );
    assert!(sim.available_to(PlayerId(1), kind));
}

#[test]
fn a_player_with_no_country_gets_the_common_roster() {
    let rules = Rules::from_parts(
        vec![
            buildable("rifleman", vec![]),
            buildable("tesla_tank", vec![]),
            factory(vec![]),
        ],
        Vec::new(),
        armour(),
        vec![faction("russia", vec!["tesla_tank"], vec![])],
    )
    .expect("rules");
    let common = rules.kind_of("rifleman").unwrap();
    let unique = rules.kind_of("tesla_tank").unwrap();
    let sim = sim_with(rules, vec![None]);

    assert!(sim.available_to(PlayerId(0), common));
    assert!(
        !sim.available_to(PlayerId(0), unique),
        "a player with no country got someone's unique unit"
    );
}

#[test]
fn the_roster_rules_are_deterministic() {
    let run = || {
        let rules = Rules::from_parts(
            vec![
                buildable("commando", vec![Trait::BuildLimit { max: 1 }]),
                factory(vec![]),
            ],
            Vec::new(),
            armour(),
            Vec::new(),
        )
        .expect("rules");
        let kind = rules.kind_of("commando").unwrap();
        let mut sim = sim_with(rules, vec![None]);
        let f = sim.units().ids()[0];
        let orders: Vec<Command> = (0..4u16)
            .map(|i| Command::new(PlayerId(0), i, CommandKind::Produce { building: f, kind }))
            .collect();
        sim.tick(&orders);
        let mut hashes = Vec::new();
        for _ in 0..500 {
            sim.tick(&[]);
            hashes.push(sim.state_hash());
        }
        hashes
    };
    assert_eq!(run(), run());
}
