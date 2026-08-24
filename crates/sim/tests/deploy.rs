//! Deploying: a unit becoming something else.
//!
//! The original has two things that look different and are not. An MCV becomes
//! a Construction Yard — plainly a transformation. A GI "changes stance" into a
//! static emplacement with a better gun — which is *also* a transformation,
//! once you accept that something which cannot move and shoots differently is
//! not the same unit with a flag set.
//!
//! Modelling both as "become another entity" is what makes this one mechanism
//! rather than two, and it means a stance is expressible in data alone. Nothing
//! in the simulation knows what an MCV is.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn armour() -> ArmourTable {
    ron::from_str(r#"( classes: ["none"], table: { "shot": { "none": 100 } } )"#).unwrap()
}

fn gun(id: &str, range: i32) -> WeaponDef {
    hitting(id, range, 10, 10)
}

/// A weapon with a stated damage and reload, for arranging an exact amount of
/// harm. Used instead of reaching into the simulation and writing a health
/// value: there is no mutable accessor on `Sim`, deliberately, and adding one
/// so a test could skip the damage path would be exactly the kind of hole that
/// later gets used in earnest.
fn hitting(id: &str, range: i32, damage: u32, reload: u32) -> WeaponDef {
    WeaponDef {
        id: id.into(),
        damage,
        warhead: "shot".into(),
        reload: Ticks(reload),
        range: Hundredths(range),
        splash_radius: Hundredths::ZERO,
        projectile_speed: Hundredths::ZERO,
        homing: false,
        targets: vec![],
        instant_kill: false,
        ammo: 0,
        intercepts: false,
    }
}

/// The pair the whole feature exists for: a mobile thing and the building it
/// becomes. Each points at the other, so one command covers both directions.
fn mcv_rules() -> Rules {
    let mcv = EntityDef {
        id: "mcv".into(),
        name_key: "unit.mcv".into(),
        side: None,
        category: "vehicle".into(),
        traits: vec![
            Trait::Health {
                max: 600,
                armour: "none".into(),
            },
            Trait::Mobile {
                speed: Hundredths(200),
                turn_rate: 3600,
                locomotor: Locomotor::Wheeled,
                surfaces: None,
                size: None,
                layer: None,
            },
            Trait::Vision {
                range: Hundredths(600),
            },
            Trait::Deploys {
                into: "yard".into(),
            },
        ],
    };
    let yard = EntityDef {
        id: "yard".into(),
        name_key: "structure.yard".into(),
        side: None,
        category: "structure".into(),
        traits: vec![
            Trait::Health {
                max: 3_000,
                armour: "none".into(),
            },
            Trait::Vision {
                range: Hundredths(600),
            },
            Trait::Footprint {
                width: 3,
                height: 3,
            },
            Trait::Produces {
                categories: vec!["structure".into()],
            },
            // Pointing back is the whole of undeploying. There is no second
            // command and no second code path.
            Trait::Deploys { into: "mcv".into() },
        ],
    };
    Rules::from_parts(vec![mcv, yard], vec![], armour(), Vec::new()).expect("valid rules")
}

/// The MCV pair plus a single-shot attacker, for arranging an exact wound.
///
/// One enormous reload, so it fires once and then never again — which is what
/// makes the resulting health figure something a test can assert on rather
/// than a race against the next shot.
fn mcv_rules_with_sniper(damage: u32) -> Rules {
    let mut entities: Vec<EntityDef> = mcv_rules().entities().map(|(_, e)| e.clone()).collect();
    entities.push(EntityDef {
        id: "sniper".into(),
        name_key: "unit.sniper".into(),
        side: None,
        category: "infantry".into(),
        traits: vec![
            Trait::Health {
                max: 100,
                armour: "none".into(),
            },
            Trait::Vision {
                range: Hundredths(900),
            },
            Trait::Armed {
                weapon: "rifle".into(),
                turret: true,
                turret_rate: 3600,
            },
        ],
    });
    Rules::from_parts(
        entities,
        vec![hitting("rifle", 500, damage, 100_000)],
        armour(),
        Vec::new(),
    )
    .expect("valid rules")
}

/// An MCV wounded by exactly `damage`, and its id.
fn wounded_mcv(damage: u32) -> (Sim, redshift_sim::EntityId) {
    let rules = mcv_rules_with_sniper(damage);
    let mcv = rules.kind_of("mcv").unwrap();
    let sniper = rules.kind_of("sniper").unwrap();
    let mut sim = Sim::new(MatchSetup {
        seed: 0x_DEB2,
        map: Map::new(32, 32),
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
                kind: mcv,
                pos: Cell::new(10, 10).centre(),
            },
            Spawn {
                owner: PlayerId(1),
                kind: sniper,
                pos: Cell::new(13, 10).centre(),
            },
        ],
        rules,
    });
    let id = sim.units().ids()[0];
    let full = sim.unit(id).unwrap().health;
    for _ in 0..200 {
        sim.tick(&[]);
        if sim.unit(id).is_some_and(|u| u.health < full) {
            break;
        }
    }
    assert_eq!(
        sim.unit(id).expect("the mcv survives one shot").health,
        full - damage,
        "the sniper should have landed exactly one shot"
    );
    (sim, id)
}

/// The other half of the feature: same footprint, different capability. A GI
/// that plants itself trades movement for reach.
fn gi_rules() -> Rules {
    let gi = EntityDef {
        id: "gi".into(),
        name_key: "unit.gi".into(),
        side: None,
        category: "infantry".into(),
        traits: vec![
            Trait::Health {
                max: 200,
                armour: "none".into(),
            },
            Trait::Mobile {
                speed: Hundredths(300),
                turn_rate: 3600,
                locomotor: Locomotor::Foot,
                surfaces: None,
                size: None,
                layer: None,
            },
            Trait::Vision {
                range: Hundredths(900),
            },
            Trait::Armed {
                weapon: "carbine".into(),
                turret: true,
                turret_rate: 3600,
            },
            Trait::Deploys {
                into: "gi_dug_in".into(),
            },
        ],
    };
    let dug_in = EntityDef {
        id: "gi_dug_in".into(),
        name_key: "unit.gi.deployed".into(),
        side: None,
        category: "infantry".into(),
        traits: vec![
            Trait::Health {
                max: 200,
                armour: "none".into(),
            },
            // No Mobile trait at all. That *is* "cannot move while deployed" —
            // not a flag the movement code has to remember to check.
            Trait::Vision {
                range: Hundredths(900),
            },
            Trait::Armed {
                weapon: "machine_gun".into(),
                turret: true,
                turret_rate: 3600,
            },
            Trait::Deploys { into: "gi".into() },
        ],
    };
    Rules::from_parts(
        vec![gi, dug_in],
        vec![gun("carbine", 400), gun("machine_gun", 800)],
        armour(),
        Vec::new(),
    )
    .expect("valid rules")
}

fn one(rules: Rules, map: Map, id: &str, at: Cell) -> Sim {
    let kind = rules.kind_of(id).unwrap_or_else(|| panic!("no {id:?}"));
    Sim::new(MatchSetup {
        seed: 0x_DEB0,
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

fn deploy(sim: &mut Sim, units: Vec<redshift_sim::EntityId>) {
    sim.tick(&[Command::new(PlayerId(0), 0, CommandKind::Deploy { units })]);
}

// -- Becoming a structure ---------------------------------------------------

#[test]
fn a_construction_vehicle_deploys_into_a_building() {
    let mut sim = one(mcv_rules(), Map::new(32, 32), "mcv", Cell::new(10, 10));
    let id = sim.units().ids()[0];
    let yard = sim.rules().kind_of("yard").unwrap();

    deploy(&mut sim, vec![id]);

    let unit = sim.unit(id).expect("deploying is not a death");
    assert_eq!(unit.kind, yard, "the vehicle should have become the yard");
    assert!(
        !sim.stats().get(PlayerId(0), unit.kind).mobile,
        "a construction yard must not be able to drive away"
    );
}

#[test]
fn the_deployed_building_claims_its_ground() {
    // A 3×3 yard where a 1×1 vehicle stood. If the larger footprint is not
    // claimed, units walk straight through the building.
    let mut sim = one(mcv_rules(), Map::new(32, 32), "mcv", Cell::new(10, 10));
    let id = sim.units().ids()[0];
    assert!(!sim.map().is_blocked(Cell::new(11, 11)));

    deploy(&mut sim, vec![id]);

    for cell in [Cell::new(9, 9), Cell::new(10, 10), Cell::new(11, 11)] {
        assert!(
            sim.map().is_blocked(cell),
            "the yard should occupy {cell:?}"
        );
    }
}

#[test]
fn undeploying_gives_the_ground_back() {
    let mut sim = one(mcv_rules(), Map::new(32, 32), "mcv", Cell::new(10, 10));
    let id = sim.units().ids()[0];

    deploy(&mut sim, vec![id]);
    deploy(&mut sim, vec![id]);

    let mcv = sim.rules().kind_of("mcv").unwrap();
    assert_eq!(sim.unit(id).unwrap().kind, mcv, "it should have packed up");
    assert!(
        !sim.map().is_blocked(Cell::new(11, 11)),
        "a yard that packed up must not leave its foundation behind"
    );
}

#[test]
fn a_vehicle_cannot_deploy_where_the_building_would_not_fit() {
    // Hard against the map edge: a 3×3 footprint centred here runs off the map.
    let mut sim = one(mcv_rules(), Map::new(32, 32), "mcv", Cell::new(0, 0));
    let id = sim.units().ids()[0];
    let mcv = sim.rules().kind_of("mcv").unwrap();

    deploy(&mut sim, vec![id]);

    assert_eq!(
        sim.unit(id).unwrap().kind,
        mcv,
        "it should still be a vehicle"
    );
}

#[test]
fn a_refused_deploy_leaves_the_ground_exactly_as_it_was() {
    // The check has to drop the unit's own claim before testing the new
    // footprint, or a 1×1 unit blocks its own 3×3 building. Dropping it and
    // then failing must not leak.
    let mut map = Map::new(32, 32);
    map.set_blocked(Cell::new(11, 9), 1, 1, true);
    let mut sim = one(mcv_rules(), map, "mcv", Cell::new(10, 10));
    let id = sim.units().ids()[0];

    deploy(&mut sim, vec![id]);

    let mcv = sim.rules().kind_of("mcv").unwrap();
    assert_eq!(sim.unit(id).unwrap().kind, mcv, "the ground was occupied");
    assert!(
        sim.map().is_blocked(Cell::new(11, 9)),
        "the pre-existing obstruction must survive a refused deploy"
    );
}

#[test]
fn deploying_does_not_need_an_existing_base_nearby() {
    // The one placement rule deploying must *not* obey. An MCV is how a player
    // gets their first building; requiring a structure within the build radius
    // would make the first one impossible.
    let mut sim = one(mcv_rules(), Map::new(64, 64), "mcv", Cell::new(40, 40));
    let id = sim.units().ids()[0];
    let yard = sim.rules().kind_of("yard").unwrap();

    deploy(&mut sim, vec![id]);

    assert_eq!(sim.unit(id).unwrap().kind, yard);
}

#[test]
fn a_building_cannot_be_placed_across_a_cliff_edge() {
    // Not deploy-specific, but the same check: a foundation is level. Without
    // it a player claims high ground by building onto it rather than taking it.
    let mut map = Map::new(32, 32);
    map.raise_rect(Cell::new(11, 9), Cell::new(11, 11), 2);
    let mut sim = one(mcv_rules(), map, "mcv", Cell::new(10, 10));
    let id = sim.units().ids()[0];
    let mcv = sim.rules().kind_of("mcv").unwrap();

    deploy(&mut sim, vec![id]);

    assert_eq!(
        sim.unit(id).unwrap().kind,
        mcv,
        "the yard would have straddled a cliff edge"
    );
}

// -- Changing stance --------------------------------------------------------

#[test]
fn deploying_infantry_trades_movement_for_reach() {
    let mut sim = one(gi_rules(), Map::new(32, 32), "gi", Cell::new(10, 10));
    let id = sim.units().ids()[0];
    let before = sim.unit(id).unwrap().kind;
    let walking_range = sim.combat().weapon(before).unwrap().range;
    assert!(sim.stats().get(PlayerId(0), before).mobile);

    deploy(&mut sim, vec![id]);

    let after = sim.unit(id).unwrap().kind;
    assert!(
        !sim.stats().get(PlayerId(0), after).mobile,
        "a dug-in soldier should not walk away"
    );
    assert!(
        sim.combat().weapon(after).unwrap().range > walking_range,
        "digging in should buy something, or nobody would do it"
    );
}

#[test]
fn a_stance_change_keeps_the_same_footprint_and_needs_no_room() {
    // A GI digs in where it stands. If this went through the same "is there
    // space for a building" check as an MCV, infantry could not deploy in a
    // crowd, which is precisely when a player wants to.
    let mut map = Map::new(32, 32);
    map.set_blocked(Cell::new(11, 10), 1, 1, true);
    let mut sim = one(gi_rules(), map, "gi", Cell::new(10, 10));
    let id = sim.units().ids()[0];

    deploy(&mut sim, vec![id]);

    let dug_in = sim.rules().kind_of("gi_dug_in").unwrap();
    assert_eq!(sim.unit(id).unwrap().kind, dug_in);
}

// -- What carries across ----------------------------------------------------

#[test]
fn damage_carries_across_as_a_proportion() {
    // The two forms have very different maximums — 600 and 3000. Copying the
    // raw number would either heal the unit for free or leave a healthy one
    // looking nearly dead.
    let (mut sim, id) = wounded_mcv(300); // exactly half of 600

    deploy(&mut sim, vec![id]);

    let unit = sim.unit(id).unwrap();
    let max = sim.stats().get(PlayerId(0), unit.kind).max_health;
    assert_eq!(max, 3_000);
    assert_eq!(unit.health, 1_500, "half a vehicle should be half a yard");
}

#[test]
fn a_nearly_dead_unit_survives_deploying() {
    // Rounding down could kill it. A player's own command should never be the
    // thing that finishes off their unit.
    let (mut sim, id) = wounded_mcv(599); // one point left of six hundred

    deploy(&mut sim, vec![id]);
    sim.tick(&[]);

    let unit = sim.unit(id).expect("deploying must not kill");
    assert!(unit.is_alive() && unit.health > 0);
}

#[test]
fn the_unit_keeps_its_identity() {
    // Removing and re-inserting would have been easier to write. It would also
    // empty the player's selection the instant they deployed, and orphan every
    // shot already in flight.
    let mut sim = one(mcv_rules(), Map::new(32, 32), "mcv", Cell::new(10, 10));
    let id = sim.units().ids()[0];

    deploy(&mut sim, vec![id]);

    assert_eq!(sim.units().ids(), vec![id], "the id should be the same one");
}

#[test]
fn a_veteran_stays_a_veteran_through_a_deploy() {
    // Kills are the unit's history, not machinery belonging to a form.
    // Killing the sniper is how the MCV gets a kill on its record, which is a
    // roundabout way to arrange it — and the only way that goes through the
    // code the assertion is actually about.
    let (mut sim, id) = wounded_mcv(1);
    let before = sim.unit(id).unwrap().kills;

    deploy(&mut sim, vec![id]);
    deploy(&mut sim, vec![id]);

    assert_eq!(sim.unit(id).unwrap().kills, before);
}

// -- Refusals ---------------------------------------------------------------

#[test]
fn a_unit_with_no_deployed_form_ignores_the_command() {
    let mut rules_source = gi_rules();
    // Strip the trait, leaving an otherwise ordinary soldier.
    let stripped: Vec<EntityDef> = rules_source
        .entities()
        .map(|(_, e)| {
            let mut e = e.clone();
            e.traits.retain(|t| !matches!(t, Trait::Deploys { .. }));
            e
        })
        .collect();
    rules_source = Rules::from_parts(
        stripped,
        vec![gun("carbine", 400), gun("machine_gun", 800)],
        armour(),
        Vec::new(),
    )
    .expect("valid rules");

    let mut sim = one(rules_source, Map::new(32, 32), "gi", Cell::new(10, 10));
    let id = sim.units().ids()[0];
    let before = sim.unit(id).unwrap().kind;

    deploy(&mut sim, vec![id]);

    assert_eq!(sim.unit(id).unwrap().kind, before);
}

#[test]
fn a_player_cannot_deploy_someone_elses_unit() {
    let rules = mcv_rules();
    let kind = rules.kind_of("mcv").unwrap();
    let mut sim = Sim::new(MatchSetup {
        seed: 0x_DEB1,
        map: Map::new(32, 32),
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
        spawns: vec![Spawn {
            owner: PlayerId(1),
            kind,
            pos: Cell::new(10, 10).centre(),
        }],
        rules,
    });
    let theirs = sim.units().ids()[0];

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Deploy {
            units: vec![theirs],
        },
    )]);

    assert_eq!(sim.unit(theirs).unwrap().kind, kind);
}

#[test]
fn deploying_is_deterministic() {
    let run = || {
        let mut sim = one(mcv_rules(), Map::new(32, 32), "mcv", Cell::new(10, 10));
        let id = sim.units().ids()[0];
        for tick in 0..40 {
            if tick % 7 == 0 {
                deploy(&mut sim, vec![id]);
            } else {
                sim.tick(&[]);
            }
        }
        sim.state_hash()
    };
    assert_eq!(run(), run());
}
