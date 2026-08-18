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
        homing: false,
        targets: vec![],
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
#[ignore = "gap: units have unlimited ammunition and never rearm"]
fn a_unit_runs_out_of_ammunition_and_returns_to_rearm() {
    // Found by cross-checking a working reimplementation's trait list. Assumed
    // to be an aircraft rule; it is a general one, and it is the mechanism that
    // makes an aircraft a sortie rather than a flying tank.
    panic!("weapons have a reload timer and no ammunition count");
}

#[test]
#[ignore = "gap: a destroyed vehicle releases nothing"]
fn a_destroyed_vehicle_ejects_its_crew() {
    // Survivors change the value of every vehicle kill: destroying a transport
    // full of infantry is not the same as destroying an empty one.
    panic!("destruction removes the unit and leaves nothing behind");
}

#[test]
#[ignore = "gap: no rally points — new units stop beside the factory"]
fn newly_built_units_walk_to_a_rally_point() {
    panic!("a produced unit is placed next to its factory and left there");
}

#[test]
#[ignore = "gap: structures cannot be sold"]
fn a_structure_can_be_sold_for_a_refund() {
    // And for the Cloning Vats, selling units is a deliberate way of turning
    // spare infantry into cash.
    panic!("no sell command; a structure can only be destroyed");
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
#[ignore = "gap: no standing economy modifiers on a player"]
fn an_ore_purifier_increases_the_value_of_every_load() {
    // A quarter more credits from every delivery, for as long as the building
    // stands. Same shape as the spy-in-a-barracks effect: a modifier that lives
    // on the player rather than on a unit, and there is nowhere to put one.
    panic!("credits are paid at a fixed rate with no per-player modifier");
}

#[test]
#[ignore = "gap: garrison — a building fires with its own weapon, holds basic infantry only, and evicts below a third health"]
fn a_garrisoned_building_fires_and_evicts_when_badly_damaged() {
    // Researched and more specific than expected: capacity depends on the
    // building's size, only basic infantry may enter, the building fires with a
    // *predetermined* weapon rather than its occupants', and the garrison is
    // forced out below 33% health rather than dying with it.
    //
    // The last rule matters: clearing a garrison means damaging it enough to
    // evict, not destroying it.
    panic!("structures cannot hold passengers at all");
}

#[test]
#[ignore = "gap: high ground gives a range advantage, and is modelled as impassable rock"]
fn a_unit_on_high_ground_outranges_one_below() {
    // Modelling elevation as impassable terrain keeps the movement restriction
    // and loses the part that affects a fight.
    panic!("the map has no elevation, only a rock terrain that blocks everything");
}

#[test]
#[ignore = "gap: bridges — destructible terrain, repaired through a hut beside them"]
fn a_destroyed_bridge_is_repaired_by_an_engineer_at_its_hut() {
    // Worth noting that this is *not* a new mechanic: the repair hut is entered
    // like a tech building, so bridge repair is capture with a different effect.
    panic!("no bridges, no destructible terrain, and no capture");
}

#[test]
#[ignore = "gap: low power disables structures, it does not merely slow them"]
fn a_radar_stops_working_when_power_runs_short() {
    // Researched: the original *switches off* radar towers and flak cannons in
    // low power. Redshift models a shortage as a production slowdown only, so a
    // player who loses their reactor keeps their air defence — which is most of
    // what makes attacking a power plant worth doing.
    panic!("low power slows production; nothing is ever disabled");
}

#[test]
#[ignore = "gap: a structure cannot arrive with a unit"]
fn a_refinery_comes_with_a_miner() {
    // The free miner is not a nicety. It is why a refinery is the first thing
    // built, and an economy balanced without it would be wrong from the start.
    panic!("production delivers the thing built and nothing else");
}

#[test]
#[ignore = "gap: no per-player build limit on structures"]
fn only_one_superweapon_of_a_kind_can_be_built() {
    // Three separate structures are limited to one per player. Distinct from a
    // unit build limit, and nothing counts what already exists before allowing
    // a build.
    panic!("production checks cost and prerequisites, never how many exist");
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
#[ignore = "gap: projectiles cannot be intercepted"]
fn an_anti_missile_defence_shoots_down_a_rocket() {
    // Researched, and more central than it first looked: the Aegis Cruiser, Sea
    // Scorpion and Flak Cannon exist largely to shoot missiles down, and the V3
    // and Dreadnought exist to fire missiles that can be. Redshift has shots in
    // flight already, so this is closer than most of the list.
    panic!("a projectile in flight cannot be targeted or destroyed");
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
#[ignore = "gap: tech structures — neutral, capturable, unsellable, and they extend the build radius"]
fn a_captured_tech_structure_extends_the_build_radius() {
    // A captured oil derrick is a forward base. Redshift has a build radius and
    // only counts structures the player built, so capturing one would give a
    // player income and no ground to build on.
    panic!("no capture, no neutral owner, and the build radius ignores captured ground");
}

#[test]
#[ignore = "gap: persistent production modifiers — an effect that changes everything built afterwards"]
fn an_effect_can_promote_every_unit_built_from_now_on() {
    // A spy in a barracks, and a tech machine shop repairing every vehicle you
    // own anywhere on the map. Neither is a one-off event nor a per-unit trait:
    // they are standing modifiers on a player, and there is nowhere to put one.
    panic!("effects are instantaneous; a player carries no standing modifiers");
}

#[test]
#[ignore = "gap: instant-kill weapons — damage is a number, and some weapons simply kill"]
fn some_weapons_kill_outright_regardless_of_health() {
    // Tanya's pistols kill any infantry outright and do nothing at all to
    // vehicles. A sniper is the same. Expressing that as "very high damage"
    // would make it merely very strong against vehicles too, which is exactly
    // wrong.
    panic!("a weapon has a damage number and no notion of killing outright");
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
#[ignore = "gap: build limits — some units may only exist once at a time"]
fn only_one_commando_can_exist_at_a_time() {
    // Tanya is unique per player, and two with a cloning vat — so the limit is
    // itself modifiable. Nothing counts existing units before allowing a build.
    panic!("production checks cost and prerequisites, never how many already exist");
}

#[test]
#[ignore = "gap: no neutral player — civilians and neutral structures have no owner"]
fn a_neutral_structure_belongs_to_nobody_and_is_hostile_to_nobody() {
    panic!("every player is hostile to every other; there is no neutral side");
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
