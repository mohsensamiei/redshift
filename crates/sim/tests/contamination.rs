//! Ground that has had something done to it.
//!
//! The deployed Desolator is the reason this exists, and the reason it is a map
//! effect rather than a weapon. A weapon picks a target, needs to see it, and
//! stops when its owner dies. Contamination does none of those things: it
//! denies an *area*, and it outlives whatever laid it — which is the only
//! reason denying the area is worth doing.
//!
//! Note what is deliberately absent: there is no "immune to radiation" flag.
//! The armour table already answers that question, so a warhead that does
//! nothing to vehicle armour makes ground infantry die on and a tank drives
//! across, with no second mechanism to disagree with the first.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Contaminate, Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::EntityId;
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

/// Radiation kills infantry and does nothing at all to armour. One row of a
/// table, and the whole of "who is immune".
fn armour() -> ArmourTable {
    ron::from_str(
        r#"( classes: ["flesh", "steel"],
             table: { "radiation": { "flesh": 100, "steel": 0 },
                      "shot": { "flesh": 100, "steel": 100 } } )"#,
    )
    .unwrap()
}

fn walker(id: &str, armour_class: &str, extra: Vec<Trait>) -> EntityDef {
    let mut traits = vec![
        Trait::Health {
            max: 400,
            armour: armour_class.into(),
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
            range: Hundredths(600),
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
}

/// The pair the feature exists for: it walks, and when it plants itself it
/// poisons everything around it.
fn desolator() -> Vec<EntityDef> {
    vec![
        walker(
            "desolator",
            "flesh",
            vec![Trait::Deploys {
                into: "desolator_dug_in".into(),
            }],
        ),
        EntityDef {
            traits: vec![
                Trait::Health {
                    max: 400,
                    armour: "flesh".into(),
                },
                Trait::Vision {
                    range: Hundredths(600),
                },
                Trait::Contaminates {
                    radius: Hundredths(250),
                    damage: 10,
                    warhead: "radiation".into(),
                    // Long enough that the ground stays denied after the
                    // Desolator itself is gone.
                    lingers: Ticks(100),
                    when: Contaminate::WhileStanding,
                },
                Trait::Deploys {
                    into: "desolator".into(),
                },
            ],
            ..walker("desolator_dug_in", "flesh", vec![])
        },
    ]
}

fn rules() -> Rules {
    let mut entities = desolator();
    entities.push(walker("infantryman", "flesh", vec![]));
    entities.push(EntityDef {
        category: "vehicle".into(),
        ..walker("tank", "steel", vec![])
    });
    entities.push(EntityDef {
        traits: vec![
            Trait::Health {
                max: 400,
                armour: "flesh".into(),
            },
            Trait::Mobile {
                speed: Hundredths(300),
                turn_rate: 3600,
                locomotor: Locomotor::Air,
                surfaces: None,
                size: None,
                layer: Some(redshift_data::traits::Layer::Air),
            },
            Trait::Vision {
                range: Hundredths(600),
            },
        ],
        ..walker("helicopter", "flesh", vec![])
    });
    Rules::from_parts(
        entities,
        vec![WeaponDef {
            id: "rifle".into(),
            damage: 10,
            warhead: "shot".into(),
            reload: Ticks(10),
            range: Hundredths(400),
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
        seed: 0x_C0FF,
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

fn deploy(sim: &mut Sim, unit: EntityId) {
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Deploy { units: vec![unit] },
    )]);
}

// -- Laying it --------------------------------------------------------------

#[test]
fn a_deployed_desolator_poisons_the_ground_around_it() {
    let mut sim = scenario(vec![(0, "desolator", 20, 20)]);
    let id = sim.units().ids()[0];
    assert!(
        sim.hazards().is_empty(),
        "nothing is poisoned to begin with"
    );

    deploy(&mut sim, id);
    sim.tick(&[]);

    assert!(!sim.hazards().is_empty(), "the ground should be dangerous");
    assert!(
        sim.hazards().iter().any(|h| h.cell == Cell::new(21, 20)),
        "the patch should reach a neighbour"
    );
}

#[test]
fn an_undeployed_desolator_poisons_nothing() {
    // Which is what makes deploying a decision. Contamination is a property of
    // the deployed *entity*, not a mode the engine has to remember.
    let mut sim = scenario(vec![(0, "desolator", 20, 20)]);
    for _ in 0..40 {
        sim.tick(&[]);
    }
    assert!(sim.hazards().is_empty());
}

#[test]
fn the_patch_is_round_rather_than_square() {
    // A square patch would be visibly a square, and a radius would mean two
    // different things along an axis and along a diagonal.
    let mut sim = scenario(vec![(0, "desolator", 20, 20)]);
    let id = sim.units().ids()[0];
    deploy(&mut sim, id);
    sim.tick(&[]);

    let hot = |c: Cell| sim.hazards().iter().any(|h| h.cell == c);
    // Radius 2.5: two cells along an axis are in, two along both axes are out.
    assert!(
        hot(Cell::new(22, 20)),
        "two cells straight out should be in"
    );
    assert!(!hot(Cell::new(22, 22)), "two cells diagonally is further");
}

// -- Standing in it ---------------------------------------------------------

#[test]
fn infantry_standing_in_it_are_hurt() {
    let mut sim = scenario(vec![(0, "desolator", 20, 20), (1, "infantryman", 21, 20)]);
    let ids = sim.units().ids();
    let victim = ids[1];
    let start = sim.unit(victim).unwrap().health;

    deploy(&mut sim, ids[0]);
    for _ in 0..10 {
        sim.tick(&[]);
    }

    assert!(
        sim.unit(victim).is_none_or(|u| u.health < start),
        "poisoned ground did nothing"
    );
}

#[test]
fn armour_decides_who_cares() {
    // The reason there is no immunity flag. Radiation does nothing to steel,
    // which is one number in the table already used by every other weapon.
    let mut sim = scenario(vec![(0, "desolator", 20, 20), (1, "tank", 21, 20)]);
    let ids = sim.units().ids();
    let tank = ids[1];
    let start = sim.unit(tank).unwrap().health;

    deploy(&mut sim, ids[0]);
    for _ in 0..60 {
        sim.tick(&[]);
    }

    assert_eq!(
        sim.unit(tank).unwrap().health,
        start,
        "a tank was hurt by radiation"
    );
}

#[test]
fn flight_crosses_it_untouched() {
    // The one thing that is genuinely above the ground rather than resistant
    // to what is on it.
    let mut sim = scenario(vec![(0, "desolator", 20, 20), (1, "helicopter", 21, 20)]);
    let ids = sim.units().ids();
    let chopper = ids[1];
    let start = sim.unit(chopper).unwrap().health;

    deploy(&mut sim, ids[0]);
    for _ in 0..60 {
        sim.tick(&[]);
    }

    assert_eq!(sim.unit(chopper).unwrap().health, start);
}

// -- It outliving its source ------------------------------------------------

#[test]
fn the_ground_stays_dangerous_after_the_desolator_leaves() {
    // The whole point. A patch that went cold the moment the Desolator packed
    // up would be a slow gun rather than an area denied.
    let mut sim = scenario(vec![(0, "desolator", 20, 20)]);
    let id = sim.units().ids()[0];

    deploy(&mut sim, id);
    sim.tick(&[]);
    assert!(!sim.hazards().is_empty());

    // Pack up and walk away.
    deploy(&mut sim, id);
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Move {
            units: vec![id],
            target: Cell::new(40, 40),
        },
    )]);
    for _ in 0..30 {
        sim.tick(&[]);
    }

    assert!(
        sim.hazards().iter().any(|h| h.cell == Cell::new(20, 20)),
        "the ground it stood on should still be hot"
    );
}

#[test]
fn the_ground_eventually_goes_cold() {
    // A permanent scar would take cells out of the match for good, and a map
    // with two Desolators would eventually have nowhere left to walk.
    let mut sim = scenario(vec![(0, "desolator", 20, 20)]);
    let id = sim.units().ids()[0];

    deploy(&mut sim, id);
    sim.tick(&[]);
    deploy(&mut sim, id);
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Move {
            units: vec![id],
            target: Cell::new(44, 44),
        },
    )]);
    for _ in 0..400 {
        sim.tick(&[]);
    }

    assert!(sim.hazards().is_empty(), "the patch never expired");
}

#[test]
fn a_dead_desolator_still_denies_the_ground_it_poisoned() {
    let mut sim = scenario(vec![(0, "desolator", 20, 20), (1, "infantryman", 26, 20)]);
    let ids = sim.units().ids();
    let (desolator, victim) = (ids[0], ids[1]);

    deploy(&mut sim, desolator);
    sim.tick(&[]);
    let hot: Vec<Cell> = sim.hazards().iter().map(|h| h.cell).collect();
    assert!(!hot.is_empty());

    // The enemy walks onto the patch after the Desolator is gone. Killing it
    // is not enough to make the ground safe, which is the trade the original
    // asks a player to make.
    let start = sim.unit(victim).unwrap().health;
    sim.tick(&[Command::new(
        PlayerId(1),
        0,
        CommandKind::Move {
            units: vec![victim],
            target: Cell::new(20, 20),
        },
    )]);
    for _ in 0..60 {
        sim.tick(&[]);
        if sim.unit(victim).is_none_or(|u| u.health < start) {
            break;
        }
    }

    assert!(
        sim.unit(victim).is_none_or(|u| u.health < start),
        "the patch stopped working"
    );
}

#[test]
fn contamination_is_deterministic() {
    let run = || {
        let mut sim = scenario(vec![(0, "desolator", 20, 20), (1, "infantryman", 22, 20)]);
        let id = sim.units().ids()[0];
        for tick in 0..300 {
            if tick == 5 || tick == 150 {
                deploy(&mut sim, id);
            } else {
                sim.tick(&[]);
            }
        }
        sim.state_hash()
    };
    assert_eq!(run(), run());
}
