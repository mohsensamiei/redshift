//! Infantry occupying a building and fighting from inside it.
//!
//! Researched, and more specific than "infantry can garrison buildings". Four
//! rules, three of which are easy to get backwards:
//!
//! - The building fires with **its own** predetermined weapon, not the weapon
//!   of whoever is inside — the exact opposite of a vehicle whose gun changes
//!   with its passenger.
//! - Only **basic** infantry may enter. A GI or a Conscript, not a commando.
//! - Capacity belongs to the **building**, since it follows from its size.
//! - The garrison is **forced out below a third health**, rather than dying
//!   with the building.
//!
//! That last one is what makes a garrisoned building worth attacking instead of
//! avoiding: clearing one means damaging it enough to evict, not destroying it.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::EntityId;
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn armour() -> ArmourTable {
    ron::from_str(r#"( classes: ["none"], table: { "shot": { "none": 100 } } )"#).unwrap()
}

fn gun(id: &str, damage: u32, range: i32) -> WeaponDef {
    WeaponDef {
        id: id.into(),
        damage,
        warhead: "shot".into(),
        reload: Ticks(10),
        range: Hundredths(range),
        splash_radius: Hundredths::ZERO,
        projectile_speed: Hundredths::ZERO,
        homing: false,
        targets: vec![],
        instant_kill: false,
        ammo: 0,
        intercepts: false,
        heals: false,
    }
}

fn foot(id: &str, category: &str, extra: Vec<Trait>) -> EntityDef {
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
            range: Hundredths(700),
        },
    ];
    traits.extend(extra);
    EntityDef {
        id: id.into(),
        name_key: format!("unit.{id}"),
        side: None,
        category: category.into(),
        traits,
    }
}

/// A civilian building with a machine-gun nest in it — but only when someone is
/// home.
fn house(capacity: u8) -> EntityDef {
    EntityDef {
        id: "house".into(),
        name_key: "structure.house".into(),
        side: None,
        category: "civilian".into(),
        traits: vec![
            Trait::Health {
                max: 600,
                armour: "none".into(),
            },
            Trait::Vision {
                range: Hundredths(600),
            },
            Trait::Footprint {
                width: 2,
                height: 2,
            },
            Trait::Garrisonable {
                capacity,
                categories: vec!["infantry".into()],
                weapon: "nest_gun".into(),
                evict_below_percent: 33,
            },
        ],
    }
}

fn rules() -> Rules {
    Rules::from_parts(
        vec![
            foot("gi", "infantry", vec![]),
            // A commando: on foot, ordinary in every way, and its own category.
            // That single word is the whole of "only basic infantry garrison".
            foot("commando", "commando", vec![]),
            // Outranges the building's nest gun, and sees far enough to use
            // that reach. Anything standing closer is simply shot first, and
            // the building never gets whittled down at all.
            EntityDef {
                traits: vec![
                    Trait::Health {
                        max: 200,
                        armour: "none".into(),
                    },
                    Trait::Vision {
                        range: Hundredths(1_200),
                    },
                    Trait::Armed {
                        weapon: "howitzer".into(),
                        turret: true,
                        turret_rate: 3600,
                    },
                ],
                ..foot("gunner", "artillery", vec![])
            },
            foot(
                "rifleman",
                "infantry",
                vec![Trait::Armed {
                    weapon: "rifle".into(),
                    turret: true,
                    turret_rate: 3600,
                }],
            ),
            house(3),
        ],
        vec![
            gun("nest_gun", 30, 600),
            // Deliberately feeble. If the building ever fired *this*, the tests
            // below would still pass on damage alone — so the occupant's weapon
            // is weak enough that using it by mistake shows up.
            gun("rifle", 2, 500),
            gun("howitzer", 20, 1_000),
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
            owner: if owner == 9 {
                PlayerId::NEUTRAL
            } else {
                PlayerId(owner)
            },
            kind: rules
                .kind_of(id)
                .unwrap_or_else(|| panic!("no entity {id:?}")),
            pos: Cell::new(x, y).centre(),
        })
        .collect();
    Sim::new(MatchSetup {
        seed: 0x_6A44,
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

fn send_in(sim: &mut Sim, owner: u8, unit: EntityId, building: EntityId) {
    sim.tick(&[Command::new(
        PlayerId(owner),
        0,
        CommandKind::EnterBuilding {
            units: vec![unit],
            target: building,
        },
    )]);
    for _ in 0..600 {
        sim.tick(&[]);
        if sim.unit(unit).is_none_or(|u| u.is_aboard()) {
            return;
        }
    }
}

// -- Getting in -------------------------------------------------------------

#[test]
fn infantry_garrison_a_civilian_building() {
    let mut sim = scenario(vec![(0, "gi", 10, 10), (9, "house", 14, 10)]);
    let ids = sim.units().ids();
    let (gi, house) = (ids[0], ids[1]);

    send_in(&mut sim, 0, gi, house);

    assert!(
        sim.unit(gi).unwrap().is_aboard(),
        "the soldier should be inside"
    );
    assert_eq!(sim.unit(house).unwrap().cargo, vec![gi]);
}

#[test]
fn an_occupied_building_fights_for_whoever_is_inside() {
    // Ownership carries the vision, the targeting and the colour with it, which
    // is why occupying is a transfer rather than a flag saying who to shoot for.
    let mut sim = scenario(vec![(0, "gi", 10, 10), (9, "house", 14, 10)]);
    let ids = sim.units().ids();
    let (gi, house) = (ids[0], ids[1]);
    assert!(sim.unit(house).unwrap().owner.is_neutral());

    send_in(&mut sim, 0, gi, house);

    assert_eq!(sim.unit(house).unwrap().owner, PlayerId(0));
}

#[test]
fn a_commando_cannot_garrison() {
    // "Only basic infantry" is one word in the building's category list.
    let mut sim = scenario(vec![(0, "commando", 10, 10), (9, "house", 14, 10)]);
    let ids = sim.units().ids();
    let (commando, house) = (ids[0], ids[1]);

    send_in(&mut sim, 0, commando, house);

    assert!(!sim.unit(commando).unwrap().is_aboard());
    assert!(sim.unit(house).unwrap().owner.is_neutral());
}

#[test]
fn a_building_holds_no_more_than_its_capacity() {
    // Capacity belongs to the building, because it follows from its size.
    let mut sim = scenario(vec![
        (0, "gi", 10, 10),
        (0, "gi", 10, 12),
        (0, "gi", 10, 14),
        (0, "gi", 10, 16),
        (9, "house", 16, 13),
    ]);
    let ids = sim.units().ids();
    let house = ids[4];

    for gi in ids.iter().take(4) {
        send_in(&mut sim, 0, *gi, house);
    }

    assert_eq!(sim.unit(house).unwrap().cargo.len(), 3);
    let outside = ids
        .iter()
        .take(4)
        .filter(|id| sim.unit(**id).is_some_and(|u| !u.is_aboard()))
        .count();
    assert_eq!(outside, 1, "the fourth should still be standing outside");
}

#[test]
fn a_building_someone_else_holds_cannot_be_moved_into() {
    let mut sim = scenario(vec![
        (0, "gi", 10, 10),
        (1, "gi", 18, 10),
        (9, "house", 14, 10),
    ]);
    let ids = sim.units().ids();
    let (mine, theirs, house) = (ids[0], ids[1], ids[2]);

    send_in(&mut sim, 0, mine, house);
    send_in(&mut sim, 1, theirs, house);

    assert_eq!(sim.unit(house).unwrap().owner, PlayerId(0));
    assert!(!sim.unit(theirs).unwrap().is_aboard());
}

// -- Shooting ---------------------------------------------------------------

#[test]
fn an_empty_building_has_no_weapon() {
    // Which is what makes garrisoning one do something.
    let mut sim = scenario(vec![(9, "house", 14, 10), (1, "gi", 16, 10)]);
    let ids = sim.units().ids();
    let victim = ids[1];
    let start = sim.unit(victim).unwrap().health;

    for _ in 0..200 {
        sim.tick(&[]);
    }

    assert_eq!(sim.unit(victim).unwrap().health, start);
}

#[test]
fn an_occupied_building_shoots_with_its_own_weapon() {
    // The rule most easily got backwards. The occupant here carries a rifle
    // doing 2 damage; the building's nest gun does 30. If the engine ever used
    // the occupant's weapon, the arithmetic below would say so.
    let mut sim = scenario(vec![
        (0, "rifleman", 10, 10),
        (9, "house", 14, 10),
        (1, "gi", 17, 10),
    ]);
    let ids = sim.units().ids();
    let (rifleman, house, victim) = (ids[0], ids[1], ids[2]);

    send_in(&mut sim, 0, rifleman, house);
    let start = sim.unit(victim).map(|v| v.health).unwrap_or(0);
    for _ in 0..40 {
        sim.tick(&[]);
        if sim.unit(victim).is_none() {
            break;
        }
    }

    let dealt = start - sim.unit(victim).map(|v| v.health).unwrap_or(0);
    assert!(dealt > 0, "an occupied building did not shoot");
    assert!(
        dealt > 2 * 40,
        "it dealt {dealt}, which is rifle damage — it used its occupant's gun"
    );
}

// -- Getting out ------------------------------------------------------------

#[test]
fn a_garrison_is_thrown_out_below_a_third_health() {
    // Not killed. A garrisoned building that murdered its occupants would be a
    // death trap nobody would use, and clearing one would mean destroying it
    // rather than shooting it up.
    let mut sim = scenario(vec![
        (0, "gi", 10, 10),
        (9, "house", 14, 10),
        (1, "gunner", 23, 10),
    ]);
    let ids = sim.units().ids();
    let (gi, house) = (ids[0], ids[1]);

    send_in(&mut sim, 0, gi, house);
    assert!(sim.unit(gi).unwrap().is_aboard());

    for _ in 0..4_000 {
        sim.tick(&[]);
        if sim.unit(gi).is_none_or(|u| !u.is_aboard()) {
            break;
        }
    }

    let soldier = sim.unit(gi).expect("the garrison should survive eviction");
    assert!(!soldier.is_aboard(), "it should have been thrown out");
    assert!(soldier.is_alive());
    let building = sim.unit(house).expect("the building should still stand");
    assert!(building.health * 3 < 600, "evicted too early");
}

#[test]
fn an_emptied_building_goes_back_to_being_nobodys() {
    let mut sim = scenario(vec![(0, "gi", 10, 10), (9, "house", 14, 10)]);
    let ids = sim.units().ids();
    let (gi, house) = (ids[0], ids[1]);

    send_in(&mut sim, 0, gi, house);
    assert_eq!(sim.unit(house).unwrap().owner, PlayerId(0));

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Unload {
            transport: house,
            at: Cell::new(12, 10),
        },
    )]);

    assert!(!sim.unit(gi).unwrap().is_aboard(), "it should have left");
    assert!(
        sim.unit(house).unwrap().owner.is_neutral(),
        "an empty civilian building should not keep fighting for its last tenant"
    );
}

#[test]
fn an_emptied_building_stops_shooting() {
    let mut sim = scenario(vec![
        (0, "gi", 10, 10),
        (9, "house", 14, 10),
        (1, "gi", 17, 10),
    ]);
    let ids = sim.units().ids();
    let (gi, house, victim) = (ids[0], ids[1], ids[2]);

    send_in(&mut sim, 0, gi, house);
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Unload {
            transport: house,
            at: Cell::new(12, 10),
        },
    )]);
    let start = sim.unit(victim).unwrap().health;
    for _ in 0..100 {
        sim.tick(&[]);
    }

    assert_eq!(sim.unit(victim).unwrap().health, start);
}

#[test]
fn garrisoning_is_deterministic() {
    let run = || {
        let mut sim = scenario(vec![
            (0, "gi", 10, 10),
            (9, "house", 14, 10),
            (1, "gunner", 23, 10),
        ]);
        let ids = sim.units().ids();
        send_in(&mut sim, 0, ids[0], ids[1]);
        for _ in 0..500 {
            sim.tick(&[]);
        }
        sim.state_hash()
    };
    assert_eq!(run(), run());
}
