//! A weapon that gives health back.
//!
//! The Medic, Yuri's repair drones, and the turret mode an engineer gives an
//! IFV. Not a damage number with a minus sign: what changes is what counts as
//! a *target*. A healing weapon looks for friends who are hurt, so a unit
//! carrying one is useless against an enemy rather than mildly helpful to them.
//!
//! It also skips the damage machinery entirely. Running a heal through the
//! armour table was the obvious shortcut, and it would make a medic's
//! effectiveness depend on what its patient was armoured against — and, worse,
//! make a veteran medic heal *less*, since a rank resists whatever is aimed at
//! it.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::EntityId;
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn armour() -> ArmourTable {
    ron::from_str(
        r#"( classes: ["flesh", "steel"],
             table: { "shot": { "flesh": 100, "steel": 10 },
                      "care": { "flesh": 100, "steel": 100 } } )"#,
    )
    .unwrap()
}

fn person(id: &str, armour_class: &str, weapon: Option<&str>) -> EntityDef {
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
            range: Hundredths(700),
        },
    ];
    if let Some(w) = weapon {
        traits.push(Trait::Armed {
            weapon: w.into(),
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
            person("medic", "flesh", Some("bandage")),
            person("rifleman", "flesh", Some("rifle")),
            person("bystander", "flesh", None),
            // Armoured against bullets, so a heal that went through the armour
            // table would be visibly wrong on it.
            EntityDef {
                category: "vehicle".into(),
                ..person("tank", "steel", None)
            },
        ],
        vec![
            WeaponDef {
                id: "rifle".into(),
                damage: 40,
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
                heals: false,
            },
            WeaponDef {
                id: "bandage".into(),
                damage: 20,
                warhead: "care".into(),
                reload: Ticks(10),
                range: Hundredths(300),
                splash_radius: Hundredths::ZERO,
                projectile_speed: Hundredths::ZERO,
                homing: false,
                targets: vec![],
                instant_kill: false,
                ammo: 0,
                intercepts: false,
                heals: true,
            },
        ],
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
        seed: 0x_11EA,
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

/// A friendly unit shot down to roughly half, with a medic standing by.
///
/// Wounded through the real damage path — there is no mutable accessor on
/// `Sim`, deliberately, and adding one so a test could write a health value
/// would be the kind of hole that later gets used in earnest.
fn a_patient_and_a_medic(patient: &str) -> (Sim, EntityId, EntityId) {
    // The medic starts across the map and is called in afterwards. Standing it
    // next to the patient from the start would mean it healed through the whole
    // wounding, and every test would begin with a patient already whole.
    let mut sim = scenario(vec![
        (0, patient, 20, 20),
        (1, "rifleman", 23, 20),
        (0, "medic", 40, 20),
    ]);
    let ids = sim.units().ids();
    let (hurt, enemy, medic) = (ids[0], ids[1], ids[2]);

    attack(&mut sim, 1, enemy, hurt);
    for _ in 0..3_000 {
        sim.tick(&[]);
        if sim.unit(hurt).is_some_and(|u| u.health <= 200) {
            break;
        }
    }
    assert!(
        sim.unit(hurt).is_some_and(|u| u.health <= 200),
        "the patient was never wounded"
    );

    // The enemy leaves, so the rest of each test is about the medic alone.
    sim.tick(&[Command::new(
        PlayerId(1),
        0,
        CommandKind::Move {
            units: vec![enemy],
            target: Cell::new(45, 45),
        },
    )]);
    for _ in 0..300 {
        sim.tick(&[]);
    }
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Move {
            units: vec![medic],
            target: Cell::new(21, 20),
        },
    )]);
    for _ in 0..400 {
        sim.tick(&[]);
        if sim
            .unit(medic)
            .is_some_and(|m| m.cell().chebyshev_to(Cell::new(21, 20)) <= 1)
        {
            break;
        }
    }
    (sim, hurt, medic)
}

// -- What it does -----------------------------------------------------------

#[test]
fn a_medic_heals_the_wounded_beside_it() {
    let (mut sim, hurt, _medic) = a_patient_and_a_medic("bystander");
    let before = sim.unit(hurt).unwrap().health;

    for _ in 0..200 {
        sim.tick(&[]);
    }

    assert!(
        sim.unit(hurt).unwrap().health > before,
        "the medic did nothing"
    );
}

#[test]
fn it_stops_at_full_health() {
    let (mut sim, hurt, _medic) = a_patient_and_a_medic("bystander");
    for _ in 0..600 {
        sim.tick(&[]);
    }

    assert_eq!(
        sim.unit(hurt).unwrap().health,
        400,
        "a patient should end up whole, and no more than whole"
    );
}

#[test]
fn armour_does_not_dilute_a_heal() {
    // The reason a heal skips the damage machinery. The tank is armoured
    // against bullets; putting a heal through the same table would make it
    // almost unhealable, which is nonsense.
    let (mut sim, tank, _medic) = a_patient_and_a_medic("tank");
    let before = sim.unit(tank).unwrap().health;

    for _ in 0..100 {
        sim.tick(&[]);
    }
    let restored = sim.unit(tank).unwrap().health - before;

    // Twenty a shot on a ten-tick reload: about two hundred in a hundred ticks.
    assert!(
        restored >= 150,
        "restored only {restored}, so armour was applied to a bandage"
    );
}

// -- What it will not do ----------------------------------------------------

#[test]
fn a_medic_will_not_heal_an_enemy() {
    let mut sim = scenario(vec![
        (1, "bystander", 20, 20),
        (0, "rifleman", 23, 20),
        (0, "medic", 21, 20),
    ]);
    let ids = sim.units().ids();
    let (enemy, gunner) = (ids[0], ids[1]);

    attack(&mut sim, 0, gunner, enemy);
    let mut lowest = sim.unit(enemy).map(|u| u.health).unwrap_or(0);
    for _ in 0..200 {
        sim.tick(&[]);
        let Some(now) = sim.unit(enemy).map(|u| u.health) else {
            break;
        };
        assert!(
            now <= lowest,
            "an enemy gained health — the medic is patching up the other side"
        );
        lowest = now;
    }
}

#[test]
fn a_medic_cannot_hurt_anything() {
    // A unit carrying a healing weapon is useless against an enemy rather than
    // mildly helpful to them. Inverting the alliance test is what buys this.
    let mut sim = scenario(vec![(0, "medic", 20, 20), (1, "bystander", 21, 20)]);
    let ids = sim.units().ids();
    let (medic, enemy) = (ids[0], ids[1]);
    let before = sim.unit(enemy).unwrap().health;

    attack(&mut sim, 0, medic, enemy);
    for _ in 0..200 {
        sim.tick(&[]);
    }

    assert_eq!(
        sim.unit(enemy).unwrap().health,
        before,
        "a bandage wounded somebody"
    );
}

#[test]
fn an_ordinary_weapon_still_will_not_shoot_a_friend() {
    // The inversion must be the healing weapon's alone. If it leaked, every
    // rifle in the game would start shooting its own side.
    let mut sim = scenario(vec![
        (0, "rifleman", 20, 20),
        (0, "bystander", 21, 20),
        (1, "bystander", 23, 20),
    ]);
    let ids = sim.units().ids();
    let friend = ids[1];
    let before = sim.unit(friend).unwrap().health;

    for _ in 0..200 {
        sim.tick(&[]);
    }

    assert_eq!(sim.unit(friend).unwrap().health, before);
}

#[test]
fn a_medic_moves_on_once_its_patient_is_whole() {
    // Without this it keeps its beam on the first friend it patched up and
    // never notices the next one.
    let (mut sim, first, medic) = a_patient_and_a_medic("bystander");
    for _ in 0..600 {
        sim.tick(&[]);
    }
    assert_eq!(sim.unit(first).unwrap().health, 400);

    assert!(
        sim.unit(medic).unwrap().combat.target.is_none(),
        "the medic is still treating somebody who is already whole"
    );
}

#[test]
fn healing_is_deterministic() {
    let run = || {
        let (mut sim, _, _) = a_patient_and_a_medic("bystander");
        for _ in 0..300 {
            sim.tick(&[]);
        }
        sim.state_hash()
    };
    assert_eq!(run(), run());
}
