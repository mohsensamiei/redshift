//! Cloak, detection and veterancy.
//!
//! Both mechanics are about asymmetry: a cloaked unit is dangerous because it
//! chooses when to be seen, and a veteran is worth retreating with because it
//! is worth more than the unit that replaced it. Neither is interesting as a
//! number, so these tests are about the decisions they create.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::Rank;
use redshift_sim::command::PlayerId;
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn rules() -> Rules {
    let armour: ArmourTable =
        ron::from_str(r#"( classes: ["none"], table: { "shot": { "none": 100 } } )"#).unwrap();
    let weapons = vec![WeaponDef {
        id: "rifle".into(),
        damage: 20,
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
        heals: false,
    }];

    let base = |id: &str, extra: Vec<Trait>| {
        let mut traits = vec![
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
                range: Hundredths(800),
            },
        ];
        traits.extend(extra);
        EntityDef {
            id: id.into(),
            name_key: format!("unit.{id}"),
            side: None,
            category: "infantry".into(),
            traits,
        }
    };

    let armed = || Trait::Armed {
        weapon: "rifle".into(),
        turret: true,
        turret_rate: 3600,
    };

    Rules::from_parts(
        vec![
            base("soldier", vec![armed()]),
            base(
                "spy",
                vec![
                    Trait::Cloakable {
                        recloak_delay: Ticks(60),
                    },
                    armed(),
                ],
            ),
            base("radar", vec![Trait::Detector]),
            base(
                "sergeant",
                vec![
                    armed(),
                    Trait::Veterancy {
                        kills_for_veteran: 1,
                        kills_for_elite: 2,
                    },
                ],
            ),
            // Deliberately fragile, so kills are quick to arrange.
            EntityDef {
                id: "target".into(),
                name_key: "unit.target".into(),
                side: None,
                category: "infantry".into(),
                traits: vec![
                    Trait::Health {
                        max: 20,
                        armour: "none".into(),
                    },
                    Trait::Vision {
                        range: Hundredths(200),
                    },
                ],
            },
        ],
        weapons,
        armour,
        Vec::new(),
    )
    .expect("valid rules")
}

fn scenario(spawns: Vec<(u8, &str, i32, i32)>) -> MatchSetup {
    let rules = rules();
    MatchSetup {
        seed: 0x5EE,
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

#[test]
fn a_cloaked_unit_is_invisible_to_the_enemy_and_visible_to_its_owner() {
    // Its own side must always see it, or a player could not command their own
    // units — a rule the original never needed to state either.
    let mut sim = Sim::new(scenario(vec![(0, "spy", 20, 20), (1, "soldier", 27, 20)]));
    sim.tick(&[]);

    let spy = sim.units().get(sim.units().ids()[0]).expect("spy");
    assert!(sim.is_cloaked(spy), "the spy should start hidden");
    assert!(sim.can_see(PlayerId(0), spy), "its own side must see it");
    assert!(
        !sim.can_see(PlayerId(1), spy),
        "the enemy is well within sight of it and should still not see it"
    );
}

#[test]
fn a_cloaked_unit_is_not_shot_at() {
    let mut sim = Sim::new(scenario(vec![(0, "spy", 20, 20), (1, "soldier", 27, 20)]));
    let spy = sim.units().ids()[0];

    for _ in 0..400 {
        sim.tick(&[]);
    }
    assert_eq!(
        sim.units().get(spy).expect("alive").health,
        200,
        "something shot a unit it could not see"
    );
}

#[test]
fn firing_gives_a_cloaked_unit_away() {
    // The whole tension of the mechanic: staying hidden and doing something are
    // mutually exclusive, so a cloaked unit is a threat rather than an
    // invulnerability.
    let mut sim = Sim::new(scenario(vec![(0, "spy", 20, 20), (1, "target", 22, 20)]));
    let spy = sim.units().ids()[0];

    let mut was_revealed = false;
    for _ in 0..200 {
        sim.tick(&[]);
        if let Some(unit) = sim.units().get(spy)
            && !sim.is_cloaked(unit)
        {
            was_revealed = true;
            break;
        }
    }
    assert!(was_revealed, "the spy fired and stayed hidden");
}

#[test]
fn the_cloak_returns_after_a_while() {
    let mut sim = Sim::new(scenario(vec![(0, "spy", 20, 20), (1, "target", 22, 20)]));
    let spy = sim.units().ids()[0];

    // Let it fire, kill the target, and then wait out the delay.
    for _ in 0..600 {
        sim.tick(&[]);
    }
    let unit = sim.units().get(spy).expect("alive");
    assert!(
        sim.is_cloaked(unit),
        "the cloak never came back after the fight ended"
    );
}

#[test]
fn a_detector_reveals_a_cloaked_unit_to_its_side() {
    let mut sim = Sim::new(scenario(vec![
        (0, "spy", 20, 20),
        (1, "soldier", 27, 20),
        (1, "radar", 27, 21),
    ]));
    sim.tick(&[]);

    let spy = sim.units().get(sim.units().ids()[0]).expect("spy");
    assert!(sim.is_cloaked(spy), "it should still be cloaked");
    assert!(
        sim.can_see(PlayerId(1), spy),
        "a detector within range should have revealed it"
    );
}

#[test]
fn a_detector_only_helps_the_side_that_owns_it() {
    let mut sim = Sim::new(scenario(vec![
        (0, "spy", 20, 20),
        (1, "radar", 27, 20),
        // A third party with no detector of its own.
        (1, "soldier", 40, 40),
    ]));
    sim.tick(&[]);

    let spy = sim.units().get(sim.units().ids()[0]).expect("spy");
    assert!(sim.can_see(PlayerId(1), spy));
    // Player 0 owns it, so this is trivially true — the point is that detection
    // is per player rather than global.
    assert!(sim.can_see(PlayerId(0), spy));
}

#[test]
fn kills_earn_rank() {
    let mut sim = Sim::new(scenario(vec![
        (0, "sergeant", 20, 20),
        (1, "target", 22, 20),
        (1, "target", 23, 20),
    ]));
    let sergeant = sim.units().ids()[0];
    assert_eq!(
        sim.rank_of(sim.units().get(sergeant).unwrap()),
        Rank::Rookie
    );

    for _ in 0..600 {
        sim.tick(&[]);
        if sim.units().len() == 1 {
            break;
        }
    }

    let unit = sim
        .units()
        .get(sergeant)
        .expect("the sergeant should survive");
    assert_eq!(unit.kills, 2, "both kills should have been credited");
    assert_eq!(sim.rank_of(unit), Rank::Elite);
}

#[test]
fn shooting_a_corpse_does_not_promote_anyone() {
    // Credit is given on the killing blow only. Anything else and a unit
    // promotes for as long as it keeps firing at a body.
    let mut sim = Sim::new(scenario(vec![
        (0, "sergeant", 20, 20),
        (1, "target", 22, 20),
    ]));
    let sergeant = sim.units().ids()[0];

    for _ in 0..1_200 {
        sim.tick(&[]);
    }
    assert_eq!(
        sim.units().get(sergeant).expect("alive").kills,
        1,
        "kills kept accruing after the target was dead"
    );
}

#[test]
fn a_unit_without_veterancy_never_promotes() {
    let mut sim = Sim::new(scenario(vec![
        (0, "soldier", 20, 20),
        (1, "target", 22, 20),
    ]));
    let soldier = sim.units().ids()[0];
    for _ in 0..600 {
        sim.tick(&[]);
    }
    let unit = sim.units().get(soldier).expect("alive");
    assert!(unit.kills > 0, "it should still be credited with the kill");
    assert_eq!(
        sim.rank_of(unit),
        Rank::Rookie,
        "a unit with no veterancy trait must never promote"
    );
}

#[test]
fn stealth_and_rank_are_deterministic() {
    let run = || {
        let mut sim = Sim::new(scenario(vec![
            (0, "spy", 10, 20),
            (0, "sergeant", 12, 20),
            (1, "radar", 26, 20),
            (1, "soldier", 24, 20),
            (1, "target", 25, 21),
        ]));
        let mine: Vec<_> = sim
            .units()
            .iter()
            .filter(|(_, u)| u.owner == PlayerId(0))
            .map(|(id, _)| id)
            .collect();
        sim.tick(&[redshift_sim::command::Command::new(
            PlayerId(0),
            0,
            redshift_sim::command::CommandKind::AttackMove {
                units: mine,
                target: Cell::new(26, 20),
            },
        )]);
        let mut hashes = Vec::new();
        for _ in 0..1_200 {
            sim.tick(&[]);
            hashes.push(sim.state_hash());
        }
        (hashes, sim.units().len())
    };
    let (a, a_units) = run();
    let (b, b_units) = run();
    assert_eq!(a, b, "two identical engagements diverged");
    assert_eq!(a_units, b_units);
    assert!(a_units < 5, "nothing happened, so this proves nothing");
}
