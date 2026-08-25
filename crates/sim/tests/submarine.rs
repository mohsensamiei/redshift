//! Submersion: the other concealment.
//!
//! Assuming the cloak already implemented covers this would be wrong in a way
//! that shows up nowhere except naval play. Three things differ:
//!
//! - Being **damaged** brings a submarine up, not just firing.
//! - A different sense finds it. A dog smells a spy and hears nothing at all
//!   under the water.
//! - It is what makes a submarine *unattackable* rather than merely unseen —
//!   and that falls out rather than being a rule, because targeting can only
//!   choose from what its owner can see.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Locomotor, Surface, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::EntityId;
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map, Terrain};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn armour() -> ArmourTable {
    ron::from_str(r#"( classes: ["none"], table: { "shot": { "none": 100 } } )"#).unwrap()
}

const RESURFACE: u32 = 40;

fn ship(id: &str, extra: Vec<Trait>) -> EntityDef {
    let mut traits = vec![
        Trait::Health {
            max: 1_000,
            armour: "none".into(),
        },
        Trait::Mobile {
            speed: Hundredths(300),
            turn_rate: 3600,
            locomotor: Locomotor::Ship,
            surfaces: Some(vec![Surface::Water]),
            size: None,
            layer: None,
        },
        Trait::Vision {
            range: Hundredths(800),
        },
    ];
    traits.extend(extra);
    EntityDef {
        id: id.into(),
        name_key: format!("unit.{id}"),
        side: None,
        category: "ship".into(),
        traits,
    }
}

fn rules() -> Rules {
    Rules::from_parts(
        vec![
            ship(
                "sub",
                vec![
                    Trait::Armed {
                        weapon: "torpedo".into(),
                        turret: true,
                        turret_rate: 3600,
                    },
                    Trait::Submersible {
                        resurface_delay: Ticks(RESURFACE),
                    },
                ],
            ),
            // Sees cloaked things, hears nothing. The dog of the sea, and the
            // whole reason the two senses are separate.
            ship(
                "lookout",
                vec![
                    Trait::Armed {
                        weapon: "gun".into(),
                        turret: true,
                        turret_rate: 3600,
                    },
                    Trait::Detector,
                ],
            ),
            // Hears. This is the answer to a submarine.
            ship(
                "destroyer",
                vec![
                    Trait::Armed {
                        weapon: "gun".into(),
                        turret: true,
                        turret_rate: 3600,
                    },
                    Trait::Sonar,
                ],
            ),
            ship("tender", vec![]),
            // Unarmed on purpose. In the damage test it is the only thing that
            // makes "being hit brought it up" mean being hit, rather than the
            // submarine shooting back and giving itself away.
            ship(
                "minisub",
                vec![Trait::Submersible {
                    resurface_delay: Ticks(RESURFACE),
                }],
            ),
        ],
        vec![
            WeaponDef {
                id: "torpedo".into(),
                damage: 60,
                warhead: "shot".into(),
                reload: Ticks(20),
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
            },
            WeaponDef {
                id: "gun".into(),
                damage: 30,
                warhead: "shot".into(),
                reload: Ticks(20),
                range: Hundredths(600),
                splash_radius: Hundredths::ZERO,
                projectile_speed: Hundredths::ZERO,
                homing: false,
                targets: vec![],
                instant_kill: false,
                ammo: 0,
                intercepts: false,
                target_categories: vec![],
                heals: false,
            },
        ],
        armour(),
        Vec::new(),
    )
    .expect("valid rules")
}

/// All water, so ships can be put anywhere.
fn sea() -> Map {
    let mut map = Map::new(48, 48);
    map.fill_rect(Cell::new(0, 0), Cell::new(47, 47), Terrain::Water);
    map
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
    let mut sim = Sim::new(MatchSetup {
        seed: 0x_5DB,
        map: sea(),
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
    });
    // Long enough for a fresh submarine to have gone under — which only
    // happens if it is left alone, so every scenario here starts ships seven
    // cells apart: inside each other's vision, outside every weapon's reach.
    // Otherwise the warm-up is a battle and nothing is ever submerged.
    for _ in 0..RESURFACE + 5 {
        sim.tick(&[]);
    }
    sim
}

fn seen_by(sim: &Sim, watcher: PlayerId, target: EntityId) -> bool {
    sim.unit(target).is_some_and(|u| sim.can_see(watcher, u))
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

// -- Being under ------------------------------------------------------------

#[test]
fn a_submarine_is_invisible_to_an_ordinary_ship() {
    let sim = scenario(vec![(0, "sub", 20, 20), (1, "tender", 27, 20)]);
    let ids = sim.units().ids();
    assert!(
        !seen_by(&sim, PlayerId(1), ids[0]),
        "a submarine was seen from the surface"
    );
}

#[test]
fn its_own_side_always_sees_it() {
    // Or a player could not command their own submarines, which is a rule the
    // original never needed to state either.
    let sim = scenario(vec![(0, "sub", 20, 20)]);
    let ids = sim.units().ids();
    assert!(seen_by(&sim, PlayerId(0), ids[0]));
}

#[test]
fn a_detector_that_only_sees_cloaks_finds_nothing() {
    // The whole reason the two senses are separate. A dog that can smell a spy
    // standing in front of it has no reason to hear a submarine.
    let sim = scenario(vec![(0, "sub", 20, 20), (1, "lookout", 27, 20)]);
    let ids = sim.units().ids();
    assert!(
        !seen_by(&sim, PlayerId(1), ids[0]),
        "a cloak detector heard a submarine"
    );
}

#[test]
fn sonar_finds_it() {
    let sim = scenario(vec![(0, "sub", 20, 20), (1, "destroyer", 27, 20)]);
    let ids = sim.units().ids();
    assert!(
        seen_by(&sim, PlayerId(1), ids[0]),
        "the sonar heard nothing"
    );
}

// -- Coming up --------------------------------------------------------------

#[test]
fn firing_brings_it_up() {
    let mut sim = scenario(vec![(0, "sub", 20, 20), (1, "tender", 27, 20)]);
    let ids = sim.units().ids();
    let (sub, victim) = (ids[0], ids[1]);
    assert!(
        !seen_by(&sim, PlayerId(1), sub),
        "it should start submerged"
    );

    // Ordered to attack, so it closes the last two cells and fires.
    attack(&mut sim, 0, sub, victim);
    for _ in 0..200 {
        sim.tick(&[]);
        if seen_by(&sim, PlayerId(1), sub) {
            break;
        }
    }

    assert!(
        seen_by(&sim, PlayerId(1), sub),
        "a submarine fired a torpedo and stayed hidden"
    );
}

#[test]
fn being_hit_brings_it_up_too() {
    // The difference from a cloak that matters most. A submarine caught by a
    // depth charge is exposed whether or not it shot back — which is what lets
    // a destroyer keep a contact once it has found one.
    //
    // The target is the *unarmed* submersible, so the only thing that could
    // have surfaced it is the hit.
    let mut sim = scenario(vec![(0, "minisub", 20, 20), (1, "destroyer", 27, 20)]);
    let ids = sim.units().ids();
    let (sub, destroyer) = (ids[0], ids[1]);
    assert!(
        sim.is_submerged(sim.unit(sub).unwrap()),
        "it should start submerged"
    );

    attack(&mut sim, 1, destroyer, sub);
    for _ in 0..300 {
        sim.tick(&[]);
        if sim.unit(sub).is_none_or(|u| u.health < 1_000) {
            break;
        }
    }

    let hurt = sim
        .unit(sub)
        .expect("the submarine should have survived a few shots");
    assert!(hurt.health < 1_000, "the destroyer never landed a shot");
    assert!(
        !sim.is_submerged(hurt),
        "a submarine that has just been hit should be on the surface"
    );
}

#[test]
fn it_goes_back_under_after_a_while() {
    // Otherwise one shot exposes it for the rest of the match, and a submarine
    // is a torpedo boat that can never fire twice.
    let mut sim = scenario(vec![(0, "sub", 20, 20), (1, "tender", 27, 20)]);
    let ids = sim.units().ids();
    let (sub, victim) = (ids[0], ids[1]);

    attack(&mut sim, 0, sub, victim);
    for _ in 0..200 {
        sim.tick(&[]);
        if seen_by(&sim, PlayerId(1), sub) {
            break;
        }
    }
    assert!(seen_by(&sim, PlayerId(1), sub));

    // Sent away, so it is neither firing nor being fired at. Stopping where it
    // is would not do: an idle armed unit still acquires whatever is in reach.
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Move {
            units: vec![sub],
            target: Cell::new(40, 40),
        },
    )]);
    for _ in 0..(RESURFACE * 6) {
        sim.tick(&[]);
    }

    assert!(
        sim.is_submerged(sim.unit(sub).unwrap()),
        "it should have gone back under"
    );
}

// -- What follows from it ---------------------------------------------------

#[test]
fn an_ordinary_ship_cannot_attack_what_it_cannot_see() {
    // Not a targeting rule of its own. Targeting can only choose from what its
    // owner can see, so "only a sonar ship can engage a submarine" falls out of
    // the concealment rather than being stated anywhere.
    let mut sim = scenario(vec![(0, "minisub", 20, 20), (1, "lookout", 27, 20)]);
    let ids = sim.units().ids();
    let (sub, lookout) = (ids[0], ids[1]);
    let before = sim.unit(sub).unwrap().health;

    attack(&mut sim, 1, lookout, sub);
    for _ in 0..200 {
        sim.tick(&[]);
    }

    assert_eq!(
        sim.unit(sub).unwrap().health,
        before,
        "something shot a submarine it could not see"
    );
}

#[test]
fn a_surfaced_submarine_can_be_shot_by_anything() {
    let mut sim = scenario(vec![
        (0, "sub", 20, 20),
        (1, "tender", 27, 20),
        (1, "lookout", 25, 20),
    ]);
    let ids = sim.units().ids();
    let (sub, victim, lookout) = (ids[0], ids[1], ids[2]);

    // It fires, which gives it away, and then the lookout can engage it.
    attack(&mut sim, 0, sub, victim);
    for _ in 0..200 {
        sim.tick(&[]);
        if seen_by(&sim, PlayerId(1), sub) {
            break;
        }
    }
    assert!(seen_by(&sim, PlayerId(1), sub));
    let before = sim.unit(sub).unwrap().health;
    attack(&mut sim, 1, lookout, sub);
    for _ in 0..80 {
        sim.tick(&[]);
    }

    assert!(
        sim.unit(sub).is_none_or(|u| u.health < before),
        "a surfaced submarine should be an ordinary target"
    );
}

#[test]
fn submersion_is_deterministic() {
    let run = || {
        let mut sim = scenario(vec![
            (0, "sub", 20, 20),
            (1, "destroyer", 27, 20),
            (1, "tender", 26, 20),
        ]);
        let ids = sim.units().ids();
        attack(&mut sim, 0, ids[0], ids[2]);
        for _ in 0..300 {
            sim.tick(&[]);
        }
        sim.state_hash()
    };
    assert_eq!(run(), run());
}
