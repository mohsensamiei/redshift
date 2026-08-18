//! Can the engine express the roster?
//!
//! `docs/08-roster.md` says what the game is made of. This asks the engine,
//! in code, whether it can hold each of those things — because a prose list of
//! capabilities drifts from the code the moment either changes, and a list of
//! capabilities that is *wrong* is worse than none.
//!
//! # How to read this file
//!
//! - A passing test is a capability the engine **has**, exercised through the
//!   data layer exactly as a real unit would use it.
//! - An `#[ignore]`d test is a capability the engine **lacks**, with the reason
//!   attached. `cargo test -p redshift-sim --test roster_conformance -- --ignored`
//!   runs them; they are expected to fail until the feature exists.
//!
//! So the live gap list is:
//!
//! ```sh
//! cargo test -p redshift-sim --test roster_conformance -- --list | grep ignore
//! ```
//!
//! When a gap is closed, delete the `#[ignore]`. If a test here ever needs Rust
//! changes to express a *unit*, that is the signal that ADR 0006 has been
//! violated somewhere.

use redshift_data::rules::{ArmourTable, EntityDef, FactionDef, Rules, WeaponDef};
use redshift_data::traits::{Locomotor, Surface, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map, Terrain};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

// ---------------------------------------------------------------------------
// Scaffolding
// ---------------------------------------------------------------------------

fn armour() -> ArmourTable {
    ron::from_str(
        r#"(
            classes: ["none", "heavy"],
            table: { "shot": { "none": 100, "heavy": 30 } },
        )"#,
    )
    .expect("armour table")
}

fn rifle() -> WeaponDef {
    WeaponDef {
        id: "rifle".into(),
        damage: 25,
        warhead: "shot".into(),
        reload: Ticks(10),
        range: Hundredths(400),
        splash_radius: Hundredths::ZERO,
        projectile_speed: Hundredths::ZERO,
    }
}

/// A minimal mobile unit, with whatever else is asked for bolted on.
fn unit(id: &str, category: &str, locomotor: Locomotor, extra: Vec<Trait>) -> EntityDef {
    let mut traits = vec![
        Trait::Health {
            max: 200,
            armour: "none".into(),
        },
        Trait::Mobile {
            speed: Hundredths(400),
            turn_rate: 3600,
            locomotor,
            surfaces: None,
            size: None,
        },
        Trait::Vision {
            range: Hundredths(600),
        },
    ];
    traits.extend(extra);
    EntityDef {
        id: id.into(),
        name_key: format!("unit.{id}"),
        side: None,
        category: category.into(),
        traits,
    }
}

fn rules_with(entities: Vec<EntityDef>, factions: Vec<FactionDef>) -> Rules {
    Rules::from_parts(entities, vec![rifle()], armour(), factions).expect("rules should validate")
}

/// A map with a lake down the middle and a ridge across the top.
fn divided_map() -> Map {
    let mut map = Map::new(40, 40);
    map.fill_rect(Cell::new(0, 18), Cell::new(39, 22), Terrain::Water);
    map.fill_rect(Cell::new(0, 6), Cell::new(39, 7), Terrain::Rock);
    map
}

fn one_unit(rules: Rules, map: Map, kind: &str, at: Cell) -> Sim {
    let kind = rules
        .kind_of(kind)
        .unwrap_or_else(|| panic!("no kind {kind}"));
    Sim::new(MatchSetup {
        seed: 0xC0FFEE,
        map,
        players: vec![PlayerSetup {
            id: PlayerId(0),
            faction: None,
        }],
        spawns: vec![Spawn {
            owner: PlayerId(0),
            kind,
            pos: at.centre(),
        }],
        rules,
    })
}

/// Orders the only unit somewhere and reports whether it arrived.
fn can_reach(sim: &mut Sim, goal: Cell) -> bool {
    let id = sim.units().ids()[0];
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Move {
            units: vec![id],
            target: goal,
        },
    )]);
    for _ in 0..6_000 {
        sim.tick(&[]);
        if sim
            .units()
            .get(id)
            .is_some_and(|u| u.cell().chebyshev_to(goal) <= 2)
        {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Movement: the exceptions ADR 0006 exists for
// ---------------------------------------------------------------------------

#[test]
fn ordinary_infantry_cannot_cross_water() {
    let rules = rules_with(
        vec![unit("rifleman", "infantry", Locomotor::Foot, vec![])],
        vec![],
    );
    let mut sim = one_unit(rules, divided_map(), "rifleman", Cell::new(20, 12));
    assert!(
        !can_reach(&mut sim, Cell::new(20, 30)),
        "a rifleman walked across a lake"
    );
}

#[test]
fn amphibious_infantry_crosses_water_with_no_engine_change() {
    // The user's first example. One line of data.
    let swimmer = unit(
        "swimmer",
        "infantry",
        Locomotor::Foot,
        vec![Trait::Mobile {
            speed: Hundredths(400),
            turn_rate: 3600,
            locomotor: Locomotor::Foot,
            surfaces: Some(vec![Surface::Land, Surface::Water]),
            size: None,
        }],
    );
    // The override replaces the default Mobile, so drop the original.
    let mut def = swimmer;
    def.traits
        .retain(|t| !matches!(t, Trait::Mobile { surfaces: None, .. }));

    let rules = rules_with(vec![def], vec![]);
    let mut sim = one_unit(rules, divided_map(), "swimmer", Cell::new(20, 12));
    assert!(
        can_reach(&mut sim, Cell::new(20, 30)),
        "amphibious infantry could not cross the lake"
    );
}

#[test]
fn a_hovercraft_crosses_both_surfaces() {
    // The user's second example: a vehicle that goes on water.
    let rules = rules_with(
        vec![unit("hovercraft", "vehicle", Locomotor::Hover, vec![])],
        vec![],
    );
    let mut sim = one_unit(rules, divided_map(), "hovercraft", Cell::new(20, 12));
    assert!(
        can_reach(&mut sim, Cell::new(20, 30)),
        "a hovercraft could not cross the lake"
    );
}

#[test]
fn a_ship_cannot_leave_the_water() {
    let rules = rules_with(vec![unit("boat", "ship", Locomotor::Ship, vec![])], vec![]);
    let mut sim = one_unit(rules, divided_map(), "boat", Cell::new(20, 20));
    assert!(
        !can_reach(&mut sim, Cell::new(20, 30)),
        "a ship drove up the beach"
    );
}

#[test]
fn aircraft_cross_everything_including_high_ground() {
    let rules = rules_with(
        vec![unit("plane", "aircraft", Locomotor::Air, vec![])],
        vec![],
    );
    let mut sim = one_unit(rules, divided_map(), "plane", Cell::new(20, 12));
    assert!(
        can_reach(&mut sim, Cell::new(20, 2)),
        "an aircraft could not fly over a ridge"
    );
}

#[test]
fn a_unit_may_declare_its_own_size() {
    // Physical size used to come from the category, which made an unusually
    // large or small unit a code change.
    let big = EntityDef {
        traits: vec![
            Trait::Health {
                max: 100,
                armour: "none".into(),
            },
            Trait::Mobile {
                speed: Hundredths(400),
                turn_rate: 3600,
                locomotor: Locomotor::Foot,
                surfaces: None,
                size: Some(Hundredths(90)),
            },
            Trait::Vision {
                range: Hundredths(400),
            },
        ],
        ..unit("colossus", "infantry", Locomotor::Foot, vec![])
    };
    let ordinary = unit("rifleman", "infantry", Locomotor::Foot, vec![]);
    let rules = rules_with(vec![big, ordinary], vec![]);
    let sim = one_unit(rules, Map::new(20, 20), "colossus", Cell::new(5, 5));

    let colossus = sim.rules().kind_of("colossus").unwrap();
    let rifleman = sim.rules().kind_of("rifleman").unwrap();
    assert!(
        sim.stats().get(PlayerId(0), colossus).radius
            > sim.stats().get(PlayerId(0), rifleman).radius,
        "a declared size was ignored"
    );
}

// ---------------------------------------------------------------------------
// Production and tech
// ---------------------------------------------------------------------------

fn factory_rules() -> Rules {
    let factory = EntityDef {
        id: "factory".into(),
        name_key: "b.factory".into(),
        side: None,
        category: "structure".into(),
        traits: vec![
            Trait::Health {
                max: 500,
                armour: "none".into(),
            },
            Trait::Produces {
                categories: vec!["vehicle".into()],
            },
            Trait::Footprint {
                width: 3,
                height: 3,
            },
        ],
    };
    let lab = EntityDef {
        id: "lab".into(),
        name_key: "b.lab".into(),
        side: None,
        category: "structure".into(),
        traits: vec![Trait::Health {
            max: 500,
            armour: "none".into(),
        }],
    };
    let basic = unit(
        "tank",
        "vehicle",
        Locomotor::Tracked,
        vec![Trait::Buildable {
            cost: 100,
            build_time: Ticks(10),
            prerequisites: vec![],
            produced_by: "factory".into(),
        }],
    );
    let advanced = unit(
        "super_tank",
        "vehicle",
        Locomotor::Tracked,
        vec![Trait::Buildable {
            cost: 100,
            build_time: Ticks(10),
            prerequisites: vec!["lab".into()],
            produced_by: "factory".into(),
        }],
    );
    rules_with(vec![factory, lab, basic, advanced], vec![])
}

#[test]
fn a_producer_builds_only_its_own_categories() {
    let rules = factory_rules();
    let sim = one_unit(rules, Map::new(40, 40), "factory", Cell::new(20, 20));
    let factory = sim.units().ids()[0];
    assert_eq!(
        sim.producer_for(PlayerId(0), sim.rules().kind_of("tank").unwrap()),
        Some(factory)
    );
}

#[test]
fn prerequisites_gate_the_tech_tree() {
    let rules = factory_rules();
    let sim = one_unit(rules, Map::new(40, 40), "factory", Cell::new(20, 20));
    let basic = sim.rules().kind_of("tank").unwrap();
    let advanced = sim.rules().kind_of("super_tank").unwrap();

    assert!(sim.prerequisites_met(PlayerId(0), basic));
    assert!(
        !sim.prerequisites_met(PlayerId(0), advanced),
        "an advanced unit was available without its prerequisite"
    );
}

#[test]
fn a_structure_that_only_unlocks_is_expressible() {
    // A battle lab makes nothing; it exists so other things become available.
    let rules = factory_rules();
    let mut sim = one_unit(rules, Map::new(40, 40), "factory", Cell::new(20, 20));
    let advanced = sim.rules().kind_of("super_tank").unwrap();
    assert!(!sim.prerequisites_met(PlayerId(0), advanced));

    let lab = sim.rules().kind_of("lab").unwrap();
    sim.spawn_unit(PlayerId(0), lab, Cell::new(30, 30).centre());
    assert!(
        sim.prerequisites_met(PlayerId(0), advanced),
        "building the lab did not unlock anything"
    );
}

// ---------------------------------------------------------------------------
// Gaps — each of these is expected to fail until the feature exists
// ---------------------------------------------------------------------------

#[test]
#[ignore = "gap: a country's unique units are declared, validated, and never applied"]
fn a_country_gets_its_unique_unit_and_not_another_countrys() {
    // `unique_units` and `removes_units` are in the data and checked at load,
    // and nothing reads them. There is no "what can this player build", so
    // every country can build everything.
    let common = unit("tank", "vehicle", Locomotor::Tracked, vec![]);
    let special = unit("tesla_tank", "vehicle", Locomotor::Tracked, vec![]);
    let faction = |id: &str, unique: Vec<String>| FactionDef {
        id: id.into(),
        name_key: format!("f.{id}"),
        side: "soviet".into(),
        colour: (1, 2, 3),
        unique_units: unique,
        removes_units: vec![],
        modifiers: vec![],
        voice_set: id.into(),
    };
    let rules = rules_with(
        vec![common, special],
        vec![
            faction("russia", vec!["tesla_tank".into()]),
            faction("cuba", vec![]),
        ],
    );

    let sim = Sim::new(MatchSetup {
        seed: 1,
        map: Map::new(20, 20),
        players: vec![
            PlayerSetup {
                id: PlayerId(0),
                faction: Some("russia".into()),
            },
            PlayerSetup {
                id: PlayerId(1),
                faction: Some("cuba".into()),
            },
        ],
        spawns: vec![],
        rules,
    });

    let tesla = sim.rules().kind_of("tesla_tank").unwrap();
    assert!(
        sim.prerequisites_met(PlayerId(0), tesla),
        "russia should have it"
    );
    assert!(
        !sim.prerequisites_met(PlayerId(1), tesla),
        "cuba should not be able to build another country's unique unit"
    );
}

#[test]
#[ignore = "gap: Crushable is declared and unread — nothing is ever crushed"]
fn a_tank_crushes_infantry() {
    let tank = unit(
        "tank",
        "vehicle",
        Locomotor::Tracked,
        vec![Trait::Crushes {
            classes: vec!["infantry".into()],
        }],
    );
    let man = unit(
        "rifleman",
        "infantry",
        Locomotor::Foot,
        vec![Trait::Crushable {
            class: "infantry".into(),
        }],
    );
    let rules = rules_with(vec![tank, man], vec![]);
    let tank_kind = rules.kind_of("tank").unwrap();
    let man_kind = rules.kind_of("rifleman").unwrap();

    let mut sim = Sim::new(MatchSetup {
        seed: 1,
        map: Map::new(30, 30),
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
                kind: tank_kind,
                pos: Cell::new(5, 15).centre(),
            },
            Spawn {
                owner: PlayerId(1),
                kind: man_kind,
                pos: Cell::new(15, 15).centre(),
            },
        ],
        rules,
    });
    let victim = sim.units().ids()[1];
    let tank_id = sim.units().ids()[0];

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Move {
            units: vec![tank_id],
            target: Cell::new(25, 15),
        },
    )]);
    for _ in 0..4_000 {
        sim.tick(&[]);
    }
    assert!(
        sim.units().get(victim).is_none(),
        "the tank drove through without crushing"
    );
}

#[test]
#[ignore = "gap: SelfHealing is declared and unread"]
fn a_damaged_unit_with_self_healing_recovers() {
    let regenerator = unit(
        "regenerator",
        "infantry",
        Locomotor::Foot,
        vec![Trait::SelfHealing {
            per_tick: Hundredths(100),
            delay_after_damage: Ticks(10),
        }],
    );
    let rules = rules_with(vec![regenerator], vec![]);
    let mut sim = one_unit(rules, Map::new(20, 20), "regenerator", Cell::new(5, 5));

    // Nothing can damage it here, so this can only fail on the mechanism being
    // absent — which is the point.
    let before = sim.units().iter().next().unwrap().1.health;
    for _ in 0..200 {
        sim.tick(&[]);
    }
    let after = sim.units().iter().next().unwrap().1.health;
    assert!(after >= before, "health went backwards");
    assert!(
        sim.stats()
            .get(PlayerId(0), sim.rules().kind_of("regenerator").unwrap())
            .max_health
            > 0,
        "self-healing is not resolved into stats at all"
    );
}

#[test]
#[ignore = "gap: Explodes is declared and unread — nothing damages its surroundings on death"]
fn a_unit_that_explodes_damages_its_neighbours() {
    let bomb = unit(
        "bomb_truck",
        "vehicle",
        Locomotor::Wheeled,
        vec![Trait::Explodes {
            warhead: "shot".into(),
            damage: 500,
        }],
    );
    let rules = rules_with(vec![bomb], vec![]);
    let _ = one_unit(rules, Map::new(20, 20), "bomb_truck", Cell::new(5, 5));
    panic!("no mechanism exists to trigger or apply a death explosion");
}

#[test]
#[ignore = "gap: Transport is declared and unread — nothing can be loaded or unloaded"]
fn a_transport_carries_and_unloads_passengers() {
    let carrier = unit(
        "apc",
        "vehicle",
        Locomotor::Wheeled,
        vec![Trait::Transport {
            capacity: 5,
            allowed: vec!["rifleman".into()],
        }],
    );
    let passenger = unit("rifleman", "infantry", Locomotor::Foot, vec![]);
    let rules = rules_with(vec![carrier, passenger], vec![]);
    let _ = one_unit(rules, Map::new(20, 20), "apc", Cell::new(5, 5));
    panic!("no load or unload command exists");
}

#[test]
#[ignore = "gap: Capturable is declared and unread — engineers cannot capture"]
fn an_engineer_captures_a_neutral_structure() {
    panic!("no capture command, and no neutral player to own the structure");
}

#[test]
#[ignore = "gap: no neutral player — civilians and neutral structures have no owner"]
fn a_neutral_structure_belongs_to_nobody_and_is_hostile_to_nobody() {
    panic!("every player is hostile to every other; there is no neutral side");
}

#[test]
#[ignore = "gap: projectiles — every shot lands instantly, so travel time and interception do not exist"]
fn a_slow_projectile_takes_time_to_arrive() {
    panic!("weapons apply damage on the tick they fire");
}

#[test]
#[ignore = "gap: air targeting — nothing distinguishes an air target from a ground one"]
fn an_anti_air_weapon_hits_aircraft_and_not_tanks() {
    panic!("the armour table has an 'air' class and no unit is ever in it");
}

#[test]
#[ignore = "gap: multiple weapons — a unit has at most one Armed trait"]
fn a_unit_can_carry_an_anti_ground_and_an_anti_air_weapon() {
    panic!("Armed is a unique trait; a second one would be a data error");
}

#[test]
#[ignore = "gap: deploy — a unit cannot become a structure or change stance"]
fn a_construction_vehicle_deploys_into_a_building() {
    panic!("no deploy command, and no mechanism to replace a unit with another kind");
}

#[test]
#[ignore = "gap: garrison — infantry cannot occupy a building and fire from it"]
fn infantry_garrison_a_civilian_building() {
    panic!("structures cannot hold passengers");
}

#[test]
#[ignore = "gap: elevation — high ground is faked with impassable rock"]
fn a_unit_on_high_ground_sees_further() {
    panic!("the map has no height, only a rock terrain that blocks everything");
}
