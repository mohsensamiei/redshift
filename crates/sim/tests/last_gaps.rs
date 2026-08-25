//! The last seven capabilities the roster asked for.
//!
//! Walls, per-structure placement, mind control, teleportation, disable,
//! planted charges and disguise. They are together in one file because they
//! were built together and because each is small — what they have in common is
//! that every one of them had a unit waiting on it.

use redshift_data::map::Ground;
use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Locomotor, PowerEffect, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::EntityId;
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map, Terrain};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn armour() -> ArmourTable {
    ron::from_str(
        r#"( classes: ["none"],
             table: { "shot": { "none": 100 }, "psychic": { "none": 100 } } )"#,
    )
    .unwrap()
}

fn gun(id: &str, extra: &str) -> WeaponDef {
    let mut w = WeaponDef {
        id: id.into(),
        damage: 20,
        warhead: "shot".into(),
        reload: Ticks(15),
        range: Hundredths(500),
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
    };
    if extra == "mind" {
        w.mind_control = true;
        w.warhead = "psychic".into();
    }
    w
}

fn mobile(id: &str, category: &str, extra: Vec<Trait>) -> EntityDef {
    let mut traits = vec![
        Trait::Health {
            max: 400,
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
        category: category.into(),
        traits,
    }
}

fn structure(id: &str, extra: Vec<Trait>) -> EntityDef {
    let mut traits = vec![
        Trait::Health {
            max: 800,
            armour: "none".into(),
        },
        Trait::Vision {
            range: Hundredths(400),
        },
        Trait::Buildable {
            cost: 500,
            build_time: Ticks(10),
            prerequisites: vec![],
            produced_by: "yard".into(),
        },
    ];
    traits.extend(extra);
    EntityDef {
        id: id.into(),
        name_key: format!("structure.{id}"),
        side: None,
        category: "structure".into(),
        traits,
    }
}

fn rules() -> Rules {
    Rules::from_parts(
        vec![
            structure(
                "yard",
                vec![
                    Trait::Footprint {
                        width: 3,
                        height: 3,
                    },
                    Trait::Produces {
                        categories: vec!["structure".into()],
                    },
                ],
            ),
            // A wall: one cell, connects to its own kind.
            structure("wall", vec![Trait::Connects]),
            // Must touch water. Everything else must not be on it.
            structure(
                "shipyard",
                vec![
                    Trait::Footprint {
                        width: 2,
                        height: 2,
                    },
                    Trait::NeedsAdjacent {
                        terrain: Ground::Water,
                    },
                ],
            ),
            structure(
                "chronosphere",
                vec![Trait::Superweapon {
                    charge: Ticks(20),
                    effect: PowerEffect::Teleport {
                        radius: Hundredths(400),
                        carries: vec!["vehicle".into()],
                    },
                }],
            ),
            structure(
                "emp",
                vec![Trait::Superweapon {
                    charge: Ticks(20),
                    effect: PowerEffect::Disables {
                        radius: Hundredths(400),
                        duration: Ticks(200),
                    },
                }],
            ),
            structure(
                "defence",
                vec![
                    Trait::Armed {
                        weapon: "rifle".into(),
                        turret: true,
                        turret_rate: 3600,
                    },
                    Trait::PowerDraw {
                        amount: 10,
                        works_unpowered: true,
                    },
                ],
            ),
            mobile("tank", "vehicle", vec![]),
            mobile(
                "gunner",
                "infantry",
                vec![Trait::Armed {
                    weapon: "rifle".into(),
                    turret: true,
                    turret_rate: 3600,
                }],
            ),
            mobile(
                "yuri",
                "infantry",
                vec![Trait::Armed {
                    weapon: "psi".into(),
                    turret: true,
                    turret_rate: 3600,
                }],
            ),
            mobile(
                "ivan",
                "infantry",
                vec![
                    Trait::Armed {
                        weapon: "rifle".into(),
                        turret: true,
                        turret_rate: 3600,
                    },
                    Trait::PlantsCharge {
                        fuse: Ticks(60),
                        damage: 600,
                        warhead: "shot".into(),
                        radius: Hundredths(150),
                        categories: vec!["vehicle".into()],
                    },
                ],
            ),
            mobile(
                "spy",
                "infantry",
                vec![Trait::Disguised {
                    looks_like: "gunner".into(),
                }],
            ),
            mobile("dog", "infantry", vec![Trait::Detector]),
        ],
        vec![gun("rifle", ""), gun("psi", "mind")],
        armour(),
        Vec::new(),
    )
    .expect("valid rules")
}

fn lake_map() -> Map {
    let mut map = Map::new(48, 48);
    map.fill_rect(Cell::new(30, 20), Cell::new(40, 30), Terrain::Water);
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
    Sim::new(MatchSetup {
        seed: 0x_1A57,
        map: lake_map(),
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

fn fire(sim: &mut Sim, owner: u8, building: EntityId, at: Cell, to: Option<Cell>) {
    sim.tick(&[Command::new(
        PlayerId(owner),
        0,
        CommandKind::FirePower { building, at, to },
    )]);
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

fn kind(sim: &Sim, id: &str) -> redshift_sim::EntityKind {
    sim.rules().kind_of(id).unwrap()
}

// -- Placement rules --------------------------------------------------------

#[test]
fn a_shipyard_must_touch_water() {
    let sim = scenario(vec![(0, "yard", 28, 24)]);
    let shipyard = kind(&sim, "shipyard");
    assert!(
        sim.can_place_kind(PlayerId(0), shipyard, Cell::new(28, 21)),
        "beside the lake should be legal"
    );
    assert!(
        !sim.can_place_kind(PlayerId(0), shipyard, Cell::new(24, 24)),
        "inland should not be"
    );
}

#[test]
fn everything_else_is_unaffected_by_the_water() {
    // The rule is a requirement on one structure, not a new global condition.
    let sim = scenario(vec![(0, "yard", 28, 24)]);
    assert!(sim.can_place_kind(PlayerId(0), kind(&sim, "wall"), Cell::new(24, 24)));
}

#[test]
fn a_wall_declares_that_it_connects() {
    // The simulation does not care — a wall blocks ground because it has a
    // footprint, like any building. This is for the renderer, and for the drag
    // placement that comes with it.
    let sim = scenario(vec![(0, "yard", 28, 24)]);
    assert!(sim.stats().get(PlayerId(0), kind(&sim, "wall")).connects);
    assert!(!sim.stats().get(PlayerId(0), kind(&sim, "yard")).connects);
}

// -- Mind control -----------------------------------------------------------

#[test]
fn mind_control_takes_a_unit_rather_than_hurting_it() {
    let mut sim = scenario(vec![(0, "yuri", 10, 10), (1, "tank", 13, 10)]);
    let ids = sim.units().ids();
    let (yuri, tank) = (ids[0], ids[1]);
    let before = sim.unit(tank).unwrap().health;

    attack(&mut sim, 0, yuri, tank);
    run(&mut sim, 60);

    let taken = sim.unit(tank).expect("it should still exist");
    assert_eq!(taken.owner, PlayerId(0), "it did not change sides");
    assert_eq!(taken.health, before, "a controlled tank should be unhurt");
}

#[test]
fn killing_the_controller_gives_it_back() {
    // The whole counter, and the reason it is an ability rather than a way to
    // win. Without it the effect is permanent and unanswerable.
    let mut sim = scenario(vec![
        (0, "yuri", 10, 10),
        (1, "tank", 13, 10),
        (1, "gunner", 12, 10),
    ]);
    let ids = sim.units().ids();
    let (yuri, tank, gunner) = (ids[0], ids[1], ids[2]);

    attack(&mut sim, 0, yuri, tank);
    run(&mut sim, 60);
    assert_eq!(sim.unit(tank).unwrap().owner, PlayerId(0));

    attack(&mut sim, 1, gunner, yuri);
    for _ in 0..400 {
        sim.tick(&[]);
        if sim.unit(yuri).is_none() {
            break;
        }
    }
    assert!(sim.unit(yuri).is_none(), "the controller would not die");

    assert_eq!(
        sim.unit(tank).unwrap().owner,
        PlayerId(1),
        "it should have gone home"
    );
}

// -- Teleportation ----------------------------------------------------------

#[test]
fn a_chronosphere_moves_what_it_covers() {
    let mut sim = scenario(vec![(0, "chronosphere", 10, 10), (0, "tank", 12, 10)]);
    let ids = sim.units().ids();
    let (sphere, tank) = (ids[0], ids[1]);
    run(&mut sim, 25);

    fire(
        &mut sim,
        0,
        sphere,
        Cell::new(12, 10),
        Some(Cell::new(40, 40)),
    );
    sim.tick(&[]);

    let at = sim.unit(tank).unwrap().cell();
    assert!(
        at.chebyshev_to(Cell::new(40, 40)) <= 5,
        "it ended up at {at:?}"
    );
}

#[test]
fn it_moves_only_what_it_is_allowed_to_carry() {
    // The original cannot move infantry, and that is a rule about the power
    // rather than about infantry — so it lives on the power.
    let mut sim = scenario(vec![(0, "chronosphere", 10, 10), (0, "gunner", 12, 10)]);
    let ids = sim.units().ids();
    let (sphere, walker) = (ids[0], ids[1]);
    run(&mut sim, 25);

    fire(
        &mut sim,
        0,
        sphere,
        Cell::new(12, 10),
        Some(Cell::new(40, 40)),
    );
    sim.tick(&[]);

    assert!(
        sim.unit(walker)
            .unwrap()
            .cell()
            .chebyshev_to(Cell::new(12, 10))
            <= 2,
        "infantry was carried"
    );
}

#[test]
fn without_a_destination_it_does_nothing() {
    let mut sim = scenario(vec![(0, "chronosphere", 10, 10), (0, "tank", 12, 10)]);
    let ids = sim.units().ids();
    run(&mut sim, 25);
    let before = sim.unit(ids[1]).unwrap().cell();

    fire(&mut sim, 0, ids[0], Cell::new(12, 10), None);
    sim.tick(&[]);

    assert_eq!(sim.unit(ids[1]).unwrap().cell(), before);
    // And the charge is not spent on an order the player did not finish.
    assert!(sim.power_ready(ids[0]));
}

// -- Disable ----------------------------------------------------------------

#[test]
fn an_emp_switches_a_defence_off() {
    let mut sim = scenario(vec![
        (0, "emp", 10, 10),
        (1, "defence", 30, 10),
        (1, "gunner", 31, 10),
    ]);
    let ids = sim.units().ids();
    let (emp, defence) = (ids[0], ids[1]);
    run(&mut sim, 25);

    fire(&mut sim, 0, emp, Cell::new(30, 10), None);
    sim.tick(&[]);

    let hit = sim.unit(defence).unwrap();
    assert!(sim.is_disabled(hit), "it kept working");
    assert_eq!(hit.health, 800, "an EMP should not damage anything");
}

#[test]
fn it_wears_off() {
    let mut sim = scenario(vec![(0, "emp", 10, 10), (1, "defence", 30, 10)]);
    let ids = sim.units().ids();
    run(&mut sim, 25);
    fire(&mut sim, 0, ids[0], Cell::new(30, 10), None);
    run(&mut sim, 300);

    assert!(!sim.is_disabled(sim.unit(ids[1]).unwrap()));
}

// -- Planted charges --------------------------------------------------------

#[test]
fn a_charge_goes_off_later_rather_than_now() {
    // The delay is the whole mechanic. A bomb that detonated on contact would
    // be an ordinary weapon with a large number.
    let mut sim = scenario(vec![(0, "ivan", 10, 10), (1, "tank", 12, 10)]);
    let ids = sim.units().ids();
    let (ivan, tank) = (ids[0], ids[1]);

    attack(&mut sim, 0, ivan, tank);
    run(&mut sim, 20);

    assert!(
        sim.unit(tank).is_some_and(|t| t.charge_planted.is_some()),
        "no charge was planted"
    );
    assert_eq!(
        sim.unit(tank).unwrap().health,
        400,
        "it went off immediately"
    );

    run(&mut sim, 120);
    assert!(sim.unit(tank).is_none(), "it never went off");
}

#[test]
fn a_charge_rides_on_what_it_was_planted_on() {
    // A bombed tank driving into a crowd takes the crowd with it, which is why
    // a player runs away from one rather than towards it.
    let mut sim = scenario(vec![(0, "ivan", 10, 10), (1, "tank", 12, 10)]);
    let ids = sim.units().ids();
    let (ivan, tank) = (ids[0], ids[1]);
    attack(&mut sim, 0, ivan, tank);
    run(&mut sim, 20);
    assert!(sim.unit(tank).unwrap().charge_planted.is_some());

    sim.tick(&[Command::new(
        PlayerId(1),
        0,
        CommandKind::Move {
            units: vec![tank],
            target: Cell::new(24, 10),
        },
    )]);
    run(&mut sim, 30);

    let carried = sim.unit(tank).unwrap();
    assert!(carried.cell().x > 12, "it did not move");
    assert!(carried.charge_planted.is_some(), "the charge fell off");
}

#[test]
fn only_one_charge_at_a_time() {
    // A second would overwrite the first, and one of the two bombs would simply
    // never have existed.
    let mut sim = scenario(vec![
        (0, "ivan", 10, 10),
        (0, "ivan", 10, 12),
        (1, "tank", 12, 11),
    ]);
    let ids = sim.units().ids();
    attack(&mut sim, 0, ids[0], ids[2]);
    attack(&mut sim, 0, ids[1], ids[2]);
    run(&mut sim, 20);

    let charge = sim.unit(ids[2]).and_then(|t| t.charge_planted);
    assert!(charge.is_some());
}

#[test]
fn it_will_not_bomb_what_it_is_not_allowed_to() {
    let mut sim = scenario(vec![(0, "ivan", 10, 10), (1, "gunner", 12, 10)]);
    let ids = sim.units().ids();
    attack(&mut sim, 0, ids[0], ids[1]);
    run(&mut sim, 40);

    assert!(
        sim.unit(ids[1]).is_none_or(|u| u.charge_planted.is_none()),
        "infantry is not on the list"
    );
}

// -- Disguise ---------------------------------------------------------------

#[test]
fn a_spy_looks_like_something_else_to_the_enemy() {
    let mut sim = scenario(vec![(0, "spy", 10, 10), (1, "gunner", 13, 10)]);
    run(&mut sim, 60);
    let spy = sim.units().ids()[0];
    let unit = sim.unit(spy).unwrap();

    assert_eq!(
        sim.appears_as(PlayerId(1), unit),
        kind(&sim, "gunner"),
        "the enemy saw through it"
    );
}

#[test]
fn its_own_side_sees_the_truth() {
    // Or a player could not tell their own spy from their own infantry, which
    // is a strange thing to do to somebody who paid for it.
    let mut sim = scenario(vec![(0, "spy", 10, 10)]);
    run(&mut sim, 60);
    let spy = sim.units().ids()[0];
    let unit = sim.unit(spy).unwrap();
    assert_eq!(sim.appears_as(PlayerId(0), unit), kind(&sim, "spy"));
}

#[test]
fn a_detector_sees_through_it() {
    // Researched, and shared with the cloak because the original shares it.
    let mut sim = scenario(vec![(0, "spy", 10, 10), (1, "dog", 12, 10)]);
    run(&mut sim, 60);
    let spy = sim.units().ids()[0];
    let unit = sim.unit(spy).unwrap();
    assert_eq!(sim.appears_as(PlayerId(1), unit), kind(&sim, "spy"));
}

#[test]
fn the_simulation_still_knows_what_it_is() {
    // A disguise fools players, not physics. A version that swapped the kind in
    // the arena would have a Mirage Tank shooting like a tree.
    let mut sim = scenario(vec![(0, "spy", 10, 10), (1, "gunner", 13, 10)]);
    run(&mut sim, 60);
    let spy = sim.units().ids()[0];
    assert_eq!(sim.unit(spy).unwrap().kind, kind(&sim, "spy"));
}

#[test]
fn the_last_gaps_are_deterministic() {
    let go = || {
        let mut sim = scenario(vec![
            (0, "yuri", 10, 10),
            (0, "ivan", 10, 12),
            (0, "spy", 10, 14),
            (1, "tank", 13, 11),
        ]);
        let ids = sim.units().ids();
        attack(&mut sim, 0, ids[0], ids[3]);
        attack(&mut sim, 0, ids[1], ids[3]);
        run(&mut sim, 300);
        sim.state_hash()
    };
    assert_eq!(go(), go());
}
