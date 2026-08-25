//! What a spy gets for reaching each kind of building.
//!
//! Verified, and considerably richer than "infiltration effects" suggested.
//! Five rows, and the finding worth writing down is that they are genuinely
//! five different mechanisms rather than one with a parameter — a persistent
//! production modifier, a timed sabotage of the power grid, a one-off theft,
//! and an addition to the tech tree.
//!
//! The effect is declared on the **building**, not on the spy. Infiltration is
//! not one effect aimed at a target; it is a table keyed on what was entered,
//! which is why an Allied lab yields something different from a Soviet one
//! without any code knowing either exists.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{InfiltrationEffect, Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::EntityId;
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn armour() -> ArmourTable {
    ron::from_str(r#"( classes: ["none"], table: { "shot": { "none": 100 } } )"#).unwrap()
}

fn spy(consumed: bool) -> EntityDef {
    EntityDef {
        id: if consumed { "spy" } else { "ghost" }.into(),
        name_key: "unit.spy".into(),
        side: None,
        category: "infantry".into(),
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
                size: None,
                layer: None,
            },
            Trait::Vision {
                range: Hundredths(900),
            },
            Trait::Infiltrator { consumed },
        ],
    }
}

/// A building carrying one row of the table.
fn target(id: &str, effect: Option<InfiltrationEffect>) -> EntityDef {
    let mut traits = vec![
        Trait::Health {
            max: 1_000,
            armour: "none".into(),
        },
        Trait::Vision {
            range: Hundredths(400),
        },
        Trait::Footprint {
            width: 3,
            height: 3,
        },
    ];
    if let Some(effect) = effect {
        traits.push(Trait::Infiltrated { effect });
    }
    EntityDef {
        id: id.into(),
        name_key: format!("structure.{id}"),
        side: None,
        category: "structure".into(),
        traits,
    }
}

/// A plant, so there is power to cut.
fn plant() -> EntityDef {
    EntityDef {
        traits: vec![
            Trait::Health {
                max: 400,
                armour: "none".into(),
            },
            Trait::Vision {
                range: Hundredths(300),
            },
            Trait::Footprint {
                width: 2,
                height: 2,
            },
            Trait::PowerSupply { output: 200 },
        ],
        ..target("plant", None)
    }
}

/// A gun that only works with power, so a blackout is visible from outside.
fn defence() -> EntityDef {
    EntityDef {
        traits: vec![
            Trait::Health {
                max: 400,
                armour: "none".into(),
            },
            Trait::Vision {
                range: Hundredths(800),
            },
            Trait::PowerDraw {
                amount: 100,
                works_unpowered: false,
            },
            Trait::Armed {
                weapon: "cannon".into(),
                turret: true,
                turret_rate: 3600,
            },
        ],
        ..target("defence", None)
    }
}

/// Something for the defence gun to shoot at for a long time, so "is it still
/// firing?" can be asked repeatedly.
fn dummy() -> EntityDef {
    EntityDef {
        id: "dummy".into(),
        name_key: "unit.dummy".into(),
        side: None,
        category: "vehicle".into(),
        traits: vec![
            Trait::Health {
                max: 100_000,
                armour: "none".into(),
            },
            Trait::Vision {
                range: Hundredths(300),
            },
        ],
    }
}

/// Something to build, gated behind a prerequisite the player cannot own.
fn stolen_commando() -> EntityDef {
    EntityDef {
        id: "chrono_commando".into(),
        name_key: "unit.chrono_commando".into(),
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
                range: Hundredths(700),
            },
            // The victim's lab, which the infiltrator will never own. That is
            // the whole reason it is worth stealing.
            Trait::Buildable {
                cost: 2_000,
                build_time: Ticks(10),
                prerequisites: vec!["lab".into()],
                produced_by: "barracks".into(),
            },
        ],
    }
}

fn barracks() -> EntityDef {
    EntityDef {
        traits: vec![
            Trait::Health {
                max: 500,
                armour: "none".into(),
            },
            Trait::Vision {
                range: Hundredths(400),
            },
            Trait::Footprint {
                width: 3,
                height: 3,
            },
            Trait::Produces {
                categories: vec!["infantry".into()],
            },
        ],
        ..target("barracks", None)
    }
}

fn cannon() -> WeaponDef {
    WeaponDef {
        id: "cannon".into(),
        damage: 40,
        warhead: "shot".into(),
        reload: Ticks(10),
        range: Hundredths(700),
        splash_radius: Hundredths::ZERO,
        projectile_speed: Hundredths::ZERO,
        homing: false,
        targets: vec![],
        instant_kill: false,
        ammo: 0,
        intercepts: false,
        target_categories: vec![],
        heals: false,
    }
}

fn rules(mut entities: Vec<EntityDef>) -> Rules {
    entities.push(spy(true));
    entities.push(spy(false));
    Rules::from_parts(entities, vec![cannon()], armour(), Vec::new()).expect("valid rules")
}

fn scenario(entities: Vec<EntityDef>, spawns: Vec<(u8, &str, i32, i32)>) -> Sim {
    let rules = rules(entities);
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
    Sim::new(MatchSetup {
        seed: 0x_5919,
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
        spawns,
        rules,
    })
}

fn send_in(sim: &mut Sim, owner: u8, unit: EntityId, building: EntityId) {
    sim.tick(&[Command::new(
        PlayerId(owner),
        0,
        CommandKind::EnterBuilding {
            units: vec![unit],
            target: building,
        },
    )]);
    for _ in 0..800 {
        sim.tick(&[]);
        if sim.unit(unit).is_none_or(|u| u.order.is_idle()) {
            return;
        }
    }
}

// -- The rows ---------------------------------------------------------------

#[test]
fn a_spy_in_a_barracks_promotes_everything_of_that_category() {
    // Persistent, not an event: everything built from then on arrives promoted.
    // And keyed on a category, because the original keys it on one — a spy in a
    // barracks does not also promote your tanks.
    let mut sim = scenario(
        vec![target(
            "enemy_barracks",
            Some(InfiltrationEffect::Promotes {
                category: "infantry".into(),
            }),
        )],
        vec![(0, "spy", 15, 10), (1, "enemy_barracks", 20, 10)],
    );
    let ids = sim.units().ids();
    let (agent, building) = (ids[0], ids[1]);
    assert!(!sim.boons().veteran_production(PlayerId(0), "infantry"));

    send_in(&mut sim, 0, agent, building);
    sim.tick(&[]);

    assert!(sim.boons().veteran_production(PlayerId(0), "infantry"));
    assert!(
        !sim.boons().veteran_production(PlayerId(0), "vehicle"),
        "a barracks should not promote tanks"
    );
}

#[test]
fn the_promotion_outlives_the_spy_and_the_building() {
    // Boons are rebuilt from scratch every tick from what a player owns, which
    // is right for a machine shop and wrong for this: the spy is consumed and
    // the barracks is still the victim's, so there is no standing source to
    // rebuild from.
    let mut sim = scenario(
        vec![target(
            "enemy_barracks",
            Some(InfiltrationEffect::Promotes {
                category: "infantry".into(),
            }),
        )],
        vec![(0, "spy", 15, 10), (1, "enemy_barracks", 20, 10)],
    );
    let ids = sim.units().ids();
    send_in(&mut sim, 0, ids[0], ids[1]);

    for _ in 0..500 {
        sim.tick(&[]);
    }

    assert!(sim.unit(ids[0]).is_none(), "the spy should be gone");
    assert!(sim.boons().veteran_production(PlayerId(0), "infantry"));
}

#[test]
fn a_spy_in_a_power_plant_blacks_the_victim_out() {
    // Cutting power is worth doing because of what stops working. The defence
    // gun below has plenty of supply and still falls silent.
    let mut sim = scenario(
        vec![
            plant(),
            defence(),
            dummy(),
            target(
                "enemy_plant",
                Some(InfiltrationEffect::Blackout { ticks: 400 }),
            ),
        ],
        vec![
            (0, "spy", 15, 20),
            (1, "enemy_plant", 20, 20),
            (1, "plant", 30, 30),
            (1, "defence", 30, 20),
            (0, "dummy", 27, 20),
        ],
    );
    let ids = sim.units().ids();
    let (agent, enemy_plant, victim) = (ids[0], ids[1], ids[4]);

    // The gun works before the sabotage.
    let start = sim.unit(victim).unwrap().health;
    for _ in 0..30 {
        sim.tick(&[]);
    }
    assert!(
        sim.unit(victim).unwrap().health < start,
        "the defence should have been shooting to begin with"
    );

    send_in(&mut sim, 0, agent, enemy_plant);
    let after_sabotage = sim.unit(victim).unwrap().health;
    for _ in 0..100 {
        sim.tick(&[]);
    }

    assert_eq!(
        sim.unit(victim).unwrap().health,
        after_sabotage,
        "the defence kept firing through a blackout"
    );
}

#[test]
fn a_blackout_wears_off() {
    // A permanent one would end the match rather than open a window in it.
    let mut sim = scenario(
        vec![
            plant(),
            defence(),
            dummy(),
            target(
                "enemy_plant",
                Some(InfiltrationEffect::Blackout { ticks: 60 }),
            ),
        ],
        vec![
            (0, "spy", 15, 20),
            (1, "enemy_plant", 20, 20),
            (1, "plant", 30, 30),
            (1, "defence", 30, 20),
            (0, "dummy", 27, 20),
        ],
    );
    let ids = sim.units().ids();
    send_in(&mut sim, 0, ids[0], ids[1]);
    for _ in 0..200 {
        sim.tick(&[]);
    }

    let recovered = sim.unit(ids[4]).map(|v| v.health).unwrap_or(0);
    for _ in 0..40 {
        sim.tick(&[]);
    }
    assert!(
        sim.unit(ids[4]).map(|v| v.health).unwrap_or(0) < recovered,
        "the defence should be firing again once the blackout ends"
    );
}

#[test]
fn a_spy_in_a_refinery_steals_a_share_of_the_funds() {
    let mut sim = scenario(
        vec![target(
            "enemy_refinery",
            Some(InfiltrationEffect::StealsFunds { percent: 20 }),
        )],
        vec![(0, "spy", 15, 10), (1, "enemy_refinery", 20, 10)],
    );
    let ids = sim.units().ids();
    let victim_before = sim.treasury().credits(PlayerId(1));
    let thief_before = sim.treasury().credits(PlayerId(0));

    send_in(&mut sim, 0, ids[0], ids[1]);
    sim.tick(&[]);

    let taken = victim_before - sim.treasury().credits(PlayerId(1));
    assert_eq!(taken, victim_before / 5, "a fifth should have moved");
    assert_eq!(
        sim.treasury().credits(PlayerId(0)) - thief_before,
        taken,
        "what one player lost the other should have gained"
    );
}

#[test]
fn a_spy_in_a_lab_unlocks_a_unit_built_from_the_victims_technology() {
    // The interesting row. What you get depends on whose lab it was, and its
    // prerequisite is a building the infiltrator can never own — which is
    // exactly why it is worth stealing.
    let mut sim = scenario(
        vec![
            barracks(),
            stolen_commando(),
            EntityDef {
                traits: vec![
                    Trait::Health {
                        max: 1_000,
                        armour: "none".into(),
                    },
                    Trait::Vision {
                        range: Hundredths(400),
                    },
                    Trait::Footprint {
                        width: 3,
                        height: 3,
                    },
                    Trait::Infiltrated {
                        effect: InfiltrationEffect::Unlocks {
                            unit: "chrono_commando".into(),
                        },
                    },
                ],
                ..target("lab", None)
            },
        ],
        vec![
            (0, "spy", 15, 10),
            (1, "lab", 20, 10),
            (0, "barracks", 6, 6),
        ],
    );
    let ids = sim.units().ids();
    let commando = sim.rules().kind_of("chrono_commando").unwrap();
    assert!(
        !sim.prerequisites_met(PlayerId(0), commando),
        "it should be out of reach to begin with"
    );

    send_in(&mut sim, 0, ids[0], ids[1]);
    sim.tick(&[]);

    assert!(
        sim.prerequisites_met(PlayerId(0), commando),
        "the spy should have bought a unit its owner's tech tree cannot reach"
    );
    // Nothing is asserted about the victim: they own the lab, so they could
    // always build it. The theft is that somebody else now can too.
}

// -- Refusals ---------------------------------------------------------------

#[test]
fn a_spy_that_reaches_a_building_with_nothing_to_steal_is_wasted() {
    // Exactly as in the original — and it must not fall through to capturing
    // the building instead.
    let mut sim = scenario(
        vec![target("dull", None)],
        vec![(0, "spy", 15, 10), (1, "dull", 20, 10)],
    );
    let ids = sim.units().ids();
    send_in(&mut sim, 0, ids[0], ids[1]);
    sim.tick(&[]);

    assert_eq!(
        sim.unit(ids[1]).unwrap().owner,
        PlayerId(1),
        "a spy captured a building"
    );
}

#[test]
fn a_spy_cannot_infiltrate_its_own_side() {
    let mut sim = scenario(
        vec![target(
            "own_barracks",
            Some(InfiltrationEffect::Promotes {
                category: "infantry".into(),
            }),
        )],
        vec![(0, "spy", 15, 10), (0, "own_barracks", 20, 10)],
    );
    let ids = sim.units().ids();
    send_in(&mut sim, 0, ids[0], ids[1]);
    sim.tick(&[]);

    assert!(!sim.boons().veteran_production(PlayerId(0), "infantry"));
}

#[test]
fn an_unconsumed_infiltrator_walks_back_out() {
    let mut sim = scenario(
        vec![target(
            "enemy_refinery",
            Some(InfiltrationEffect::StealsFunds { percent: 10 }),
        )],
        vec![(0, "ghost", 15, 10), (1, "enemy_refinery", 20, 10)],
    );
    let ids = sim.units().ids();
    send_in(&mut sim, 0, ids[0], ids[1]);
    sim.tick(&[]);

    let agent = sim.unit(ids[0]).expect("it is not consumed");
    assert!(agent.is_alive() && !agent.is_aboard());
}

#[test]
fn infiltration_is_deterministic() {
    let run = || {
        let mut sim = scenario(
            vec![target(
                "enemy_refinery",
                Some(InfiltrationEffect::StealsFunds { percent: 20 }),
            )],
            vec![(0, "spy", 15, 10), (1, "enemy_refinery", 20, 10)],
        );
        let ids = sim.units().ids();
        send_in(&mut sim, 0, ids[0], ids[1]);
        for _ in 0..200 {
            sim.tick(&[]);
        }
        sim.state_hash()
    };
    assert_eq!(run(), run());
}
