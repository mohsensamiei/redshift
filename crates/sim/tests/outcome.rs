//! When a match is over.
//!
//! Two ways for it to be. One player left standing is the obvious one. The
//! other is that nobody *can* win — two sides with nothing left to build and no
//! way to reach each other will sit there until somebody gets bored, and
//! "somebody got bored" is not a result.
//!
//! The stalemate rule approximates "can these two still reach each other" with
//! a long silence. Answering it honestly means a reachability search per player
//! per tick, over a map with bridges that can be cut and buildings that come
//! and go; a five-minute quiet means the same thing in practice and costs a
//! comparison.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::EntityId;
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, Outcome, PlayerSetup, Sim, Spawn};

/// Matches the constant in the simulation.
const QUIET: u32 = 20 * 60 * 5;

fn armour() -> ArmourTable {
    ron::from_str(r#"( classes: ["none"], table: { "shot": { "none": 100 } } )"#).unwrap()
}

fn rules() -> Rules {
    Rules::from_parts(
        vec![
            EntityDef {
                id: "gunner".into(),
                name_key: "unit.gunner".into(),
                side: None,
                category: "infantry".into(),
                traits: vec![
                    Trait::Health {
                        max: 100,
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
                        weapon: "rifle".into(),
                        turret: true,
                        turret_rate: 3600,
                    },
                ],
            },
            // Unarmed and immobile: two of these on opposite corners is exactly
            // the situation a stalemate rule exists for.
            EntityDef {
                id: "bunker".into(),
                name_key: "structure.bunker".into(),
                side: None,
                category: "structure".into(),
                traits: vec![
                    Trait::Health {
                        max: 500,
                        armour: "none".into(),
                    },
                    Trait::Vision {
                        range: Hundredths(300),
                    },
                ],
            },
            EntityDef {
                id: "factory".into(),
                name_key: "structure.factory".into(),
                side: None,
                category: "structure".into(),
                traits: vec![
                    Trait::Health {
                        max: 500,
                        armour: "none".into(),
                    },
                    Trait::Vision {
                        range: Hundredths(300),
                    },
                    Trait::Produces {
                        categories: vec!["infantry".into()],
                    },
                ],
            },
            // Builds nothing itself, and becomes something that does. A player
            // reduced to one of these is behind, not beaten.
            EntityDef {
                id: "mcv".into(),
                name_key: "unit.mcv".into(),
                side: None,
                category: "vehicle".into(),
                traits: vec![
                    Trait::Health {
                        max: 500,
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
                        range: Hundredths(400),
                    },
                    Trait::Deploys {
                        into: "factory".into(),
                    },
                ],
            },
        ],
        vec![WeaponDef {
            id: "rifle".into(),
            damage: 50,
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
            mind_control: false,
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
        seed: 0x_0C0,
        map: Map::new(64, 64),
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

fn run(sim: &mut Sim, ticks: u32) {
    for _ in 0..ticks {
        sim.tick(&[]);
    }
}

fn attack(sim: &mut Sim, owner: u8, unit: EntityId, target: EntityId) {
    sim.tick(&[Command::new(
        PlayerId(owner),
        0,
        CommandKind::Attack {
            units: vec![unit],
            target,
        },
    )]);
}

// -- Victory ----------------------------------------------------------------

#[test]
fn a_match_in_progress_has_no_outcome() {
    let mut sim = scenario(vec![(0, "bunker", 10, 10), (1, "bunker", 50, 50)]);
    run(&mut sim, 100);
    assert_eq!(sim.outcome(), None);
}

#[test]
fn the_last_player_standing_wins() {
    let mut sim = scenario(vec![(0, "gunner", 10, 10), (1, "bunker", 13, 10)]);
    let ids = sim.units().ids();

    attack(&mut sim, 0, ids[0], ids[1]);
    for _ in 0..600 {
        sim.tick(&[]);
        if sim.outcome().is_some() {
            break;
        }
    }

    assert_eq!(sim.outcome(), Some(Outcome::Victory(PlayerId(0))));
}

#[test]
fn an_outcome_is_final() {
    // Whatever happens afterwards, the match ended when it ended. A result
    // that could be revised is not a result.
    let mut sim = scenario(vec![(0, "gunner", 10, 10), (1, "bunker", 13, 10)]);
    let ids = sim.units().ids();
    attack(&mut sim, 0, ids[0], ids[1]);
    for _ in 0..600 {
        sim.tick(&[]);
        if sim.outcome().is_some() {
            break;
        }
    }
    let decided = sim.outcome();

    let kind = sim.rules().kind_of("bunker").unwrap();
    sim.spawn_unit(PlayerId(1), kind, Cell::new(50, 50).centre());
    run(&mut sim, 100);

    assert_eq!(sim.outcome(), decided);
}

// -- Stalemate --------------------------------------------------------------

#[test]
fn two_players_who_can_neither_build_nor_fight_are_stalemated() {
    // Both sides hold a bunker in opposite corners. Neither can produce
    // anything and neither can hurt the other, so this is the situation the
    // rule exists for.
    let mut sim = scenario(vec![(0, "bunker", 5, 5), (1, "bunker", 58, 58)]);
    run(&mut sim, QUIET / 2);
    assert_eq!(sim.outcome(), None, "called too early");

    run(&mut sim, QUIET);

    assert_eq!(sim.outcome(), Some(Outcome::Stalemate));
}

#[test]
fn a_player_who_can_still_build_is_not_stalemated() {
    // However quiet it is. Somebody with a factory is one build order away
    // from an army, and ending their match for them would be wrong.
    let mut sim = scenario(vec![(0, "factory", 5, 5), (1, "bunker", 58, 58)]);
    run(&mut sim, QUIET * 2);
    assert_eq!(sim.outcome(), None);
}

#[test]
fn a_player_reduced_to_an_mcv_is_behind_but_not_beaten() {
    // An MCV builds nothing and becomes something that does. Counting only
    // finished producers would call a stalemate on a player who is one keypress
    // from a base.
    let mut sim = scenario(vec![(0, "mcv", 5, 5), (1, "bunker", 58, 58)]);
    run(&mut sim, QUIET * 2);
    assert_eq!(sim.outcome(), None);
}

#[test]
fn fighting_keeps_the_match_alive() {
    // Both halves of the rule are needed. Two players who cannot build might
    // still be mid-battle, and a quiet timer alone would end it under them.
    let mut sim = scenario(vec![
        (0, "gunner", 20, 20),
        (1, "bunker", 23, 20),
        (1, "bunker", 50, 50),
    ]);
    let ids = sim.units().ids();
    attack(&mut sim, 0, ids[0], ids[1]);
    run(&mut sim, QUIET / 2);

    // Still shooting the second bunker, so the clock keeps resetting.
    let remaining: Vec<EntityId> = sim
        .units()
        .ids()
        .into_iter()
        .filter(|i| sim.unit(*i).is_some_and(|u| u.owner == PlayerId(1)))
        .collect();
    if let Some(next) = remaining.first() {
        attack(&mut sim, 0, ids[0], *next);
    }
    run(&mut sim, 100);

    assert_eq!(sim.outcome(), None, "a match being played was called off");
}

#[test]
fn outcomes_are_deterministic() {
    let go = || {
        let mut sim = scenario(vec![(0, "bunker", 5, 5), (1, "bunker", 58, 58)]);
        run(&mut sim, QUIET + 200);
        (sim.state_hash(), sim.outcome())
    };
    assert_eq!(go(), go());
}
