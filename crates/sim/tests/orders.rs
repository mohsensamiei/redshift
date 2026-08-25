//! Attack, attack-move and guard.
//!
//! These three exist because a plain move is not enough to fight with. The
//! tests are mostly about the differences between them, since that is where the
//! design lives: what happens when something shoots at a unit on its way
//! somewhere, and whether it is supposed to care.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::EntityId;
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};
use redshift_sim::unit::Order;

fn rules() -> Rules {
    let armour: ArmourTable =
        ron::from_str(r#"( classes: ["none"], table: { "shot": { "none": 100 } } )"#).unwrap();
    let weapons = vec![WeaponDef {
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
    }];
    let soldier = EntityDef {
        id: "soldier".into(),
        name_key: "unit.soldier".into(),
        side: None,
        category: "infantry".into(),
        traits: vec![
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
                range: Hundredths(700),
            },
            Trait::Armed {
                weapon: "rifle".into(),
                turret: true,
                turret_rate: 3600,
            },
        ],
    };
    Rules::from_parts(vec![soldier], weapons, armour, Vec::new()).expect("valid rules")
}

fn scenario(spawns: Vec<(u8, i32, i32)>) -> MatchSetup {
    let rules = rules();
    let kind = rules.kind_of("soldier").unwrap();
    MatchSetup {
        seed: 0x0_D3E,
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
        spawns: spawns
            .into_iter()
            .map(|(owner, x, y)| Spawn {
                owner: PlayerId(owner),
                kind,
                pos: Cell::new(x, y).centre(),
            })
            .collect(),
        rules,
    }
}

fn cmd(kind: CommandKind) -> Command {
    Command::new(PlayerId(0), 0, kind)
}

#[test]
fn a_plain_move_walks_past_a_fight() {
    // A player repositioning an army expects it to arrive, not to stop at the
    // first thing that shoots at it.
    let mut sim = Sim::new(scenario(vec![(0, 5, 20), (1, 20, 20)]));
    let mover = sim.units().ids()[0];
    let goal = Cell::new(40, 20);

    sim.tick(&[cmd(CommandKind::Move {
        units: vec![mover],
        target: goal,
    })]);
    for _ in 0..3_000 {
        sim.tick(&[]);
        if sim.units().get(mover).is_none_or(|u| u.order.is_idle()) {
            break;
        }
    }

    let unit = sim.units().get(mover).expect("it should survive the trip");
    assert!(
        unit.cell().chebyshev_to(goal) <= 3,
        "a plain move stopped at {:?} instead of reaching {goal:?}",
        unit.cell()
    );
}

#[test]
fn an_attack_move_stops_to_fight() {
    // The whole reason both orders exist.
    let mut sim = Sim::new(scenario(vec![(0, 5, 20), (1, 20, 20)]));
    let mover = sim.units().ids()[0];
    let goal = Cell::new(40, 20);

    sim.tick(&[cmd(CommandKind::AttackMove {
        units: vec![mover],
        target: goal,
    })]);
    let mut engaged_somewhere = false;
    for _ in 0..600 {
        sim.tick(&[]);
        if let Some(u) = sim.units().get(mover)
            && u.combat.target.is_some()
        {
            engaged_somewhere = true;
            break;
        }
    }
    assert!(engaged_somewhere, "an attack-move never engaged anything");

    let unit = sim.units().get(mover).expect("alive");
    assert!(
        unit.cell().chebyshev_to(goal) > 10,
        "it kept walking to the destination instead of stopping to fight"
    );
}

#[test]
fn an_attack_move_carries_on_once_the_fight_is_over() {
    // Stopping forever would make it useless as a way of advancing.
    let mut sim = Sim::new(scenario(vec![
        (0, 5, 20),
        (0, 6, 20),
        (0, 5, 21),
        (1, 20, 20),
    ]));
    let movers: Vec<EntityId> = sim
        .units()
        .iter()
        .filter(|(_, u)| u.owner == PlayerId(0))
        .map(|(id, _)| id)
        .collect();
    let enemy = sim.units().ids()[3];
    let goal = Cell::new(40, 20);

    sim.tick(&[cmd(CommandKind::AttackMove {
        units: movers.clone(),
        target: goal,
    })]);
    for _ in 0..6_000 {
        sim.tick(&[]);
        if sim.units().get(enemy).is_none()
            && movers
                .iter()
                .all(|id| sim.units().get(*id).is_none_or(|u| u.order.is_idle()))
        {
            break;
        }
    }

    assert!(
        sim.units().get(enemy).is_none(),
        "the enemy survived three attackers"
    );
    let arrived = movers
        .iter()
        .filter_map(|id| sim.units().get(*id))
        .filter(|u| u.cell().chebyshev_to(goal) <= 4)
        .count();
    assert!(
        arrived > 0,
        "nobody resumed the advance after winning the fight"
    );
}

#[test]
fn an_attack_order_closes_and_kills() {
    // Six cells apart: inside the seven-cell sight, outside the four-cell
    // weapon. So the order is legal and the attacker has to close first.
    let mut sim = Sim::new(scenario(vec![(0, 5, 20), (1, 11, 20)]));
    let attacker = sim.units().ids()[0];
    let victim = sim.units().ids()[1];

    sim.tick(&[cmd(CommandKind::Attack {
        units: vec![attacker],
        target: victim,
    })]);
    for _ in 0..4_000 {
        sim.tick(&[]);
        if sim.units().get(victim).is_none() {
            break;
        }
    }
    assert!(
        sim.units().get(victim).is_none(),
        "the target was never killed"
    );
}

#[test]
fn an_attack_order_stops_when_the_target_dies() {
    // Left in place, the unit would keep walking to where the target used to
    // be — into whatever killed it.
    //
    // Three attackers against one, because two evenly matched soldiers with the
    // same rifle are a coin toss, and a test that depends on which way the coin
    // lands is worse than no test.
    let mut sim = Sim::new(scenario(vec![
        (0, 5, 20),
        (0, 5, 21),
        (0, 5, 19),
        (1, 11, 20),
    ]));
    let attackers: Vec<EntityId> = sim
        .units()
        .iter()
        .filter(|(_, u)| u.owner == PlayerId(0))
        .map(|(id, _)| id)
        .collect();
    let victim = sim.units().ids()[3];

    sim.tick(&[cmd(CommandKind::Attack {
        units: attackers.clone(),
        target: victim,
    })]);
    for _ in 0..4_000 {
        sim.tick(&[]);
        if sim.units().get(victim).is_none() {
            break;
        }
    }
    assert!(
        sim.units().get(victim).is_none(),
        "three attackers failed to kill one"
    );

    for _ in 0..20 {
        sim.tick(&[]);
    }
    let survivors: Vec<_> = attackers
        .iter()
        .filter_map(|id| sim.units().get(*id))
        .collect();
    assert!(!survivors.is_empty(), "all three attackers died");
    for unit in survivors {
        assert!(
            unit.order.is_idle(),
            "an attacker is still chasing a dead target"
        );
    }
}

#[test]
fn a_unit_cannot_be_ordered_to_attack_something_it_cannot_see() {
    // Allowing it would let a client with the fog switched off pick out units
    // it has no business knowing about — and desync, since the simulation and
    // the interface would disagree about which orders are legal.
    let mut sim = Sim::new(scenario(vec![(0, 5, 5), (1, 55, 55)]));
    let attacker = sim.units().ids()[0];
    let hidden = sim.units().ids()[1];
    sim.tick(&[]);

    assert!(
        !sim.can_see(PlayerId(0), sim.units().get(hidden).unwrap()),
        "the test needs a target in fog"
    );
    sim.tick(&[cmd(CommandKind::Attack {
        units: vec![attacker],
        target: hidden,
    })]);
    assert!(
        sim.units().get(attacker).unwrap().order.is_idle(),
        "an attack order was accepted against a target in fog"
    );
}

#[test]
fn a_guard_holds_its_post_and_returns_to_it() {
    let mut sim = Sim::new(scenario(vec![(0, 20, 20), (1, 26, 20)]));
    let guard = sim.units().ids()[0];
    let post = sim.units().get(guard).unwrap().cell();

    sim.tick(&[cmd(CommandKind::Guard { units: vec![guard] })]);
    assert!(matches!(
        sim.units().get(guard).unwrap().order,
        Order::Guard { .. }
    ));

    for _ in 0..4_000 {
        sim.tick(&[]);
    }

    let unit = sim.units().get(guard).expect("the guard should survive");
    assert!(
        unit.cell().chebyshev_to(post) <= 5,
        "the guard wandered to {:?} from its post at {post:?}",
        unit.cell()
    );
    assert!(
        matches!(unit.order, Order::Guard { .. }),
        "the guard forgot its order"
    );
}

#[test]
fn orders_are_deterministic() {
    let run = || {
        let mut sim = Sim::new(scenario(vec![
            (0, 5, 20),
            (0, 6, 20),
            (0, 5, 21),
            (1, 25, 20),
            (1, 26, 21),
        ]));
        let mine: Vec<EntityId> = sim
            .units()
            .iter()
            .filter(|(_, u)| u.owner == PlayerId(0))
            .map(|(id, _)| id)
            .collect();
        sim.tick(&[cmd(CommandKind::AttackMove {
            units: mine,
            target: Cell::new(45, 20),
        })]);
        let mut hashes = Vec::new();
        for _ in 0..1_500 {
            sim.tick(&[]);
            hashes.push(sim.state_hash());
        }
        (hashes, sim.units().len())
    };
    let (a, a_units) = run();
    let (b, b_units) = run();
    assert_eq!(a, b, "two identical advances diverged");
    assert_eq!(a_units, b_units);
    assert!(a_units < 5, "nobody died, so this proves nothing");
}
