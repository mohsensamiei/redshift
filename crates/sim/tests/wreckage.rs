//! What a death leaves behind.
//!
//! Three things in the original, and two of them turned out to be one
//! mechanism: rubble where a building stood and the crew climbing out of a
//! wrecked vehicle are both "put something where it fell". The third — a
//! reactor poisoning the ground it stood on — is the contamination that already
//! exists, with a different trigger.
//!
//! The crew is the one that changes how a match is played. Survivors mean
//! destroying a full transport stops being the same as destroying an empty one.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Contaminate, Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::EntityId;
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn armour() -> ArmourTable {
    ron::from_str(
        r#"( classes: ["none"],
             table: { "shot": { "none": 100 }, "fallout": { "none": 100 } } )"#,
    )
    .unwrap()
}

fn infantry(id: &str, armed: bool) -> EntityDef {
    let mut traits = vec![
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
            range: Hundredths(900),
        },
    ];
    if armed {
        traits.push(Trait::Armed {
            weapon: "cannon".into(),
            turret: true,
            turret_rate: 3600,
        });
    }
    EntityDef {
        id: id.into(),
        name_key: format!("unit.{id}"),
        side: None,
        category: "infantry".into(),
        traits,
    }
}

fn rules() -> Rules {
    Rules::from_parts(
        vec![
            infantry("gunner", true),
            // The crew. Ordinary infantry, which is the point: what climbs out
            // of a wreck is a unit like any other.
            infantry("crew", false),
            EntityDef {
                id: "tank".into(),
                name_key: "unit.tank".into(),
                side: None,
                category: "vehicle".into(),
                traits: vec![
                    Trait::Health {
                        max: 300,
                        armour: "none".into(),
                    },
                    Trait::Mobile {
                        speed: Hundredths(300),
                        turn_rate: 3600,
                        locomotor: Locomotor::Tracked,
                        surfaces: None,
                        size: None,
                        layer: None,
                    },
                    Trait::Vision {
                        range: Hundredths(400),
                    },
                    Trait::Leaves {
                        units: vec!["crew".into()],
                        chance_percent: 100,
                    },
                ],
            },
            // Never survives. A crew that always got out would make a vehicle a
            // free infantry squad, so the chance has to be able to fail.
            EntityDef {
                id: "sealed_tank".into(),
                name_key: "unit.sealed_tank".into(),
                side: None,
                category: "vehicle".into(),
                traits: vec![
                    Trait::Health {
                        max: 300,
                        armour: "none".into(),
                    },
                    Trait::Mobile {
                        speed: Hundredths(300),
                        turn_rate: 3600,
                        locomotor: Locomotor::Tracked,
                        surfaces: None,
                        size: None,
                        layer: None,
                    },
                    Trait::Vision {
                        range: Hundredths(400),
                    },
                    Trait::Leaves {
                        units: vec!["crew".into()],
                        chance_percent: 0,
                    },
                ],
            },
            EntityDef {
                id: "rubble".into(),
                name_key: "structure.rubble".into(),
                side: None,
                category: "terrain".into(),
                traits: vec![Trait::Health {
                    max: 1,
                    armour: "none".into(),
                }],
            },
            EntityDef {
                id: "barracks".into(),
                name_key: "structure.barracks".into(),
                side: None,
                category: "structure".into(),
                traits: vec![
                    Trait::Health {
                        max: 300,
                        armour: "none".into(),
                    },
                    Trait::Vision {
                        range: Hundredths(300),
                    },
                    Trait::Footprint {
                        width: 3,
                        height: 3,
                    },
                    Trait::Leaves {
                        units: vec!["rubble".into()],
                        chance_percent: 100,
                    },
                ],
            },
            // The reactor: a blast *and* ground that stays dangerous. Two
            // separate consequences of one death.
            EntityDef {
                id: "reactor".into(),
                name_key: "structure.reactor".into(),
                side: None,
                category: "structure".into(),
                traits: vec![
                    Trait::Health {
                        max: 300,
                        armour: "none".into(),
                    },
                    Trait::Vision {
                        range: Hundredths(300),
                    },
                    Trait::Explodes {
                        warhead: "shot".into(),
                        damage: 150,
                    },
                    Trait::Contaminates {
                        radius: Hundredths(250),
                        damage: 5,
                        warhead: "fallout".into(),
                        lingers: Ticks(200),
                        when: Contaminate::OnDeath,
                    },
                ],
            },
        ],
        vec![WeaponDef {
            id: "cannon".into(),
            damage: 100,
            warhead: "shot".into(),
            reload: Ticks(5),
            range: Hundredths(600),
            splash_radius: Hundredths::ZERO,
            projectile_speed: Hundredths::ZERO,
            homing: false,
            targets: vec![],
            instant_kill: false,
            ammo: 0,
            intercepts: false,
            heals: false,
        }],
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
    Sim::new(MatchSetup {
        seed: 0x_9EC,
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

/// Shoots `victim` down and returns once it is gone.
fn destroy(sim: &mut Sim, gunner: EntityId, victim: EntityId) {
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Attack {
            units: vec![gunner],
            target: victim,
        },
    )]);
    for _ in 0..600 {
        sim.tick(&[]);
        if sim.unit(victim).is_none() {
            return;
        }
    }
    panic!("it would not come down");
}

fn count(sim: &Sim, id: &str) -> usize {
    let kind = sim.rules().kind_of(id).unwrap();
    sim.units()
        .iter()
        .filter(|(_, u)| u.kind == kind && u.is_alive())
        .count()
}

// -- Rubble -----------------------------------------------------------------

#[test]
fn a_destroyed_building_leaves_rubble() {
    let mut sim = scenario(vec![(0, "gunner", 20, 20), (1, "barracks", 24, 20)]);
    let ids = sim.units().ids();
    assert_eq!(count(&sim, "rubble"), 0);

    destroy(&mut sim, ids[0], ids[1]);

    assert_eq!(count(&sim, "rubble"), 1, "the building left nothing behind");
}

#[test]
fn rubble_stands_where_the_building_did() {
    let mut sim = scenario(vec![(0, "gunner", 20, 20), (1, "barracks", 24, 20)]);
    let ids = sim.units().ids();
    destroy(&mut sim, ids[0], ids[1]);

    let kind = sim.rules().kind_of("rubble").unwrap();
    let at = sim
        .units()
        .iter()
        .find(|(_, u)| u.kind == kind)
        .map(|(_, u)| u.cell())
        .expect("no rubble");
    assert!(
        at.chebyshev_to(Cell::new(24, 20)) <= 3,
        "rubble appeared at {at:?}, nowhere near the building"
    );
}

#[test]
fn a_destroyed_building_still_frees_its_ground() {
    // Rubble is something to look at, not a wall. Leaving the footprint claimed
    // would mean a player could never rebuild where a building fell, which is
    // not what the original does and would be maddening.
    let mut sim = scenario(vec![(0, "gunner", 20, 20), (1, "barracks", 24, 20)]);
    let ids = sim.units().ids();
    destroy(&mut sim, ids[0], ids[1]);

    assert!(
        !sim.map().is_blocked(Cell::new(24, 20)),
        "the ground under a wreck is still ground"
    );
}

// -- Crew -------------------------------------------------------------------

#[test]
fn a_destroyed_vehicle_ejects_its_crew() {
    // The one that changes how a match is played: destroying a full transport
    // stops being the same as destroying an empty one.
    let mut sim = scenario(vec![(0, "gunner", 20, 20), (1, "tank", 24, 20)]);
    let ids = sim.units().ids();
    destroy(&mut sim, ids[0], ids[1]);

    assert_eq!(count(&sim, "crew"), 1, "nobody got out");
}

#[test]
fn the_crew_belongs_to_whoever_owned_the_vehicle() {
    // Survivors are the loser's consolation, not the winner's prize.
    let mut sim = scenario(vec![(0, "gunner", 20, 20), (1, "tank", 24, 20)]);
    let ids = sim.units().ids();
    destroy(&mut sim, ids[0], ids[1]);

    let kind = sim.rules().kind_of("crew").unwrap();
    let owner = sim
        .units()
        .iter()
        .find(|(_, u)| u.kind == kind)
        .map(|(_, u)| u.owner)
        .expect("no crew");
    assert_eq!(owner, PlayerId(1));
}

#[test]
fn a_crew_that_never_survives_does_not_appear() {
    // The chance has to be able to fail, or a vehicle is a free infantry squad
    // with a gun on top.
    let mut sim = scenario(vec![(0, "gunner", 20, 20), (1, "sealed_tank", 24, 20)]);
    let ids = sim.units().ids();
    destroy(&mut sim, ids[0], ids[1]);

    assert_eq!(count(&sim, "crew"), 0);
}

#[test]
fn a_vehicle_with_nothing_to_leave_leaves_nothing() {
    let mut sim = scenario(vec![(0, "gunner", 20, 20), (1, "gunner", 24, 20)]);
    let ids = sim.units().ids();
    destroy(&mut sim, ids[0], ids[1]);

    assert_eq!(count(&sim, "crew"), 0);
    assert_eq!(count(&sim, "rubble"), 0);
}

// -- The reactor ------------------------------------------------------------

#[test]
fn a_nuclear_reactor_explodes_when_destroyed() {
    let mut sim = scenario(vec![
        (0, "gunner", 20, 20),
        (1, "reactor", 24, 20),
        // Unarmed, so the only thing that can hurt it is the reactor going up.
        (1, "crew", 25, 20),
    ]);
    let ids = sim.units().ids();
    let bystander = ids[2];
    let before = sim.unit(bystander).map(|u| u.health).unwrap_or(0);

    destroy(&mut sim, ids[0], ids[1]);

    assert!(
        sim.unit(bystander).is_none_or(|u| u.health < before),
        "the reactor went quietly"
    );
}

#[test]
fn a_reactor_leaves_the_ground_dangerous() {
    // The blast is one consequence; the fallout is another, and it lasts. A
    // reactor whose only effect was the explosion would be a large bomb rather
    // than a place you have to stop going.
    let mut sim = scenario(vec![(0, "gunner", 20, 20), (1, "reactor", 26, 20)]);
    let ids = sim.units().ids();
    assert!(sim.hazards().is_empty());

    destroy(&mut sim, ids[0], ids[1]);

    assert!(
        sim.hazards().iter().any(|h| h.cell == Cell::new(26, 20)),
        "the ground where a reactor stood should not be safe"
    );
}

#[test]
fn a_working_reactor_poisons_nothing() {
    // Its trigger is dying. A reactor that irradiated its own base while
    // running would be a strange thing to build.
    let mut sim = scenario(vec![(1, "reactor", 26, 20)]);
    for _ in 0..100 {
        sim.tick(&[]);
    }

    assert!(sim.hazards().is_empty());
}

#[test]
fn the_fallout_eventually_clears() {
    let mut sim = scenario(vec![(0, "gunner", 20, 20), (1, "reactor", 26, 20)]);
    let ids = sim.units().ids();
    destroy(&mut sim, ids[0], ids[1]);
    assert!(!sim.hazards().is_empty());

    for _ in 0..400 {
        sim.tick(&[]);
    }

    assert!(sim.hazards().is_empty(), "the fallout never cleared");
}

#[test]
fn wreckage_is_deterministic() {
    let run = || {
        let mut sim = scenario(vec![
            (0, "gunner", 20, 20),
            (1, "tank", 24, 20),
            (1, "barracks", 24, 26),
            (1, "reactor", 30, 20),
        ]);
        let ids = sim.units().ids();
        destroy(&mut sim, ids[0], ids[1]);
        for _ in 0..100 {
            sim.tick(&[]);
        }
        sim.state_hash()
    };
    assert_eq!(run(), run());
}
