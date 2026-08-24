//! Standing modifiers, against the rules the game ships.
//!
//! The mechanism only matters through its effects, so these check the effects.
//! A grant that adds up correctly and changes nothing would pass a unit test
//! and be useless in a match.

use redshift_sim::Rules;
use redshift_sim::command::PlayerId;
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn shipped_rules() -> Rules {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rules");
    Rules::load_from(&root).expect("the shipped rules should load")
}

fn base(buildings: &[(&str, i32, i32)]) -> Sim {
    let rules = shipped_rules();
    let spawns = buildings
        .iter()
        .map(|(id, x, y)| Spawn {
            owner: PlayerId(0),
            kind: rules
                .kind_of(id)
                .unwrap_or_else(|| panic!("no entity {id:?}")),
            pos: Cell::new(*x, *y).centre(),
        })
        .collect();
    Sim::new(MatchSetup {
        seed: 0x0B00,
        map: Map::new(48, 48),
        rules,
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
    })
}

#[test]
fn a_player_with_nothing_gets_the_baseline() {
    let sim = base(&[]);
    assert_eq!(sim.boons().ore_value(PlayerId(0)).0, 100);
    assert!(!sim.boons().veteran_production(PlayerId(0), "infantry"));
}

#[test]
fn an_ore_purifier_raises_the_value_of_every_load() {
    let sim = base(&[
        ("power_plant", 10, 10),
        ("power_plant", 14, 10),
        ("power_plant", 10, 14),
        ("ore_purifier", 20, 20),
    ]);
    assert_eq!(
        sim.boons().ore_value(PlayerId(0)).0,
        125,
        "the purifier is not granting its bonus"
    );
}

#[test]
fn two_purifiers_compound() {
    // Overwriting rather than multiplying would make the second free to build
    // and useless to own.
    let sim = base(&[
        ("power_plant", 10, 10),
        ("power_plant", 14, 10),
        ("power_plant", 10, 14),
        ("power_plant", 14, 14),
        ("power_plant", 10, 18),
        ("ore_purifier", 20, 20),
        ("ore_purifier", 26, 20),
    ]);
    assert!(
        sim.boons().ore_value(PlayerId(0)).0 > 125,
        "two purifiers were no better than one"
    );
}

#[test]
fn a_purifier_with_no_power_grants_nothing() {
    // One that kept paying while blacked out would make cutting an enemy's
    // power much less worth doing.
    let sim = base(&[("ore_purifier", 20, 20)]);
    assert!(
        !sim.power().is_satisfied(PlayerId(0)),
        "the test needs a shortage"
    );
    assert_eq!(
        sim.boons().ore_value(PlayerId(0)).0,
        100,
        "an unpowered purifier is still granting its bonus"
    );
}

#[test]
fn losing_the_purifier_takes_the_bonus_with_it() {
    // The grants are rebuilt from scratch every tick precisely so this needs no
    // bookkeeping.
    let mut sim = base(&[
        ("power_plant", 10, 10),
        ("power_plant", 14, 10),
        ("power_plant", 10, 14),
        ("ore_purifier", 20, 20),
    ]);
    sim.tick(&[]);
    assert_eq!(sim.boons().ore_value(PlayerId(0)).0, 125);

    let purifier = *sim.units().ids().last().expect("the purifier");
    let tank = sim.rules().kind_of("grizzly_tank").expect("tank");
    for i in 0..8i32 {
        sim.spawn_unit(
            PlayerId(1),
            tank,
            Cell::new(24 + i % 4, 20 + i / 4).centre(),
        );
    }
    for _ in 0..8_000 {
        sim.tick(&[]);
        if sim.units().get(purifier).is_none() {
            break;
        }
    }
    assert!(sim.units().get(purifier).is_none(), "the purifier survived");
    assert_eq!(
        sim.boons().ore_value(PlayerId(0)).0,
        100,
        "the bonus outlived the building that granted it"
    );
}

#[test]
fn one_player_does_not_get_anothers_bonus() {
    let rules = shipped_rules();
    let sim = Sim::new(MatchSetup {
        seed: 1,
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
        spawns: (0..3)
            .map(|i| Spawn {
                owner: PlayerId(0),
                kind: rules.kind_of("power_plant").unwrap(),
                pos: Cell::new(10 + i * 4, 10).centre(),
            })
            .chain(std::iter::once(Spawn {
                owner: PlayerId(0),
                kind: rules.kind_of("ore_purifier").unwrap(),
                pos: Cell::new(20, 20).centre(),
            }))
            .collect(),
        rules,
    });
    assert_eq!(sim.boons().ore_value(PlayerId(0)).0, 125);
    assert_eq!(
        sim.boons().ore_value(PlayerId(1)).0,
        100,
        "one player's purifier is enriching another"
    );
}

#[test]
fn boons_are_deterministic() {
    let run = || {
        let mut sim = base(&[
            ("power_plant", 10, 10),
            ("power_plant", 14, 10),
            ("power_plant", 10, 14),
            ("ore_purifier", 20, 20),
            ("refinery", 28, 28),
        ]);
        let mut hashes = Vec::new();
        for _ in 0..600 {
            sim.tick(&[]);
            hashes.push(sim.state_hash());
        }
        (hashes, sim.boons().ore_value(PlayerId(0)).0)
    };
    let (a, a_value) = run();
    let (b, b_value) = run();
    assert_eq!(a, b, "two identical bases diverged");
    assert_eq!(a_value, b_value);
}
