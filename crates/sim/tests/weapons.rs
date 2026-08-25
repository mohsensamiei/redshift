//! Two weapons, instant kills, ammunition and interception.

use redshift_data::rules::{ArmourTable, EntityDef, Rules, WeaponDef};
use redshift_data::traits::{Layer, Locomotor, Trait};
use redshift_data::value::{Hundredths, Ticks};
use redshift_sim::command::PlayerId;
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};

fn armour() -> ArmourTable {
    ron::from_str(r#"( classes: ["none", "air"], table: { "shot": { "none": 100, "air": 100 } } )"#)
        .unwrap()
}

fn weapon(id: &str, targets: Vec<Layer>, extra: fn(&mut WeaponDef)) -> WeaponDef {
    let mut w = WeaponDef {
        id: id.into(),
        damage: 25,
        warhead: "shot".into(),
        reload: Ticks(8),
        range: Hundredths(600),
        splash_radius: Hundredths::ZERO,
        projectile_speed: Hundredths::ZERO,
        homing: false,
        targets,
        instant_kill: false,
        ammo: 0,
        intercepts: false,
        heals: false,
    };
    extra(&mut w);
    w
}

fn mobile(locomotor: Locomotor) -> Trait {
    Trait::Mobile {
        speed: Hundredths(400),
        turn_rate: 3600,
        locomotor,
        surfaces: None,
        size: None,
        layer: None,
    }
}

fn unit(id: &str, locomotor: Locomotor, armour_class: &str, extra: Vec<Trait>) -> EntityDef {
    let mut traits = vec![
        Trait::Health {
            max: 200,
            armour: armour_class.into(),
        },
        mobile(locomotor),
        Trait::Vision {
            range: Hundredths(900),
        },
    ];
    traits.extend(extra);
    EntityDef {
        id: id.into(),
        name_key: format!("u.{id}"),
        side: None,
        category: "vehicle".into(),
        traits,
    }
}

fn duel(rules: Rules, a: &str, b: &str, gap: i32) -> Sim {
    let ka = rules.kind_of(a).unwrap_or_else(|| panic!("no {a}"));
    let kb = rules.kind_of(b).unwrap_or_else(|| panic!("no {b}"));
    Sim::new(MatchSetup {
        seed: 7,
        map: Map::new(40, 40),
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
        spawns: vec![
            Spawn {
                owner: PlayerId(0),
                kind: ka,
                pos: Cell::new(10, 20).centre(),
            },
            Spawn {
                owner: PlayerId(1),
                kind: kb,
                pos: Cell::new(10 + gap, 20).centre(),
            },
        ],
        rules,
    })
}

fn hurt(sim: &mut Sim, ticks: u32) -> bool {
    let victim = sim.units().ids()[1];
    for _ in 0..ticks {
        sim.tick(&[]);
        if sim.units().get(victim).is_none_or(|u| u.health < 200) {
            return true;
        }
    }
    false
}

#[test]
fn a_unit_with_two_weapons_engages_both_layers() {
    // An Apocalypse fires a cannon at the ground and missiles at the air, and
    // needs both at once rather than choosing a stance.
    let apoc = unit(
        "apoc",
        Locomotor::Tracked,
        "none",
        vec![
            Trait::Armed {
                weapon: "cannon".into(),
                turret: true,
                turret_rate: 3600,
            },
            Trait::Secondary {
                weapon: "missile".into(),
                turret: true,
                turret_rate: 3600,
            },
        ],
    );
    let plane = unit("plane", Locomotor::Air, "air", vec![]);
    let truck = unit("truck", Locomotor::Wheeled, "none", vec![]);
    let rules = Rules::from_parts(
        vec![apoc, plane, truck],
        vec![
            weapon("cannon", vec![], |_| {}),
            weapon("missile", vec![Layer::Air], |_| {}),
        ],
        armour(),
        Vec::new(),
    )
    .expect("rules");

    let mut vs_air = duel(rules.clone(), "apoc", "plane", 4);
    assert!(hurt(&mut vs_air, 300), "it could not hit an aircraft");

    let mut vs_ground = duel(rules, "apoc", "truck", 4);
    assert!(
        hurt(&mut vs_ground, 300),
        "it could not hit a ground vehicle"
    );
}

#[test]
fn one_weapon_alone_still_only_reaches_its_own_layer() {
    let tank = unit(
        "tank",
        Locomotor::Tracked,
        "none",
        vec![Trait::Armed {
            weapon: "cannon".into(),
            turret: true,
            turret_rate: 3600,
        }],
    );
    let plane = unit("plane", Locomotor::Air, "air", vec![]);
    let rules = Rules::from_parts(
        vec![tank, plane],
        vec![weapon("cannon", vec![], |_| {})],
        armour(),
        Vec::new(),
    )
    .expect("rules");

    let mut sim = duel(rules, "tank", "plane", 4);
    assert!(!hurt(&mut sim, 300), "a ground-only weapon hit an aircraft");
}

#[test]
fn an_instant_kill_weapon_kills_outright() {
    let sniper = unit(
        "sniper",
        Locomotor::Foot,
        "none",
        vec![Trait::Armed {
            weapon: "rifle".into(),
            turret: true,
            turret_rate: 3600,
        }],
    );
    let victim = unit("victim", Locomotor::Foot, "none", vec![]);
    let rules = Rules::from_parts(
        vec![sniper, victim],
        vec![weapon("rifle", vec![], |w| {
            w.instant_kill = true;
            // Deliberately feeble, so only the instant-kill rule can explain a
            // death.
            w.damage = 1;
        })],
        armour(),
        Vec::new(),
    )
    .expect("rules");

    let mut sim = duel(rules, "sniper", "victim", 4);
    let target = sim.units().ids()[1];
    for _ in 0..200 {
        sim.tick(&[]);
        if sim.units().get(target).is_none() {
            return;
        }
    }
    panic!("an instant-kill weapon left its target standing");
}

#[test]
fn an_instant_kill_weapon_does_nothing_to_what_it_cannot_hurt() {
    // The distinction from very high damage: a sniper kills any infantryman and
    // does nothing at all to a tank. Enormous damage would make it excellent
    // against both.
    let armour: ArmourTable = ron::from_str(
        r#"( classes: ["none", "heavy"], table: { "shot": { "none": 100, "heavy": 0 } } )"#,
    )
    .unwrap();
    let sniper = unit(
        "sniper",
        Locomotor::Foot,
        "none",
        vec![Trait::Armed {
            weapon: "rifle".into(),
            turret: true,
            turret_rate: 3600,
        }],
    );
    let tank = unit("tank", Locomotor::Tracked, "heavy", vec![]);
    let rules = Rules::from_parts(
        vec![sniper, tank],
        vec![weapon("rifle", vec![], |w| w.instant_kill = true)],
        armour,
        Vec::new(),
    )
    .expect("rules");

    let mut sim = duel(rules, "sniper", "tank", 4);
    assert!(
        !hurt(&mut sim, 400),
        "an instant-kill weapon destroyed something its warhead cannot hurt"
    );
}

#[test]
fn a_unit_runs_out_of_ammunition() {
    let gunner = unit(
        "gunner",
        Locomotor::Tracked,
        "none",
        vec![Trait::Armed {
            weapon: "cannon".into(),
            turret: true,
            turret_rate: 3600,
        }],
    );
    let target = unit("target", Locomotor::Tracked, "none", vec![]);
    let rules = Rules::from_parts(
        vec![gunner, target],
        vec![weapon("cannon", vec![], |w| {
            w.ammo = 2;
            w.damage = 10;
        })],
        armour(),
        Vec::new(),
    )
    .expect("rules");

    let mut sim = duel(rules, "gunner", "target", 4);
    let victim = sim.units().ids()[1];
    for _ in 0..600 {
        sim.tick(&[]);
    }
    let taken = 200 - sim.units().get(victim).expect("alive").health;
    assert_eq!(
        taken, 20,
        "two shots of ten damage should be all it ever fires, took {taken}"
    );
}

#[test]
fn unlimited_ammunition_keeps_firing() {
    let gunner = unit(
        "gunner",
        Locomotor::Tracked,
        "none",
        vec![Trait::Armed {
            weapon: "cannon".into(),
            turret: true,
            turret_rate: 3600,
        }],
    );
    let target = unit("target", Locomotor::Tracked, "none", vec![]);
    let rules = Rules::from_parts(
        vec![gunner, target],
        vec![weapon("cannon", vec![], |w| w.damage = 10)],
        armour(),
        Vec::new(),
    )
    .expect("rules");

    let mut sim = duel(rules, "gunner", "target", 4);
    let victim = sim.units().ids()[1];
    for _ in 0..600 {
        sim.tick(&[]);
        if sim.units().get(victim).is_none() {
            return;
        }
    }
    panic!("a weapon with no ammunition limit stopped firing");
}

#[test]
fn an_interceptor_shoots_down_a_missile_in_flight() {
    // Three units in the original exist largely to do this, and two exist to
    // fire the missiles they stop.
    let launcher = unit(
        "launcher",
        Locomotor::Tracked,
        "none",
        vec![Trait::Armed {
            weapon: "slow_missile".into(),
            turret: true,
            turret_rate: 3600,
        }],
    );
    let aegis = unit(
        "aegis",
        Locomotor::Tracked,
        "none",
        vec![Trait::Armed {
            weapon: "interceptor".into(),
            turret: true,
            turret_rate: 3600,
        }],
    );
    let rules = Rules::from_parts(
        vec![launcher, aegis],
        vec![
            weapon("slow_missile", vec![], |w| {
                w.projectile_speed = Hundredths(60);
                w.range = Hundredths(1200);
                w.reload = Ticks(200);
            }),
            weapon("interceptor", vec![], |w| {
                w.intercepts = true;
                w.range = Hundredths(800);
            }),
        ],
        armour(),
        Vec::new(),
    )
    .expect("rules");

    // Eight cells: inside the launcher's nine-cell sight and its twelve-cell
    // range, and inside the interceptor's eight-cell reach.
    let mut sim = duel(rules, "launcher", "aegis", 8);
    let aegis_id = sim.units().ids()[1];

    let mut saw_a_shot = false;
    for _ in 0..600 {
        sim.tick(&[]);
        if !sim.projectiles().is_empty() {
            saw_a_shot = true;
        }
    }
    assert!(
        saw_a_shot,
        "the launcher never fired, so this proves nothing"
    );
    assert_eq!(
        sim.units().get(aegis_id).map(|u| u.health),
        Some(200),
        "a missile got through an interceptor"
    );
}

#[test]
fn weapons_are_deterministic() {
    let run = || {
        let apoc = unit(
            "apoc",
            Locomotor::Tracked,
            "none",
            vec![
                Trait::Armed {
                    weapon: "cannon".into(),
                    turret: true,
                    turret_rate: 3600,
                },
                Trait::Secondary {
                    weapon: "missile".into(),
                    turret: true,
                    turret_rate: 3600,
                },
            ],
        );
        let plane = unit("plane", Locomotor::Air, "air", vec![]);
        let rules = Rules::from_parts(
            vec![apoc, plane],
            vec![
                weapon("cannon", vec![], |w| w.ammo = 5),
                weapon("missile", vec![Layer::Air], |_| {}),
            ],
            armour(),
            Vec::new(),
        )
        .expect("rules");
        let mut sim = duel(rules, "apoc", "plane", 4);
        let mut hashes = Vec::new();
        for _ in 0..600 {
            sim.tick(&[]);
            hashes.push(sim.state_hash());
        }
        hashes
    };
    assert_eq!(run(), run());
}
