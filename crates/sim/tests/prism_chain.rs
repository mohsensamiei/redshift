//! A structure whose strength depends on what is standing next to it.
//!
//! Every other stat in the engine is resolved per kind, once, at match start.
//! A Prism Tower's damage is a fact about the world as it currently stands —
//! so, like the power grid, it is rebuilt from scratch every tick rather than
//! maintained. Anything incremental would need correcting on every build,
//! death, capture and sale, and the one that got missed would be a tower
//! quietly firing at the wrong strength for the rest of the match.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::EntityId;
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn armour() -> ArmourTable {
    ron::from_str(r#"( classes: ["none"], table: { "beam": { "none": 100 } } )"#).unwrap()
}

/// A tower that chains, optionally drawing power so a blackout can be tested.
fn tower(id: &str, draws_power: bool) -> EntityDef {
    let mut traits = vec![
        Trait::Health {
            max: 1_000,
            armour: "none".into(),
        },
        Trait::Vision {
            range: Hundredths(900),
        },
        Trait::Armed {
            weapon: "beam".into(),
            turret: true,
            turret_rate: 3600,
        },
        Trait::Chains {
            radius: Hundredths(500),
            bonus_percent: 50,
            max_supporters: 2,
        },
        Trait::Buildable {
            cost: 1_500,
            build_time: Ticks(40),
            prerequisites: vec![],
            produced_by: "plant".into(),
        },
    ];
    if draws_power {
        traits.push(Trait::PowerDraw {
            amount: 300,
            works_unpowered: false,
        });
    }
    EntityDef {
        id: id.into(),
        name_key: "structure.tower".into(),
        side: None,
        category: "structure".into(),
        traits,
    }
}

fn plant() -> EntityDef {
    EntityDef {
        id: "plant".into(),
        name_key: "structure.plant".into(),
        side: None,
        category: "structure".into(),
        traits: vec![
            Trait::Health {
                max: 400,
                armour: "none".into(),
            },
            Trait::Vision {
                range: Hundredths(300),
            },
            Trait::PowerSupply { output: 100 },
            Trait::Buildable {
                cost: 300,
                build_time: Ticks(10),
                prerequisites: vec![],
                produced_by: "plant".into(),
            },
        ],
    }
}

fn victim() -> EntityDef {
    EntityDef {
        id: "victim".into(),
        name_key: "unit.victim".into(),
        side: None,
        category: "vehicle".into(),
        traits: vec![
            Trait::Health {
                max: 100_000,
                armour: "none".into(),
            },
            Trait::Mobile {
                speed: Hundredths(1),
                turn_rate: 3600,
                locomotor: Locomotor::Wheeled,
                surfaces: None,
                size: None,
                layer: None,
            },
            Trait::Vision {
                range: Hundredths(100),
            },
        ],
    }
}

fn rules() -> Rules {
    Rules::from_parts(
        vec![
            tower("tower", false),
            tower("hungry_tower", true),
            plant(),
            victim(),
        ],
        vec![WeaponDef {
            id: "beam".into(),
            damage: 100,
            warhead: "beam".into(),
            reload: Ticks(10),
            range: Hundredths(800),
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
    let mut sim = Sim::new(MatchSetup {
        seed: 0x_9815,
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
    });
    sim.tick(&[]);
    sim
}

/// Damage the first tower deals to the victim over a fixed window.
///
/// The only honest way to ask how strong a beam is from outside.
fn output(sim: &mut Sim, shooter: EntityId, victim_id: EntityId) -> u32 {
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Attack {
            units: vec![shooter],
            target: victim_id,
        },
    )]);
    let before = sim.unit(victim_id).unwrap().health;
    for _ in 0..100 {
        sim.tick(&[]);
    }
    before - sim.unit(victim_id).unwrap().health
}

// -- Counting the chain -----------------------------------------------------

#[test]
fn a_lone_tower_has_no_support() {
    let sim = scenario(vec![(0, "tower", 10, 10)]);
    assert_eq!(sim.unit(sim.units().ids()[0]).unwrap().support, 0);
}

#[test]
fn a_different_kind_of_tower_does_not_count() {
    // Prism Towers chain with Prism Towers. An "everything of mine within
    // radius" count would have every defence in a base feeding every other.
    let sim = scenario(vec![
        (0, "tower", 10, 10),
        (0, "hungry_tower", 12, 10),
        (0, "plant", 20, 20),
        (0, "plant", 22, 20),
        (0, "plant", 24, 20),
        (0, "plant", 26, 20),
    ]);
    let ids = sim.units().ids();
    assert_eq!(sim.unit(ids[0]).unwrap().support, 0);
    assert_eq!(sim.unit(ids[1]).unwrap().support, 0);
}

#[test]
fn two_towers_side_by_side_feed_each_other() {
    let sim = scenario(vec![(0, "tower", 10, 10), (0, "tower", 13, 10)]);
    let ids = sim.units().ids();
    assert_eq!(sim.unit(ids[0]).unwrap().support, 1);
    assert_eq!(sim.unit(ids[1]).unwrap().support, 1);
}

#[test]
fn a_tower_too_far_away_feeds_nothing() {
    let sim = scenario(vec![(0, "tower", 10, 10), (0, "tower", 30, 10)]);
    let ids = sim.units().ids();
    assert_eq!(sim.unit(ids[0]).unwrap().support, 0);
}

#[test]
fn an_enemys_towers_do_not_help() {
    let sim = scenario(vec![(0, "tower", 10, 10), (1, "tower", 13, 10)]);
    let ids = sim.units().ids();
    assert_eq!(sim.unit(ids[0]).unwrap().support, 0);
    assert_eq!(sim.unit(ids[1]).unwrap().support, 0);
}

#[test]
fn support_is_capped() {
    // Without a ceiling, a player who can afford twenty towers in one corner
    // gets a weapon that nothing else in the game answers.
    let sim = scenario(vec![
        (0, "tower", 10, 10),
        (0, "tower", 12, 10),
        (0, "tower", 14, 10),
        (0, "tower", 10, 12),
        (0, "tower", 12, 12),
    ]);
    let support = sim.unit(sim.units().ids()[0]).unwrap().support;
    assert_eq!(support, 2, "the cap is two, and four are in range");
}

#[test]
fn a_dark_tower_feeds_nothing() {
    // One more reason to cut an enemy's power: it does not merely silence the
    // defences, it weakens the ones still firing.
    let sim = scenario(vec![
        (0, "hungry_tower", 10, 10),
        (0, "hungry_tower", 13, 10),
    ]);
    let ids = sim.units().ids();
    assert_eq!(
        sim.unit(ids[0]).unwrap().support,
        0,
        "an unpowered tower fed its neighbour"
    );
}

// -- What it changes --------------------------------------------------------

#[test]
fn a_supported_tower_hits_harder() {
    let mut alone = scenario(vec![(0, "tower", 10, 10), (1, "victim", 14, 10)]);
    let ids = alone.units().ids();
    let solo = output(&mut alone, ids[0], ids[1]);

    // The supporter stands five cells the *other* way: inside the chain radius
    // of five, and nine cells from the victim, which is outside its own reach
    // of eight. Otherwise this would measure two towers shooting rather than
    // one tower shooting harder — and would pass either way.
    let mut chained = scenario(vec![
        (0, "tower", 10, 10),
        (0, "tower", 5, 10),
        (1, "victim", 14, 10),
    ]);
    let ids = chained.units().ids();
    let paired = output(&mut chained, ids[0], ids[2]);

    assert!(solo > 0, "the lone tower never fired");
    // Fifty percent per supporter, one supporter.
    assert!(
        paired > solo,
        "a chained tower dealt {paired} where a lone one dealt {solo}"
    );
    let expected = solo + solo / 2;
    assert!(
        paired.abs_diff(expected) <= solo / 8,
        "expected about {expected}, got {paired}"
    );
}

#[test]
fn losing_the_neighbour_takes_the_bonus_away_again() {
    // The reason this is recomputed rather than maintained. A bonus that
    // outlived its source would be invisible and permanent.
    let mut sim = scenario(vec![
        (0, "tower", 10, 10),
        (0, "tower", 10, 13),
        (1, "victim", 14, 10),
    ]);
    let ids = sim.units().ids();
    assert_eq!(sim.unit(ids[0]).unwrap().support, 1);

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Sell { building: ids[1] },
    )]);
    for _ in 0..20 {
        sim.tick(&[]);
    }

    assert_eq!(
        sim.unit(ids[0]).unwrap().support,
        0,
        "the bonus outlived the tower that gave it"
    );
}

#[test]
fn chaining_is_deterministic() {
    let run = || {
        let mut sim = scenario(vec![
            (0, "tower", 10, 10),
            (0, "tower", 10, 13),
            (0, "tower", 13, 10),
            (1, "victim", 14, 12),
        ]);
        let ids = sim.units().ids();
        output(&mut sim, ids[0], ids[3]);
        sim.state_hash()
    };
    assert_eq!(run(), run());
}
