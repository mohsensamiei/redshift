//! A vehicle whose weapon is a function of what it is carrying.
//!
//! Twenty-four turret modes in the original, four more in the expansion. The
//! shape that matters is not the count but where the list lives: on the
//! *passengers*, not as a table on the vehicle. A table on the IFV would mean
//! the vehicle knows the name of every infantryman in the game, and adding a
//! unit would mean editing a different file to teach the IFV about it.
//!
//! It is also the first thing in the engine whose weapon is a property of a
//! unit rather than of its kind. Everything else reads the same answer for its
//! whole life.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Layer, Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::EntityId;
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn armour() -> ArmourTable {
    ron::from_str(
        r#"( classes: ["none", "air"],
             table: { "shot": { "none": 100, "air": 0 },
                      "flak": { "none": 0, "air": 100 },
                      "rocket": { "none": 100, "air": 0 } } )"#,
    )
    .unwrap()
}

fn gun(id: &str, warhead: &str, damage: u32, range: i32, targets: Vec<Layer>) -> WeaponDef {
    WeaponDef {
        id: id.into(),
        damage,
        warhead: warhead.into(),
        reload: Ticks(10),
        range: Hundredths(range),
        splash_radius: Hundredths::ZERO,
        projectile_speed: Hundredths::ZERO,
        homing: false,
        targets,
        instant_kill: false,
        ammo: 0,
        intercepts: false,
        target_categories: vec![],
        mind_control: false,
        heals: false,
    }
}

fn infantry(id: &str, crews: Option<&str>) -> EntityDef {
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
    if let Some(weapon) = crews {
        traits.push(Trait::Crews {
            weapon: weapon.into(),
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

/// The vehicle. Anti-air when empty — which is the IFV's own gun, not a
/// special case for having nobody inside.
fn ifv(from_cargo: bool) -> EntityDef {
    let mut traits = vec![
        Trait::Health {
            max: 400,
            armour: "none".into(),
        },
        Trait::Mobile {
            speed: Hundredths(400),
            turn_rate: 3600,
            locomotor: Locomotor::Wheeled,
            surfaces: None,
            size: None,
            layer: None,
        },
        Trait::Vision {
            range: Hundredths(900),
        },
        Trait::Armed {
            weapon: "flak_gun".into(),
            turret: true,
            turret_rate: 3600,
        },
        Trait::Transport {
            capacity: 1,
            allowed: vec!["rifleman".into(), "rocketeer".into(), "porter".into()],
        },
    ];
    if from_cargo {
        traits.push(Trait::WeaponFromCargo);
    }
    EntityDef {
        id: if from_cargo { "ifv" } else { "apc" }.into(),
        name_key: "unit.ifv".into(),
        side: None,
        category: "vehicle".into(),
        traits,
    }
}

fn target(id: &str, armour_class: &str, layer: Option<Layer>) -> EntityDef {
    EntityDef {
        id: id.into(),
        name_key: format!("unit.{id}"),
        side: None,
        category: "vehicle".into(),
        traits: vec![
            Trait::Health {
                max: 4_000,
                armour: armour_class.into(),
            },
            Trait::Mobile {
                speed: Hundredths(100),
                turn_rate: 3600,
                locomotor: if layer.is_some() {
                    Locomotor::Air
                } else {
                    Locomotor::Wheeled
                },
                surfaces: None,
                size: None,
                layer,
            },
            Trait::Vision {
                range: Hundredths(300),
            },
        ],
    }
}

fn rules() -> Rules {
    Rules::from_parts(
        vec![
            ifv(true),
            ifv(false),
            infantry("rifleman", Some("rifle")),
            infantry("rocketeer", Some("rocket")),
            // No turret mode of its own. Riding in an IFV should leave the
            // vehicle with the gun it already had.
            infantry("porter", None),
            target("truck", "none", None),
            target("plane", "air", Some(Layer::Air)),
        ],
        vec![
            gun("flak_gun", "flak", 20, 500, vec![Layer::Air]),
            gun("rifle", "shot", 20, 500, vec![Layer::Ground]),
            // Longer than either, so "which weapon is this" is answerable from
            // the outside by how far the vehicle engages.
            gun("rocket", "rocket", 40, 900, vec![Layer::Ground]),
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
        seed: 0x_1F0,
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
        if sim.unit(passenger).is_some_and(|u| u.is_aboard()) {
            return true;
        }
    }
    false
}

/// Whether the vehicle hurts the target within `ticks`. The only honest way to
/// ask "which weapon is this" from outside: the armour table answers
/// differently for each, and nothing else does.
fn hurts(sim: &mut Sim, shooter: EntityId, victim: EntityId, ticks: u32) -> bool {
    let start = sim.unit(victim).map(|v| v.health).unwrap_or(0);
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Attack {
            units: vec![shooter],
            target: victim,
        },
    )]);
    for _ in 0..ticks {
        sim.tick(&[]);
        if sim.unit(victim).is_none_or(|v| v.health < start) {
            return true;
        }
    }
    false
}

// -- What it does when empty ------------------------------------------------

#[test]
fn an_empty_ifv_uses_its_own_gun() {
    // Its `Armed` is what it does with nobody inside, rather than a special
    // case for being empty.
    let mut sim = scenario(vec![(0, "ifv", 10, 10), (1, "plane", 13, 10)]);
    let ids = sim.units().ids();
    assert!(
        hurts(&mut sim, ids[0], ids[1], 60),
        "the flak gun did nothing"
    );
}

#[test]
fn an_empty_ifv_cannot_touch_the_ground() {
    let mut sim = scenario(vec![(0, "ifv", 10, 10), (1, "truck", 13, 10)]);
    let ids = sim.units().ids();
    assert!(
        !hurts(&mut sim, ids[0], ids[1], 60),
        "an anti-air gun hit a truck"
    );
}

// -- What a passenger changes -----------------------------------------------

#[test]
fn a_rifleman_inside_gives_the_ifv_a_ground_weapon() {
    let mut sim = scenario(vec![
        (0, "ifv", 10, 10),
        (0, "rifleman", 12, 10),
        (1, "truck", 13, 10),
    ]);
    let ids = sim.units().ids();
    let (vehicle, rifleman, truck) = (ids[0], ids[1], ids[2]);
    assert!(load(&mut sim, rifleman, vehicle));

    assert!(
        hurts(&mut sim, vehicle, truck, 60),
        "the passenger's turret mode never took effect"
    );
}

#[test]
fn the_passenger_decides_the_reach_too() {
    // A turret mode is a whole weapon, not a damage number. The rocketeer's
    // reach is longer than anything the vehicle has of its own, which is how
    // the mode is observable from outside.
    let mut sim = scenario(vec![
        (0, "ifv", 10, 10),
        (0, "rocketeer", 12, 10),
        (1, "truck", 17, 10),
    ]);
    let ids = sim.units().ids();
    let (vehicle, rocketeer, truck) = (ids[0], ids[1], ids[2]);

    // Seven cells: out of the rifle's five, inside the rocket's nine.
    assert!(load(&mut sim, rocketeer, vehicle));
    assert!(
        hurts(&mut sim, vehicle, truck, 120),
        "the vehicle should have engaged at the rocket's range"
    );
}

#[test]
fn a_passenger_with_no_turret_mode_changes_nothing() {
    // Most infantry are not turret modes. Riding along should leave the
    // vehicle with the gun it already had rather than disarming it.
    let mut sim = scenario(vec![
        (0, "ifv", 10, 10),
        (0, "porter", 12, 10),
        (1, "plane", 13, 10),
    ]);
    let ids = sim.units().ids();
    let (vehicle, porter, plane) = (ids[0], ids[1], ids[2]);
    assert!(load(&mut sim, porter, vehicle));

    assert!(
        hurts(&mut sim, vehicle, plane, 60),
        "a passenger with nothing to offer disarmed the vehicle"
    );
}

#[test]
fn unloading_gives_the_vehicle_its_own_gun_back() {
    let mut sim = scenario(vec![
        (0, "ifv", 10, 10),
        (0, "rifleman", 12, 10),
        (1, "plane", 13, 10),
    ]);
    let ids = sim.units().ids();
    let (vehicle, rifleman, plane) = (ids[0], ids[1], ids[2]);
    assert!(load(&mut sim, rifleman, vehicle));
    assert!(
        !hurts(&mut sim, vehicle, plane, 40),
        "a rifle should not reach an aircraft"
    );

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Unload {
            transport: vehicle,
            at: Cell::new(10, 12),
        },
    )]);
    for _ in 0..60 {
        sim.tick(&[]);
    }

    assert!(
        hurts(&mut sim, vehicle, plane, 60),
        "the flak gun should have come back"
    );
}

// -- What it is not ---------------------------------------------------------

#[test]
fn an_ordinary_transport_is_unaffected_by_its_cargo() {
    // The distinction the trait exists to draw. Without `WeaponFromCargo` a
    // vehicle carries people and keeps its own gun, which is what a transport
    // normally is.
    let mut sim = scenario(vec![
        (0, "apc", 10, 10),
        (0, "rifleman", 12, 10),
        (1, "truck", 13, 10),
    ]);
    let ids = sim.units().ids();
    let (apc, rifleman, truck) = (ids[0], ids[1], ids[2]);
    assert!(load(&mut sim, rifleman, apc));

    assert!(
        !hurts(&mut sim, apc, truck, 60),
        "a plain transport took a weapon from its cargo"
    );
}

#[test]
fn the_vehicle_fires_the_mode_once_not_twice() {
    // A turret mode is the vehicle shooting, not the passenger shooting out.
    // The failure worth guarding against is both happening: the vehicle firing
    // the borrowed weapon *and* the passenger firing its own from inside, which
    // would silently double an IFV's output and look like nothing at all.
    let mut sim = scenario(vec![
        (0, "ifv", 10, 10),
        (0, "rifleman", 12, 10),
        (1, "truck", 13, 10),
    ]);
    let ids = sim.units().ids();
    let (vehicle, rifleman, truck) = (ids[0], ids[1], ids[2]);
    assert!(load(&mut sim, rifleman, vehicle));

    let before = sim.unit(truck).unwrap().health;
    for _ in 0..60 {
        sim.tick(&[]);
    }
    let dealt = before - sim.unit(truck).unwrap().health;

    // Twenty damage on a ten-tick reload: six shots in sixty ticks, give or
    // take where the cycle started.
    assert!(
        (100..=140).contains(&dealt),
        "took {dealt} damage in sixty ticks — one rifle should do about 120, \
         and two would look like this test passing for the wrong reason"
    );
    assert!(
        sim.unit(rifleman).unwrap().is_aboard(),
        "the passenger should still be inside"
    );
}

#[test]
fn turret_modes_are_deterministic() {
    let run = || {
        let mut sim = scenario(vec![
            (0, "ifv", 10, 10),
            (0, "rocketeer", 12, 10),
            (1, "truck", 16, 10),
        ]);
        let ids = sim.units().ids();
        load(&mut sim, ids[1], ids[0]);
        hurts(&mut sim, ids[0], ids[2], 200);
        sim.state_hash()
    };
    assert_eq!(run(), run());
}
