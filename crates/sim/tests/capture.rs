//! Engineers, capture, and the neutral side.
//!
//! The engineer is three mechanics in one action: it captures what is not
//! yours, repairs what is, and is consumed either way. The original never asked
//! the player to choose between them — they chose a building.
//!
//! The neutral side is here too, because tech buildings need an owner and
//! civilians need a rule: nobody shoots them by accident, and anybody can shoot
//! them on purpose.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::EntityId;
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn rules() -> Rules {
    let armour: ArmourTable =
        ron::from_str(r#"( classes: ["none"], table: { "shot": { "none": 100 } } )"#).unwrap();

    let mobile = |locomotor| Trait::Mobile {
        speed: Hundredths(500),
        turn_rate: 3600,
        locomotor,
        surfaces: None,
        size: None,
        layer: None,
    };

    let infantry = |id: &str, extra: Vec<Trait>| {
        let mut traits = vec![
            Trait::Health {
                max: 100,
                armour: "none".into(),
            },
            mobile(Locomotor::Foot),
            Trait::Vision {
                range: Hundredths(1200),
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
    };

    let structure = |id: &str, capturable: bool| {
        let mut traits = vec![
            Trait::Health {
                max: 500,
                armour: "none".into(),
            },
            Trait::Vision {
                range: Hundredths(400),
            },
        ];
        // A real footprint, because every real building has one and its own
        // cells are impassable once it occupies them. With a one-cell building
        // an engineer walks to the centre; with a three-by-three one there is
        // no route to the centre at all, and the engineer stops where it
        // stands. That distinction went unnoticed here for a long time, so it
        // is now part of the fixture rather than something a future test has
        // to remember to arrange.
        traits.push(Trait::Footprint {
            width: 3,
            height: 3,
        });
        if capturable {
            traits.push(Trait::Capturable);
        }
        EntityDef {
            id: id.into(),
            name_key: format!("b.{id}"),
            side: None,
            category: "structure".into(),
            traits,
        }
    };

    Rules::from_parts(
        vec![
            infantry("engineer", vec![Trait::Engineer { consumed: true }]),
            infantry("mechanic", vec![Trait::Engineer { consumed: false }]),
            infantry(
                "rifleman",
                vec![Trait::Armed {
                    weapon: "rifle".into(),
                    turret: true,
                    turret_rate: 3600,
                }],
            ),
            infantry("civilian", vec![]),
            structure("derrick", true),
            structure("bunker", false),
        ],
        vec![WeaponDef {
            id: "rifle".into(),
            damage: 25,
            warhead: "shot".into(),
            reload: Ticks(10),
            range: Hundredths(500),
            splash_radius: Hundredths::ZERO,
            projectile_speed: Hundredths::ZERO,
            homing: false,
            targets: vec![],
            instant_kill: false,
            ammo: 0,
            intercepts: false,
        }],
        armour,
        Vec::new(),
    )
    .expect("rules")
}

fn scenario(spawns: Vec<(PlayerId, &str, i32, i32)>) -> Sim {
    let rules = rules();
    Sim::new(MatchSetup {
        seed: 0xCA9,
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
        spawns: spawns
            .into_iter()
            .map(|(owner, kind, x, y)| Spawn {
                owner,
                kind: rules.kind_of(kind).unwrap_or_else(|| panic!("no {kind}")),
                pos: Cell::new(x, y).centre(),
            })
            .collect(),
        rules,
    })
}

fn send_in(sim: &mut Sim, engineer: EntityId, target: EntityId) {
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::EnterBuilding {
            units: vec![engineer],
            target,
        },
    )]);
    for _ in 0..2_000 {
        sim.tick(&[]);
        if sim.units().get(engineer).is_none_or(|u| u.order.is_idle()) {
            return;
        }
    }
}

#[test]
fn an_engineer_captures_a_neutral_building_and_is_consumed() {
    let mut sim = scenario(vec![
        (PlayerId(0), "engineer", 10, 10),
        (PlayerId::NEUTRAL, "derrick", 20, 10),
    ]);
    let (engineer, derrick) = (sim.units().ids()[0], sim.units().ids()[1]);

    send_in(&mut sim, engineer, derrick);

    assert_eq!(
        sim.units().get(derrick).expect("the derrick").owner,
        PlayerId(0),
        "the derrick was not captured"
    );
    assert!(
        sim.units().get(engineer).is_none(),
        "the engineer survived, and it should not have"
    );
}

#[test]
fn an_engineer_captures_an_enemy_building() {
    let mut sim = scenario(vec![
        (PlayerId(0), "engineer", 10, 10),
        (PlayerId(1), "derrick", 20, 10),
    ]);
    let (engineer, derrick) = (sim.units().ids()[0], sim.units().ids()[1]);
    send_in(&mut sim, engineer, derrick);
    assert_eq!(sim.units().get(derrick).unwrap().owner, PlayerId(0));
}

#[test]
fn a_building_that_is_not_capturable_is_left_alone() {
    let mut sim = scenario(vec![
        (PlayerId(0), "engineer", 10, 10),
        (PlayerId(1), "bunker", 20, 10),
    ]);
    let (engineer, bunker) = (sim.units().ids()[0], sim.units().ids()[1]);
    send_in(&mut sim, engineer, bunker);

    assert_eq!(sim.units().get(bunker).unwrap().owner, PlayerId(1));
    assert!(
        sim.units().get(engineer).is_some(),
        "the engineer was consumed for nothing"
    );
}

#[test]
fn an_engineer_repairs_its_own_damaged_building() {
    // The same action, a different outcome, decided by whose building it is.
    let mut sim = scenario(vec![
        (PlayerId(0), "engineer", 10, 10),
        (PlayerId(0), "derrick", 20, 10),
        (PlayerId(1), "rifleman", 22, 10),
    ]);
    let (engineer, derrick) = (sim.units().ids()[0], sim.units().ids()[1]);

    // Let the enemy do some damage — but not enough to destroy it, since a
    // rifle takes almost exactly 200 ticks to level a 500-health building.
    for _ in 0..100 {
        sim.tick(&[]);
    }
    let damaged = sim.units().get(derrick).expect("the derrick").health;
    assert!(damaged < 500, "the test needs a damaged building");

    send_in(&mut sim, engineer, derrick);
    assert!(
        sim.units().get(derrick).unwrap().health > damaged,
        "the engineer did not repair its own building"
    );
}

#[test]
fn an_engineer_is_not_wasted_on_an_undamaged_building() {
    // Consuming one for nothing is a pure loss with nothing to show for it.
    let mut sim = scenario(vec![
        (PlayerId(0), "engineer", 10, 10),
        (PlayerId(0), "derrick", 20, 10),
    ]);
    let (engineer, derrick) = (sim.units().ids()[0], sim.units().ids()[1]);
    send_in(&mut sim, engineer, derrick);
    assert!(
        sim.units().get(engineer).is_some(),
        "an engineer was consumed on a building that needed nothing"
    );
}

#[test]
fn a_unit_that_is_not_consumed_walks_back_out() {
    // Whether entering destroys the unit is data, not a rule of the engine.
    let mut sim = scenario(vec![
        (PlayerId(0), "mechanic", 10, 10),
        (PlayerId::NEUTRAL, "derrick", 20, 10),
    ]);
    let (mechanic, derrick) = (sim.units().ids()[0], sim.units().ids()[1]);
    send_in(&mut sim, mechanic, derrick);

    assert_eq!(sim.units().get(derrick).unwrap().owner, PlayerId(0));
    assert!(
        sim.units().get(mechanic).is_some(),
        "a unit that declares it is not consumed was consumed anyway"
    );
}

#[test]
fn nobody_shoots_a_civilian_by_accident() {
    // The rule from the original: civilians standing beside an army do not
    // start a battle.
    let mut sim = scenario(vec![
        (PlayerId(0), "rifleman", 10, 10),
        (PlayerId::NEUTRAL, "civilian", 12, 10),
    ]);
    let civilian = sim.units().ids()[1];

    for _ in 0..500 {
        sim.tick(&[]);
    }
    assert_eq!(
        sim.units().get(civilian).map(|u| u.health),
        Some(100),
        "a civilian was shot without anyone ordering it"
    );
}

#[test]
fn a_player_can_shoot_a_civilian_on_purpose() {
    // And the other half of the same rule. Automatic targeting skips neutrals;
    // a deliberate order does not, or the distinction would be an immunity.
    let mut sim = scenario(vec![
        (PlayerId(0), "rifleman", 10, 10),
        (PlayerId::NEUTRAL, "civilian", 12, 10),
    ]);
    let (rifleman, civilian) = (sim.units().ids()[0], sim.units().ids()[1]);

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Attack {
            units: vec![rifleman],
            target: civilian,
        },
    )]);
    for _ in 0..500 {
        sim.tick(&[]);
        if sim.units().get(civilian).is_none() {
            return;
        }
    }
    let rifle = sim.units().get(rifleman).expect("the rifleman");
    let civ = sim.units().get(civilian).expect("the civilian");
    panic!(
        "explicit attack did nothing: order={:?} target={:?} civilian_hp={} visible={}",
        rifle.order,
        rifle.combat.target,
        civ.health,
        sim.can_see(PlayerId(0), civ)
    );
}

#[test]
fn a_captured_power_plant_powers_its_new_owner() {
    // Power is rebuilt from scratch every tick, so this follows from capture
    // with nothing arranging it — which is the point of rebuilding rather than
    // maintaining a running total.
    let rules = rules();
    let plant = EntityDef {
        id: "plant".into(),
        name_key: "b.plant".into(),
        side: None,
        category: "structure".into(),
        traits: vec![
            Trait::Health {
                max: 500,
                armour: "none".into(),
            },
            Trait::Vision {
                range: Hundredths(400),
            },
            Trait::Capturable,
            Trait::PowerSupply { output: 200 },
        ],
    };
    let mut entities: Vec<EntityDef> = rules.entities().map(|(_, e)| e.clone()).collect();
    entities.push(plant);
    let rules = Rules::from_parts(
        entities,
        vec![WeaponDef {
            id: "rifle".into(),
            damage: 25,
            warhead: "shot".into(),
            reload: Ticks(10),
            range: Hundredths(500),
            splash_radius: Hundredths::ZERO,
            projectile_speed: Hundredths::ZERO,
            homing: false,
            targets: vec![],
            instant_kill: false,
            ammo: 0,
            intercepts: false,
        }],
        ron::from_str(r#"( classes: ["none"], table: { "shot": { "none": 100 } } )"#).unwrap(),
        Vec::new(),
    )
    .expect("rules");

    let engineer_kind = rules.kind_of("engineer").unwrap();
    let plant_kind = rules.kind_of("plant").unwrap();
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
                kind: engineer_kind,
                pos: Cell::new(10, 10).centre(),
            },
            Spawn {
                owner: PlayerId::NEUTRAL,
                kind: plant_kind,
                pos: Cell::new(20, 10).centre(),
            },
        ],
        rules,
    });
    let (engineer, plant_id) = (sim.units().ids()[0], sim.units().ids()[1]);

    assert_eq!(
        sim.power().supply(PlayerId(0)),
        0,
        "nothing should be supplying yet"
    );
    send_in(&mut sim, engineer, plant_id);
    sim.tick(&[]);

    assert_eq!(
        sim.power().supply(PlayerId(0)),
        200,
        "a captured plant did not supply its new owner"
    );
}

#[test]
fn capture_is_deterministic() {
    let run = || {
        let mut sim = scenario(vec![
            (PlayerId(0), "engineer", 10, 10),
            (PlayerId(0), "engineer", 10, 12),
            (PlayerId::NEUTRAL, "derrick", 20, 10),
            (PlayerId(1), "derrick", 20, 14),
        ]);
        let engineers = [sim.units().ids()[0], sim.units().ids()[1]];
        let targets = [sim.units().ids()[2], sim.units().ids()[3]];
        sim.tick(&[
            Command::new(
                PlayerId(0),
                0,
                CommandKind::EnterBuilding {
                    units: vec![engineers[0]],
                    target: targets[0],
                },
            ),
            Command::new(
                PlayerId(0),
                1,
                CommandKind::EnterBuilding {
                    units: vec![engineers[1]],
                    target: targets[1],
                },
            ),
        ]);
        let mut hashes = Vec::new();
        for _ in 0..1_500 {
            sim.tick(&[]);
            hashes.push(sim.state_hash());
        }
        (hashes, sim.units().len())
    };
    let (a, a_units) = run();
    let (b, b_units) = run();
    assert_eq!(a, b, "two identical captures diverged");
    assert_eq!(a_units, b_units);
    assert!(a_units < 4, "nothing was captured, so this proves nothing");
}
