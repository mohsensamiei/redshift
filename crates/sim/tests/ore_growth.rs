//! Ore that comes back.
//!
//! The widest-reaching rule in the game. An economy where ore runs out and one
//! where it does not are different games, not the same game with a different
//! number: it decides how long a match runs, whether a contested field is worth
//! holding rather than stripping, and whether a player who is behind can ever
//! come back.
//!
//! Growth is a *rule*, not a random walk. A field that spread by dice would be
//! one more thing the RNG had to stay in step about across peers, for no gain
//! over filling outward from the middle — which is also what it looks like.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map, Terrain};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

const INTERVAL: u32 = 5;
const CELL_LIMIT: u16 = 20;

fn armour() -> ArmourTable {
    ron::from_str(r#"( classes: ["none"], table: { "shot": { "none": 100 } } )"#).unwrap()
}

fn mine() -> EntityDef {
    EntityDef {
        id: "ore_mine".into(),
        name_key: "structure.ore_mine".into(),
        side: None,
        category: "structure".into(),
        traits: vec![
            Trait::Health {
                max: 1_000,
                armour: "none".into(),
            },
            Trait::Vision {
                range: Hundredths(200),
            },
            Trait::Grows {
                radius: Hundredths(300),
                interval: Ticks(INTERVAL),
                cell_limit: CELL_LIMIT,
            },
        ],
    }
}

fn derrick() -> EntityDef {
    EntityDef {
        id: "derrick".into(),
        name_key: "structure.derrick".into(),
        side: None,
        category: "structure".into(),
        traits: vec![
            Trait::Health {
                max: 1_000,
                armour: "none".into(),
            },
            Trait::Vision {
                range: Hundredths(200),
            },
            Trait::Footprint {
                width: 3,
                height: 3,
            },
        ],
    }
}

fn rules() -> Rules {
    Rules::from_parts(
        vec![
            mine(),
            derrick(),
            EntityDef {
                id: "scout".into(),
                name_key: "unit.scout".into(),
                side: None,
                category: "infantry".into(),
                traits: vec![
                    Trait::Health {
                        max: 100,
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
                        range: Hundredths(900),
                    },
                    Trait::Armed {
                        weapon: "cannon".into(),
                        turret: true,
                        turret_rate: 3600,
                    },
                ],
            },
        ],
        vec![WeaponDef {
            id: "cannon".into(),
            damage: 200,
            warhead: "shot".into(),
            reload: Ticks(5),
            range: Hundredths(600),
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
        }],
        armour(),
        Vec::new(),
    )
    .expect("valid rules")
}

fn scenario(map: Map, spawns: Vec<(&str, i32, i32)>) -> Sim {
    let rules = rules();
    let spawns = spawns
        .into_iter()
        .map(|(id, x, y)| Spawn {
            owner: PlayerId::NEUTRAL,
            kind: rules
                .kind_of(id)
                .unwrap_or_else(|| panic!("no entity {id:?}")),
            pos: Cell::new(x, y).centre(),
        })
        .collect();
    Sim::new(MatchSetup {
        seed: 0x_0DE,
        map,
        players: vec![PlayerSetup {
            id: PlayerId(0),
            faction: None,
        }],
        spawns,
        rules,
    })
}

fn run(sim: &mut Sim, ticks: u32) {
    for _ in 0..ticks {
        sim.tick(&[]);
    }
}

// -- That it grows at all ---------------------------------------------------

#[test]
fn a_mine_puts_ore_back_on_the_map() {
    let mut sim = scenario(Map::new(48, 48), vec![("ore_mine", 20, 20)]);
    assert_eq!(sim.map().total_ore(), 0, "the map should start bare");

    run(&mut sim, INTERVAL * 20);

    assert!(sim.map().total_ore() > 0, "nothing grew");
}

#[test]
fn a_map_with_no_mine_never_gains_ore() {
    // A plain field is worth stripping and leaving. That is the whole contrast
    // the mine exists to draw, and it would be lost if ore grew everywhere.
    let mut map = Map::new(48, 48);
    map.set_ore(Cell::new(20, 20), 10);
    let mut sim = scenario(map, vec![("derrick", 30, 30)]);
    let before = sim.map().total_ore();

    run(&mut sim, 500);

    assert_eq!(sim.map().total_ore(), before);
}

#[test]
fn it_grows_at_the_stated_rate() {
    let mut sim = scenario(Map::new(48, 48), vec![("ore_mine", 20, 20)]);
    run(&mut sim, INTERVAL * 10);
    let grown = sim.map().total_ore();

    // One unit per interval, give or take where the tick counter started.
    assert!(
        (8..=12).contains(&grown),
        "grew {grown} units in ten intervals"
    );
}

// -- Where it grows ---------------------------------------------------------

#[test]
fn it_fills_outward_from_the_middle() {
    // Nearest cell with room first. A field that filled one corner would look
    // wrong, and one that picked at random would need the RNG.
    let mut sim = scenario(Map::new(48, 48), vec![("ore_mine", 20, 20)]);
    run(&mut sim, INTERVAL * 3);

    assert!(
        sim.map().ore(Cell::new(20, 20)) > 0,
        "the cell under the mine should fill first"
    );
    assert_eq!(
        sim.map().ore(Cell::new(23, 20)),
        0,
        "the edge should still be bare while the middle has room"
    );
}

#[test]
fn it_stays_inside_its_radius() {
    let mut sim = scenario(Map::new(48, 48), vec![("ore_mine", 20, 20)]);
    // Long enough to fill everything it can reach several times over.
    run(&mut sim, INTERVAL * 2_000);

    for cell in [Cell::new(24, 20), Cell::new(20, 24), Cell::new(16, 16)] {
        assert_eq!(
            sim.map().ore(cell),
            0,
            "ore appeared at {cell:?}, outside the mine's reach"
        );
    }
}

#[test]
fn it_will_not_grow_into_water() {
    let mut map = Map::new(48, 48);
    map.fill_rect(Cell::new(21, 18), Cell::new(23, 22), Terrain::Water);
    let mut sim = scenario(map, vec![("ore_mine", 20, 20)]);
    run(&mut sim, INTERVAL * 2_000);

    assert_eq!(sim.map().ore(Cell::new(22, 20)), 0, "ore grew in a lake");
}

#[test]
fn it_will_not_grow_under_a_building() {
    // Ore under a foundation would be unreachable for good, and a mine that
    // slowly made its own surroundings unbuildable would be a strange thing to
    // put on a map.
    let mut sim = scenario(
        Map::new(48, 48),
        vec![("ore_mine", 20, 20), ("derrick", 22, 20)],
    );
    run(&mut sim, INTERVAL * 2_000);

    assert_eq!(sim.map().ore(Cell::new(22, 20)), 0, "ore grew indoors");
}

// -- How much ---------------------------------------------------------------

#[test]
fn a_cell_stops_at_its_limit() {
    let mut sim = scenario(Map::new(48, 48), vec![("ore_mine", 20, 20)]);
    run(&mut sim, INTERVAL * 2_000);

    assert_eq!(
        sim.map().ore(Cell::new(20, 20)),
        CELL_LIMIT,
        "a grown cell should stop where the rules say"
    );
}

#[test]
fn a_field_stops_growing_once_it_is_full() {
    // Otherwise a mine left alone for an hour funds an army on its own.
    let mut sim = scenario(Map::new(48, 48), vec![("ore_mine", 20, 20)]);
    run(&mut sim, INTERVAL * 2_000);
    let full = sim.map().total_ore();
    run(&mut sim, INTERVAL * 500);

    assert_eq!(sim.map().total_ore(), full);
}

#[test]
fn a_destroyed_mine_grows_nothing() {
    // A mine is a thing on the map that can be taken away, and taking it away
    // has to turn a renewable field back into a finite one — otherwise the
    // ground keeps producing for a structure that no longer exists.
    let mut sim = scenario(Map::new(48, 48), vec![("ore_mine", 20, 20)]);
    let gunner = sim.spawn_unit(
        PlayerId(0),
        sim.rules().kind_of("scout").unwrap(),
        Cell::new(24, 20).centre(),
    );
    let mine_id = sim.units().ids()[0];
    run(&mut sim, INTERVAL * 5);
    assert!(sim.map().total_ore() > 0, "it should have been growing");

    // Shot down through the ordinary damage path. Auto-targeting leaves
    // neutrals alone, which is why this takes an explicit order — the same
    // distinction that lets a player shoot a civilian on purpose.
    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Attack {
            units: vec![gunner],
            target: mine_id,
        },
    )]);
    for _ in 0..400 {
        sim.tick(&[]);
        if sim.unit(mine_id).is_none() {
            break;
        }
    }
    assert!(sim.unit(mine_id).is_none(), "the mine would not come down");

    let after = sim.map().total_ore();
    run(&mut sim, INTERVAL * 50);
    assert_eq!(
        sim.map().total_ore(),
        after,
        "the ground is still producing for a mine that is gone"
    );
}

#[test]
fn ore_growth_is_deterministic() {
    let go = || {
        let mut sim = scenario(Map::new(48, 48), vec![("ore_mine", 20, 20)]);
        run(&mut sim, 300);
        (sim.state_hash(), sim.map().total_ore())
    };
    assert_eq!(go(), go());
}
