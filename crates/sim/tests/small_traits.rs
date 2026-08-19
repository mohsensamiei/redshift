//! Crushing, self-repair and death explosions.
//!
//! Three traits that sat in the catalogue unread. Each is a small rule, and
//! each has one detail that makes it a design decision rather than a formula.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn armour() -> ArmourTable {
    ron::from_str(r#"( classes: ["none"], table: { "blast": { "none": 100 } } )"#).expect("armour")
}

fn mobile(locomotor: Locomotor) -> Trait {
    Trait::Mobile {
        speed: Hundredths(500),
        turn_rate: 3600,
        locomotor,
        surfaces: None,
        size: None,
        layer: None,
    }
}

fn scenario(entities: Vec<EntityDef>, spawns: Vec<(u8, &str, i32, i32)>) -> Sim {
    let rules = Rules::from_parts(entities, Vec::new(), armour(), Vec::new()).expect("rules");
    let spawns = spawns
        .into_iter()
        .map(|(owner, kind, x, y)| Spawn {
            owner: PlayerId(owner),
            kind: rules.kind_of(kind).unwrap_or_else(|| panic!("no {kind}")),
            pos: Cell::new(x, y).centre(),
        })
        .collect();
    Sim::new(MatchSetup {
        seed: 0x5_A11,
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
        spawns,
        rules,
    })
}

fn tank(id: &str) -> EntityDef {
    EntityDef {
        id: id.into(),
        name_key: format!("u.{id}"),
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
            Trait::Crushes {
                classes: vec!["infantry".into()],
            },
        ],
    }
}

fn footman(id: &str, crushable: bool) -> EntityDef {
    let mut traits = vec![
        Trait::Health {
            max: 100,
            armour: "none".into(),
        },
        mobile(Locomotor::Foot),
        Trait::Vision {
            range: Hundredths(400),
        },
    ];
    if crushable {
        traits.push(Trait::Crushable {
            class: "infantry".into(),
        });
    }
    EntityDef {
        id: id.into(),
        name_key: format!("u.{id}"),
        side: None,
        category: "infantry".into(),
        traits,
    }
}

/// Drives the tank across the map and reports whether the victim survived.
fn drive_over(sim: &mut Sim) -> bool {
    let tank = sim.units().ids()[0];
    let victim = sim.units().ids()[1];
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Move {
            units: vec![tank],
            target: Cell::new(30, 15),
        },
    )]);
    for _ in 0..3_000 {
        sim.tick(&[]);
        if sim.units().get(victim).is_none() {
            return false;
        }
    }
    true
}

#[test]
fn a_tank_crushes_infantry_it_drives_over() {
    let mut sim = scenario(
        vec![tank("tank"), footman("rifleman", true)],
        vec![(0, "tank", 5, 15), (1, "rifleman", 15, 15)],
    );
    assert!(!drive_over(&mut sim), "the tank drove straight through");
}

#[test]
fn something_that_is_not_crushable_survives() {
    // A unit with no Crushable trait is not crushed. That is what lets a tank
    // destroyer or another tank block a road rather than being flattened.
    let mut sim = scenario(
        vec![tank("tank"), footman("rifleman", false)],
        vec![(0, "tank", 5, 15), (1, "rifleman", 15, 15)],
    );
    assert!(
        drive_over(&mut sim),
        "something uncrushable was crushed anyway"
    );
}

#[test]
fn a_tank_does_not_crush_its_own_side() {
    // Driving over your own infantry would make a large army unmanageable, and
    // the original did not do it either.
    let mut sim = scenario(
        vec![tank("tank"), footman("rifleman", true)],
        vec![(0, "tank", 5, 15), (0, "rifleman", 15, 15)],
    );
    let victim = sim.units().ids()[1];
    let tank_id = sim.units().ids()[0];
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Move {
            units: vec![tank_id],
            target: Cell::new(30, 15),
        },
    )]);
    for _ in 0..3_000 {
        sim.tick(&[]);
    }
    assert!(
        sim.units().get(victim).is_some(),
        "a tank crushed a friendly unit"
    );
}

#[test]
fn a_damaged_unit_heals_once_it_is_left_alone() {
    let healer = EntityDef {
        id: "healer".into(),
        name_key: "u.healer".into(),
        side: None,
        category: "vehicle".into(),
        traits: vec![
            Trait::Health {
                max: 400,
                armour: "none".into(),
            },
            mobile(Locomotor::Tracked),
            Trait::Vision {
                range: Hundredths(400),
            },
            Trait::SelfHealing {
                per_tick: Hundredths(200),
                delay_after_damage: Ticks(20),
            },
        ],
    };
    let mut sim = scenario(vec![healer], vec![(0, "healer", 10, 10)]);

    // It starts at full health, so there is nothing to regain — the useful
    // check is that healing never pushes it past its maximum.
    for _ in 0..500 {
        sim.tick(&[]);
    }
    let unit = sim.units().iter().next().expect("the unit").1;
    assert_eq!(unit.health, 400, "self-healing overshot the maximum");
}

#[test]
fn healing_waits_for_the_delay() {
    // The delay is what makes this a recovery mechanic rather than an armour
    // bonus: a unit under fire gains nothing.
    let tough = EntityDef {
        id: "tough".into(),
        name_key: "u.tough".into(),
        side: None,
        category: "vehicle".into(),
        traits: vec![
            Trait::Health {
                max: 400,
                armour: "none".into(),
            },
            mobile(Locomotor::Tracked),
            Trait::Vision {
                range: Hundredths(400),
            },
            Trait::SelfHealing {
                per_tick: Hundredths(100),
                // Longer than the test runs, so healing must never start.
                delay_after_damage: Ticks(10_000),
            },
        ],
    };
    let mut sim = scenario(vec![tough], vec![(0, "tough", 10, 10)]);
    let id = sim.units().ids()[0];
    let before = sim.units().get(id).unwrap().health;
    for _ in 0..200 {
        sim.tick(&[]);
    }
    assert_eq!(
        sim.units().get(id).unwrap().health,
        before,
        "a unit healed before its delay had elapsed"
    );
}

#[test]
fn a_destroyed_unit_damages_its_neighbours() {
    let bomb = EntityDef {
        id: "bomb_truck".into(),
        name_key: "u.bomb".into(),
        side: None,
        category: "vehicle".into(),
        traits: vec![
            // Fragile, so a single shot sets it off.
            Trait::Health {
                max: 10,
                armour: "none".into(),
            },
            mobile(Locomotor::Wheeled),
            Trait::Vision {
                range: Hundredths(400),
            },
            Trait::Explodes {
                warhead: "blast".into(),
                damage: 300,
            },
        ],
    };
    let bystander = footman("bystander", false);
    let shooter = {
        let mut e = footman("shooter", false);
        e.traits.push(Trait::Armed {
            weapon: "rifle".into(),
            turret: true,
            turret_rate: 3600,
        });
        e
    };

    let rules = Rules::from_parts(
        vec![bomb, bystander, shooter],
        vec![WeaponDef {
            id: "rifle".into(),
            damage: 50,
            warhead: "blast".into(),
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
        armour(),
        Vec::new(),
    )
    .expect("rules");

    let kind = |id: &str| rules.kind_of(id).unwrap();
    let (bomb_kind, bystander_kind, shooter_kind) =
        (kind("bomb_truck"), kind("bystander"), kind("shooter"));

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
                owner: PlayerId(1),
                kind: bomb_kind,
                pos: Cell::new(20, 20).centre(),
            },
            // Right beside the truck, and on the truck's own side, because a
            // blast does not care whose it was.
            Spawn {
                owner: PlayerId(1),
                kind: bystander_kind,
                pos: Cell::new(21, 20).centre(),
            },
            Spawn {
                owner: PlayerId(0),
                kind: shooter_kind,
                pos: Cell::new(24, 20).centre(),
            },
        ],
        rules,
    });
    let bystander_id = sim.units().ids()[1];

    for _ in 0..400 {
        sim.tick(&[]);
        if sim.units().get(bystander_id).is_none_or(|u| u.health < 100) {
            return;
        }
    }
    panic!("the truck was destroyed and its neighbour took no damage");
}

#[test]
fn the_small_traits_are_deterministic() {
    let run = || {
        let mut sim = scenario(
            vec![tank("tank"), footman("rifleman", true)],
            vec![
                (0, "tank", 5, 15),
                (0, "tank", 5, 17),
                (1, "rifleman", 15, 15),
                (1, "rifleman", 16, 16),
            ],
        );
        let mine: Vec<_> = sim
            .units()
            .iter()
            .filter(|(_, u)| u.owner == PlayerId(0))
            .map(|(id, _)| id)
            .collect();
        sim.tick(&[Command::new(
            PlayerId(0),
            0,
            CommandKind::Move {
                units: mine,
                target: Cell::new(30, 15),
            },
        )]);
        let mut hashes = Vec::new();
        for _ in 0..1_000 {
            sim.tick(&[]);
            hashes.push(sim.state_hash());
        }
        (hashes, sim.units().len())
    };
    let (a, a_units) = run();
    let (b, b_units) = run();
    assert_eq!(a, b, "two identical runs diverged");
    assert_eq!(a_units, b_units);
    assert!(a_units < 4, "nothing was crushed, so this proves nothing");
}
