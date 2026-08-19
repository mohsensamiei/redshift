//! Fog of war, and what it changes.
//!
//! Visibility would be a presentation detail if it only decided what to draw.
//! It is not: units acquire only targets they can see, so fog decides who
//! shoots whom. These tests are mostly about that consequence, because a fog
//! that hid things on screen while the simulation fought through it would be
//! worse than no fog at all.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::Sight;
use redshift_sim::command::PlayerId;
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

/// A scout that sees a long way, and a sniper that shoots further than it sees.
///
/// The second is deliberate: it is the only way to tell "held fire because it
/// was out of range" apart from "held fire because it could not see".
fn rules() -> Rules {
    let armour: ArmourTable =
        ron::from_str(r#"( classes: ["none"], table: { "shot": { "none": 100 } } )"#)
            .expect("armour");

    let weapons = vec![WeaponDef {
        id: "long_rifle".into(),
        damage: 25,
        warhead: "shot".into(),
        reload: Ticks(10),
        // Reaches twelve cells.
        range: Hundredths(1200),
        splash_radius: Hundredths::ZERO,
        projectile_speed: Hundredths::ZERO,
        homing: false,
        targets: vec![],
        instant_kill: false,
        ammo: 0,
        intercepts: false,
    }];

    let unit = |id: &str, vision: i32| EntityDef {
        id: id.into(),
        name_key: format!("unit.{id}"),
        side: None,
        category: "infantry".into(),
        traits: vec![
            Trait::Health {
                max: 200,
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
                range: Hundredths(vision),
            },
            Trait::Armed {
                weapon: "long_rifle".into(),
                turret: true,
                turret_rate: 3600,
            },
        ],
    };

    Rules::from_parts(
        // Sees four cells, shoots twelve.
        vec![unit("sniper", 400), unit("scout", 1000)],
        weapons,
        armour,
        Vec::new(),
    )
    .expect("valid rules")
}

fn scenario(spawns: Vec<(u8, &str, i32, i32)>) -> MatchSetup {
    let rules = rules();
    MatchSetup {
        seed: 0xF06,
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
                owner: PlayerId(owner),
                kind: rules
                    .kind_of(kind)
                    .unwrap_or_else(|| panic!("no kind {kind}")),
                pos: Cell::new(x, y).centre(),
            })
            .collect(),
        rules,
    }
}

#[test]
fn a_unit_reveals_the_ground_around_itself_from_the_first_tick() {
    // Before any tick: a player who could not see their own units at match
    // start would open on a black screen.
    let sim = Sim::new(scenario(vec![(0, "scout", 20, 20)]));
    assert!(sim.visibility().is_visible(PlayerId(0), Cell::new(20, 20)));
    assert_eq!(
        sim.visibility().sight(PlayerId(0), Cell::new(40, 40)),
        Sight::Unseen
    );
}

#[test]
fn ground_a_unit_walks_away_from_is_remembered_but_no_longer_watched() {
    let mut sim = Sim::new(scenario(vec![(0, "scout", 5, 5)]));
    sim.tick(&[]);
    assert!(sim.visibility().is_visible(PlayerId(0), Cell::new(5, 5)));

    // Move it far away by spawning elsewhere is not possible, so walk it.
    let id = sim.units().ids()[0];
    sim.tick(&[redshift_sim::command::Command::new(
        PlayerId(0),
        0,
        redshift_sim::command::CommandKind::Move {
            units: vec![id],
            target: Cell::new(40, 40),
        },
    )]);
    for _ in 0..2_000 {
        sim.tick(&[]);
    }

    assert_eq!(
        sim.visibility().sight(PlayerId(0), Cell::new(5, 5)),
        Sight::Fogged,
        "ground it explored should be remembered rather than forgotten"
    );
    assert!(sim.visibility().is_visible(PlayerId(0), Cell::new(40, 40)));
}

#[test]
fn a_unit_will_not_shoot_what_it_cannot_see() {
    // The sniper reaches twelve cells and sees four. An enemy at eight is well
    // within range and entirely invisible.
    let mut sim = Sim::new(scenario(vec![(0, "sniper", 10, 10), (1, "sniper", 18, 10)]));
    for _ in 0..400 {
        sim.tick(&[]);
    }
    for (_, unit) in sim.units().iter() {
        assert_eq!(
            unit.health, 200,
            "someone fired through fog at a target they could not see"
        );
    }
    assert_eq!(sim.units().len(), 2);
}

#[test]
fn a_scout_lets_its_side_shoot_what_it_spots() {
    // The same sniper, with a scout of its own nearby to see for it. This is
    // the whole reason vision is shared across a side rather than per unit.
    let mut sim = Sim::new(scenario(vec![
        (0, "sniper", 10, 10),
        (0, "scout", 14, 10),
        (1, "sniper", 18, 10),
    ]));
    let enemy = sim.units().ids()[2];

    for _ in 0..400 {
        sim.tick(&[]);
        if sim.units().get(enemy).is_none_or(|u| u.health < 200) {
            break;
        }
    }
    assert!(
        sim.units().get(enemy).is_none_or(|u| u.health < 200),
        "the spotted enemy was never fired on"
    );
}

#[test]
fn one_players_scouting_does_not_reveal_the_map_to_the_other() {
    let mut sim = Sim::new(scenario(vec![(0, "scout", 20, 20)]));
    sim.tick(&[]);
    assert!(sim.visibility().is_visible(PlayerId(0), Cell::new(20, 20)));
    assert_eq!(
        sim.visibility().sight(PlayerId(1), Cell::new(20, 20)),
        Sight::Unseen,
        "one player's vision leaked to another"
    );
}

#[test]
fn a_destroyed_scout_stops_revealing_ground_on_the_tick_it_dies() {
    // Vision is rebuilt after deaths for exactly this reason. A tick of
    // posthumous sight would be a small thing that is very confusing to watch.
    let mut sim = Sim::new(scenario(vec![
        (0, "scout", 20, 20),
        (1, "sniper", 24, 20),
        (1, "sniper", 25, 20),
        (1, "sniper", 26, 20),
    ]));
    let scout = sim.units().ids()[0];

    let mut died = false;
    for _ in 0..2_000 {
        sim.tick(&[]);
        if sim.units().get(scout).is_none() {
            died = true;
            break;
        }
    }
    assert!(died, "the scout survived, so this proves nothing");

    // Ground only the scout could see is fogged, not still watched.
    assert_eq!(
        sim.visibility().sight(PlayerId(0), Cell::new(14, 20)),
        Sight::Fogged,
        "a dead scout was still revealing ground"
    );
}

#[test]
fn revealing_everything_lets_a_spectator_see_the_whole_map() {
    let mut sim = Sim::new(scenario(vec![(0, "scout", 20, 20)]));
    sim.reveal_all();
    sim.tick(&[]);
    assert!(sim.visibility().is_visible(PlayerId(1), Cell::new(2, 45)));
}

#[test]
fn vision_is_deterministic() {
    let run = || {
        let mut sim = Sim::new(scenario(vec![
            (0, "scout", 10, 10),
            (0, "sniper", 12, 10),
            (1, "scout", 30, 30),
            (1, "sniper", 28, 30),
        ]));
        let ids = sim.units().ids();
        sim.tick(&[redshift_sim::command::Command::new(
            PlayerId(0),
            0,
            redshift_sim::command::CommandKind::Move {
                units: ids,
                target: Cell::new(30, 30),
            },
        )]);
        let mut hashes = Vec::new();
        for _ in 0..1_200 {
            sim.tick(&[]);
            hashes.push(sim.state_hash());
        }
        (
            hashes,
            sim.visibility().explored_percent(PlayerId(0)),
            sim.units().len(),
        )
    };
    let (a, a_seen, a_units) = run();
    let (b, b_seen, b_units) = run();
    assert_eq!(a, b, "two identical scouting runs diverged");
    assert_eq!((a_seen, a_units), (b_seen, b_units));
    assert!(a_seen > 5, "nothing was explored, so this proves nothing");
}
