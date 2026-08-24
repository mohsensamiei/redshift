//! The Service Depot, and the thing it exists to undo.
//!
//! Two capabilities that only make sense together. A Terror Drone gets inside
//! a vehicle and takes it apart from within, where nothing can shoot it — and
//! the answer is not a better gun but a building. Implementing either alone
//! would leave the other looking arbitrary: a repair shed nobody needs, or a
//! weapon with no counter.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::EntityId;
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn armour() -> ArmourTable {
    ron::from_str(
        r#"( classes: ["none", "heavy"],
             table: { "shot": { "none": 100, "heavy": 50 },
                      "gnaw": { "none": 100, "heavy": 100 } } )"#,
    )
    .unwrap()
}

fn weapons() -> Vec<WeaponDef> {
    vec![
        WeaponDef {
            id: "pot_shot".into(),
            damage: 40,
            warhead: "shot".into(),
            reload: Ticks(20),
            // Short reach and matching sight, so a tank ordered away from it is
            // genuinely out of the fight.
            range: Hundredths(300),
            splash_radius: Hundredths::ZERO,
            projectile_speed: Hundredths::ZERO,
            homing: false,
            targets: vec![],
            instant_kill: false,
            ammo: 0,
            intercepts: false,
        },
        WeaponDef {
            id: "claws".into(),
            damage: 5,
            warhead: "shot".into(),
            reload: Ticks(10),
            // Short on purpose. The drone's reach is what makes it run all the way
            // up to a tank rather than plinking at it from a distance — a number
            // in the rules, not a special movement rule in the engine.
            range: Hundredths(150),
            splash_radius: Hundredths::ZERO,
            projectile_speed: Hundredths::ZERO,
            homing: false,
            targets: vec![],
            instant_kill: false,
            ammo: 0,
            intercepts: false,
        },
    ]
}

/// An immobile gun with a short reach, for wounding something on purpose.
fn turret() -> EntityDef {
    EntityDef {
        id: "turret".into(),
        name_key: "structure.turret".into(),
        side: None,
        category: "structure".into(),
        traits: vec![
            Trait::Health {
                max: 1_000,
                armour: "none".into(),
            },
            Trait::Vision {
                range: Hundredths(300),
            },
            Trait::Armed {
                weapon: "pot_shot".into(),
                turret: true,
                turret_rate: 3600,
            },
        ],
    }
}

fn tank(cost: u32) -> EntityDef {
    EntityDef {
        id: "tank".into(),
        name_key: "unit.tank".into(),
        side: None,
        category: "vehicle".into(),
        traits: vec![
            Trait::Health {
                max: 400,
                armour: "heavy".into(),
            },
            Trait::Mobile {
                speed: Hundredths(300),
                turn_rate: 3600,
                locomotor: Locomotor::Tracked,
                surfaces: None,
                size: None,
                layer: None,
            },
            Trait::Vision {
                range: Hundredths(600),
            },
            Trait::Buildable {
                cost,
                build_time: Ticks(10),
                prerequisites: vec![],
                produced_by: "depot".into(),
            },
        ],
    }
}

fn depot(rate: u32, cost_percent: u32, cures: bool) -> EntityDef {
    EntityDef {
        id: "depot".into(),
        name_key: "structure.depot".into(),
        side: None,
        category: "structure".into(),
        traits: vec![
            Trait::Health {
                max: 1_000,
                armour: "none".into(),
            },
            Trait::Vision {
                range: Hundredths(400),
            },
            Trait::Footprint {
                width: 3,
                height: 3,
            },
            Trait::Repairs {
                categories: vec!["vehicle".into()],
                rate,
                cost_percent,
                cures_infestation: cures,
            },
        ],
    }
}

fn drone_biting(damage: u32) -> EntityDef {
    EntityDef {
        id: "drone".into(),
        name_key: "unit.drone".into(),
        side: None,
        category: "infantry".into(),
        traits: vec![
            Trait::Health {
                max: 100,
                armour: "none".into(),
            },
            Trait::Mobile {
                speed: Hundredths(500),
                turn_rate: 3600,
                locomotor: Locomotor::Foot,
                surfaces: None,
                size: None,
                layer: None,
            },
            Trait::Vision {
                range: Hundredths(700),
            },
            // It keeps an ordinary weapon for everything it cannot get inside,
            // which is how the original's behaves like an attack dog against
            // infantry and like something else entirely against a tank.
            Trait::Armed {
                weapon: "claws".into(),
                turret: true,
                turret_rate: 3600,
            },
            Trait::Infests {
                categories: vec!["vehicle".into()],
                damage,
                warhead: "gnaw".into(),
            },
        ],
    }
}

/// A drone that eats slowly enough for its host to reach a depot.
fn drone() -> EntityDef {
    drone_biting(2)
}

fn scenario(entities: Vec<EntityDef>, spawns: Vec<(u8, &str, i32, i32)>) -> Sim {
    let rules = Rules::from_parts(entities, weapons(), armour(), Vec::new()).expect("valid rules");
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
        seed: 0x_5EED,
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

/// Points a unit at an enemy. An idle unit acquires what is already in reach
/// but does not go looking, and the drone's reach is one and a half cells —
/// so without an order it stands where it spawned and nothing happens.
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

fn send_into(sim: &mut Sim, unit: EntityId, building: EntityId) {
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::EnterBuilding {
            units: vec![unit],
            target: building,
        },
    )]);
}

/// A tank wounded by a short-ranged gun it can then drive away from, plus a
/// depot far enough off to be out of that gun's reach.
///
/// The roundabout way to arrange a damaged vehicle, and the only one that goes
/// through the code these tests are actually about: `Sim` has no mutable
/// accessor, deliberately, and adding one so a test could write a health value
/// would be exactly the kind of hole that later gets used in earnest.
fn damaged_tank(rate: u32, cost_percent: u32, tank_cost: u32) -> (Sim, EntityId, EntityId) {
    let mut sim = scenario(
        vec![
            tank(tank_cost),
            depot(rate, cost_percent, true),
            drone(),
            turret(),
        ],
        vec![
            (0, "tank", 10, 10),
            (1, "turret", 12, 10),
            (0, "depot", 30, 30),
        ],
    );
    let ids = sim.units().ids();
    let (tank_id, depot_id) = (ids[0], ids[2]);

    for _ in 0..400 {
        sim.tick(&[]);
        if sim.unit(tank_id).is_some_and(|t| t.health <= 200) {
            break;
        }
    }
    assert!(
        sim.unit(tank_id).is_some_and(|t| t.health <= 200),
        "the turret should have wounded the tank"
    );
    (sim, tank_id, depot_id)
}

// -- Repair -----------------------------------------------------------------

#[test]
fn a_damaged_vehicle_sent_to_a_depot_is_repaired() {
    let (mut sim, tank_id, depot_id) = damaged_tank(5, 0, 1_000);
    let hurt = sim.unit(tank_id).unwrap().health;

    send_into(&mut sim, tank_id, depot_id);
    for _ in 0..400 {
        sim.tick(&[]);
    }

    let now = sim.unit(tank_id).unwrap().health;
    assert!(now > hurt, "the depot did nothing");
    assert_eq!(now, 400, "it should have been repaired to full");
}

#[test]
fn repair_is_gradual_rather_than_instant() {
    // The whole reason a depot is a decision: the unit is out of the fight for
    // a while. Restoring it the instant it touches the building would make the
    // depot free.
    let (mut sim, tank_id, depot_id) = damaged_tank(1, 0, 1_000);

    send_into(&mut sim, tank_id, depot_id);
    // Driving there takes a while of its own, which is not the thing being
    // measured. Wait for the first point of health to come back, then judge.
    let mut arrived = sim.unit(tank_id).unwrap().health;
    for _ in 0..600 {
        let before = sim.unit(tank_id).unwrap().health;
        sim.tick(&[]);
        let now = sim.unit(tank_id).unwrap().health;
        if now > before {
            arrived = now;
            break;
        }
    }
    for _ in 0..40 {
        sim.tick(&[]);
    }
    let partway = sim.unit(tank_id).unwrap().health;

    assert!(partway > arrived, "it should have kept going");
    assert!(partway < 400, "it should not have finished this quickly");
}

#[test]
fn repair_costs_credits_in_proportion_to_the_damage() {
    let (mut sim, tank_id, depot_id) = damaged_tank(5, 20, 1_000);
    let before = sim.treasury().credits(PlayerId(0));

    send_into(&mut sim, tank_id, depot_id);
    // The lowest it reaches is where repair started from — it picks up a few
    // more hits driving out of the turret's reach, and billing against the
    // figure from before the drive would measure the wrong repair.
    let mut lowest = sim.unit(tank_id).unwrap().health;
    for _ in 0..600 {
        sim.tick(&[]);
        lowest = lowest.min(sim.unit(tank_id).unwrap().health);
    }

    let spent = before - sim.treasury().credits(PlayerId(0));
    assert_eq!(sim.unit(tank_id).unwrap().health, 400);
    // A full repair of a 1000-credit tank costs 20% of it — 200. This one undid
    // `400 - lowest` of 400 points, so it should cost that fraction.
    let expected = 200 * (400 - lowest) / 400;
    assert!(
        spent.abs_diff(expected) <= 5,
        "spent {spent} to undo {} damage, expected about {expected}",
        400 - lowest
    );
}

#[test]
fn a_player_who_cannot_pay_gets_no_repair() {
    // A tank so expensive that a single step of repair costs more than the
    // player has. Nothing in the treasury, nothing restored.
    let (mut sim, tank_id, depot_id) = damaged_tank(5, 20, 4_000_000);
    let hurt = sim.unit(tank_id).unwrap().health;

    send_into(&mut sim, tank_id, depot_id);
    for _ in 0..200 {
        sim.tick(&[]);
    }

    assert!(
        sim.unit(tank_id).unwrap().health <= hurt,
        "a repair was given away for free"
    );
}

#[test]
fn a_depot_will_not_service_a_category_it_does_not_list() {
    // A Naval Shipyard is a Service Depot that says "ship" instead of
    // "vehicle". Nothing separates them but this list.
    // A depot that lists "ship" instead of "vehicle" — which is the entirety
    // of the difference between a Service Depot and a Naval Shipyard.
    let shipyard = EntityDef {
        traits: depot(5, 0, false)
            .traits
            .into_iter()
            .map(|t| match t {
                Trait::Repairs {
                    rate,
                    cost_percent,
                    cures_infestation,
                    ..
                } => Trait::Repairs {
                    categories: vec!["ship".into()],
                    rate,
                    cost_percent,
                    cures_infestation,
                },
                other => other,
            })
            .collect(),
        ..depot(5, 0, false)
    };
    let mut sim = scenario(
        vec![tank(1_000), shipyard, drone(), turret()],
        vec![
            (0, "tank", 10, 10),
            (1, "turret", 12, 10),
            (0, "depot", 30, 30),
        ],
    );
    let ids = sim.units().ids();
    let (tank_id, depot_id) = (ids[0], ids[2]);
    for _ in 0..400 {
        sim.tick(&[]);
        if sim.unit(tank_id).is_some_and(|t| t.health <= 200) {
            break;
        }
    }
    let hurt = sim.unit(tank_id).unwrap().health;
    assert!(hurt < 400);

    send_into(&mut sim, tank_id, depot_id);
    for _ in 0..400 {
        sim.tick(&[]);
    }

    // Not "unchanged" — the tank takes a few more hits driving out of the
    // turret's reach. The claim is that nothing gave it any health back.
    assert!(
        sim.unit(tank_id).unwrap().health <= hurt,
        "a vehicle was serviced by a shipyard"
    );
}

// -- Infestation ------------------------------------------------------------

#[test]
fn a_drone_gets_inside_a_vehicle_rather_than_shooting_it() {
    let mut sim = scenario(
        vec![tank(1_000), depot(5, 0, true), drone()],
        vec![(0, "tank", 10, 10), (1, "drone", 14, 10)],
    );
    let ids = sim.units().ids();
    let (tank_id, drone_id) = (ids[0], ids[1]);
    attack(&mut sim, 1, drone_id, tank_id);

    for _ in 0..60 {
        sim.tick(&[]);
        if sim.unit(tank_id).and_then(|t| t.infestation).is_some() {
            break;
        }
    }

    assert_eq!(
        sim.unit(tank_id).unwrap().infestation,
        Some(drone_id),
        "the drone should have burrowed in"
    );
    assert!(
        sim.unit(drone_id).unwrap().is_aboard(),
        "a burrowed drone is not on the field, which is why nothing can shoot it"
    );
}

#[test]
fn an_infested_vehicle_is_eaten_from_inside() {
    let mut sim = scenario(
        vec![tank(1_000), depot(5, 0, true), drone()],
        vec![(0, "tank", 10, 10), (1, "drone", 14, 10)],
    );
    let ids = sim.units().ids();
    let (tank_id, drone_id) = (ids[0], ids[1]);
    attack(&mut sim, 1, drone_id, tank_id);

    for _ in 0..60 {
        sim.tick(&[]);
        if sim.unit(tank_id).and_then(|t| t.infestation).is_some() {
            break;
        }
    }
    let on_entry = sim.unit(tank_id).unwrap().health;
    for _ in 0..20 {
        sim.tick(&[]);
    }

    assert!(
        sim.unit(tank_id).unwrap().health < on_entry,
        "the drone should be taking the tank apart"
    );
}

#[test]
fn a_drone_shoots_what_it_cannot_get_inside() {
    // Two drones, one hostile to the other. Infantry is not on the list, so
    // the ordinary weapon is what happens.
    let mut sim = scenario(
        vec![tank(1_000), depot(5, 0, true), drone()],
        vec![(0, "drone", 10, 10), (1, "drone", 12, 10)],
    );
    let ids = sim.units().ids();
    let (mine, theirs) = (ids[0], ids[1]);
    let start = sim.unit(mine).unwrap().health;
    attack(&mut sim, 1, theirs, mine);

    for _ in 0..60 {
        sim.tick(&[]);
    }

    assert!(sim.unit(mine).is_none_or(|u| u.health < start));
    assert!(
        sim.unit(mine).is_none_or(|u| u.infestation.is_none()),
        "infantry is not on the drone's list"
    );
    assert!(sim.unit(theirs).is_none_or(|u| !u.is_aboard()));
}

#[test]
fn only_one_drone_gets_into_a_vehicle() {
    // Two drones reaching the same tank must not both get in: the second would
    // overwrite the first, quietly deleting a unit from the match.
    let mut sim = scenario(
        vec![tank(1_000), depot(5, 0, true), drone()],
        vec![
            (0, "tank", 10, 10),
            (1, "drone", 13, 10),
            (1, "drone", 10, 13),
        ],
    );
    let ids = sim.units().ids();
    let (tank_id, first, second) = (ids[0], ids[1], ids[2]);
    attack(&mut sim, 1, first, tank_id);
    attack(&mut sim, 1, second, tank_id);

    for _ in 0..80 {
        sim.tick(&[]);
    }

    let aboard = [first, second]
        .iter()
        .filter(|id| sim.unit(**id).is_some_and(|u| u.is_aboard()))
        .count();
    assert!(aboard <= 1, "{aboard} drones got into one tank");
    assert!(
        sim.unit(tank_id).is_none_or(|t| t.infestation.is_some()),
        "at least one should have got in"
    );
}

#[test]
fn a_drone_crawls_back_out_of_a_wreck() {
    // It killed something and should get to do it again. That is what makes
    // one drone worth spending — and what makes the depot worth building.
    // A hungrier drone than the others use, so the tank dies inside the test.
    let mut sim = scenario(
        vec![tank(1_000), depot(5, 0, true), drone_biting(8)],
        vec![(0, "tank", 10, 10), (1, "drone", 14, 10)],
    );
    let ids = sim.units().ids();
    let (tank_id, drone_id) = (ids[0], ids[1]);
    attack(&mut sim, 1, drone_id, tank_id);

    for _ in 0..400 {
        sim.tick(&[]);
        if sim.unit(tank_id).is_none_or(|t| !t.is_alive()) {
            break;
        }
    }

    assert!(sim.unit(tank_id).is_none(), "the tank should have died");
    let drone_now = sim.unit(drone_id).expect("the drone outlives its host");
    assert!(drone_now.is_alive() && !drone_now.is_aboard());
}

// -- The counter ------------------------------------------------------------

#[test]
fn a_depot_removes_the_drone() {
    let mut sim = scenario(
        vec![tank(1_000), depot(5, 0, true), drone()],
        vec![
            (0, "tank", 10, 10),
            (1, "drone", 14, 10),
            (0, "depot", 24, 24),
        ],
    );
    let ids = sim.units().ids();
    let (tank_id, drone_id, depot_id) = (ids[0], ids[1], ids[2]);
    attack(&mut sim, 1, drone_id, tank_id);

    for _ in 0..60 {
        sim.tick(&[]);
        if sim.unit(tank_id).and_then(|t| t.infestation).is_some() {
            break;
        }
    }
    assert!(sim.unit(tank_id).unwrap().infestation.is_some());

    send_into(&mut sim, tank_id, depot_id);
    for _ in 0..400 {
        sim.tick(&[]);
    }

    assert_eq!(
        sim.unit(tank_id).unwrap().infestation,
        None,
        "the depot should have dislodged it"
    );
    assert!(
        sim.unit(drone_id).is_none(),
        "and killed it — an evicted drone could be farmed across a column"
    );
    assert_eq!(
        sim.unit(tank_id).unwrap().health,
        400,
        "and then repaired what it did"
    );
}

#[test]
fn a_depot_that_does_not_cure_leaves_the_drone_in_place() {
    let mut sim = scenario(
        vec![tank(1_000), depot(5, 0, false), drone()],
        vec![
            (0, "tank", 10, 10),
            (1, "drone", 14, 10),
            (0, "depot", 24, 24),
        ],
    );
    let ids = sim.units().ids();
    let (tank_id, depot_id) = (ids[0], ids[2]);
    attack(&mut sim, 1, ids[1], tank_id);

    for _ in 0..60 {
        sim.tick(&[]);
        if sim.unit(tank_id).and_then(|t| t.infestation).is_some() {
            break;
        }
    }
    send_into(&mut sim, tank_id, depot_id);
    for _ in 0..100 {
        sim.tick(&[]);
        if sim.unit(tank_id).is_none() {
            break;
        }
    }

    assert!(
        sim.unit(tank_id).is_none_or(|t| t.infestation.is_some()),
        "nothing here cures anything"
    );
}

#[test]
fn repair_and_infestation_are_deterministic() {
    let run = || {
        let mut sim = scenario(
            vec![tank(1_000), depot(5, 20, true), drone()],
            vec![
                (0, "tank", 10, 10),
                (1, "drone", 14, 10),
                (0, "depot", 24, 24),
            ],
        );
        let ids = sim.units().ids();
        let (tank_id, depot_id) = (ids[0], ids[2]);
        attack(&mut sim, 1, ids[1], tank_id);
        for tick in 0..300 {
            if tick == 60 {
                send_into(&mut sim, tank_id, depot_id);
            } else {
                sim.tick(&[]);
            }
        }
        sim.state_hash()
    };
    assert_eq!(run(), run());
}
