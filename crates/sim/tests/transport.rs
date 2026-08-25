//! Carrying units.
//!
//! The interesting part of a transport is not loading and unloading — it is
//! that a passenger has to disappear from the world in *every* respect while
//! keeping its identity. Missing one of those is the classic bug here, and it
//! looks like a rifleman shooting from inside a sealed truck.
//!
//! So most of these tests are about what a passenger must stop doing.

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
        speed: Hundredths(400),
        turn_rate: 3600,
        locomotor,
        surfaces: None,
        size: None,
        layer: None,
    };

    let rifleman = EntityDef {
        id: "rifleman".into(),
        name_key: "u.rifleman".into(),
        side: None,
        category: "infantry".into(),
        traits: vec![
            Trait::Health {
                max: 100,
                armour: "none".into(),
            },
            mobile(Locomotor::Foot),
            Trait::Vision {
                range: Hundredths(600),
            },
            Trait::Armed {
                weapon: "rifle".into(),
                turret: true,
                turret_rate: 3600,
            },
        ],
    };
    let tank = EntityDef {
        id: "tank".into(),
        name_key: "u.tank".into(),
        side: None,
        category: "vehicle".into(),
        traits: vec![
            Trait::Health {
                max: 400,
                armour: "none".into(),
            },
            mobile(Locomotor::Tracked),
            Trait::Vision {
                range: Hundredths(600),
            },
        ],
    };
    let apc = EntityDef {
        id: "apc".into(),
        name_key: "u.apc".into(),
        side: None,
        category: "vehicle".into(),
        traits: vec![
            Trait::Health {
                max: 300,
                armour: "none".into(),
            },
            mobile(Locomotor::Wheeled),
            Trait::Vision {
                range: Hundredths(600),
            },
            // Infantry only. A tank is refused.
            Trait::Transport {
                capacity: 2,
                allowed: vec!["rifleman".into()],
            },
        ],
    };

    Rules::from_parts(
        vec![rifleman, tank, apc],
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
            target_categories: vec![],
            heals: false,
        }],
        armour,
        Vec::new(),
    )
    .expect("rules")
}

fn scenario(spawns: Vec<(u8, &str, i32, i32)>) -> Sim {
    let rules = rules();
    Sim::new(MatchSetup {
        seed: 0xCA_60,
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
                owner: PlayerId(owner),
                kind: rules.kind_of(kind).unwrap_or_else(|| panic!("no {kind}")),
                pos: Cell::new(x, y).centre(),
            })
            .collect(),
        rules,
    })
}

fn load(sim: &mut Sim, passenger: EntityId, transport: EntityId) -> bool {
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Load {
            units: vec![passenger],
            transport,
        },
    )]);
    for _ in 0..2_000 {
        sim.tick(&[]);
        if sim.units().get(passenger).is_some_and(|u| u.is_aboard()) {
            return true;
        }
    }
    false
}

#[test]
fn a_passenger_walks_over_and_climbs_aboard() {
    let mut sim = scenario(vec![(0, "apc", 10, 10), (0, "rifleman", 20, 10)]);
    let (apc, rifleman) = (sim.units().ids()[0], sim.units().ids()[1]);

    assert!(load(&mut sim, rifleman, apc), "the rifleman never boarded");
    assert_eq!(
        sim.units().get(apc).unwrap().cargo,
        vec![rifleman],
        "the transport does not know it has a passenger"
    );
}

#[test]
fn a_passenger_cannot_be_seen_or_shot() {
    // The check that matters most. A passenger that is still visible is still a
    // target, and a transport becomes a way of making units invulnerable
    // *while* they keep fighting.
    let mut sim = scenario(vec![
        (0, "apc", 10, 10),
        (0, "rifleman", 12, 10),
        (1, "rifleman", 14, 10),
    ]);
    let (apc, mine, enemy) = (
        sim.units().ids()[0],
        sim.units().ids()[1],
        sim.units().ids()[2],
    );
    assert!(load(&mut sim, mine, apc));

    let health_when_boarded = sim.units().get(mine).unwrap().health;
    // Short of the time it takes the enemy to destroy the transport, since a
    // passenger dying *with* its transport is correct and would mask what this
    // is checking.
    for _ in 0..60 {
        sim.tick(&[]);
    }
    assert!(
        sim.units().get(apc).is_some(),
        "the transport was destroyed too quickly for this test to mean anything"
    );

    let passenger = sim.units().get(mine).expect("still in the arena");
    assert!(
        !sim.can_see(PlayerId(1), passenger),
        "the enemy can see a passenger"
    );
    assert_eq!(
        passenger.health, health_when_boarded,
        "a passenger was shot while inside the transport"
    );
    assert_ne!(
        sim.units().get(enemy).map(|u| u.combat.target),
        Some(Some(mine)),
        "an enemy targeted a unit that is inside a transport"
    );
}

#[test]
fn a_passenger_does_not_shoot_from_inside() {
    // The other half. Passengers firing from inside is a real mechanic in the
    // original, but it belongs to specific transports and must not be the
    // default for all of them.
    let mut sim = scenario(vec![
        (0, "apc", 10, 10),
        (0, "rifleman", 12, 10),
        (1, "rifleman", 13, 10),
    ]);
    let (apc, mine, enemy) = (
        sim.units().ids()[0],
        sim.units().ids()[1],
        sim.units().ids()[2],
    );
    assert!(load(&mut sim, mine, apc));

    let enemy_health = sim.units().get(enemy).map(|u| u.health);
    for _ in 0..300 {
        sim.tick(&[]);
        // The transport itself is unarmed, so any damage came from inside.
        if sim.units().get(enemy).map(|u| u.health) != enemy_health {
            // Unless the enemy shot the transport and took splash — there is
            // none here, so this is a genuine failure.
            panic!("a passenger fired from inside a sealed transport");
        }
    }
}

#[test]
fn a_passenger_reveals_no_ground() {
    let mut sim = scenario(vec![(0, "apc", 10, 10), (0, "rifleman", 30, 30)]);
    let (apc, rifleman) = (sim.units().ids()[0], sim.units().ids()[1]);

    // Somewhere only the rifleman's own vision reaches on its way over.
    assert!(load(&mut sim, rifleman, apc));
    for _ in 0..50 {
        sim.tick(&[]);
    }
    // Once aboard, the passenger contributes nothing: the far corner it started
    // near is no longer watched.
    assert!(
        !sim.visibility().is_visible(PlayerId(0), Cell::new(30, 30)),
        "a passenger is still revealing ground"
    );
}

#[test]
fn unloading_puts_everyone_back_on_distinct_cells() {
    let mut sim = scenario(vec![
        (0, "apc", 10, 10),
        (0, "rifleman", 11, 10),
        (0, "rifleman", 12, 10),
    ]);
    let apc = sim.units().ids()[0];
    let a = sim.units().ids()[1];
    let b = sim.units().ids()[2];
    assert!(load(&mut sim, a, apc));
    assert!(load(&mut sim, b, apc));
    assert_eq!(sim.units().get(apc).unwrap().cargo.len(), 2);

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Unload {
            transport: apc,
            at: Cell::new(20, 20),
        },
    )]);

    assert!(
        sim.units().get(apc).unwrap().cargo.is_empty(),
        "the cargo was not emptied"
    );
    let cells: Vec<Cell> = [a, b]
        .iter()
        .filter_map(|id| sim.units().get(*id))
        .map(|u| {
            assert!(!u.is_aboard(), "a passenger is still marked as aboard");
            u.cell()
        })
        .collect();
    assert_eq!(cells.len(), 2);
    assert_ne!(
        cells[0], cells[1],
        "both passengers were unloaded onto one cell"
    );
}

#[test]
fn a_transport_refuses_what_it_may_not_carry() {
    // The APC takes infantry. A tank is refused, and the refusal happens in the
    // simulation rather than in the interface — every peer has to agree that
    // nothing happened.
    let mut sim = scenario(vec![(0, "apc", 10, 10), (0, "tank", 12, 10)]);
    let (apc, tank) = (sim.units().ids()[0], sim.units().ids()[1]);

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Load {
            units: vec![tank],
            transport: apc,
        },
    )]);
    for _ in 0..500 {
        sim.tick(&[]);
    }
    assert!(
        !sim.units().get(tank).unwrap().is_aboard(),
        "a tank boarded an infantry carrier"
    );
    assert!(sim.units().get(apc).unwrap().cargo.is_empty());
}

#[test]
fn a_transport_refuses_more_than_it_holds() {
    let mut sim = scenario(vec![
        (0, "apc", 10, 10),
        (0, "rifleman", 11, 10),
        (0, "rifleman", 12, 10),
        (0, "rifleman", 13, 10),
    ]);
    let apc = sim.units().ids()[0];
    let ids: Vec<EntityId> = sim.units().ids()[1..].to_vec();

    for id in &ids {
        let _ = load(&mut sim, *id, apc);
    }
    assert_eq!(
        sim.units().get(apc).unwrap().cargo.len(),
        2,
        "the transport took more than its capacity"
    );
}

#[test]
fn passengers_die_with_their_transport() {
    // Spilling them out would make a loaded transport safer than an empty one,
    // which is exactly backwards.
    let mut sim = scenario(vec![
        (0, "apc", 10, 10),
        (0, "rifleman", 11, 10),
        (1, "rifleman", 13, 10),
        (1, "rifleman", 13, 11),
        (1, "rifleman", 13, 9),
    ]);
    let apc = sim.units().ids()[0];
    let passenger = sim.units().ids()[1];
    assert!(load(&mut sim, passenger, apc));

    for _ in 0..4_000 {
        sim.tick(&[]);
        if sim.units().get(apc).is_none() {
            break;
        }
    }
    assert!(
        sim.units().get(apc).is_none(),
        "the transport survived, so this proves nothing"
    );
    assert!(
        sim.units().get(passenger).is_none(),
        "the passenger outlived the transport it was inside"
    );
}

#[test]
fn boarding_is_abandoned_if_the_transport_dies_on_the_way() {
    // Arriving is not the same as boarding, which is why this is its own order.
    let mut sim = scenario(vec![
        (0, "apc", 10, 10),
        (0, "rifleman", 30, 10),
        (1, "rifleman", 12, 10),
        (1, "rifleman", 12, 11),
        (1, "rifleman", 12, 9),
    ]);
    let (apc, walker) = (sim.units().ids()[0], sim.units().ids()[1]);

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Load {
            units: vec![walker],
            transport: apc,
        },
    )]);
    for _ in 0..4_000 {
        sim.tick(&[]);
        if sim.units().get(apc).is_none() {
            break;
        }
    }
    assert!(sim.units().get(apc).is_none(), "the transport survived");

    for _ in 0..100 {
        sim.tick(&[]);
    }
    let unit = sim.units().get(walker).expect("the walker should survive");
    assert!(!unit.is_aboard());
    assert!(
        unit.order.is_idle(),
        "it is still walking to a transport that no longer exists"
    );
}

#[test]
fn transports_are_deterministic() {
    let run = || {
        let mut sim = scenario(vec![
            (0, "apc", 10, 10),
            (0, "rifleman", 14, 10),
            (0, "rifleman", 15, 11),
            (1, "rifleman", 26, 10),
        ]);
        let apc = sim.units().ids()[0];
        let riders = vec![sim.units().ids()[1], sim.units().ids()[2]];
        sim.tick(&[Command::new(
            PlayerId(0),
            0,
            CommandKind::Load {
                units: riders,
                transport: apc,
            },
        )]);
        let mut hashes = Vec::new();
        for tick in 0..1_200 {
            if tick == 600 {
                sim.tick(&[Command::new(
                    PlayerId(0),
                    1,
                    CommandKind::Unload {
                        transport: apc,
                        at: Cell::new(22, 10),
                    },
                )]);
            } else {
                sim.tick(&[]);
            }
            hashes.push(sim.state_hash());
        }
        (hashes, sim.units().len())
    };
    let (a, a_units) = run();
    let (b, b_units) = run();
    assert_eq!(a, b, "two identical transport runs diverged");
    assert_eq!(a_units, b_units);
}
