//! High ground.
//!
//! Elevation used to be modelled as impassable rock, which kept exactly half of
//! what it means: a unit could not walk up, and that was all. In the original,
//! high ground is somewhere a unit *stands* — the cliff is the edge between two
//! levels, not the plateau — and standing there is worth doing, because it
//! lengthens your reach.
//!
//! So these tests come in two halves: the cliff blocks a step, and the plateau
//! pays for itself.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map, Surface, SurfaceMask};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn rules() -> Rules {
    let armour: ArmourTable =
        ron::from_str(r#"( classes: ["none"], table: { "shot": { "none": 100 } } )"#).unwrap();
    let weapons = vec![WeaponDef {
        id: "rifle".into(),
        damage: 10,
        warhead: "shot".into(),
        reload: Ticks(10),
        // Deliberately shorter than vision. A weapon that outranges its own
        // eyes never fires, and the test would then be measuring the wrong
        // thing entirely.
        range: Hundredths(400),
        splash_radius: Hundredths::ZERO,
        projectile_speed: Hundredths::ZERO,
        homing: false,
        targets: vec![],
        instant_kill: false,
        ammo: 0,
        intercepts: false,
        target_categories: vec![],
        heals: false,
    }];
    let soldier = EntityDef {
        id: "soldier".into(),
        name_key: "unit.soldier".into(),
        side: None,
        category: "infantry".into(),
        traits: vec![
            Trait::Health {
                max: 4_000,
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
                range: Hundredths(1_200),
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

fn scenario(map: Map, spawns: Vec<(u8, i32, i32)>) -> MatchSetup {
    let rules = rules();
    let kind = rules.kind_of("soldier").unwrap();
    MatchSetup {
        seed: 0x_E1E7,
        map,
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

fn land() -> SurfaceMask {
    SurfaceMask::from_surfaces(&[Surface::Land])
}

fn air() -> SurfaceMask {
    SurfaceMask::from_surfaces(&[Surface::Height])
}

// -- The cliff blocks a step, not a cell ------------------------------------

#[test]
fn flat_ground_is_walkable_in_every_direction() {
    let map = Map::new(16, 16);
    assert!(map.step_is_climbable(Cell::new(4, 4), Cell::new(5, 4), land()));
    assert!(map.step_is_climbable(Cell::new(4, 4), Cell::new(5, 5), land()));
}

#[test]
fn a_single_level_is_a_slope_and_can_be_walked_up() {
    // Maps are built from ramps between adjacent levels. If one step were a
    // wall, no plateau would ever be reachable and high ground would be
    // decoration.
    let mut map = Map::new(16, 16);
    map.set_elevation(Cell::new(5, 4), 1);
    assert!(map.step_is_climbable(Cell::new(4, 4), Cell::new(5, 4), land()));
    assert!(map.step_is_climbable(Cell::new(5, 4), Cell::new(4, 4), land()));
}

#[test]
fn two_levels_at_once_is_a_cliff() {
    let mut map = Map::new(16, 16);
    map.set_elevation(Cell::new(5, 4), 2);
    assert!(!map.step_is_climbable(Cell::new(4, 4), Cell::new(5, 4), land()));
    // And equally in the other direction — you cannot jump down a cliff face.
    assert!(!map.step_is_climbable(Cell::new(5, 4), Cell::new(4, 4), land()));
}

#[test]
fn the_plateau_itself_is_perfectly_walkable() {
    // The distinction from the rock this replaces. A unit that has reached the
    // top must be able to move around up there.
    let mut map = Map::new(16, 16);
    map.raise_rect(Cell::new(4, 4), Cell::new(8, 8), 2);
    assert!(map.is_passable(Cell::new(6, 6), land()));
    assert!(map.step_is_climbable(Cell::new(6, 6), Cell::new(7, 7), land()));
}

#[test]
fn flight_ignores_elevation_entirely() {
    let mut map = Map::new(16, 16);
    map.raise_rect(Cell::new(4, 4), Cell::new(8, 8), 9);
    assert!(map.step_is_climbable(Cell::new(3, 4), Cell::new(4, 4), air()));
}

#[test]
fn off_map_reads_as_ground_level() {
    let map = Map::new(16, 16);
    assert_eq!(map.elevation(Cell::new(-1, -1)), 0);
}

#[test]
fn a_unit_walks_around_a_cliff_rather_than_over_it() {
    // A wall of high ground across the middle of the map, with a gap at the
    // bottom. The unit must arrive, and must have gone the long way.
    let mut map = Map::new(48, 48);
    map.raise_rect(Cell::new(20, 0), Cell::new(21, 40), 3);
    let mut sim = Sim::new(scenario(map, vec![(0, 5, 5)]));
    let mover = sim.units().ids()[0];

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Move {
            units: vec![mover],
            target: Cell::new(40, 5),
        },
    )]);

    let mut crossed_high_ground = false;
    for _ in 0..6_000 {
        sim.tick(&[]);
        let unit = sim.unit(mover).expect("the mover survives an empty map");
        if sim.map().elevation(unit.cell()) > 1 {
            crossed_high_ground = true;
        }
        if unit.cell().x >= 39 {
            break;
        }
    }

    let arrived = sim.unit(mover).unwrap().cell();
    assert!(
        arrived.x >= 39,
        "should have reached the far side, at {arrived:?}"
    );
    assert!(
        !crossed_high_ground,
        "should have gone around the cliff, not over it"
    );
}

#[test]
fn a_walled_off_plateau_is_unreachable() {
    let mut map = Map::new(48, 48);
    map.raise_rect(Cell::new(20, 20), Cell::new(28, 28), 4);
    let mut sim = Sim::new(scenario(map, vec![(0, 5, 5)]));
    let mover = sim.units().ids()[0];

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Move {
            units: vec![mover],
            target: Cell::new(24, 24),
        },
    )]);
    for _ in 0..4_000 {
        sim.tick(&[]);
    }

    let at = sim.unit(mover).unwrap().cell();
    assert_eq!(
        sim.map().elevation(at),
        0,
        "a foot unit must not end up on top of a cliff, but stood at {at:?}"
    );
}

// -- The plateau pays for itself --------------------------------------------

#[test]
fn high_ground_lengthens_the_reach() {
    let map = Map::new(16, 16);
    assert_eq!(map.elevation_bonus(Cell::new(4, 4)), 100);

    let mut raised = Map::new(16, 16);
    raised.set_elevation(Cell::new(4, 4), 2);
    assert!(
        raised.elevation_bonus(Cell::new(4, 4)) > 100,
        "standing higher must be worth something"
    );
}

#[test]
fn a_unit_on_a_hill_shoots_first() {
    // The point of the whole feature. Two identical soldiers, one on a hill,
    // placed so that only the extended reach spans the gap between them. The
    // one below must take damage without dealing any.
    let mut map = Map::new(48, 48);
    map.raise_rect(Cell::new(16, 16), Cell::new(24, 24), 3);

    // Range is 4 cells; three levels of hill make it 5.8. Placed 5 apart: in
    // reach from above, out of reach from below.
    let mut sim = Sim::new(scenario(map, vec![(0, 20, 20), (1, 20, 25)]));
    let ids = sim.units().ids();
    let (high, low) = (ids[0], ids[1]);
    assert_eq!(sim.map().elevation(sim.unit(high).unwrap().cell()), 3);
    assert_eq!(sim.map().elevation(sim.unit(low).unwrap().cell()), 0);

    let low_start = sim.unit(low).unwrap().health;
    let high_start = sim.unit(high).unwrap().health;

    // Both hold position, so neither closes the distance and the only thing
    // being measured is reach.
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Stop { units: vec![high] },
    )]);
    for _ in 0..200 {
        sim.tick(&[]);
    }

    let low_now = sim
        .unit(low)
        .expect("the low unit survives 200 ticks")
        .health;
    let high_now = sim.unit(high).unwrap().health;

    assert!(
        low_now < low_start,
        "the unit on the hill should be able to reach the one below"
    );
    assert_eq!(
        high_now, high_start,
        "the unit below should not be able to reach back"
    );
}

#[test]
fn elevation_is_part_of_the_state_hash() {
    // A map layer the hash ignores is a desync waiting to happen: two peers
    // could disagree about the terrain and agree about the hash.
    use redshift_sim::hash::{StateHash, StateHasher};

    let flat = Map::new(16, 16);
    let mut raised = Map::new(16, 16);
    raised.set_elevation(Cell::new(4, 4), 1);

    let hash = |m: &Map| {
        let mut h = StateHasher::new();
        m.state_hash(&mut h);
        h.finish()
    };
    assert_ne!(hash(&flat), hash(&raised));
}
