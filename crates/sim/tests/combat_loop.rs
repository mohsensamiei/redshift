//! Combat as it runs inside the tick loop.
//!
//! The unit tests in `combat.rs` cover targeting and the damage table in
//! isolation. These check the parts only the loop can show: that shots are
//! collected before any are applied, that a dying unit still gets to fire, and
//! that none of it costs determinism.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::EntityId;
use redshift_sim::command::PlayerId;
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn rules() -> Rules {
    let armour: ArmourTable = ron::from_str(
        r#"(
            classes: ["none", "heavy"],
            table: {
                "small_arms": { "none": 100, "heavy": 10 },
                "ap_shell":   { "none": 40,  "heavy": 100 },
                "blast":      { "none": 100, "heavy": 50 },
            },
        )"#,
    )
    .expect("armour");

    let weapons = vec![
        WeaponDef {
            id: "rifle".into(),
            damage: 20,
            warhead: "small_arms".into(),
            reload: Ticks(10),
            range: Hundredths(500),
            splash_radius: Hundredths::ZERO,
            projectile_speed: Hundredths::ZERO,
        },
        WeaponDef {
            id: "cannon".into(),
            damage: 50,
            warhead: "ap_shell".into(),
            reload: Ticks(20),
            range: Hundredths(600),
            splash_radius: Hundredths::ZERO,
            projectile_speed: Hundredths::ZERO,
        },
        WeaponDef {
            id: "mortar".into(),
            damage: 30,
            warhead: "blast".into(),
            reload: Ticks(30),
            range: Hundredths(800),
            // Wide enough to catch a neighbouring cell.
            splash_radius: Hundredths(150),
            projectile_speed: Hundredths(800),
        },
    ];

    let soldier = |id: &str, weapon: Option<&str>, health: u32| EntityDef {
        id: id.into(),
        name_key: format!("unit.{id}"),
        side: None,
        category: "infantry".into(),
        traits: {
            let mut t = vec![
                Trait::Health {
                    max: health,
                    armour: "none".into(),
                },
                Trait::Mobile {
                    speed: Hundredths(200),
                    turn_rate: 3600,
                    locomotor: Locomotor::Foot,
                },
                Trait::Vision {
                    range: Hundredths(900),
                },
            ];
            if let Some(w) = weapon {
                t.push(Trait::Armed {
                    weapon: w.into(),
                    turret: true,
                    turret_rate: 3600,
                });
            }
            t
        },
    };

    let entities = vec![
        soldier("rifleman", Some("rifle"), 100),
        // Health exactly equal to one rifle shot, so the first exchange is
        // fatal both ways. Lets the mutual-destruction test set itself up
        // through the normal spawn path rather than reaching into the world.
        soldier("duellist", Some("rifle"), 20),
        soldier("gunner", Some("cannon"), 100),
        soldier("mortarman", Some("mortar"), 100),
        soldier("civilian", None, 100),
    ];

    Rules::from_parts(entities, weapons, armour, Vec::new()).expect("valid rules")
}

fn setup(spawns: Vec<(u8, &str, i32, i32)>) -> MatchSetup {
    let rules = rules();
    let mut players: Vec<u8> = spawns.iter().map(|(p, ..)| *p).collect();
    players.sort();
    players.dedup();

    MatchSetup {
        seed: 0x5EED,
        map: Map::new(32, 32),
        players: players
            .into_iter()
            .map(|id| PlayerSetup {
                id: PlayerId(id),
                faction: None,
            })
            .collect(),
        spawns: spawns
            .into_iter()
            .map(|(owner, kind, x, y)| Spawn {
                owner: PlayerId(owner),
                kind: rules
                    .kind_of(kind)
                    .unwrap_or_else(|| panic!("no kind {kind}")),
                pos: Cell::new(x, y).centre(),
            })
            .collect(),
        rules,
    }
}

fn health_of(sim: &Sim, id: EntityId) -> Option<u32> {
    sim.units().get(id).map(|u| u.health)
}

#[test]
fn enemies_in_range_shoot_each_other_without_being_ordered_to() {
    let mut sim = Sim::new(setup(vec![(0, "rifleman", 5, 5), (1, "rifleman", 7, 5)]));
    let ids = sim.units().ids();
    let (a, b) = (ids[0], ids[1]);

    for _ in 0..40 {
        sim.tick(&[]);
    }

    assert!(
        health_of(&sim, a).is_none_or(|h| h < 100),
        "the first unit took no damage"
    );
    assert!(
        health_of(&sim, b).is_none_or(|h| h < 100),
        "the second unit took no damage"
    );
}

#[test]
fn an_unarmed_unit_is_shot_and_never_shoots_back() {
    let mut sim = Sim::new(setup(vec![(0, "rifleman", 5, 5), (1, "civilian", 6, 5)]));
    let ids = sim.units().ids();
    let (shooter, victim) = (ids[0], ids[1]);

    for _ in 0..200 {
        sim.tick(&[]);
    }

    assert_eq!(
        health_of(&sim, shooter),
        Some(100),
        "an unarmed victim cannot fight back"
    );
    assert!(
        health_of(&sim, victim).is_none(),
        "the victim should be destroyed"
    );
}

#[test]
fn allies_do_not_shoot_each_other() {
    let mut sim = Sim::new(setup(vec![(0, "rifleman", 5, 5), (0, "rifleman", 6, 5)]));
    for _ in 0..100 {
        sim.tick(&[]);
    }
    assert_eq!(sim.units().len(), 2, "friendly units destroyed each other");
    for (_, unit) in sim.units().iter() {
        assert_eq!(unit.health, 100, "a friendly unit was damaged");
    }
}

#[test]
fn nothing_happens_out_of_range() {
    // Two enemies at opposite corners, well beyond any weapon here.
    let mut sim = Sim::new(setup(vec![(0, "rifleman", 2, 2), (1, "rifleman", 29, 29)]));
    for _ in 0..100 {
        sim.tick(&[]);
    }
    assert_eq!(sim.units().len(), 2);
    for (_, unit) in sim.units().iter() {
        assert_eq!(unit.health, 100);
    }
}

#[test]
fn armour_decides_who_wins() {
    // The counterplay, played out rather than asserted on a table. A cannon
    // and a rifle trade shots; the cannon's warhead is the wrong one for
    // unarmoured infantry, so the rifle should come out ahead.
    let mut sim = Sim::new(setup(vec![(0, "rifleman", 5, 5), (1, "gunner", 7, 5)]));
    let ids = sim.units().ids();
    let (rifle, cannon) = (ids[0], ids[1]);

    for _ in 0..60 {
        sim.tick(&[]);
    }

    let rifle_health = health_of(&sim, rifle);
    let cannon_health = health_of(&sim, cannon);
    assert!(
        rifle_health > cannon_health,
        "rifle {rifle_health:?} should be ahead of cannon {cannon_health:?} against soft targets"
    );
}

#[test]
fn evenly_matched_units_can_destroy_each_other_in_the_same_tick() {
    // Deaths are resolved after all damage, so a unit that dies this tick still
    // got to fire. Removing the dead mid-pass would make whoever the arena
    // visited first the automatic winner — a silent, arena-order-dependent
    // advantage that would show up as "the left player always wins ties".
    let mut sim = Sim::new(setup(vec![(0, "duellist", 5, 5), (1, "duellist", 6, 5)]));

    for _ in 0..30 {
        sim.tick(&[]);
        if sim.units().is_empty() {
            break;
        }
    }

    assert_eq!(
        sim.units().len(),
        0,
        "one survived, so the loser was removed before it could fire back"
    );
}

#[test]
fn splash_catches_neighbours_including_friendly_ones() {
    // Sparing friendly units would make artillery strictly better than it
    // should be, and the original did not spare them either.
    let mut sim = Sim::new(setup(vec![
        (0, "mortarman", 5, 5),
        (1, "civilian", 12, 5),
        // Standing right next to the target, and on the firer's own side.
        (0, "civilian", 12, 6),
    ]));
    let ids = sim.units().ids();
    let bystander = ids[2];

    for _ in 0..80 {
        sim.tick(&[]);
    }

    let health = health_of(&sim, bystander);
    assert!(
        health.is_none_or(|h| h < 100),
        "the friendly bystander should have been caught by splash, had {health:?}"
    );
}

#[test]
fn reload_paces_the_shots() {
    // A rifle reloading every 10 ticks must not empty a 100-health target in
    // fewer than four shots' worth of time. Catching a reload that never
    // decrements matters: the unit would fire every tick and everything would
    // die instantly.
    let mut sim = Sim::new(setup(vec![(0, "rifleman", 5, 5), (1, "civilian", 6, 5)]));
    let victim = sim.units().ids()[1];

    let mut ticks_to_kill = None;
    for tick in 0..500 {
        sim.tick(&[]);
        if health_of(&sim, victim).is_none() {
            ticks_to_kill = Some(tick);
            break;
        }
    }

    let ticks = ticks_to_kill.expect("the victim should eventually die");
    // 100 health at 20 damage is five shots, at ten ticks apart.
    assert!(
        ticks >= 40,
        "killed in {ticks} ticks, which is faster than the reload allows"
    );
}

#[test]
fn combat_is_deterministic() {
    // The whole point. A messy skirmish, run twice, must land in exactly the
    // same place — combat introduces target selection and death ordering, both
    // of which are easy to make arena-order dependent.
    let spawns = || {
        let mut v = Vec::new();
        for i in 0..8i32 {
            v.push((0u8, "rifleman", 4 + i % 4, 4 + i / 4));
            v.push((1u8, "gunner", 8 + i % 4, 5 + i / 4));
            v.push((0u8, "mortarman", 3 + i % 2, 9 + i / 2));
        }
        v
    };

    let run = || {
        let mut sim = Sim::new(setup(spawns()));
        let mut hashes = Vec::new();
        for _ in 0..300 {
            sim.tick(&[]);
            hashes.push(sim.state_hash());
        }
        (hashes, sim.units().len())
    };

    let (first, first_alive) = run();
    let (second, second_alive) = run();

    assert_eq!(first, second, "two runs of the same battle diverged");
    assert_eq!(first_alive, second_alive);
    assert!(
        first_alive < 24,
        "nobody died in 300 ticks, so this proves nothing"
    );
}

#[test]
fn a_restored_match_still_has_weapons() {
    // The combat tables are derived from the rules, which makes them tempting
    // to skip when serialising. Skipping them would leave a restored match with
    // no weapons at all: every unit would quietly stop shooting, with nothing
    // to say why.
    let mut sim = Sim::new(setup(vec![(0, "rifleman", 5, 5), (1, "civilian", 6, 5)]));
    for _ in 0..15 {
        sim.tick(&[]);
    }

    let encoded = ron::to_string(&sim).expect("serialise");
    let mut restored: Sim = ron::from_str(&encoded).expect("deserialise");

    assert_eq!(restored.state_hash(), sim.state_hash());

    for _ in 0..60 {
        sim.tick(&[]);
        restored.tick(&[]);
    }
    assert_eq!(
        restored.state_hash(),
        sim.state_hash(),
        "the restored match diverged, which means something was not carried across"
    );
    assert!(
        restored.units().len() < 2,
        "the restored match stopped fighting"
    );
}
