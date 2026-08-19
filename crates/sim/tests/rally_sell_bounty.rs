//! Rally points, selling, and bounties.
//!
//! Three small independent rules, each with one detail that makes it a decision
//! rather than a formula.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::EntityId;
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn rules() -> Rules {
    let armour: ArmourTable =
        ron::from_str(r#"( classes: ["none"], table: { "shot": { "none": 100 } } )"#).unwrap();

    let mobile = Trait::Mobile {
        speed: Hundredths(600),
        turn_rate: 3600,
        locomotor: Locomotor::Foot,
        surfaces: None,
        size: None,
        layer: None,
    };

    let tank = EntityDef {
        id: "tank".into(),
        name_key: "u.tank".into(),
        side: None,
        category: "vehicle".into(),
        traits: vec![
            Trait::Health {
                max: 200,
                armour: "none".into(),
            },
            mobile.clone(),
            Trait::Vision {
                range: Hundredths(600),
            },
            Trait::Buildable {
                cost: 100,
                build_time: Ticks(10),
                prerequisites: vec![],
                produced_by: "factory".into(),
            },
        ],
    };
    let shooter = EntityDef {
        id: "shooter".into(),
        name_key: "u.shooter".into(),
        side: None,
        category: "infantry".into(),
        traits: vec![
            Trait::Health {
                max: 200,
                armour: "none".into(),
            },
            mobile.clone(),
            Trait::Vision {
                range: Hundredths(800),
            },
            Trait::Armed {
                weapon: "rifle".into(),
                turret: true,
                turret_rate: 3600,
            },
        ],
    };
    let civilian = EntityDef {
        id: "civilian".into(),
        name_key: "u.civilian".into(),
        side: None,
        category: "infantry".into(),
        traits: vec![
            Trait::Health {
                max: 50,
                armour: "none".into(),
            },
            mobile,
            Trait::Vision {
                range: Hundredths(200),
            },
            Trait::Bounty { credits: 5 },
        ],
    };
    let factory = EntityDef {
        id: "factory".into(),
        name_key: "b.factory".into(),
        side: None,
        category: "structure".into(),
        traits: vec![
            Trait::Health {
                max: 1000,
                armour: "none".into(),
            },
            Trait::Vision {
                range: Hundredths(400),
            },
            Trait::Produces {
                categories: vec!["vehicle".into()],
            },
            Trait::Buildable {
                cost: 2000,
                build_time: Ticks(50),
                prerequisites: vec![],
                produced_by: "factory".into(),
            },
        ],
    };

    Rules::from_parts(
        vec![tank, shooter, civilian, factory],
        vec![WeaponDef {
            id: "rifle".into(),
            damage: 30,
            warhead: "shot".into(),
            reload: Ticks(8),
            range: Hundredths(600),
            splash_radius: Hundredths::ZERO,
            projectile_speed: Hundredths::ZERO,
            homing: false,
            targets: vec![],
            instant_kill: false,
            ammo: 0,
            intercepts: false,
        }],
        armour,
        Vec::new(),
    )
    .expect("rules")
}

fn scenario(spawns: Vec<(PlayerId, &str, i32, i32)>) -> Sim {
    let rules = rules();
    Sim::new(MatchSetup {
        seed: 0x_A11,
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
                owner,
                kind: rules.kind_of(kind).unwrap_or_else(|| panic!("no {kind}")),
                pos: Cell::new(x, y).centre(),
            })
            .collect(),
        rules,
    })
}

/// Builds one tank and returns it.
fn build_a_tank(sim: &mut Sim, factory: EntityId) -> Option<EntityId> {
    let kind = sim.rules().kind_of("tank").expect("tank");
    let before: Vec<EntityId> = sim.units().ids();
    sim.tick(&[Command::new(
        PlayerId(0),
        1,
        CommandKind::Produce {
            building: factory,
            kind,
        },
    )]);
    for _ in 0..2_000 {
        sim.tick(&[]);
        if let Some(id) = sim
            .units()
            .iter()
            .find(|(id, u)| u.kind == kind && !before.contains(id))
            .map(|(id, _)| id)
        {
            return Some(id);
        }
    }
    None
}

#[test]
fn a_new_unit_walks_to_the_rally_point() {
    // Without this a factory builds a wall of its own units in front of its
    // own exit.
    let mut sim = scenario(vec![(PlayerId(0), "factory", 10, 10)]);
    let factory = sim.units().ids()[0];
    let rally = Cell::new(30, 30);

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::SetRally {
            building: factory,
            at: rally,
        },
    )]);
    let tank = build_a_tank(&mut sim, factory).expect("a tank should be built");

    for _ in 0..4_000 {
        sim.tick(&[]);
        if sim
            .units()
            .get(tank)
            .is_some_and(|u| u.cell().chebyshev_to(rally) <= 3)
        {
            return;
        }
    }
    let where_it_is = sim.units().get(tank).map(|u| u.cell());
    panic!("the tank stopped at {where_it_is:?} instead of walking to {rally:?}");
}

#[test]
fn a_rally_point_outlives_the_thing_that_was_being_built() {
    // It is set on the building rather than on its queue, because a player
    // expects it to persist across an empty queue.
    let mut sim = scenario(vec![(PlayerId(0), "factory", 10, 10)]);
    let factory = sim.units().ids()[0];
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::SetRally {
            building: factory,
            at: Cell::new(30, 30),
        },
    )]);
    build_a_tank(&mut sim, factory).expect("first tank");

    for _ in 0..200 {
        sim.tick(&[]);
    }
    assert_eq!(
        sim.units().get(factory).unwrap().rally,
        Some(Cell::new(30, 30)),
        "the rally point was forgotten once the queue emptied"
    );
}

#[test]
fn selling_a_structure_pays_and_removes_it() {
    let mut sim = scenario(vec![(PlayerId(0), "factory", 10, 10)]);
    let factory = sim.units().ids()[0];
    let before = sim.treasury().credits(PlayerId(0));

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Sell { building: factory },
    )]);
    sim.tick(&[]);

    assert!(
        sim.units().get(factory).is_none(),
        "the factory was not demolished"
    );
    assert!(
        sim.treasury().credits(PlayerId(0)) > before,
        "selling paid nothing"
    );
}

#[test]
fn a_damaged_structure_is_worth_less() {
    // Otherwise selling becomes a way to launder damage into money.
    let mut whole = scenario(vec![(PlayerId(0), "factory", 10, 10)]);
    let id = whole.units().ids()[0];
    let before = whole.treasury().credits(PlayerId(0));
    whole.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Sell { building: id },
    )]);
    let full_price = whole.treasury().credits(PlayerId(0)) - before;

    let mut hurt = scenario(vec![
        (PlayerId(0), "factory", 10, 10),
        (PlayerId(1), "shooter", 14, 10),
    ]);
    let id = hurt.units().ids()[0];
    // Enough to hurt it, well short of the ~270 ticks it takes to level a
    // thousand-health building at this rate of fire.
    for _ in 0..120 {
        hurt.tick(&[]);
    }
    assert!(
        hurt.units().get(id).unwrap().health < 1000,
        "the test needs a damaged building"
    );
    let before = hurt.treasury().credits(PlayerId(0));
    hurt.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Sell { building: id },
    )]);
    let hurt_price = hurt.treasury().credits(PlayerId(0)) - before;

    assert!(
        hurt_price < full_price,
        "a wreck sold for the same as a fresh building: {hurt_price} against {full_price}"
    );
}

#[test]
fn a_mobile_unit_cannot_be_sold() {
    // Selling a tank would be an odd thing to allow and a very easy way to turn
    // an army into cash mid-battle.
    let mut sim = scenario(vec![(PlayerId(0), "tank", 10, 10)]);
    let tank = sim.units().ids()[0];
    let before = sim.treasury().credits(PlayerId(0));

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Sell { building: tank },
    )]);
    sim.tick(&[]);

    assert!(sim.units().get(tank).is_some(), "a tank was sold");
    assert_eq!(sim.treasury().credits(PlayerId(0)), before);
}

#[test]
fn another_player_cannot_sell_your_building() {
    let mut sim = scenario(vec![(PlayerId(0), "factory", 10, 10)]);
    let factory = sim.units().ids()[0];
    sim.tick(&[Command::new(
        PlayerId(1),
        0,
        CommandKind::Sell { building: factory },
    )]);
    sim.tick(&[]);
    assert!(sim.units().get(factory).is_some(), "someone else sold it");
}

#[test]
fn killing_something_with_a_bounty_pays_the_killer() {
    let mut sim = scenario(vec![
        (PlayerId(0), "shooter", 10, 10),
        (PlayerId::NEUTRAL, "civilian", 13, 10),
    ]);
    let (shooter, civilian) = (sim.units().ids()[0], sim.units().ids()[1]);
    let before = sim.treasury().credits(PlayerId(0));

    // Neutrals are not shot at by accident, so the kill has to be ordered.
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Attack {
            units: vec![shooter],
            target: civilian,
        },
    )]);
    for _ in 0..500 {
        sim.tick(&[]);
        if sim.units().get(civilian).is_none() {
            break;
        }
    }
    assert!(sim.units().get(civilian).is_none(), "the civilian survived");
    assert_eq!(
        sim.treasury().credits(PlayerId(0)),
        before + 5,
        "the bounty was not paid"
    );
}

#[test]
fn killing_something_without_a_bounty_pays_nothing() {
    let mut sim = scenario(vec![
        (PlayerId(0), "shooter", 10, 10),
        (PlayerId(1), "tank", 13, 10),
    ]);
    let victim = sim.units().ids()[1];
    let before = sim.treasury().credits(PlayerId(0));

    for _ in 0..600 {
        sim.tick(&[]);
        if sim.units().get(victim).is_none() {
            break;
        }
    }
    assert!(sim.units().get(victim).is_none(), "the tank survived");
    assert_eq!(
        sim.treasury().credits(PlayerId(0)),
        before,
        "something with no bounty paid one anyway"
    );
}

#[test]
fn all_three_are_deterministic() {
    let run = || {
        let mut sim = scenario(vec![
            (PlayerId(0), "factory", 10, 10),
            (PlayerId(0), "shooter", 20, 20),
            (PlayerId::NEUTRAL, "civilian", 23, 20),
        ]);
        let factory = sim.units().ids()[0];
        let shooter = sim.units().ids()[1];
        let civilian = sim.units().ids()[2];
        sim.tick(&[
            Command::new(
                PlayerId(0),
                0,
                CommandKind::SetRally {
                    building: factory,
                    at: Cell::new(30, 30),
                },
            ),
            Command::new(
                PlayerId(0),
                1,
                CommandKind::Attack {
                    units: vec![shooter],
                    target: civilian,
                },
            ),
        ]);
        let kind = sim.rules().kind_of("tank").unwrap();
        sim.tick(&[Command::new(
            PlayerId(0),
            2,
            CommandKind::Produce {
                building: factory,
                kind,
            },
        )]);

        let mut hashes = Vec::new();
        for _ in 0..1_200 {
            sim.tick(&[]);
            hashes.push(sim.state_hash());
        }
        (hashes, sim.treasury().credits(PlayerId(0)))
    };
    let (a, a_credits) = run();
    let (b, b_credits) = run();
    assert_eq!(a, b, "two identical runs diverged");
    assert_eq!(a_credits, b_credits);
}
