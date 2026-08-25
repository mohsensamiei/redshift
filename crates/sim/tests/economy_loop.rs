//! The harvester cycle, running.
//!
//! This is the project's first autonomous behaviour: nothing here comes from a
//! player command. A harvester picks a field, walks to it, works, finds a
//! refinery, unloads, and goes back — thousands of decisions with no command
//! stream to correct a peer that chose differently.
//!
//! So these tests care about two things in roughly equal measure: that the loop
//! makes money, and that it never wedges. A harvester that stops working is
//! worse than one that works slowly, because the player has no way to tell it
//! to start again.

use redshift_data::rules::{ArmourTable, EntityDef, Rules};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Percent};
use redshift_sim::command::PlayerId;
use redshift_sim::map::{Cell, Map, Terrain};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn rules() -> Rules {
    let armour: ArmourTable =
        ron::from_str(r#"( classes: ["none"], table: { "blast": { "none": 100 } } )"#)
            .expect("armour");

    let entities = vec![
        EntityDef {
            id: "harvester".into(),
            name_key: "unit.harvester".into(),
            side: None,
            category: "vehicle".into(),
            traits: vec![
                Trait::Health {
                    max: 600,
                    armour: "none".into(),
                },
                Trait::Mobile {
                    speed: Hundredths(500),
                    turn_rate: 720,
                    locomotor: Locomotor::Wheeled,
                    surfaces: None,
                    size: None,
                    layer: None,
                },
                Trait::Harvester {
                    capacity: 100,
                    gather_rate: Hundredths(100),
                },
            ],
        },
        // The same miner at a quarter of the speed. Two rates are the only way
        // to show that the rate is read at all — a single harvester works
        // exactly as convincingly whether its declared rate is consulted or
        // silently replaced by a constant, which is how that went unnoticed.
        EntityDef {
            id: "slow_harvester".into(),
            name_key: "unit.slow_harvester".into(),
            side: None,
            category: "vehicle".into(),
            traits: vec![
                Trait::Health {
                    max: 600,
                    armour: "none".into(),
                },
                Trait::Mobile {
                    speed: Hundredths(500),
                    turn_rate: 720,
                    locomotor: Locomotor::Wheeled,
                    surfaces: None,
                    size: None,
                    layer: None,
                },
                Trait::Harvester {
                    capacity: 100,
                    gather_rate: Hundredths(25),
                },
            ],
        },
        EntityDef {
            id: "refinery".into(),
            name_key: "building.refinery".into(),
            side: None,
            category: "structure".into(),
            traits: vec![
                Trait::Health {
                    max: 1000,
                    armour: "none".into(),
                },
                Trait::Refinery {
                    value_per_unit: Percent(100),
                },
                Trait::Footprint {
                    width: 3,
                    height: 3,
                },
            ],
        },
    ];

    Rules::from_parts(entities, Vec::new(), armour, Vec::new()).expect("valid rules")
}

/// A map with a refinery, some harvesters, and an ore field a short walk away.
fn scenario(harvesters: usize, ore_at: Cell, ore_radius: i32) -> MatchSetup {
    let rules = rules();
    let mut map = Map::new(48, 48);
    map.add_ore_field(ore_at, ore_radius, 400);

    let harvester = rules.kind_of("harvester").expect("harvester");
    let refinery = rules.kind_of("refinery").expect("refinery");

    let mut spawns = vec![Spawn {
        owner: PlayerId(0),
        kind: refinery,
        pos: Cell::new(10, 10).centre(),
    }];
    for i in 0..harvesters as i32 {
        spawns.push(Spawn {
            owner: PlayerId(0),
            kind: harvester,
            pos: Cell::new(13 + i % 3, 10 + i / 3).centre(),
        });
    }

    MatchSetup {
        seed: 0x0FE_5EED,
        map,
        rules,
        players: vec![PlayerSetup {
            id: PlayerId(0),
            faction: None,
        }],
        spawns,
    }
}

fn credits(sim: &Sim) -> u32 {
    sim.treasury().credits(PlayerId(0))
}

#[test]
fn a_harvester_earns_credits_without_being_told_to() {
    // Nothing in this test issues a command. If the harvester needed one, the
    // credits would never move.
    let mut sim = Sim::new(scenario(1, Cell::new(24, 24), 3));
    let starting = credits(&sim);
    let ore_before = sim.map().total_ore();

    for _ in 0..1_500 {
        sim.tick(&[]);
    }

    assert!(
        credits(&sim) > starting,
        "a full cycle earned nothing: {} credits",
        credits(&sim)
    );
    assert!(
        sim.map().total_ore() < ore_before,
        "credits appeared without ore being consumed"
    );
}

#[test]
fn ore_taken_and_credits_paid_balance() {
    // Credits must come from somewhere. A mismatch here means the map is
    // funding ore it never held, which is the kind of thing that only shows up
    // as an economy that feels subtly wrong.
    let mut sim = Sim::new(scenario(2, Cell::new(24, 24), 3));
    let ore_before = sim.map().total_ore();
    let credits_before = credits(&sim);

    for _ in 0..3_000 {
        sim.tick(&[]);
    }

    let consumed = ore_before - sim.map().total_ore();
    let earned = (credits(&sim) - credits_before) as u64;

    // Ore still aboard a harvester has been taken but not yet paid for, so
    // earnings trail consumption rather than matching it exactly.
    assert!(earned > 0, "nothing was earned");
    assert!(
        earned <= consumed,
        "earned {earned} from {consumed} ore, which is money from nowhere"
    );
    assert!(
        consumed - earned <= 200,
        "{} ore is unaccounted for, which is more than two full loads",
        consumed - earned
    );
}

#[test]
fn several_harvesters_work_different_squares() {
    // Piling onto one cell while the rest of a field sits untouched is both
    // slower and visibly silly.
    let mut sim = Sim::new(scenario(4, Cell::new(24, 24), 3));

    let mut saw_spread = false;
    for _ in 0..900 {
        sim.tick(&[]);
        let fields: Vec<_> = sim
            .units()
            .iter()
            .filter_map(|(_, u)| u.harvest.and_then(|h| h.field))
            .collect();
        let mut unique = fields.clone();
        unique.sort_by_key(|c| (c.x, c.y));
        unique.dedup();
        if fields.len() >= 2 && unique.len() == fields.len() {
            saw_spread = true;
        }
        assert_eq!(
            unique.len(),
            fields.len(),
            "two harvesters claimed one cell"
        );
    }
    assert!(saw_spread, "harvesters never worked at the same time");
}

#[test]
fn a_harvester_keeps_working_after_its_field_runs_out() {
    // The failure this pins: a harvester walks to a field, finds it mined out,
    // and stops forever. The player cannot tell it to resume, so the economy
    // silently dies.
    let mut sim = Sim::new(scenario(3, Cell::new(24, 24), 1));
    let mut last = credits(&sim);
    let mut stalls = 0;

    for round in 0..12 {
        for _ in 0..500 {
            sim.tick(&[]);
        }
        let now = credits(&sim);
        if now == last {
            stalls += 1;
        }
        last = now;
        // Once the small field is exhausted no further income is possible, so
        // only complain if it stalls while ore remains.
        if sim.map().total_ore() == 0 {
            break;
        }
        assert!(
            stalls < 2,
            "income stopped at round {round} with ore still on the map"
        );
    }
    assert!(credits(&sim) > 5_000, "the starting credits never grew");
}

#[test]
fn a_harvester_with_nowhere_to_unload_keeps_its_load() {
    // A refinery lost mid-run must not silently destroy the player's income.
    let rules = rules();
    let mut map = Map::new(48, 48);
    map.add_ore_field(Cell::new(24, 24), 3, 400);

    // No refinery at all.
    let setup = MatchSetup {
        seed: 1,
        map,
        players: vec![PlayerSetup {
            id: PlayerId(0),
            faction: None,
        }],
        spawns: vec![Spawn {
            owner: PlayerId(0),
            kind: rules.kind_of("harvester").unwrap(),
            pos: Cell::new(20, 20).centre(),
        }],
        rules,
    };
    let mut sim = Sim::new(setup);

    for _ in 0..2_000 {
        sim.tick(&[]);
    }

    let carrying = sim
        .units()
        .iter()
        .filter_map(|(_, u)| u.harvest.map(|h| h.load))
        .max()
        .unwrap_or(0);
    assert!(
        carrying > 0,
        "the harvester dropped its load rather than keeping it"
    );
    assert_eq!(
        credits(&sim),
        5_000,
        "credits appeared with no refinery to pay them"
    );
}

#[test]
fn a_harvester_on_a_bare_map_idles_rather_than_searching_forever() {
    // Idling is visible and diagnosable. Rescanning the whole map every tick
    // would look like a performance bug instead.
    let rules = rules();
    let setup = MatchSetup {
        seed: 1,
        map: Map::new(48, 48),
        players: vec![PlayerSetup {
            id: PlayerId(0),
            faction: None,
        }],
        spawns: vec![Spawn {
            owner: PlayerId(0),
            kind: rules.kind_of("harvester").unwrap(),
            pos: Cell::new(20, 20).centre(),
        }],
        rules,
    };
    let mut sim = Sim::new(setup);

    for _ in 0..600 {
        sim.tick(&[]);
    }
    for (_, unit) in sim.units().iter() {
        assert!(
            unit.order.is_idle(),
            "a harvester is still chasing ore that is not there"
        );
    }
}

#[test]
fn harvesters_never_drive_into_water_to_reach_ore() {
    let rules = rules();
    let mut map = Map::new(48, 48);
    map.fill_rect(Cell::new(16, 16), Cell::new(32, 20), Terrain::Water);
    map.add_ore_field(Cell::new(24, 30), 3, 400);

    let setup = MatchSetup {
        seed: 1,
        map,
        players: vec![PlayerSetup {
            id: PlayerId(0),
            faction: None,
        }],
        spawns: vec![
            Spawn {
                owner: PlayerId(0),
                kind: rules.kind_of("refinery").unwrap(),
                pos: Cell::new(10, 10).centre(),
            },
            Spawn {
                owner: PlayerId(0),
                kind: rules.kind_of("harvester").unwrap(),
                pos: Cell::new(12, 12).centre(),
            },
        ],
        rules,
    };
    let mut sim = Sim::new(setup);

    for _ in 0..2_000 {
        sim.tick(&[]);
        for (_, unit) in sim.units().iter() {
            assert_ne!(
                sim.map().terrain(unit.cell()),
                Terrain::Water,
                "a harvester drove into the lake at {:?}",
                unit.cell()
            );
        }
    }
}

#[test]
fn a_harvester_reaches_a_refinery_that_occupies_its_own_ground() {
    // The regression this pins: once buildings started blocking the cells they
    // stand on, harvesters were still being sent to the refinery's *centre*.
    // Pathfinding correctly reported no route into a solid building, so every
    // harvester walked to the edge, gave up, and the economy silently stopped
    // earning while still looking busy.
    let mut sim = Sim::new(scenario(2, Cell::new(24, 24), 3));

    let refinery_cell = sim
        .units()
        .iter()
        .find(|(_, u)| u.harvest.is_none())
        .map(|(_, u)| u.cell())
        .expect("a refinery");
    assert!(
        sim.map().is_blocked(refinery_cell),
        "the test needs a refinery that actually occupies ground"
    );

    let before = credits(&sim);
    for _ in 0..2_000 {
        sim.tick(&[]);
    }
    assert!(
        credits(&sim) > before,
        "harvesters never delivered to a refinery standing on its own footprint"
    );
}

#[test]
fn the_economy_is_deterministic() {
    // The point of the whole exercise. Free choice with no command stream, run
    // twice, must land in exactly the same place.
    let run = || {
        let mut sim = Sim::new(scenario(4, Cell::new(28, 28), 4));
        let mut hashes = Vec::new();
        for _ in 0..1_200 {
            sim.tick(&[]);
            hashes.push(sim.state_hash());
        }
        (hashes, credits(&sim), sim.map().total_ore())
    };

    let (first, first_credits, first_ore) = run();
    let (second, second_credits, second_ore) = run();

    assert_eq!(first, second, "two identical economies diverged");
    assert_eq!(first_credits, second_credits);
    assert_eq!(first_ore, second_ore);
    assert!(
        first_credits > 5_000,
        "nothing was earned, so this proves nothing"
    );
}

#[test]
fn a_harvester_gathers_at_its_own_declared_rate() {
    // `gather_rate` was resolved into the stat table and never read: the bite
    // size was a flat constant, so every miner in the game worked at the same
    // speed however its rules were written. A faster one was not expressible,
    // which looks fine right up until somebody adds a Chrono Miner and cannot
    // work out why it changes nothing.
    //
    // Two rates, one field each, same distance. The fast one should deliver
    // meaningfully more.
    let earned = |kind: &str| -> u32 {
        let rules = rules();
        let miner = rules.kind_of(kind).unwrap();
        let refinery = rules.kind_of("refinery").unwrap();
        let mut map = Map::new(48, 48);
        map.add_ore_field(Cell::new(24, 24), 3, 400);
        let mut sim = Sim::new(MatchSetup {
            seed: 0x_6A7E,
            map,
            players: vec![PlayerSetup {
                id: PlayerId(0),
                faction: None,
            }],
            spawns: vec![
                Spawn {
                    owner: PlayerId(0),
                    kind: refinery,
                    pos: Cell::new(10, 10).centre(),
                },
                Spawn {
                    owner: PlayerId(0),
                    kind: miner,
                    pos: Cell::new(13, 10).centre(),
                },
            ],
            rules,
        });
        let start = sim.treasury().credits(PlayerId(0));
        for _ in 0..4_000 {
            sim.tick(&[]);
        }
        sim.treasury().credits(PlayerId(0)) - start
    };

    let fast = earned("harvester");
    let slow = earned("slow_harvester");

    assert!(fast > 0, "the fast miner delivered nothing at all");
    assert!(
        fast > slow,
        "a miner declaring four times the rate earned {fast} against {slow} — \
         the declared rate is not being read"
    );
}
