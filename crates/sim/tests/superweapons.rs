//! Weapons that charge over time and are fired at a place.
//!
//! Not a very long-ranged gun with a very long reload. A superweapon has no
//! range, no target and no reload — it has a *timer*, and the player chooses
//! where it lands. Everything below turns on that difference.
//!
//! The charge lives on the building, which is faithful and is also what makes a
//! silo worth attacking: losing one three seconds before it fires costs the
//! whole wait.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Locomotor, PowerEffect, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::EntityId;
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

const CHARGE: u32 = 60;

fn armour() -> ArmourTable {
    ron::from_str(
        r#"( classes: ["flesh", "steel"],
             table: { "atomic": { "flesh": 100, "steel": 100 },
                      "shot": { "flesh": 100, "steel": 100 } } )"#,
    )
    .unwrap()
}

fn silo(id: &str, effect: PowerEffect, draws_power: bool) -> EntityDef {
    let mut traits = vec![
        Trait::Health {
            max: 1_000,
            armour: "steel".into(),
        },
        Trait::Vision {
            range: Hundredths(400),
        },
        Trait::Superweapon {
            charge: Ticks(CHARGE),
            effect,
        },
        Trait::Buildable {
            cost: 5_000,
            build_time: Ticks(100),
            prerequisites: vec![],
            produced_by: "silo".into(),
        },
    ];
    if draws_power {
        traits.push(Trait::PowerDraw {
            amount: 200,
            works_unpowered: false,
        });
    }
    EntityDef {
        id: id.into(),
        name_key: format!("structure.{id}"),
        side: None,
        category: "structure".into(),
        traits,
    }
}

fn body(id: &str, category: &str, crushable: bool) -> EntityDef {
    let mut traits = vec![
        Trait::Health {
            max: 400,
            armour: if crushable { "flesh" } else { "steel" }.into(),
        },
        Trait::Mobile {
            speed: Hundredths(200),
            turn_rate: 3600,
            locomotor: Locomotor::Foot,
            surfaces: None,
            size: None,
            layer: None,
        },
        Trait::Vision {
            range: Hundredths(300),
        },
    ];
    if crushable {
        traits.push(Trait::Crushable {
            class: "infantry".into(),
        });
    }
    EntityDef {
        id: id.into(),
        name_key: format!("unit.{id}"),
        side: None,
        category: category.into(),
        traits,
    }
}

fn rules() -> Rules {
    Rules::from_parts(
        vec![
            silo(
                "silo",
                PowerEffect::Blast {
                    radius: Hundredths(300),
                    damage: 500,
                    warhead: "atomic".into(),
                    fallout: Ticks(200),
                },
                false,
            ),
            silo(
                "hungry_silo",
                PowerEffect::Blast {
                    radius: Hundredths(300),
                    damage: 500,
                    warhead: "atomic".into(),
                    fallout: Ticks(0),
                },
                true,
            ),
            silo(
                "spy_plane",
                PowerEffect::Reveal {
                    radius: Hundredths(800),
                },
                false,
            ),
            silo(
                "airfield",
                PowerEffect::Paradrop {
                    units: vec!["trooper".into(), "trooper".into(), "trooper".into()],
                },
                false,
            ),
            silo(
                "curtain",
                PowerEffect::IronCurtain {
                    radius: Hundredths(400),
                    duration: Ticks(100),
                },
                false,
            ),
            body("trooper", "infantry", true),
            body("tank", "vehicle", false),
            EntityDef {
                traits: vec![
                    Trait::Health {
                        max: 400,
                        armour: "steel".into(),
                    },
                    Trait::Vision {
                        range: Hundredths(300),
                    },
                    Trait::Armed {
                        weapon: "gun".into(),
                        turret: true,
                        turret_rate: 3600,
                    },
                ],
                ..body("turret", "structure", false)
            },
        ],
        vec![WeaponDef {
            id: "gun".into(),
            damage: 100,
            warhead: "shot".into(),
            // Slow, so a target survives the charge and the test is about the
            // shield rather than about who shot first.
            reload: Ticks(45),
            range: Hundredths(600),
            splash_radius: Hundredths::ZERO,
            projectile_speed: Hundredths::ZERO,
            homing: false,
            targets: vec![],
            target_categories: vec![],
            instant_kill: false,
            ammo: 0,
            intercepts: false,
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
        seed: 0x_5AFE,
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

fn fire(sim: &mut Sim, owner: u8, building: EntityId, at: Cell) {
    sim.tick(&[Command::new(
        PlayerId(owner),
        0,
        CommandKind::FirePower {
            building,
            at,
            to: None,
        },
    )]);
}

// -- Charging ---------------------------------------------------------------

#[test]
fn a_superweapon_is_not_ready_immediately() {
    let sim = scenario(vec![(0, "silo", 10, 10)]);
    let id = sim.units().ids()[0];
    assert!(!sim.power_ready(id));
    assert_eq!(sim.power_progress(id), Some(0));
}

#[test]
fn it_becomes_ready_after_its_charge_time() {
    let mut sim = scenario(vec![(0, "silo", 10, 10)]);
    let id = sim.units().ids()[0];
    run(&mut sim, CHARGE + 2);
    assert!(sim.power_ready(id));
}

#[test]
fn an_unpowered_silo_does_not_charge() {
    // And does not lose what it has either, so cutting an enemy's power delays
    // their missile rather than cancelling it — the more interesting of the two.
    let mut sim = scenario(vec![(0, "hungry_silo", 10, 10)]);
    let id = sim.units().ids()[0];
    run(&mut sim, CHARGE * 2);
    assert!(!sim.power_ready(id));
    assert_eq!(sim.power_progress(id), Some(0));
}

#[test]
fn firing_spends_the_charge() {
    let mut sim = scenario(vec![(0, "silo", 10, 10), (1, "tank", 30, 30)]);
    let ids = sim.units().ids();
    run(&mut sim, CHARGE + 2);
    fire(&mut sim, 0, ids[0], Cell::new(30, 30));
    assert!(!sim.power_ready(ids[0]));
}

#[test]
fn an_uncharged_superweapon_does_nothing() {
    let mut sim = scenario(vec![(0, "silo", 10, 10), (1, "tank", 30, 30)]);
    let ids = sim.units().ids();
    let before = sim.unit(ids[1]).unwrap().health;
    fire(&mut sim, 0, ids[0], Cell::new(30, 30));
    run(&mut sim, 5);
    assert_eq!(sim.unit(ids[1]).unwrap().health, before);
}

#[test]
fn a_player_cannot_fire_someone_elses_silo() {
    let mut sim = scenario(vec![(1, "silo", 10, 10), (1, "tank", 30, 30)]);
    let ids = sim.units().ids();
    run(&mut sim, CHARGE + 2);
    fire(&mut sim, 0, ids[0], Cell::new(30, 30));
    assert!(sim.power_ready(ids[0]), "the charge was spent by an enemy");
}

#[test]
fn the_charge_dies_with_the_building() {
    // The reason a silo is worth attacking. Losing one three seconds before it
    // fires costs the whole wait.
    let mut sim = scenario(vec![(0, "silo", 10, 10)]);
    let id = sim.units().ids()[0];
    run(&mut sim, CHARGE / 2);
    assert!(sim.power_progress(id).is_some_and(|p| p > 0));

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Sell { building: id },
    )]);
    run(&mut sim, 10);
    assert!(sim.unit(id).is_none());
}

// -- What they do -----------------------------------------------------------

#[test]
fn a_blast_hurts_what_it_lands_on() {
    let mut sim = scenario(vec![(0, "silo", 10, 10), (1, "tank", 30, 30)]);
    let ids = sim.units().ids();
    run(&mut sim, CHARGE + 2);
    fire(&mut sim, 0, ids[0], Cell::new(30, 30));
    run(&mut sim, 3);
    assert!(sim.unit(ids[1]).is_none_or(|u| u.health < 400));
}

#[test]
fn a_blast_leaves_the_ground_dangerous() {
    // The crater outlasting the blast is most of what separates a nuclear
    // missile from a very large shell.
    let mut sim = scenario(vec![(0, "silo", 10, 10)]);
    let id = sim.units().ids()[0];
    assert!(sim.hazards().is_empty());
    run(&mut sim, CHARGE + 2);
    fire(&mut sim, 0, id, Cell::new(30, 30));
    run(&mut sim, 2);
    assert!(sim.hazards().iter().any(|h| h.cell == Cell::new(30, 30)));
}

#[test]
fn a_reveal_shows_ground_the_player_had_not_seen() {
    let mut sim = scenario(vec![(0, "spy_plane", 10, 10)]);
    let id = sim.units().ids()[0];
    let far = Cell::new(40, 40);
    assert!(!sim.visibility().is_explored(PlayerId(0), far));

    run(&mut sim, CHARGE + 2);
    fire(&mut sim, 0, id, far);

    assert!(sim.visibility().is_explored(PlayerId(0), far));
}

#[test]
fn a_paradrop_puts_units_on_the_ground() {
    let mut sim = scenario(vec![(0, "airfield", 10, 10)]);
    let id = sim.units().ids()[0];
    run(&mut sim, CHARGE + 2);
    fire(&mut sim, 0, id, Cell::new(40, 40));

    let trooper = sim.rules().kind_of("trooper").unwrap();
    let dropped: Vec<Cell> = sim
        .units()
        .iter()
        .filter(|(_, u)| u.kind == trooper)
        .map(|(_, u)| u.cell())
        .collect();
    assert_eq!(dropped.len(), 3, "three were meant to land");
    for cell in &dropped {
        assert!(
            cell.chebyshev_to(Cell::new(40, 40)) <= 5,
            "one landed at {cell:?}, nowhere near the drop"
        );
    }
    // Beside each other rather than stacked: a squad that landed on one cell
    // would spend the next few seconds untangling itself.
    assert_eq!(
        dropped
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
}

// -- The Iron Curtain -------------------------------------------------------

#[test]
fn an_iron_curtain_makes_vehicles_invulnerable() {
    let mut sim = scenario(vec![
        (0, "curtain", 10, 10),
        (0, "tank", 30, 30),
        (1, "turret", 32, 30),
    ]);
    let ids = sim.units().ids();
    let (curtain, tank) = (ids[0], ids[1]);
    run(&mut sim, CHARGE + 2);
    fire(&mut sim, 0, curtain, Cell::new(30, 30));

    let before = sim.unit(tank).unwrap().health;
    run(&mut sim, 60);

    assert_eq!(
        sim.unit(tank).unwrap().health,
        before,
        "a shielded tank took damage"
    );
}

#[test]
fn it_wears_off() {
    let mut sim = scenario(vec![
        (0, "curtain", 10, 10),
        (0, "tank", 30, 30),
        (1, "turret", 32, 30),
    ]);
    let ids = sim.units().ids();
    let (curtain, tank) = (ids[0], ids[1]);
    run(&mut sim, CHARGE + 2);
    fire(&mut sim, 0, curtain, Cell::new(30, 30));
    run(&mut sim, 200);

    assert!(
        sim.unit(tank).is_none_or(|u| u.health < 400),
        "the shield never expired"
    );
}

#[test]
fn it_kills_the_infantry_it_covers() {
    // The part everyone forgets. It is not a protective bubble that happens to
    // exclude infantry — it kills them.
    let mut sim = scenario(vec![
        (0, "curtain", 10, 10),
        (0, "trooper", 30, 30),
        (0, "tank", 31, 30),
    ]);
    let ids = sim.units().ids();
    let (curtain, trooper, tank) = (ids[0], ids[1], ids[2]);
    run(&mut sim, CHARGE + 2);
    fire(&mut sim, 0, curtain, Cell::new(30, 30));
    run(&mut sim, 3);

    assert!(sim.unit(trooper).is_none(), "the infantryman survived it");
    assert!(sim.unit(tank).is_some(), "the tank should have been spared");
}

#[test]
fn superweapons_are_deterministic() {
    let go = || {
        let mut sim = scenario(vec![
            (0, "silo", 10, 10),
            (0, "curtain", 14, 10),
            (1, "tank", 30, 30),
        ]);
        let ids = sim.units().ids();
        run(&mut sim, CHARGE + 2);
        fire(&mut sim, 0, ids[0], Cell::new(30, 30));
        fire(&mut sim, 0, ids[1], Cell::new(10, 10));
        run(&mut sim, 100);
        sim.state_hash()
    };
    assert_eq!(go(), go());
}
