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
use redshift_sim::map::{Cell, Map, SurfaceMask, Terrain};
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
        homing: false,
        targets: vec![],
        instant_kill: false,
        ammo: 0,
        intercepts: false,
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
            layer: None,
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
            layer: None,
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
                layer: None,
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
fn a_country_gets_its_unique_unit_and_not_another_countrys() {
    // Closed. `unique_units` and `removes_units` were declared in the data and
    // validated at load, and nothing read them — so every country could build
    // every other country's unique unit.
    //
    // A unit named as unique by *any* country is available only to that
    // country. That is what makes it unique, and it means a country needs no
    // list of the things it cannot have. Exercised in
    // tests/limits_and_rosters.rs.
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
    let tesla = rules.kind_of("tesla_tank").unwrap();

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

    assert!(
        sim.available_to(PlayerId(0), tesla),
        "russia should have it"
    );
    assert!(
        !sim.available_to(PlayerId(1), tesla),
        "cuba can build another country's unique unit"
    );
}

#[test]
fn a_tank_crushes_infantry() {
    // Closed. Crush classes are interned to a bitmask at load rather than
    // compared as strings on the movement path — both faster and free of the
    // iteration-order hazard a per-unit Vec<String> would carry.
    //
    // Exercised properly in tests/small_traits.rs; this asserts the data
    // resolves, which is what the audit is for.
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
    let sim = one_unit(rules, Map::new(20, 20), "tank", Cell::new(5, 5));

    let tank_kind = sim.rules().kind_of("tank").unwrap();
    let man_kind = sim.rules().kind_of("rifleman").unwrap();
    let stats = sim.stats();
    assert!(
        stats.get(PlayerId(0), tank_kind).crushes != 0,
        "the tank crushes nothing"
    );
    assert!(
        stats.get(PlayerId(0), man_kind).crush_class != 0,
        "the rifleman is not crushable"
    );
    assert!(
        stats.get(PlayerId(0), tank_kind).crushes & stats.get(PlayerId(0), man_kind).crush_class
            != 0,
        "the classes do not match, so nothing would ever be crushed"
    );
}

#[test]
fn a_damaged_unit_with_self_healing_recovers() {
    // Closed. The delay is the part that matters: it makes this a recovery
    // mechanic rather than an armour bonus, since a unit under fire gains
    // nothing.
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
    let sim = one_unit(rules, Map::new(20, 20), "regenerator", Cell::new(5, 5));
    let kind = sim.rules().kind_of("regenerator").unwrap();
    let stats = sim.stats().get(PlayerId(0), kind);

    assert!(stats.self_heal > 0, "self-healing was not resolved");
    assert_eq!(stats.heal_delay, 10, "the delay was not resolved");
}

#[test]
fn a_unit_that_explodes_damages_its_neighbours() {
    // Closed. Chain reactions resolve one tick at a time — a unit killed by a
    // blast detonates on the *next* tick — which is both bounded and visibly
    // correct, since a chain of explosions should look like a chain.
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
    let sim = one_unit(rules, Map::new(20, 20), "bomb_truck", Cell::new(5, 5));
    let kind = sim.rules().kind_of("bomb_truck").unwrap();

    assert_eq!(
        sim.stats().get(PlayerId(0), kind).death_damage,
        500,
        "the death explosion was not resolved"
    );
}

#[test]
fn a_transport_carries_and_unloads_passengers() {
    // Closed. The interesting part was never loading and unloading — it is that
    // a passenger has to leave the world in *every* respect while keeping its
    // identity: not moving, not shooting, not shot at, not seen, not revealing
    // ground, not taking up room, not crushing or being crushed.
    //
    // Missing one of those looks like a rifleman firing from inside a sealed
    // truck. Exercised properly in tests/transport.rs.
    let apc = unit(
        "apc",
        "vehicle",
        Locomotor::Wheeled,
        vec![Trait::Transport {
            capacity: 5,
            allowed: vec!["rifleman".into()],
        }],
    );
    let passenger = unit("rifleman", "infantry", Locomotor::Foot, vec![]);
    let rules = rules_with(vec![apc, passenger], vec![]);
    let sim = one_unit(rules, Map::new(20, 20), "apc", Cell::new(5, 5));

    let kind = sim.rules().kind_of("apc").unwrap();
    assert_eq!(
        sim.stats().get(PlayerId(0), kind).capacity,
        5,
        "the transport capacity was not resolved"
    );
}

#[test]
fn an_engineer_captures_a_neutral_structure() {
    // Closed. One action with three outcomes decided by whose building it is —
    // capture what is not yours, repair what is, and be consumed either way.
    // The original never asked a player to choose between capture and repair;
    // they chose a building.
    //
    // Exercised properly in tests/capture.rs.
    let engineer = unit(
        "engineer",
        "infantry",
        Locomotor::Foot,
        vec![Trait::Engineer { consumed: true }],
    );
    let derrick = EntityDef {
        id: "derrick".into(),
        name_key: "b.derrick".into(),
        side: None,
        category: "structure".into(),
        traits: vec![
            Trait::Health {
                max: 500,
                armour: "none".into(),
            },
            Trait::Capturable,
        ],
    };
    let rules = rules_with(vec![engineer, derrick], vec![]);
    let sim = one_unit(rules, Map::new(20, 20), "engineer", Cell::new(5, 5));

    let e = sim.rules().kind_of("engineer").unwrap();
    let d = sim.rules().kind_of("derrick").unwrap();
    assert!(sim.stats().get(PlayerId(0), e).is_engineer);
    assert!(sim.stats().get(PlayerId(0), e).consumed_on_use);
    assert!(sim.stats().get(PlayerId(0), d).capturable);
}

#[test]
#[ignore = "gap: a destroyed building clears its ground completely"]
fn a_destroyed_building_leaves_rubble() {
    // From OpenRA's data rather than any description: a destroyed structure
    // leaves a separate rubble object standing on its footprint. Redshift frees
    // the ground the moment the building dies.
    panic!("destruction removes the entity and releases its ground");
}

#[test]
fn killing_a_civilian_pays_a_small_bounty() {
    // Closed. Any unit may carry a payout, and it goes to the owner of whatever
    // landed the killing blow — read before the attacker is looked up, so a
    // shell already in the air still pays out even if the unit that fired it
    // has since died.
    let civilian = unit(
        "civilian",
        "infantry",
        Locomotor::Foot,
        vec![Trait::Bounty { credits: 5 }],
    );
    let rules = rules_with(vec![civilian], vec![]);
    let sim = one_unit(rules, Map::new(20, 20), "civilian", Cell::new(5, 5));
    let kind = sim.rules().kind_of("civilian").unwrap();

    assert_eq!(
        sim.stats().get(PlayerId(0), kind).bounty,
        5,
        "the bounty was not resolved"
    );
}

#[test]
fn a_unit_runs_out_of_ammunition_and_returns_to_rearm() {
    // Half closed, honestly. A unit with an ammunition limit now stops firing
    // when it is spent, which is the rule that makes an aircraft a sortie
    // rather than a flying gun — and it is general rather than an aircraft
    // special case.
    //
    // *Returning to rearm* is not done: nothing refills the count, so a unit
    // that runs dry stays dry. That belongs with aircraft basing, which is
    // still open.
    let gunner = unit(
        "gunner",
        "vehicle",
        Locomotor::Tracked,
        vec![Trait::Armed {
            weapon: "limited".into(),
            turret: true,
            turret_rate: 3600,
        }],
    );
    let mut limited = rifle();
    limited.id = "limited".into();
    limited.ammo = 3;

    let rules = Rules::from_parts(vec![gunner], vec![limited], armour(), vec![]).expect("rules");
    let sim = one_unit(rules, Map::new(20, 20), "gunner", Cell::new(5, 5));
    let kind = sim.rules().kind_of("gunner").unwrap();
    assert_eq!(sim.combat().weapon(kind).map(|w| w.ammo), Some(3));
}

#[test]
#[ignore = "gap: a destroyed vehicle releases nothing"]
fn a_destroyed_vehicle_ejects_its_crew() {
    // Survivors change the value of every vehicle kill: destroying a transport
    // full of infantry is not the same as destroying an empty one.
    panic!("destruction removes the unit and leaves nothing behind");
}

#[test]
fn newly_built_units_walk_to_a_rally_point() {
    // Closed. Set on the building rather than on its production queue, because
    // a rally point outlives any particular thing being built and a player
    // expects it to survive an empty queue. Exercised in
    // tests/rally_sell_bounty.rs.
    let rules = factory_rules();
    let mut sim = one_unit(rules, Map::new(40, 40), "factory", Cell::new(20, 20));
    let factory = sim.units().ids()[0];

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::SetRally {
            building: factory,
            at: Cell::new(30, 30),
        },
    )]);
    assert_eq!(
        sim.units().get(factory).unwrap().rally,
        Some(Cell::new(30, 30)),
        "the rally point was not recorded"
    );
}

#[test]
fn a_structure_can_be_sold_for_a_refund() {
    // Closed. Paid on the building's condition rather than its full price, so
    // selling cannot be used to launder damage into money, and refused for
    // anything mobile — selling a tank would be a very easy way to turn an army
    // into cash mid-battle.
    // The factory in `factory_rules` has no cost, and a structure worth nothing
    // is correctly worth nothing to sell — so this needs one with a price.
    let priced = EntityDef {
        id: "depot".into(),
        name_key: "b.depot".into(),
        side: None,
        category: "structure".into(),
        traits: vec![
            Trait::Health {
                max: 500,
                armour: "none".into(),
            },
            Trait::Buildable {
                cost: 1000,
                build_time: Ticks(10),
                prerequisites: vec![],
                produced_by: "depot".into(),
            },
        ],
    };
    let rules = rules_with(vec![priced], vec![]);
    let mut sim = one_unit(rules, Map::new(40, 40), "depot", Cell::new(20, 20));
    let factory = sim.units().ids()[0];
    let before = sim.treasury().credits(PlayerId(0));

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Sell { building: factory },
    )]);
    sim.tick(&[]);

    assert!(
        sim.units().get(factory).is_none(),
        "the structure was not demolished"
    );
    assert!(
        sim.treasury().credits(PlayerId(0)) > before,
        "selling paid nothing"
    );
}

#[test]
#[ignore = "gap: ore is finite — it does not regrow"]
fn an_ore_field_regrows_from_its_source() {
    // The widest-reaching of the gaps found by cross-check. Redshift's economy
    // assumes a fixed quantity on the map; a renewable one changes how long a
    // match runs and whether holding a contested field is worth it.
    panic!("ore is placed once and only ever decreases");
}

#[test]
#[ignore = "gap: no persistent terrain effects — radiation, fire, contamination"]
fn irradiated_ground_damages_what_stands_on_it() {
    // The Desolator leaves ground that hurts. Terrain that has state and acts
    // on units, which the map has no concept of.
    panic!("terrain is static and never damages anything");
}

#[test]
#[ignore = "gap: no idle behaviour — a unit with nothing to do does nothing at all"]
fn an_idle_civilian_wanders() {
    // What makes a town read as alive rather than as a set of props. It is
    // deliberately not an AI: a loop of aimless movement, and nothing more.
    panic!("an idle unit stands perfectly still forever");
}

#[test]
#[ignore = "gap: a match cannot detect that nobody can win"]
fn a_stalemate_is_detected() {
    // Two players with no production and no way to reach each other should not
    // leave the match running until someone quits.
    panic!("there are no victory or stalemate conditions at all");
}

#[test]
#[ignore = "gap: a structure's strength cannot depend on its neighbours"]
fn prism_towers_chain_to_strengthen_each_other() {
    // Researched: adjacent Prism Towers combine beams, and the result is
    // proportionally stronger with each tower in the chain. Every stat in the
    // engine is resolved per kind at match start; nothing can depend on what is
    // standing next to it.
    panic!("stats are per kind and fixed; there is no notion of a neighbour");
}

#[test]
#[ignore = "gap: visibility is additive — nothing can hide ground from an opponent"]
fn a_gap_generator_hides_a_base_from_the_enemy() {
    // The Gap Generator does not reveal ground for its owner. It *hides* ground
    // from everyone else, which Redshift's visibility model has no way to
    // express: explored is cumulative and never taken away.
    panic!("visibility only ever adds; explored ground cannot be un-explored");
}

#[test]
fn an_ore_purifier_increases_the_value_of_every_load() {
    // Closed, along with the shape it shares with two other gaps: a modifier
    // that lives on the *player* rather than on a unit, held for as long as its
    // source stands.
    //
    // Ore value multiplies rather than overwrites, so a second purifier is
    // worth building. A source with no power grants nothing, or cutting an
    // enemy's power would matter much less. Exercised in tests/boons_loop.rs.
    use redshift_data::traits::PlayerEffect;
    use redshift_data::value::Percent;

    let purifier = EntityDef {
        id: "purifier".into(),
        name_key: "b.purifier".into(),
        side: None,
        category: "structure".into(),
        traits: vec![
            Trait::Health {
                max: 500,
                armour: "none".into(),
            },
            Trait::Grants {
                effect: PlayerEffect::OreValue(Percent(125)),
            },
        ],
    };
    let rules = rules_with(vec![purifier], vec![]);
    let sim = one_unit(rules, Map::new(20, 20), "purifier", Cell::new(5, 5));

    assert_eq!(
        sim.boons().ore_value(PlayerId(0)).0,
        125,
        "a standing grant was not applied"
    );
}

#[test]
fn a_garrisoned_building_fires_and_evicts_when_badly_damaged() {
    // Researched and more specific than expected: capacity depends on the
    // building's size, only basic infantry may enter, the building fires with a
    // *predetermined* weapon rather than its occupants', and the garrison is
    // forced out below 33% health rather than dying with it.
    //
    // The last rule matters: clearing a garrison means damaging it enough to
    // evict, not destroying it — which is what makes a garrisoned building
    // worth attacking rather than avoiding. Exercised end to end in
    // tests/garrison.rs.
    let mut house = unit(
        "house",
        "civilian",
        Locomotor::Foot,
        vec![
            Trait::Footprint {
                width: 2,
                height: 2,
            },
            Trait::Garrisonable {
                capacity: 3,
                categories: vec!["infantry".into()],
                weapon: "rifle".into(),
                evict_below_percent: 33,
            },
        ],
    );
    house.traits.retain(|t| !matches!(t, Trait::Mobile { .. }));

    let rules = rules_with(vec![house], vec![]);
    let sim = one_unit(rules, Map::new(24, 24), "house", Cell::new(10, 10));
    let kind = sim.rules().kind_of("house").unwrap();
    let stats = sim.stats().get(PlayerId(0), kind);

    assert_eq!(stats.garrison_capacity, 3, "capacity is the building's");
    assert_eq!(stats.evict_below_percent, 33);
    assert!(
        sim.combat().garrison_weapon(kind).is_some(),
        "the building has no weapon of its own to fire"
    );
    assert!(
        sim.combat().weapon(kind).is_none(),
        "an unoccupied building must have no weapon at all, or garrisoning it \
         would change nothing"
    );
}

#[test]
fn a_unit_on_high_ground_outranges_one_below() {
    // Elevation is a layer on the map rather than a kind of terrain, which is
    // what lets a plateau be somewhere a unit stands and fights. The cliff is
    // the *step* between levels: one level is a ramp, two is a wall.
    //
    // The advantage is exercised end to end in tests/elevation.rs, where two
    // identical soldiers are placed so that only the extended reach spans the
    // gap between them.
    let mut map = Map::new(16, 16);
    map.raise_rect(Cell::new(4, 4), Cell::new(8, 8), 2);
    let land = SurfaceMask::from_surfaces(&[Surface::Land]);

    assert!(
        map.is_passable(Cell::new(6, 6), land),
        "the plateau must be standable, or high ground is just rock again"
    );
    assert!(
        !map.step_is_climbable(Cell::new(3, 6), Cell::new(4, 6), land),
        "two levels at once is a cliff face"
    );
    assert!(
        map.elevation_bonus(Cell::new(6, 6)) > map.elevation_bonus(Cell::new(1, 1)),
        "holding the hill has to be worth something"
    );
}

#[test]
#[ignore = "gap: bridges — destructible terrain, repaired through a hut beside them"]
fn a_destroyed_bridge_is_repaired_by_an_engineer_at_its_hut() {
    // Worth noting that this is *not* a new mechanic: the repair hut is entered
    // like a tech building, so bridge repair is capture with a different effect.
    panic!("no bridges, no destructible terrain, and no capture");
}

#[test]
fn a_structure_stops_working_when_power_runs_short() {
    // The correction this closed: the original *switches off* what it cannot
    // power. Modelling a shortage as a production slowdown alone left a player
    // who lost their reactor with their radar and air defence intact, which
    // removes most of the reason to attack a power plant.
    //
    // Structures say for themselves whether they carry on — a refinery does, a
    // radar does not — so "low power" degrades a base rather than destroying it.
    use redshift_data::traits::Trait;

    let needs_power = |id: &str, works_unpowered: bool| EntityDef {
        id: id.into(),
        name_key: format!("b.{id}"),
        side: None,
        category: "structure".into(),
        traits: vec![
            Trait::Health {
                max: 500,
                armour: "none".into(),
            },
            Trait::Vision {
                range: Hundredths(600),
            },
            Trait::PowerDraw {
                amount: 100,
                works_unpowered,
            },
        ],
    };

    let rules = rules_with(
        vec![needs_power("radar", false), needs_power("refinery", true)],
        vec![],
    );
    let radar = rules.kind_of("radar").unwrap();
    let refinery = rules.kind_of("refinery").unwrap();

    // No power plant anywhere, so both are in a shortage.
    let sim = Sim::new(MatchSetup {
        seed: 1,
        map: Map::new(40, 40),
        players: vec![PlayerSetup {
            id: PlayerId(0),
            faction: None,
        }],
        spawns: vec![
            Spawn {
                owner: PlayerId(0),
                kind: radar,
                pos: Cell::new(10, 10).centre(),
            },
            Spawn {
                owner: PlayerId(0),
                kind: refinery,
                pos: Cell::new(20, 20).centre(),
            },
        ],
        rules,
    });

    assert!(
        !sim.power().is_satisfied(PlayerId(0)),
        "the base should be short"
    );

    let ids = sim.units().ids();
    assert!(
        sim.is_unpowered(sim.units().get(ids[0]).unwrap()),
        "the radar should have gone dark"
    );
    assert!(
        !sim.is_unpowered(sim.units().get(ids[1]).unwrap()),
        "the refinery declared it works unpowered and should have carried on"
    );

    // And the disabling is real, not just a flag: the dark radar reveals
    // nothing while the refinery still sees.
    assert!(
        !sim.visibility().is_visible(PlayerId(0), Cell::new(10, 10)),
        "an unpowered radar is still revealing ground"
    );
    assert!(
        sim.visibility().is_visible(PlayerId(0), Cell::new(20, 20)),
        "the refinery should still see"
    );
}

#[test]
fn a_refinery_comes_with_a_miner() {
    // Closed. Delivered beside the new structure, and skipped if there is
    // nowhere to stand — a free miner is a bonus, not a reason to fail a build.
    let miner = unit("miner", "vehicle", Locomotor::Tracked, vec![]);
    let refinery = EntityDef {
        id: "refinery".into(),
        name_key: "b.refinery".into(),
        side: None,
        category: "structure".into(),
        traits: vec![
            Trait::Health {
                max: 500,
                armour: "none".into(),
            },
            Trait::Delivers {
                units: vec!["miner".into()],
            },
        ],
    };
    let rules = rules_with(vec![miner, refinery], vec![]);
    assert!(
        rules
            .entity(rules.kind_of("refinery").unwrap())
            .traits
            .iter()
            .any(|t| matches!(t, Trait::Delivers { .. })),
        "the delivery trait was not read"
    );
}

#[test]
fn only_one_superweapon_of_a_kind_can_be_built() {
    // Closed by the same mechanism as a unit build limit — the rule is about a
    // count, and a structure is no different.
    let silo = EntityDef {
        id: "silo".into(),
        name_key: "b.silo".into(),
        side: None,
        category: "structure".into(),
        traits: vec![
            Trait::Health {
                max: 500,
                armour: "none".into(),
            },
            Trait::BuildLimit { max: 1 },
        ],
    };
    let rules = rules_with(vec![silo], vec![]);
    let sim = one_unit(rules, Map::new(20, 20), "silo", Cell::new(5, 5));
    let kind = sim.rules().kind_of("silo").unwrap();
    assert!(!sim.within_build_limit(PlayerId(0), kind));
}

#[test]
#[ignore = "gap: a destroyed structure has no death effect"]
fn a_nuclear_reactor_explodes_when_destroyed() {
    // `Explodes` is in the trait catalogue and unread, and the reactor case
    // adds lasting ground contamination on top of the blast.
    panic!("destruction removes the unit and does nothing else");
}

#[test]
#[ignore = "gap: a unit cannot modify a structure"]
fn tesla_troopers_charge_a_tesla_coil() {
    // Troopers standing at a coil extend its range and power, and three of them
    // make it work with no power at all. That is a unit changing a structure's
    // stats and its relationship to the power grid — nothing in the engine can
    // express it.
    panic!("stats are resolved per kind at match start and never change");
}

#[test]
fn an_anti_missile_defence_shoots_down_a_rocket() {
    // Closed. Interception runs before flight, so a shot is stopped where it is
    // rather than after it has moved, and nobody shoots down their own.
    // Exercised in tests/weapons.rs.
    let aegis = unit(
        "aegis",
        "vehicle",
        Locomotor::Tracked,
        vec![Trait::Armed {
            weapon: "interceptor".into(),
            turret: true,
            turret_rate: 3600,
        }],
    );
    let mut interceptor = rifle();
    interceptor.id = "interceptor".into();
    interceptor.intercepts = true;

    let rules = Rules::from_parts(vec![aegis], vec![interceptor], armour(), vec![]).expect("rules");
    let sim = one_unit(rules, Map::new(20, 20), "aegis", Cell::new(5, 5));
    let kind = sim.rules().kind_of("aegis").unwrap();
    assert!(
        sim.combat().weapon(kind).is_some_and(|w| w.intercepts),
        "the interception flag was not resolved"
    );
}

#[test]
#[ignore = "gap: submersion is a third visibility state, distinct from cloak"]
fn a_submarine_surfaces_when_it_attacks_or_is_damaged() {
    // Not the same rule as the cloak already implemented: a submarine is
    // revealed by *being damaged* as well as by firing, and specific units
    // detect it. Assuming cloak covers it would be wrong in a way that only
    // shows up in naval play.
    panic!("cloak breaks on firing only, and there is no submerged state");
}

#[test]
#[ignore = "gap: infiltration — no effect table keyed on what was entered"]
fn a_spy_gets_a_different_effect_from_each_kind_of_building() {
    // Researched, and richer than "infiltration works": a barracks promotes
    // everything you build from then on, a refinery hands over a fifth of the
    // victim's money, a power plant goes dark for a minute, and a battle lab
    // unlocks a commando built from *the victim's* technology.
    //
    // So this is a table keyed on the infiltrated building, not one effect with
    // a target. Two of the entries are persistent production modifiers rather
    // than events, which is a third shape again.
    panic!("no infiltration action, and no per-building effect table");
}

#[test]
fn a_depot_repairs_vehicles_and_shakes_off_a_parasite() {
    // Two capabilities that only make sense together, which is why they landed
    // together. A Terror Drone gets *inside* a vehicle, where nothing can shoot
    // it, so the counter cannot be a better gun — it has to be a building. A
    // repair shed with nothing to undo, or a parasite with no answer, would
    // each look arbitrary on its own.
    //
    // The Naval Shipyard and Yuri's Outpost are the same structure with a
    // different list of what they service. Exercised end to end in
    // tests/repair_and_infestation.rs.
    let depot = unit(
        "depot",
        "structure",
        Locomotor::Wheeled,
        vec![
            Trait::Footprint {
                width: 3,
                height: 3,
            },
            Trait::Repairs {
                categories: vec!["vehicle".into()],
                rate: 5,
                cost_percent: 20,
                cures_infestation: true,
            },
        ],
    );
    let drone = unit(
        "drone",
        "infantry",
        Locomotor::Foot,
        vec![Trait::Infests {
            categories: vec!["vehicle".into()],
            damage: 8,
            warhead: "shot".into(),
        }],
    );
    let rules = rules_with(vec![depot, drone], vec![]);
    let sim = one_unit(rules, Map::new(24, 24), "drone", Cell::new(5, 5));

    let depot_kind = sim.rules().kind_of("depot").unwrap();
    let drone_kind = sim.rules().kind_of("drone").unwrap();
    let stats = sim.stats().get(PlayerId(0), depot_kind);

    assert!(stats.repair_rate > 0, "the depot repairs nothing");
    assert!(
        stats.cures_infestation,
        "the depot has no answer to the thing it exists to answer"
    );
    assert!(
        sim.combat().infestation(drone_kind).is_some(),
        "the drone cannot get inside anything"
    );
}

#[test]
#[ignore = "gap: tech structures — neutral, capturable, unsellable, and they extend the build radius"]
fn a_captured_tech_structure_extends_the_build_radius() {
    // A captured oil derrick is a forward base. Redshift has a build radius and
    // only counts structures the player built, so capturing one would give a
    // player income and no ground to build on.
    panic!("no capture, no neutral owner, and the build radius ignores captured ground");
}

#[test]
fn an_effect_can_promote_every_unit_built_from_now_on() {
    // Closed by the same mechanism. A spy in a barracks and a captured machine
    // shop are the same shape: neither is an event nor a per-unit trait, and
    // both needed somewhere on the *player* to live.
    use redshift_data::traits::PlayerEffect;

    let barracks = EntityDef {
        id: "barracks".into(),
        name_key: "b.barracks".into(),
        side: None,
        category: "structure".into(),
        traits: vec![
            Trait::Health {
                max: 500,
                armour: "none".into(),
            },
            Trait::Grants {
                effect: PlayerEffect::VeteranProduction,
            },
        ],
    };
    let rules = rules_with(vec![barracks], vec![]);
    let sim = one_unit(rules, Map::new(20, 20), "barracks", Cell::new(5, 5));

    assert!(
        sim.boons().veteran_production(PlayerId(0)),
        "a standing production modifier was not applied"
    );
}

#[test]
fn some_weapons_kill_outright_regardless_of_health() {
    // Closed. Not the same as very high damage: an instant-kill weapon kills
    // whatever its warhead can hurt at all and does *nothing* to what it
    // cannot, whereas an enormous damage number would make a sniper excellent
    // against tanks.
    let sniper = unit(
        "sniper",
        "infantry",
        Locomotor::Foot,
        vec![Trait::Armed {
            weapon: "sniper_rifle".into(),
            turret: true,
            turret_rate: 3600,
        }],
    );
    let mut rifle = rifle();
    rifle.id = "sniper_rifle".into();
    rifle.instant_kill = true;

    let rules = Rules::from_parts(vec![sniper], vec![rifle], armour(), vec![]).expect("rules");
    let sim = one_unit(rules, Map::new(20, 20), "sniper", Cell::new(5, 5));
    let kind = sim.rules().kind_of("sniper").unwrap();
    assert!(
        sim.combat().weapon(kind).is_some_and(|w| w.instant_kill),
        "the instant-kill flag was not resolved"
    );
}

#[test]
#[ignore = "gap: a unit's weapon cannot depend on its cargo"]
fn an_ifv_changes_weapon_with_its_passenger() {
    // Twenty-four turret modes in the original, four more in the expansion, and
    // an engineer inside turns it into a repair vehicle. The vehicle's weapon
    // is a function of what it is carrying, resolved at runtime.
    panic!("no transport, and weapons are fixed per entity kind");
}

#[test]
fn only_one_commando_can_exist_at_a_time() {
    // Closed. Queued items count towards the limit, or a player fills the queue
    // and gets every one of them — the limit would bite only on the last.
    let commando = unit(
        "commando",
        "infantry",
        Locomotor::Foot,
        vec![Trait::BuildLimit { max: 1 }],
    );
    let rules = rules_with(vec![commando], vec![]);
    let sim = one_unit(rules, Map::new(20, 20), "commando", Cell::new(5, 5));
    let kind = sim.rules().kind_of("commando").unwrap();

    assert_eq!(sim.stats().get(PlayerId(0), kind).build_limit, 1);
    assert!(
        !sim.within_build_limit(PlayerId(0), kind),
        "one already exists, so another should be refused"
    );
}

#[test]
fn a_neutral_structure_belongs_to_nobody_and_is_hostile_to_nobody() {
    // Closed. `PlayerId::NEUTRAL` is a real slot rather than an `Option`,
    // because everything that touches ownership already works in terms of a
    // `PlayerId` and threading a null case through all of it would be far more
    // invasive than reserving a number.
    //
    // Automatic targeting skips neutrals — civilians beside an army start
    // nothing — while a deliberate attack order still works, which is the
    // distinction the original drew.
    assert!(PlayerId::NEUTRAL.is_neutral());
    assert!(!PlayerId(0).is_neutral());

    // The stat table has a row for it. Without one, every neutral unit
    // resolved to zero maximum health and died on the tick it was created.
    let rules = rules_with(
        vec![unit("civilian", "infantry", Locomotor::Foot, vec![])],
        vec![],
    );
    let kind = rules.kind_of("civilian").unwrap();
    let sim = one_unit(rules, Map::new(20, 20), "civilian", Cell::new(5, 5));
    assert!(
        sim.stats().get(PlayerId::NEUTRAL, kind).max_health > 0,
        "the neutral side has no stats, so its units would die instantly"
    );
}

#[test]
fn a_slow_projectile_takes_time_to_arrive() {
    // The gap this closed. A shot that lands on the tick it is fired cannot be
    // dodged, cannot be intercepted, and makes outranging strictly better than
    // it should be.
    let artillery = WeaponDef {
        id: "artillery".into(),
        damage: 100,
        warhead: "shot".into(),
        reload: Ticks(60),
        range: Hundredths(1000),
        splash_radius: Hundredths(50),
        // Two cells a second: slow enough to watch.
        projectile_speed: Hundredths(200),
        homing: false,
        targets: vec![],
        instant_kill: false,
        ammo: 0,
        intercepts: false,
    };
    // Sight to match the gun. A weapon that outranges its own vision cannot
    // fire without a spotter, which is realistic and not what this is testing.
    let mut gunner = unit(
        "gunner",
        "vehicle",
        Locomotor::Tracked,
        vec![Trait::Armed {
            weapon: "artillery".into(),
            turret: true,
            turret_rate: 3600,
        }],
    );
    gunner.traits.retain(|t| !matches!(t, Trait::Vision { .. }));
    gunner.traits.push(Trait::Vision {
        range: Hundredths(1600),
    });
    let victim = unit("victim", "infantry", Locomotor::Foot, vec![]);

    let rules = Rules::from_parts(
        vec![gunner, victim],
        vec![rifle(), artillery],
        armour(),
        vec![],
    )
    .expect("rules");

    let gunner_kind = rules.kind_of("gunner").unwrap();
    let victim_kind = rules.kind_of("victim").unwrap();
    let mut sim = Sim::new(MatchSetup {
        seed: 1,
        map: Map::new(40, 40),
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
                kind: gunner_kind,
                pos: Cell::new(10, 20).centre(),
            },
            Spawn {
                owner: PlayerId(1),
                kind: victim_kind,
                pos: Cell::new(18, 20).centre(),
            },
        ],
        rules,
    });
    let target = sim.units().ids()[1];

    // Find the tick it fires on, and the tick the damage lands.
    let mut fired_at = None;
    let mut landed_at = None;
    for tick in 0..400u32 {
        sim.tick(&[]);
        if fired_at.is_none() && !sim.projectiles().is_empty() {
            fired_at = Some(tick);
        }
        if landed_at.is_none() && sim.units().get(target).is_none_or(|u| u.health < 200) {
            landed_at = Some(tick);
            break;
        }
    }

    let fired = fired_at.expect("the gun never fired");
    let landed = landed_at.expect("the shot never landed");
    assert!(
        landed > fired,
        "the shot landed on the tick it was fired ({fired}), which is the instant-hit model"
    );
    assert!(
        landed - fired > 2,
        "eight cells at two cells a second should take about eighty ticks, not {}",
        landed - fired
    );
}

#[test]
fn an_instant_weapon_still_hits_on_the_tick_it_fires() {
    // A rifle needs no special case, and the behaviour everything was built on
    // must be preserved exactly.
    let shooter = unit(
        "shooter",
        "infantry",
        Locomotor::Foot,
        vec![Trait::Armed {
            weapon: "rifle".into(),
            turret: true,
            turret_rate: 3600,
        }],
    );
    let victim = unit("victim", "infantry", Locomotor::Foot, vec![]);
    let rules = rules_with(vec![shooter, victim], vec![]);

    let shooter_kind = rules.kind_of("shooter").unwrap();
    let victim_kind = rules.kind_of("victim").unwrap();
    let mut sim = Sim::new(MatchSetup {
        seed: 1,
        map: Map::new(20, 20),
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
                kind: shooter_kind,
                pos: Cell::new(5, 5).centre(),
            },
            Spawn {
                owner: PlayerId(1),
                kind: victim_kind,
                pos: Cell::new(7, 5).centre(),
            },
        ],
        rules,
    });
    let target = sim.units().ids()[1];

    for _ in 0..100 {
        sim.tick(&[]);
        assert!(
            sim.projectiles().is_empty(),
            "an instant weapon created a projectile"
        );
        if sim.units().get(target).is_none_or(|u| u.health < 200) {
            return;
        }
    }
    panic!("the rifle never did any damage");
}

#[test]
fn a_ballistic_shot_misses_a_target_that_moves() {
    // The difference between artillery and a tank gun, and the reason homing is
    // a weapon property rather than a global rule.
    let artillery = WeaponDef {
        id: "artillery".into(),
        damage: 100,
        warhead: "shot".into(),
        reload: Ticks(200),
        range: Hundredths(1500),
        splash_radius: Hundredths::ZERO,
        projectile_speed: Hundredths(100),
        homing: false,
        targets: vec![],
        instant_kill: false,
        ammo: 0,
        intercepts: false,
    };
    // Sight to match the gun. A weapon that outranges its own vision cannot
    // fire without a spotter, which is realistic and not what this is testing.
    let mut gunner = unit(
        "gunner",
        "vehicle",
        Locomotor::Tracked,
        vec![Trait::Armed {
            weapon: "artillery".into(),
            turret: true,
            turret_rate: 3600,
        }],
    );
    gunner.traits.retain(|t| !matches!(t, Trait::Vision { .. }));
    gunner.traits.push(Trait::Vision {
        range: Hundredths(1600),
    });
    let runner = unit("runner", "infantry", Locomotor::Foot, vec![]);
    let rules = Rules::from_parts(
        vec![gunner, runner],
        vec![rifle(), artillery],
        armour(),
        vec![],
    )
    .expect("rules");

    let gunner_kind = rules.kind_of("gunner").unwrap();
    let runner_kind = rules.kind_of("runner").unwrap();
    let mut sim = Sim::new(MatchSetup {
        seed: 1,
        map: Map::new(60, 60),
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
                kind: gunner_kind,
                pos: Cell::new(10, 30).centre(),
            },
            Spawn {
                owner: PlayerId(1),
                kind: runner_kind,
                pos: Cell::new(22, 30).centre(),
            },
        ],
        rules,
    });
    let runner_id = sim.units().ids()[1];

    // Wait for the shot to be in the air, then run.
    for _ in 0..200 {
        sim.tick(&[]);
        if !sim.projectiles().is_empty() {
            break;
        }
    }
    assert!(!sim.projectiles().is_empty(), "the gun never fired");

    sim.tick(&[Command::new(
        PlayerId(1),
        0,
        CommandKind::Move {
            units: vec![runner_id],
            target: Cell::new(22, 45),
        },
    )]);
    for _ in 0..400 {
        sim.tick(&[]);
    }

    assert!(
        sim.units().get(runner_id).is_some_and(|u| u.health == 200),
        "a ballistic shell tracked a target that ran away from it"
    );
}

/// Builds a scenario with one shooter and one victim, four cells apart.
fn duel(rules: Rules, shooter: &str, victim: &str) -> Sim {
    let shooter_kind = rules
        .kind_of(shooter)
        .unwrap_or_else(|| panic!("no {shooter}"));
    let victim_kind = rules
        .kind_of(victim)
        .unwrap_or_else(|| panic!("no {victim}"));
    Sim::new(MatchSetup {
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
                kind: shooter_kind,
                pos: Cell::new(10, 15).centre(),
            },
            Spawn {
                owner: PlayerId(1),
                kind: victim_kind,
                pos: Cell::new(13, 15).centre(),
            },
        ],
        rules,
    })
}

/// Rules with a ground gun, an anti-air gun, a tank and an aircraft.
fn air_rules() -> Rules {
    let weapon = |id: &str, targets: Vec<redshift_data::traits::Layer>| WeaponDef {
        id: id.into(),
        damage: 40,
        warhead: "shot".into(),
        reload: Ticks(8),
        range: Hundredths(600),
        splash_radius: Hundredths::ZERO,
        projectile_speed: Hundredths::ZERO,
        homing: false,
        targets,
        instant_kill: false,
        ammo: 0,
        intercepts: false,
    };

    let armed = |id: &str, weapon: &str, locomotor: Locomotor| {
        unit(
            id,
            "vehicle",
            locomotor,
            vec![Trait::Armed {
                weapon: weapon.into(),
                turret: true,
                turret_rate: 3600,
            }],
        )
    };

    Rules::from_parts(
        vec![
            armed("tank", "cannon", Locomotor::Tracked),
            armed("flak", "flak_gun", Locomotor::Tracked),
            // An aircraft, which the locomotor puts in the air layer.
            unit("plane", "aircraft", Locomotor::Air, vec![]),
            unit("truck", "vehicle", Locomotor::Wheeled, vec![]),
        ],
        vec![
            weapon("cannon", vec![]),
            weapon("flak_gun", vec![redshift_data::traits::Layer::Air]),
        ],
        armour(),
        vec![],
    )
    .expect("rules")
}

/// Runs a duel and reports whether the victim was hurt.
fn victim_was_hit(sim: &mut Sim) -> bool {
    let victim = sim.units().ids()[1];
    for _ in 0..300 {
        sim.tick(&[]);
        if sim.units().get(victim).is_none_or(|u| u.health < 200) {
            return true;
        }
    }
    false
}

#[test]
fn a_ground_weapon_ignores_aircraft() {
    // The bug this closes: without a targeting layer, a tank locks onto an
    // aircraft and fires at it uselessly for the rest of the match while the
    // enemy walks past.
    let mut sim = duel(air_rules(), "tank", "plane");
    assert!(!victim_was_hit(&mut sim), "a tank shot down an aircraft");

    let tank = sim.units().ids()[0];
    assert!(
        sim.units().get(tank).unwrap().combat.target.is_none(),
        "the tank locked onto a target it cannot engage"
    );
}

#[test]
fn an_anti_air_weapon_hits_aircraft() {
    let mut sim = duel(air_rules(), "flak", "plane");
    assert!(
        victim_was_hit(&mut sim),
        "an anti-air gun could not hit an aircraft"
    );
}

#[test]
fn an_anti_air_weapon_ignores_ground_targets() {
    // The other half. A flak gun that could also shoot tanks would be strictly
    // better than a tank gun rather than a trade.
    let mut sim = duel(air_rules(), "flak", "truck");
    assert!(
        !victim_was_hit(&mut sim),
        "an anti-air gun shot a ground vehicle"
    );
}

#[test]
fn a_ground_weapon_still_hits_ground_targets() {
    // The default has to be ground-only, or every existing rules file changes
    // meaning silently.
    let mut sim = duel(air_rules(), "tank", "truck");
    assert!(victim_was_hit(&mut sim), "a tank could not shoot a truck");
}

#[test]
#[ignore = "gap: a unit has one weapon and one kind of action, not a set of them"]
fn a_unit_chooses_between_several_actions_by_what_it_is_aimed_at() {
    // Tanya shoots infantry and vehicles with a gun, and destroys *buildings*
    // with charges — a different action with different valid targets, chosen by
    // what she is pointed at. A tank fires at what it can reach and crushes
    // what it drives over, both at once.
    //
    // This is not the same gap as "two weapons". The model is currently "a unit
    // has *the* weapon and *the* target"; the reality is a set of actions, each
    // with its own targeting rule, and something deciding which applies. That
    // is closer to ADR 0006 than to a data field: capability is a list, not a
    // slot.
    panic!(
        "Armed is a single unique trait, and there is no notion of an action with its own valid targets"
    );
}

#[test]
fn a_unit_can_carry_an_anti_ground_and_an_anti_air_weapon() {
    // Closed. `Armed` stays unique — a unit has one primary weapon and the code
    // needs to know which — and `Secondary` is the other one. Targeting looks
    // for anything *either* can reach and then fires whichever does, rather
    // than asking the unit to choose a stance.
    //
    // Consulting only the primary was the first attempt, and left the secondary
    // resolved and never fired. Exercised in tests/weapons.rs.
    let apoc = unit(
        "apoc",
        "vehicle",
        Locomotor::Tracked,
        vec![
            Trait::Armed {
                weapon: "rifle".into(),
                turret: true,
                turret_rate: 3600,
            },
            Trait::Secondary {
                weapon: "rifle".into(),
                turret: true,
                turret_rate: 3600,
            },
        ],
    );
    let rules = rules_with(vec![apoc], vec![]);
    let sim = one_unit(rules, Map::new(20, 20), "apoc", Cell::new(5, 5));
    let kind = sim.rules().kind_of("apoc").unwrap();

    assert!(sim.combat().weapon(kind).is_some(), "no primary weapon");
    assert!(
        sim.combat().secondary(kind).is_some(),
        "no secondary weapon"
    );
}

#[test]
fn a_construction_vehicle_deploys_into_a_building() {
    // Both halves of what the original calls deploying are one mechanism here.
    // An MCV becoming a Construction Yard is plainly a transformation; a GI
    // "changing stance" into a static emplacement is also one, once you accept
    // that something which cannot move and shoots differently is not the same
    // unit with a flag set.
    //
    // So the deployed form is an ordinary entity, and its own `Deploys` points
    // back. Undeploying is deploying in the other direction — no second command
    // and no second code path. Exercised end to end in tests/deploy.rs.
    let mcv = unit(
        "mcv",
        "vehicle",
        Locomotor::Wheeled,
        vec![Trait::Deploys {
            into: "yard".into(),
        }],
    );
    let mut yard = unit(
        "yard",
        "structure",
        Locomotor::Wheeled,
        vec![
            Trait::Footprint {
                width: 3,
                height: 3,
            },
            Trait::Deploys { into: "mcv".into() },
        ],
    );
    // What "cannot move while deployed" *is*: no Mobile trait at all, rather
    // than a flag the movement code has to remember to check.
    yard.traits.retain(|t| !matches!(t, Trait::Mobile { .. }));

    let rules = rules_with(vec![mcv, yard], vec![]);
    let sim = one_unit(rules, Map::new(24, 24), "mcv", Cell::new(10, 10));
    let mcv_kind = sim.rules().kind_of("mcv").unwrap();
    let yard_kind = sim.rules().kind_of("yard").unwrap();

    assert_eq!(
        sim.stats().get(PlayerId(0), mcv_kind).deploys_into,
        Some(yard_kind),
        "the vehicle should know what it becomes"
    );
    assert_eq!(
        sim.stats().get(PlayerId(0), yard_kind).deploys_into,
        Some(mcv_kind),
        "and the building should know how to pack up again"
    );
    assert!(!sim.stats().get(PlayerId(0), yard_kind).mobile);
}

#[test]
fn infantry_garrison_a_civilian_building() {
    // Only a *neutral* building can be occupied, and an emptied one goes back
    // to neutral. That is what the original does — these are the civilian
    // buildings scattered across a map — and it is also what saves the engine
    // from having to remember who owned the building first.
    let mut house = unit(
        "house",
        "civilian",
        Locomotor::Foot,
        vec![Trait::Garrisonable {
            capacity: 2,
            categories: vec!["infantry".into()],
            weapon: "rifle".into(),
            evict_below_percent: 33,
        }],
    );
    house.traits.retain(|t| !matches!(t, Trait::Mobile { .. }));
    let gi = unit("gi", "infantry", Locomotor::Foot, vec![]);
    let commando = unit("commando", "commando", Locomotor::Foot, vec![]);

    let rules = rules_with(vec![house, gi, commando], vec![]);
    let sim = one_unit(rules, Map::new(24, 24), "gi", Cell::new(5, 5));
    let kind = sim.rules().kind_of("house").unwrap();

    // "Only basic infantry garrison" is one word in a category list — which is
    // the whole of ADR 0006. A commando is refused for saying it is a commando,
    // not for being a commando.
    let allows = |category: &str| {
        sim.rules().entity(kind).traits.iter().any(|t| match t {
            Trait::Garrisonable { categories, .. } => categories.iter().any(|c| c == category),
            _ => false,
        })
    };
    assert!(allows("infantry"));
    assert!(!allows("commando"));
}

#[test]
fn a_unit_on_high_ground_sees_further() {
    // Sight and reach take the same bonus, deliberately. If they diverged, a
    // unit would either shoot into fog or see things it could not touch.
    let mut map = Map::new(16, 16);
    map.set_elevation(Cell::new(6, 6), 3);
    assert!(map.elevation_bonus(Cell::new(6, 6)) > map.elevation_bonus(Cell::new(0, 0)));
}
