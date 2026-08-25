//! A unit with more than one kind of thing it can do.
//!
//! Tanya shoots people with a pistol and destroys *buildings* with charges.
//! That is not "two weapons" in the sense the engine already had — a cannon and
//! a missile, chosen by whether the target is on the ground or in the air. A
//! layer mask cannot tell a building from an infantryman, because both are on
//! the ground.
//!
//! What was missing is an **action with its own valid targets**, chosen by what
//! the unit is pointed at rather than by a stance the player sets. Once a
//! weapon can say which categories it applies to, the choice falls out of the
//! selection that primary and secondary already went through.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::EntityId;
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

/// The pistol is useless against concrete and the charges are useless against
/// people — which is what makes "which action was that" answerable from
/// outside, by whether anything happened at all.
fn armour() -> ArmourTable {
    ron::from_str(
        r#"( classes: ["flesh", "concrete"],
             table: { "pistol": { "flesh": 100, "concrete": 1 },
                      "c4": { "flesh": 1, "concrete": 100 } } )"#,
    )
    .unwrap()
}

fn gun(id: &str, warhead: &str, range: i32, categories: Vec<String>) -> WeaponDef {
    WeaponDef {
        id: id.into(),
        damage: 100,
        warhead: warhead.into(),
        reload: Ticks(10),
        range: Hundredths(range),
        splash_radius: Hundredths::ZERO,
        projectile_speed: Hundredths::ZERO,
        homing: false,
        targets: vec![],
        target_categories: categories,
        instant_kill: false,
        ammo: 0,
        intercepts: false,
        mind_control: false,
        heals: false,
    }
}

fn rules() -> Rules {
    Rules::from_parts(
        vec![
            EntityDef {
                id: "commando".into(),
                name_key: "unit.commando".into(),
                side: None,
                category: "infantry".into(),
                traits: vec![
                    Trait::Health {
                        max: 400,
                        armour: "flesh".into(),
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
                        range: Hundredths(1_200),
                    },
                    Trait::Armed {
                        weapon: "pistol".into(),
                        turret: true,
                        turret_rate: 3600,
                    },
                    Trait::Secondary {
                        weapon: "charges".into(),
                        turret: true,
                        turret_rate: 3600,
                    },
                ],
            },
            EntityDef {
                id: "soldier".into(),
                name_key: "unit.soldier".into(),
                side: None,
                category: "infantry".into(),
                traits: vec![
                    Trait::Health {
                        max: 2_000,
                        armour: "flesh".into(),
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
                        range: Hundredths(300),
                    },
                ],
            },
            EntityDef {
                id: "militia".into(),
                name_key: "unit.militia".into(),
                side: None,
                category: "infantry".into(),
                traits: vec![
                    Trait::Health {
                        max: 400,
                        armour: "flesh".into(),
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
                        weapon: "old_rifle".into(),
                        turret: true,
                        turret_rate: 3600,
                    },
                ],
            },
            EntityDef {
                id: "bunker".into(),
                name_key: "structure.bunker".into(),
                side: None,
                category: "structure".into(),
                traits: vec![
                    Trait::Health {
                        max: 2_000,
                        armour: "concrete".into(),
                    },
                    Trait::Vision {
                        range: Hundredths(300),
                    },
                ],
            },
        ],
        vec![
            // People and vehicles, explicitly. Leaving the pistol unrestricted
            // was the first attempt and it quietly won every contest: an
            // unrestricted weapon applies to a building too, so the primary was
            // always chosen and the charges never fired.
            //
            // The fix is data, not a rule about rules. Preferring "the more
            // specific action" would mean the engine guessing which of two
            // applicable weapons the author meant — two weapons that both apply
            // is a mistake in the rules, and it should be stated rather than
            // resolved by precedence.
            gun(
                "pistol",
                "pistol",
                400,
                vec!["infantry".into(), "vehicle".into()],
            ),
            // Buildings only, and reaching further than the pistol on purpose:
            // if the search took the longer weapon's restriction it would never
            // find a soldier at all.
            gun("charges", "c4", 900, vec!["structure".into()]),
            // Names nothing, like every weapon in the rules before this
            // existed. Hurts flesh and concrete alike, at different rates.
            gun("old_rifle", "c4", 400, vec![]),
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
        seed: 0x_7A47,
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

/// Damage dealt to the victim over a window, after an explicit order.
fn dealt(sim: &mut Sim, attacker: EntityId, victim: EntityId, ticks: u32) -> u32 {
    let before = sim.unit(victim).map(|v| v.health).unwrap_or(0);
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Attack {
            units: vec![attacker],
            target: victim,
        },
    )]);
    for _ in 0..ticks {
        sim.tick(&[]);
    }
    before - sim.unit(victim).map(|v| v.health).unwrap_or(0)
}

// -- Choosing by what it is aimed at ----------------------------------------

#[test]
fn aimed_at_a_building_it_uses_the_charges() {
    // The pistol does one percent to concrete; the charges do all of it. If the
    // wrong action were chosen the number would be visibly tiny.
    let mut sim = scenario(vec![(0, "commando", 20, 20), (1, "bunker", 24, 20)]);
    let ids = sim.units().ids();
    let harm = dealt(&mut sim, ids[0], ids[1], 100);

    assert!(
        harm > 500,
        "did {harm} to a building, which is a pistol rather than a charge"
    );
}

#[test]
fn aimed_at_a_soldier_it_uses_the_pistol() {
    let mut sim = scenario(vec![(0, "commando", 20, 20), (1, "soldier", 23, 20)]);
    let ids = sim.units().ids();
    let harm = dealt(&mut sim, ids[0], ids[1], 100);

    assert!(
        harm > 500,
        "did {harm} to a soldier, which is a charge rather than a pistol"
    );
}

#[test]
fn the_choice_is_not_a_stance_the_player_sets() {
    // The same unit, no orders in between, both things done well. A stance
    // would mean the second one came out wrong.
    let mut sim = scenario(vec![
        (0, "commando", 20, 20),
        (1, "bunker", 24, 20),
        (1, "soldier", 20, 23),
    ]);
    let ids = sim.units().ids();
    let (commando, bunker, soldier) = (ids[0], ids[1], ids[2]);

    let on_concrete = dealt(&mut sim, commando, bunker, 60);
    let on_flesh = dealt(&mut sim, commando, soldier, 60);

    assert!(
        on_concrete > 300,
        "the building barely noticed: {on_concrete}"
    );
    assert!(on_flesh > 300, "the soldier barely noticed: {on_flesh}");
}

// -- What the restriction must not break ------------------------------------

#[test]
fn a_restricted_action_does_not_narrow_the_search() {
    // The trap this repeats one level down from the layer mask: the search uses
    // the longest-reaching weapon, and taking its restriction with it would
    // leave a commando unable to notice a soldier at all.
    let mut sim = scenario(vec![(0, "commando", 20, 20), (1, "soldier", 22, 20)]);
    let ids = sim.units().ids();
    let (commando, soldier) = (ids[0], ids[1]);
    let before = sim.unit(soldier).unwrap().health;

    // No order at all — this is auto-acquisition, which is where the union of
    // the two actions' categories matters.
    for _ in 0..100 {
        sim.tick(&[]);
    }

    let _ = commando;
    assert!(
        sim.unit(soldier).unwrap().health < before,
        "the commando never noticed a soldier standing next to it"
    );
}

#[test]
fn an_unrestricted_weapon_still_shoots_everything() {
    // Every rules file written before actions existed names no categories and
    // must keep meaning "anything". The soldier's rifle here is exactly such a
    // weapon, and it has to reach a building as happily as a person.
    let mut sim = scenario(vec![(0, "militia", 20, 20), (1, "bunker", 23, 20)]);
    let ids = sim.units().ids();
    assert!(
        dealt(&mut sim, ids[0], ids[1], 100) > 0,
        "an unrestricted weapon stopped reaching a building"
    );

    let mut against_people = scenario(vec![(0, "militia", 20, 20), (1, "soldier", 23, 20)]);
    let ids = against_people.units().ids();
    assert!(
        dealt(&mut against_people, ids[0], ids[1], 100) > 0,
        "an unrestricted weapon stopped reaching a person"
    );
}

#[test]
fn actions_are_deterministic() {
    let run = || {
        let mut sim = scenario(vec![
            (0, "commando", 20, 20),
            (1, "bunker", 24, 20),
            (1, "soldier", 20, 23),
        ]);
        let ids = sim.units().ids();
        dealt(&mut sim, ids[0], ids[1], 100);
        dealt(&mut sim, ids[0], ids[2], 100);
        sim.state_hash()
    };
    assert_eq!(run(), run());
}
