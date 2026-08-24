//! Bridges: terrain you can shoot, and a hut that puts it back.
//!
//! Two things make this worth its own file. A bridge *opens* its footprint
//! instead of blocking it — the only entity that does — and it is destroyed
//! without being removed, because the ruined span is still visibly there and
//! an engineer at the hut beside it rebuilds it.
//!
//! The researched correction: bridges are repaired **through a hut**, not by
//! touching the bridge. That makes bridge repair the same act as capturing a
//! tech building rather than a new mechanic, which is why there is no
//! bridge-repair command anywhere in the engine.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::EntityId;
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map, Surface, SurfaceMask, Terrain};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn armour() -> ArmourTable {
    ron::from_str(r#"( classes: ["none"], table: { "shot": { "none": 100 } } )"#).unwrap()
}

fn tank() -> EntityDef {
    EntityDef {
        id: "tank".into(),
        name_key: "unit.tank".into(),
        side: None,
        category: "vehicle".into(),
        traits: vec![
            Trait::Health {
                max: 400,
                armour: "none".into(),
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
                range: Hundredths(900),
            },
            Trait::Armed {
                weapon: "cannon".into(),
                turret: true,
                turret_rate: 3600,
            },
        ],
    }
}

fn engineer() -> EntityDef {
    EntityDef {
        id: "engineer".into(),
        name_key: "unit.engineer".into(),
        side: None,
        category: "infantry".into(),
        traits: vec![
            Trait::Health {
                max: 100,
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
            Trait::Engineer { consumed: true },
        ],
    }
}

/// The span. Four cells wide, one deep — a crossing.
fn bridge() -> EntityDef {
    EntityDef {
        id: "bridge".into(),
        name_key: "structure.bridge".into(),
        side: None,
        category: "terrain".into(),
        traits: vec![
            Trait::Health {
                max: 500,
                armour: "none".into(),
            },
            Trait::Footprint {
                width: 5,
                height: 1,
            },
            Trait::Bridge,
        ],
    }
}

fn hut() -> EntityDef {
    EntityDef {
        id: "hut".into(),
        name_key: "structure.hut".into(),
        side: None,
        category: "structure".into(),
        traits: vec![
            Trait::Health {
                max: 300,
                armour: "none".into(),
            },
            Trait::Vision {
                range: Hundredths(500),
            },
            Trait::RepairsBridges { radius: 8 },
        ],
    }
}

fn rules() -> Rules {
    Rules::from_parts(
        vec![tank(), engineer(), bridge(), hut()],
        vec![WeaponDef {
            id: "cannon".into(),
            damage: 60,
            warhead: "shot".into(),
            reload: Ticks(10),
            range: Hundredths(500),
            splash_radius: Hundredths::ZERO,
            projectile_speed: Hundredths::ZERO,
            homing: false,
            targets: vec![],
            instant_kill: false,
            ammo: 0,
            intercepts: false,
        }],
        armour(),
        Vec::new(),
    )
    .expect("valid rules")
}

/// A river across the middle with a bridge over it.
///
/// The bridge sits at (20, 24) with a five-by-one footprint, so it covers
/// x = 18..=22 on the river's row.
fn river_map() -> Map {
    let mut map = Map::new(48, 48);
    map.fill_rect(Cell::new(0, 24), Cell::new(47, 24), Terrain::Water);
    map
}

fn scenario(spawns: Vec<(u8, &str, i32, i32)>) -> Sim {
    let rules = rules();
    let spawns = spawns
        .into_iter()
        .map(|(owner, id, x, y)| Spawn {
            // 9 means nobody's. Bridges and repair huts belong to the map, not
            // to a player — and a player id with no stats row resolves to a
            // blank one, which is how a neutral thing ends up with no health
            // and no traits at all.
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
        seed: 0x_B41D,
        map: river_map(),
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

fn land() -> SurfaceMask {
    SurfaceMask::from_surfaces(&[Surface::Land])
}

// -- Standing --------------------------------------------------------------

#[test]
fn a_bridge_opens_its_footprint_rather_than_blocking_it() {
    // The only footprint in the engine that does. Everything else claims its
    // ground; this one hands it over.
    let bare = Sim::new(MatchSetup {
        seed: 0,
        map: river_map(),
        rules: rules(),
        players: vec![PlayerSetup {
            id: PlayerId(0),
            faction: None,
        }],
        spawns: vec![],
    });
    assert!(
        !bare.map().is_passable(Cell::new(20, 24), land()),
        "the river should be in the way to begin with"
    );

    let sim = scenario(vec![(9, "bridge", 20, 24)]);
    assert!(
        sim.map().is_passable(Cell::new(20, 24), land()),
        "the bridge should carry a tank"
    );
    assert!(
        !sim.map().is_blocked(Cell::new(20, 24)),
        "a bridge must not block the ground it covers"
    );
}

#[test]
fn the_span_is_only_as_wide_as_the_bridge() {
    let sim = scenario(vec![(9, "bridge", 20, 24)]);
    assert!(sim.map().is_passable(Cell::new(18, 24), land()));
    assert!(sim.map().is_passable(Cell::new(22, 24), land()));
    assert!(
        !sim.map().is_passable(Cell::new(23, 24), land()),
        "the river should still be a river beside the bridge"
    );
}

#[test]
fn a_tank_crosses_the_river_over_the_bridge() {
    let mut sim = scenario(vec![(0, "tank", 20, 20), (9, "bridge", 20, 24)]);
    let id = sim.units().ids()[0];

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Move {
            units: vec![id],
            target: Cell::new(20, 30),
        },
    )]);
    for _ in 0..600 {
        sim.tick(&[]);
        if sim.unit(id).unwrap().cell().y >= 29 {
            break;
        }
    }

    assert!(
        sim.unit(id).unwrap().cell().y >= 29,
        "it should have got across"
    );
}

// -- Wrecking it ------------------------------------------------------------

/// Shoots the bridge down and returns the sim with it wrecked.
fn wreck(sim: &mut Sim, gunner: EntityId, span: EntityId) {
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Attack {
            units: vec![gunner],
            target: span,
        },
    )]);
    for _ in 0..600 {
        sim.tick(&[]);
        if sim.unit(span).is_none_or(|b| !b.is_alive()) {
            return;
        }
    }
    panic!("the bridge would not come down");
}

#[test]
fn a_wrecked_bridge_puts_the_river_back() {
    let mut sim = scenario(vec![(0, "tank", 20, 21), (9, "bridge", 20, 24)]);
    let ids = sim.units().ids();
    let (gunner, span) = (ids[0], ids[1]);

    wreck(&mut sim, gunner, span);
    sim.tick(&[]);

    assert!(
        !sim.map().is_passable(Cell::new(20, 24), land()),
        "the water should be back"
    );
}

#[test]
fn a_wrecked_bridge_is_not_removed() {
    // The one entity destroyed without being taken away. Remove it and there
    // is nothing left for the hut to repair — and nothing to look at, when the
    // ruined span is exactly what a player expects to see.
    let mut sim = scenario(vec![(0, "tank", 20, 21), (9, "bridge", 20, 24)]);
    let ids = sim.units().ids();
    let (gunner, span) = (ids[0], ids[1]);

    wreck(&mut sim, gunner, span);
    for _ in 0..40 {
        sim.tick(&[]);
    }

    let wreckage = sim.unit(span).expect("the wreck should still be there");
    assert!(!wreckage.is_alive());
}

#[test]
fn a_tank_cannot_cross_a_wrecked_bridge() {
    let mut sim = scenario(vec![(0, "tank", 20, 21), (9, "bridge", 20, 24)]);
    let ids = sim.units().ids();
    let (gunner, span) = (ids[0], ids[1]);
    wreck(&mut sim, gunner, span);

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Move {
            units: vec![gunner],
            target: Cell::new(20, 30),
        },
    )]);
    for _ in 0..600 {
        sim.tick(&[]);
    }

    assert!(
        sim.unit(gunner).unwrap().cell().y < 24,
        "it should still be on this side of the river"
    );
}

// -- Putting it back --------------------------------------------------------

#[test]
fn an_engineer_at_the_hut_rebuilds_the_bridge() {
    let mut sim = scenario(vec![
        (0, "tank", 20, 21),
        (9, "bridge", 20, 24),
        (9, "hut", 24, 21),
        (0, "engineer", 22, 20),
    ]);
    let ids = sim.units().ids();
    let (gunner, span, the_hut, eng) = (ids[0], ids[1], ids[2], ids[3]);

    wreck(&mut sim, gunner, span);
    assert!(!sim.map().is_passable(Cell::new(20, 24), land()));

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::EnterBuilding {
            units: vec![eng],
            target: the_hut,
        },
    )]);
    for _ in 0..800 {
        sim.tick(&[]);
        if sim.unit(span).is_some_and(|b| b.is_alive()) {
            break;
        }
    }

    assert!(
        sim.unit(span).unwrap().is_alive(),
        "the bridge should have come back"
    );
    assert!(
        sim.map().is_passable(Cell::new(20, 24), land()),
        "and should carry a tank again"
    );
    assert!(sim.unit(eng).is_none(), "the engineer is consumed");
}

#[test]
fn a_hut_too_far_away_serves_nothing() {
    // Proximity is how the original says "this hut is for that bridge". A hut
    // on the other side of the map is a different hut.
    let mut sim = scenario(vec![
        (0, "tank", 20, 21),
        (9, "bridge", 20, 24),
        (9, "hut", 44, 4),
        (0, "engineer", 42, 4),
    ]);
    let ids = sim.units().ids();
    let (gunner, span, the_hut, eng) = (ids[0], ids[1], ids[2], ids[3]);

    wreck(&mut sim, gunner, span);
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::EnterBuilding {
            units: vec![eng],
            target: the_hut,
        },
    )]);
    for _ in 0..800 {
        sim.tick(&[]);
    }

    assert!(
        !sim.unit(span).unwrap().is_alive(),
        "a distant hut repaired it"
    );
}

#[test]
fn an_engineer_is_not_spent_on_a_hut_with_nothing_to_do() {
    // Walking one into a hut beside an intact bridge should cost the player
    // nothing, for the same reason an engineer is not wasted on an undamaged
    // building.
    let mut sim = scenario(vec![
        (9, "bridge", 20, 24),
        (9, "hut", 24, 21),
        (0, "engineer", 22, 20),
    ]);
    let ids = sim.units().ids();
    let (the_hut, eng) = (ids[1], ids[2]);

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::EnterBuilding {
            units: vec![eng],
            target: the_hut,
        },
    )]);
    for _ in 0..400 {
        sim.tick(&[]);
    }

    assert!(
        sim.unit(eng).is_some_and(|u| u.is_alive()),
        "the engineer was thrown away for nothing"
    );
}

#[test]
fn bridges_are_deterministic() {
    let run = || {
        let mut sim = scenario(vec![
            (0, "tank", 20, 21),
            (9, "bridge", 20, 24),
            (9, "hut", 24, 21),
            (0, "engineer", 22, 20),
        ]);
        let ids = sim.units().ids();
        wreck(&mut sim, ids[0], ids[1]);
        sim.tick(&[Command::new(
            PlayerId(0),
            0,
            CommandKind::EnterBuilding {
                units: vec![ids[3]],
                target: ids[2],
            },
        )]);
        for _ in 0..400 {
            sim.tick(&[]);
        }
        sim.state_hash()
    };
    assert_eq!(run(), run());
}
