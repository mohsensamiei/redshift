//! Loading the project's actual rules files, and the validation that guards
//! them.
//!
//! The unit tests cover the types. This covers the data — because a rules file
//! that parses but refers to a weapon nobody defined is exactly the kind of
//! mistake that reaches a match instead of a compiler.

use std::path::{Path, PathBuf};

use redshift_data::rules::{ArmourTable, EntityDef, Rules, RulesError, WeaponDef};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Percent, Ticks};

fn rules_dir() -> PathBuf {
    // Tests run with the crate as the working directory.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rules")
}

fn load() -> Rules {
    match Rules::load_from(&rules_dir()) {
        Ok(rules) => rules,
        Err(e) => panic!("the project's own rules failed to load:\n{e}"),
    }
}

#[test]
fn the_projects_rules_load_and_validate() {
    let rules = load();
    assert!(
        rules.entity_count() >= 8,
        "only {} entities loaded",
        rules.entity_count()
    );
    assert!(rules.weapon("120mm").is_some());
    assert!(rules.faction("america").is_some());
}

#[test]
fn a_unit_is_reachable_by_id_and_by_index() {
    let rules = load();
    let (kind, tank) = rules
        .entity_by_id("grizzly_tank")
        .expect("the tank should exist");
    assert_eq!(rules.entity(kind).id, "grizzly_tank");
    assert_eq!(tank.category, "vehicle");
    assert!(tank.has_trait("Armed"));
    assert!(tank.has_trait("Crushes"));
}

#[test]
fn entity_indices_do_not_depend_on_file_order() {
    // Two peers must agree on every index, since the simulation works in
    // indices and hashes them. Sorting by id makes that independent of how the
    // filesystem happened to enumerate the directory.
    let a = load();
    let b = load();
    for (kind, entity) in a.entities() {
        assert_eq!(
            b.entity(kind).id,
            entity.id,
            "index {} named a different entity",
            kind.0
        );
    }
}

#[test]
fn the_rules_hash_is_stable_across_loads() {
    // Exchanged at the handshake. If it varied between loads of identical
    // files, every join would be refused over rules the peers actually agree
    // on.
    let first = load().hash();
    for _ in 0..5 {
        assert_eq!(load().hash(), first);
    }
}

#[test]
fn the_rules_hash_changes_when_a_value_changes() {
    // The property the handshake depends on: same build, edited stats, refused.
    // That is just as fatal to lockstep as a different binary, and much easier
    // to end up with by accident.
    let base = load();
    let mut entities: Vec<EntityDef> = base.entities().map(|(_, e)| e.clone()).collect();

    let tank = entities
        .iter_mut()
        .find(|e| e.id == "grizzly_tank")
        .expect("tank");
    for t in &mut tank.traits {
        if let Trait::Health { max, .. } = t {
            *max += 1;
        }
    }

    let weapons: Vec<WeaponDef> = ["rifle", "120mm", "artillery", "aa_gun"]
        .iter()
        .filter_map(|id| base.weapon(id).cloned())
        .collect();
    let factions = base.factions().cloned().collect();
    let altered = Rules::from_parts(entities, weapons, base.armour().clone(), factions)
        .expect("the altered rules should still be valid");

    assert_ne!(
        base.hash(),
        altered.hash(),
        "a one-point health change went unnoticed"
    );
}

#[test]
fn every_weapon_reference_resolves() {
    let rules = load();
    for (_, entity) in rules.entities() {
        for t in &entity.traits {
            if let Trait::Armed { weapon, .. } = t {
                assert!(
                    rules.weapon(weapon).is_some(),
                    "{} is armed with \"{weapon}\", which does not exist",
                    entity.id
                );
            }
        }
    }
}

#[test]
fn every_prerequisite_is_buildable_or_exists() {
    let rules = load();
    for (_, entity) in rules.entities() {
        for t in &entity.traits {
            if let Trait::Buildable {
                prerequisites,
                produced_by,
                ..
            } = t
            {
                for p in prerequisites {
                    assert!(
                        rules.kind_of(p).is_some(),
                        "{} requires missing \"{p}\"",
                        entity.id
                    );
                }
                assert!(
                    rules.kind_of(produced_by).is_some(),
                    "{} is produced by missing \"{produced_by}\"",
                    entity.id
                );
            }
        }
    }
}

#[test]
fn the_tech_tree_has_no_cycles() {
    // A prerequisite loop makes a branch permanently unbuildable, and the game
    // gives no hint why — the button is simply always greyed out.
    let rules = load();
    for (kind, entity) in rules.entities() {
        let mut seen = vec![kind];
        let mut frontier = vec![entity.id.clone()];
        let mut depth = 0;

        while let Some(current) = frontier.pop() {
            depth += 1;
            assert!(
                depth < 200,
                "{}: prerequisite chain did not terminate",
                entity.id
            );
            let Some((k, def)) = rules.entity_by_id(&current) else {
                continue;
            };
            for t in &def.traits {
                if let Trait::Buildable { prerequisites, .. } = t {
                    for p in prerequisites {
                        if let Some(pk) = rules.kind_of(p) {
                            assert!(
                                !seen.contains(&pk) || pk == k,
                                "{} is in a prerequisite cycle via \"{p}\"",
                                entity.id
                            );
                            seen.push(pk);
                            frontier.push(p.clone());
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn the_armour_table_covers_every_warhead_in_use() {
    // A warhead with no row does full damage to everything, which reads as a
    // balance bug rather than as missing data.
    let rules = load();
    for (_, entity) in rules.entities() {
        for t in &entity.traits {
            if let Trait::Armed { weapon, .. } = t {
                let w = rules.weapon(weapon).expect("weapon exists");
                assert!(
                    rules.armour().table.contains_key(&w.warhead),
                    "warhead \"{}\" (used by {}) has no armour row",
                    w.warhead,
                    entity.id
                );
            }
        }
    }
}

#[test]
fn counterplay_actually_exists_in_the_table() {
    // The table is only doing its job if some weapons are markedly better
    // against some armour than others. A uniform table would parse fine and
    // quietly remove all counterplay from the game.
    let rules = load();
    let armour = rules.armour();
    let small = armour.multiplier("small_arms", "none");
    let small_vs_heavy = armour.multiplier("small_arms", "heavy");
    let ap_vs_heavy = armour.multiplier("ap_shell", "heavy");

    assert!(
        small.0 > small_vs_heavy.0 * 3,
        "small arms should be poor against heavy armour"
    );
    assert!(
        ap_vs_heavy.0 > small_vs_heavy.0 * 3,
        "shells should beat rifles against armour"
    );
    assert_eq!(
        armour.multiplier("ap_shell", "air"),
        Percent::ZERO,
        "shells cannot hit aircraft"
    );
}

#[test]
fn an_unlisted_pairing_means_full_damage() {
    let rules = load();
    // Forgiving on purpose: a missing entry should mean "nothing special", not
    // "immune", because immunity by omission is invisible until it matters.
    assert_eq!(
        rules.armour().multiplier("nonexistent", "none"),
        Percent::FULL
    );
    assert_eq!(
        rules.armour().multiplier("small_arms", "nonexistent"),
        Percent::FULL
    );
}

// -- Validation ------------------------------------------------------------

fn minimal_armour() -> ArmourTable {
    ron::from_str(
        r#"(
            classes: ["none"],
            table: { "small_arms": { "none": 100 } },
        )"#,
    )
    .expect("armour table")
}

fn entity_with(id: &str, traits: Vec<Trait>) -> EntityDef {
    EntityDef {
        id: id.into(),
        name_key: format!("unit.{id}"),
        side: None,
        category: "infantry".into(),
        traits,
    }
}

#[test]
fn a_missing_weapon_is_refused_with_a_useful_message() {
    let result = Rules::from_parts(
        vec![entity_with(
            "ghost",
            vec![
                Trait::Vision {
                    range: redshift_data::value::Hundredths(500),
                },
                Trait::Armed {
                    weapon: "nonexistent".into(),
                    turret: false,
                    turret_rate: 0,
                },
            ],
        )],
        Vec::new(),
        minimal_armour(),
        Vec::new(),
    );
    match result {
        Err(RulesError::Invalid { problems }) => {
            assert_eq!(problems.len(), 1);
            assert!(
                problems[0].contains("ghost"),
                "the message should name the unit"
            );
            assert!(problems[0].contains("nonexistent"), "and the missing thing");
        }
        other => panic!("expected a validation failure, got {other:?}"),
    }
}

#[test]
fn a_missing_armour_class_is_refused() {
    let result = Rules::from_parts(
        vec![entity_with(
            "ghost",
            vec![Trait::Health {
                max: 100,
                armour: "titanium".into(),
            }],
        )],
        Vec::new(),
        minimal_armour(),
        Vec::new(),
    );
    assert!(matches!(result, Err(RulesError::Invalid { .. })));
}

#[test]
fn duplicate_unique_traits_are_refused() {
    // Two Health traits is a data error, not an interesting combination — the
    // simulation would silently use whichever it found first.
    let result = Rules::from_parts(
        vec![entity_with(
            "confused",
            vec![
                Trait::Health {
                    max: 100,
                    armour: "none".into(),
                },
                Trait::Health {
                    max: 250,
                    armour: "none".into(),
                },
            ],
        )],
        Vec::new(),
        minimal_armour(),
        Vec::new(),
    );
    match result {
        Err(RulesError::Invalid { problems }) => {
            assert!(problems[0].contains("Health"), "got: {}", problems[0]);
        }
        other => panic!("expected a validation failure, got {other:?}"),
    }
}

#[test]
fn a_weapon_without_an_armour_row_is_refused() {
    let result = Rules::from_parts(
        Vec::new(),
        vec![WeaponDef {
            id: "mystery".into(),
            damage: 10,
            warhead: "unlisted".into(),
            reload: Ticks(10),
            range: Hundredths(300),
            splash_radius: Hundredths::ZERO,
            projectile_speed: Hundredths::ZERO,
        }],
        minimal_armour(),
        Vec::new(),
    );
    assert!(matches!(result, Err(RulesError::Invalid { .. })));
}

#[test]
fn every_problem_is_reported_not_just_the_first() {
    // Fixing rules one error per run is miserable. The loader lists everything
    // it found.
    let result = Rules::from_parts(
        vec![
            entity_with(
                "a",
                vec![Trait::Health {
                    max: 1,
                    armour: "missing".into(),
                }],
            ),
            entity_with(
                "b",
                vec![
                    Trait::Vision {
                        range: redshift_data::value::Hundredths(500),
                    },
                    Trait::Armed {
                        weapon: "missing".into(),
                        turret: false,
                        turret_rate: 0,
                    },
                ],
            ),
        ],
        Vec::new(),
        minimal_armour(),
        Vec::new(),
    );
    match result {
        Err(RulesError::Invalid { problems }) => assert_eq!(problems.len(), 2, "{problems:?}"),
        other => panic!("expected two problems, got {other:?}"),
    }
}

#[test]
fn a_missing_rules_directory_says_so_plainly() {
    let result = Rules::load_from(Path::new("/nonexistent/rules/path"));
    match result {
        Err(e @ RulesError::MissingDirectory { .. }) => {
            assert!(e.to_string().contains("no rules directory"));
        }
        other => panic!("expected a missing-directory error, got {other:?}"),
    }
}

#[test]
fn a_new_country_needs_no_rust() {
    // The project's stated goal, exercised directly: a country is one data
    // entry. If this test ever needs a code change to keep passing, the goal
    // has slipped.
    let base = load();
    let entities: Vec<EntityDef> = base.entities().map(|(_, e)| e.clone()).collect();
    let weapons: Vec<WeaponDef> = ["rifle", "120mm", "artillery", "aa_gun"]
        .iter()
        .filter_map(|id| base.weapon(id).cloned())
        .collect();

    let mut factions: Vec<_> = base.factions().cloned().collect();
    let before = factions.len();
    factions.push(
        ron::from_str(
            r#"(
                id: "persia",
                name_key: "faction.persia",
                side: "allied",
                colour: (0, 150, 130),
                unique_units: [],
                modifiers: [
                    UnitSpeed(unit: "grizzly_tank", multiplier: 115),
                    UnitCost(unit: "harvester", multiplier: 90),
                ],
                voice_set: "persia",
            )"#,
        )
        .expect("a country should be plain data"),
    );

    let extended = Rules::from_parts(entities, weapons, base.armour().clone(), factions)
        .expect("the new country should validate");

    assert_eq!(extended.factions().count(), before + 1);
    let persia = extended.faction("persia").expect("persia should be loaded");
    assert_eq!(persia.side, "allied");
    assert_eq!(persia.modifiers.len(), 2);
    assert_ne!(
        base.hash(),
        extended.hash(),
        "adding a country must change the rules hash"
    );
}

#[test]
fn locomotors_survive_a_roundtrip() {
    for locomotor in [
        Locomotor::Foot,
        Locomotor::Wheeled,
        Locomotor::Tracked,
        Locomotor::Hover,
        Locomotor::Ship,
        Locomotor::Air,
    ] {
        let t = Trait::Mobile {
            speed: Hundredths(300),
            turn_rate: 90,
            locomotor,
            surfaces: None,
            size: None,
        };
        let text = ron::to_string(&t).unwrap();
        assert_eq!(ron::from_str::<Trait>(&text).unwrap(), t);
    }
}
