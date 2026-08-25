//! Loading, validating and hashing the rules.
//!
//! # Validation is the point
//!
//! Loading is easy; catching mistakes is the job. A unit naming a weapon that
//! does not exist must be an error at load, with the file and the name in the
//! message — not a crash three minutes into a match, and certainly not a
//! silently harmless unit that never fires.
//!
//! # The rules hash
//!
//! Every peer computes a hash of the loaded rules, and the lobby refuses a
//! client whose hash differs. Same build, edited unit stats, is just as fatal
//! to a lockstep match as a different binary — and far easier to end up with by
//! accident.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::traits::{Trait, UNIQUE_TRAITS};
use crate::value::Percent;

/// A stable index into the rules, resolved once at load.
///
/// The simulation works in indices rather than strings: comparing and hashing
/// integers is cheap, and iterating a `Vec` in index order is deterministic in
/// a way that iterating a map of names would not be.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct EntityKind(pub u16);

/// One buildable or spawnable thing — a unit or a structure. The distinction
/// is a matter of which traits it carries, not of type.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct EntityDef {
    pub id: String,
    /// Localisation key. No user-visible string is ever written here.
    pub name_key: String,
    /// Which side's roster this belongs to, if any.
    #[serde(default)]
    pub side: Option<String>,
    /// Grouping for production tabs and AI: "infantry", "vehicle", "structure".
    pub category: String,
    #[serde(default)]
    pub traits: Vec<Trait>,
}

impl EntityDef {
    /// The first trait matching a predicate, if any.
    pub fn find_trait<F: Fn(&Trait) -> bool>(&self, matches: F) -> Option<&Trait> {
        self.traits.iter().find(|t| matches(t))
    }

    pub fn has_trait(&self, name: &str) -> bool {
        self.traits.iter().any(|t| t.name() == name)
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct WeaponDef {
    pub id: String,
    pub damage: u32,
    pub warhead: String,
    /// Ticks between shots.
    pub reload: crate::value::Ticks,
    pub range: crate::value::Hundredths,
    /// Cells the damage spreads over. Zero for a single target.
    #[serde(default)]
    pub splash_radius: crate::value::Hundredths,
    /// Kills its target outright, whatever its health.
    ///
    /// Not the same as very high damage: a sniper kills any infantryman and
    /// does nothing at all to a tank, whereas a huge damage number would make
    /// it excellent against both.
    #[serde(default)]
    pub instant_kill: bool,
    /// Shots before it must rearm. Zero means unlimited.
    #[serde(default)]
    pub ammo: u32,
    /// Whether this weapon can shoot down projectiles in flight.
    #[serde(default)]
    pub intercepts: bool,
    /// Categories this may be used against. Empty means anything.
    ///
    /// The difference between a weapon and an *action*. Tanya shoots people
    /// with a pistol and destroys buildings with charges: two things with
    /// different valid targets, chosen by what she is pointed at rather than by
    /// a stance the player sets. A layer mask cannot say this — a building and
    /// an infantryman are both on the ground.
    #[serde(default)]
    pub target_categories: Vec<String>,
    /// Whether this takes the target's side rather than damaging it.
    ///
    /// Yuri, and the Psychic Beacon. Not damage of any amount: a mind-controlled
    /// tank arrives at full health with its veterancy intact, and the player who
    /// lost it has lost a tank rather than had one destroyed. It is also the
    /// only weapon whose effect is undone when the *attacker* dies.
    #[serde(default)]
    pub mind_control: bool,
    /// Whether this restores health instead of taking it away.
    ///
    /// A Medic, Yuri's repair drones, and the turret mode an engineer gives an
    /// IFV. Not a damage number with a minus sign: what changes is what counts
    /// as a *target*. A healing weapon looks for friends who are hurt, and a
    /// unit carrying one is useless against an enemy rather than mildly
    /// helpful to them.
    #[serde(default)]
    pub heals: bool,
    /// Which layers this weapon can engage.
    ///
    /// Empty means ground only, which is what almost every weapon wants and
    /// keeps ordinary rules files short. An anti-air gun lists `[Air]` and
    /// cannot touch a tank; something that lists both can do either.
    ///
    /// This is *targeting*, not damage. The armour table already decides how
    /// much a hit hurts; this decides whether the shot is taken at all. Both
    /// are needed: without this a tank acquires an aircraft and fires at it
    /// uselessly forever.
    #[serde(default)]
    pub targets: Vec<crate::traits::Layer>,
    /// Whether the shot follows its target once fired.
    ///
    /// A missile hits what it was aimed at; a shell flies to where the target
    /// was standing and misses if it moved. Ignored when the shot is instant.
    #[serde(default)]
    pub homing: bool,
    /// Projectile speed in cells per second. Zero means the shot lands at once.
    #[serde(default)]
    pub projectile_speed: crate::value::Hundredths,
}

/// Armour classes and the damage table between them.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct ArmourTable {
    pub classes: Vec<String>,
    /// `warhead -> armour class -> multiplier`.
    ///
    /// This lookup is what makes counterplay work. Keeping it as plain data
    /// means balance is edited rather than recompiled.
    pub table: BTreeMap<String, BTreeMap<String, Percent>>,
}

impl ArmourTable {
    /// The damage multiplier for a warhead against an armour class.
    ///
    /// An unlisted pairing is full damage. That is the forgiving default on
    /// purpose: a missing entry should mean "nothing special", not "immune",
    /// because immunity by omission is invisible until a match hinges on it.
    /// The armour classes, in declaration order.
    pub fn classes(&self) -> &[String] {
        &self.classes
    }

    /// The warheads, in a stable order.
    ///
    /// `BTreeMap` keys, so the order is the sorted one and identical on every
    /// machine. That matters because the simulation interns these to integer
    /// indices: a different order would give the same warhead a different index
    /// on two peers, and every damage lookup after that would disagree.
    pub fn warheads(&self) -> Vec<String> {
        self.table.keys().cloned().collect()
    }

    pub fn multiplier(&self, warhead: &str, armour: &str) -> Percent {
        self.table
            .get(warhead)
            .and_then(|row| row.get(armour))
            .copied()
            .unwrap_or(Percent::FULL)
    }
}

/// A country: a small overlay on a side's shared roster.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct FactionDef {
    pub id: String,
    pub name_key: String,
    /// Whose tech tree this country shares.
    pub side: String,
    pub colour: (u8, u8, u8),
    #[serde(default)]
    pub unique_units: Vec<String>,
    /// Units this country does not get, usually because a unique replaces one.
    #[serde(default)]
    pub removes_units: Vec<String>,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
    pub voice_set: String,
}

/// A country's passive advantage.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Modifier {
    UnitCost {
        unit: String,
        multiplier: Percent,
    },
    BuildSpeed {
        category: String,
        multiplier: Percent,
    },
    UnitSpeed {
        unit: String,
        multiplier: Percent,
    },
    UnitHealth {
        unit: String,
        multiplier: Percent,
    },
    WeaponRange {
        weapon: String,
        multiplier: Percent,
    },
}

impl Modifier {
    fn references(&self) -> (&'static str, String) {
        match self {
            Modifier::UnitCost { unit, .. }
            | Modifier::UnitSpeed { unit, .. }
            | Modifier::UnitHealth { unit, .. } => ("unit", unit.clone()),
            Modifier::BuildSpeed { category, .. } => ("category", category.clone()),
            Modifier::WeaponRange { weapon, .. } => ("weapon", weapon.clone()),
        }
    }
}

/// Everything loaded, resolved and checked.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Rules {
    entities: Vec<EntityDef>,
    entity_index: BTreeMap<String, EntityKind>,
    weapons: BTreeMap<String, WeaponDef>,
    armour: ArmourTable,
    factions: BTreeMap<String, FactionDef>,
}

impl Rules {
    pub fn entity(&self, kind: EntityKind) -> &EntityDef {
        &self.entities[kind.0 as usize]
    }

    pub fn entity_by_id(&self, id: &str) -> Option<(EntityKind, &EntityDef)> {
        let kind = *self.entity_index.get(id)?;
        Some((kind, &self.entities[kind.0 as usize]))
    }

    pub fn kind_of(&self, id: &str) -> Option<EntityKind> {
        self.entity_index.get(id).copied()
    }

    /// Every entity, in index order.
    pub fn entities(&self) -> impl Iterator<Item = (EntityKind, &EntityDef)> {
        self.entities
            .iter()
            .enumerate()
            .map(|(i, e)| (EntityKind(i as u16), e))
    }

    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    /// Every weapon, in id order.
    ///
    /// A `BTreeMap`, so the order is the ids' order rather than whatever the
    /// files happened to say — which matters because anything built from this
    /// feeds the rules hash, and two peers must agree.
    pub fn weapons(&self) -> impl Iterator<Item = &WeaponDef> {
        self.weapons.values()
    }

    pub fn weapon(&self, id: &str) -> Option<&WeaponDef> {
        self.weapons.get(id)
    }

    pub fn armour(&self) -> &ArmourTable {
        &self.armour
    }

    pub fn faction(&self, id: &str) -> Option<&FactionDef> {
        self.factions.get(id)
    }

    pub fn factions(&self) -> impl Iterator<Item = &FactionDef> {
        self.factions.values()
    }

    /// A stable hash of everything loaded.
    ///
    /// Exchanged at the handshake. FNV-1a over the canonical serialisation, for
    /// the same reason the simulation avoids `DefaultHasher`: the value must be
    /// identical across Rust versions, or peers on different toolchains would
    /// refuse each other over rules they actually agree on.
    /// How many weapons are defined.
    pub fn weapon_count(&self) -> usize {
        self.weapons.len()
    }

    /// How many countries are defined.
    pub fn faction_count(&self) -> usize {
        self.factions.len()
    }

    pub fn hash(&self) -> u64 {
        const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const PRIME: u64 = 0x0000_0100_0000_01b3;

        // `BTreeMap` and index order make the serialisation canonical, so the
        // same rules always produce the same bytes.
        let canonical = ron::to_string(self).unwrap_or_default();
        let mut hash = OFFSET;
        for byte in canonical.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(PRIME);
        }
        hash
    }

    /// Loads every rules file under a directory tree.
    pub fn load_from(root: &Path) -> Result<Rules, RulesError> {
        let mut builder = RulesBuilder::default();
        builder.load_tree(root)?;
        builder.finish()
    }

    /// Builds rules from in-memory definitions. Used by tests and tooling.
    pub fn from_parts(
        entities: Vec<EntityDef>,
        weapons: Vec<WeaponDef>,
        armour: ArmourTable,
        factions: Vec<FactionDef>,
    ) -> Result<Rules, RulesError> {
        RulesBuilder {
            entities,
            weapons,
            armour: Some(armour),
            factions,
            sources: BTreeMap::new(),
        }
        .finish()
    }
}

/// One file's worth of definitions.
///
/// Every field optional, so a file may contain just units, just weapons, or a
/// mixture — whatever grouping makes the data easiest to read.
#[derive(Debug, Default, Deserialize)]
struct RulesFile {
    #[serde(default)]
    entities: Vec<EntityDef>,
    #[serde(default)]
    weapons: Vec<WeaponDef>,
    #[serde(default)]
    armour: Option<ArmourTable>,
    #[serde(default)]
    factions: Vec<FactionDef>,
}

#[derive(Default)]
struct RulesBuilder {
    entities: Vec<EntityDef>,
    weapons: Vec<WeaponDef>,
    armour: Option<ArmourTable>,
    factions: Vec<FactionDef>,
    /// Where each id came from, so a duplicate names both files.
    sources: BTreeMap<String, PathBuf>,
}

impl RulesBuilder {
    fn load_tree(&mut self, root: &Path) -> Result<(), RulesError> {
        if !root.exists() {
            return Err(RulesError::MissingDirectory {
                path: root.to_path_buf(),
            });
        }
        let mut files = Vec::new();
        collect_ron_files(root, &mut files)?;
        // Sorted, so load order does not depend on the filesystem. Two machines
        // must produce the same rules hash from the same files.
        files.sort();

        for path in files {
            let text = std::fs::read_to_string(&path).map_err(|e| RulesError::Io {
                path: path.clone(),
                source: e.to_string(),
            })?;
            let file: RulesFile = ron_options()
                .from_str(&text)
                .map_err(|e| RulesError::Parse {
                    path: path.clone(),
                    message: e.to_string(),
                })?;

            for entity in file.entities {
                if let Some(first) = self.sources.get(&entity.id) {
                    return Err(RulesError::DuplicateId {
                        id: entity.id,
                        first: first.clone(),
                        second: path.clone(),
                    });
                }
                self.sources.insert(entity.id.clone(), path.clone());
                self.entities.push(entity);
            }
            self.weapons.extend(file.weapons);
            self.factions.extend(file.factions);
            if let Some(table) = file.armour {
                if self.armour.is_some() {
                    return Err(RulesError::DuplicateArmourTable { path });
                }
                self.armour = Some(table);
            }
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Rules, RulesError> {
        // Sorted by id so the index — and therefore the hash and every
        // simulation iteration — does not depend on file order.
        self.entities.sort_by(|a, b| a.id.cmp(&b.id));

        let mut entity_index = BTreeMap::new();
        for (i, entity) in self.entities.iter().enumerate() {
            if i > u16::MAX as usize {
                return Err(RulesError::TooManyEntities {
                    limit: u16::MAX as usize,
                });
            }
            entity_index.insert(entity.id.clone(), EntityKind(i as u16));
        }

        let mut weapons = BTreeMap::new();
        for weapon in self.weapons {
            if weapons.insert(weapon.id.clone(), weapon.clone()).is_some() {
                return Err(RulesError::DuplicateWeapon { id: weapon.id });
            }
        }

        let mut factions = BTreeMap::new();
        for faction in self.factions {
            if factions
                .insert(faction.id.clone(), faction.clone())
                .is_some()
            {
                return Err(RulesError::DuplicateFaction { id: faction.id });
            }
        }

        let rules = Rules {
            entities: self.entities,
            entity_index,
            weapons,
            armour: self.armour.unwrap_or_default(),
            factions,
        };
        rules.validate()?;
        Ok(rules)
    }
}

impl Rules {
    /// Checks every cross-reference resolves.
    fn validate(&self) -> Result<(), RulesError> {
        let mut problems = Vec::new();
        let armour_classes: Vec<&str> = self.armour.classes.iter().map(|s| s.as_str()).collect();

        for (_, entity) in self.entities() {
            // Something that can shoot but cannot see never fires a shot, and
            // does so silently — it simply sits there while everything around
            // it fights. Far better caught here than puzzled over in a match.
            let armed = entity
                .traits
                .iter()
                .any(|t| matches!(t, Trait::Armed { .. }));
            let sighted = entity
                .traits
                .iter()
                .any(|t| matches!(t, Trait::Vision { .. }));
            if armed && !sighted {
                problems.push(format!(
                    "{}: has a weapon but no Vision, so it can never acquire a target",
                    entity.id
                ));
            }

            let mut seen: BTreeMap<&str, u32> = BTreeMap::new();
            for t in &entity.traits {
                *seen.entry(t.name()).or_insert(0) += 1;

                for (what, name) in t.references() {
                    let exists = match what {
                        "armour" => armour_classes.contains(&name.as_str()),
                        "weapon" => self.weapons.contains_key(&name),
                        "warhead" => self.armour.table.contains_key(&name),
                        // Prerequisites, producers and transportables all name
                        // entities.
                        _ => self.entity_index.contains_key(&name),
                    };
                    if !exists {
                        problems.push(format!(
                            "{}: trait {} refers to {what} \"{name}\", which does not exist",
                            entity.id,
                            t.name()
                        ));
                    }
                }
            }
            for name in UNIQUE_TRAITS {
                if seen.get(name).copied().unwrap_or(0) > 1 {
                    problems.push(format!(
                        "{}: has more than one {name} trait; the simulation would silently use \
                         only one of them",
                        entity.id
                    ));
                }
            }
        }

        for weapon in self.weapons.values() {
            if !self.armour.table.contains_key(&weapon.warhead) {
                problems.push(format!(
                    "weapon {}: warhead \"{}\" has no row in the armour table, so it would do \
                     full damage to everything",
                    weapon.id, weapon.warhead
                ));
            }
        }

        for faction in self.factions.values() {
            for unit in faction.unique_units.iter().chain(&faction.removes_units) {
                if !self.entity_index.contains_key(unit) {
                    problems.push(format!(
                        "faction {}: refers to unit \"{unit}\", which does not exist",
                        faction.id
                    ));
                }
            }
            for modifier in &faction.modifiers {
                let (what, name) = modifier.references();
                let exists = match what {
                    "unit" => self.entity_index.contains_key(&name),
                    "weapon" => self.weapons.contains_key(&name),
                    // A category is free-form, so there is nothing to check
                    // against beyond it being non-empty.
                    _ => !name.is_empty(),
                };
                if !exists {
                    problems.push(format!(
                        "faction {}: modifier refers to {what} \"{name}\", which does not exist",
                        faction.id
                    ));
                }
            }
        }

        for row in self.armour.table.values() {
            for class in row.keys() {
                if !armour_classes.contains(&class.as_str()) {
                    problems.push(format!(
                        "armour table has a column for \"{class}\", which is not a declared class"
                    ));
                }
            }
        }

        if problems.is_empty() {
            Ok(())
        } else {
            Err(RulesError::Invalid { problems })
        }
    }
}

/// How rules files are parsed.
///
/// `IMPLICIT_SOME` lets an optional field be written as the value itself —
/// `side: "allied"` rather than `side: Some("allied")`. These files are edited
/// by hand far more often than they are read by a program, and the wrapper adds
/// nothing a person needs to see.
fn ron_options() -> ron::Options {
    ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
}

fn collect_ron_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), RulesError> {
    let entries = std::fs::read_dir(dir).map_err(|e| RulesError::Io {
        path: dir.to_path_buf(),
        source: e.to_string(),
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| RulesError::Io {
            path: dir.to_path_buf(),
            source: e.to_string(),
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_ron_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "ron") {
            out.push(path);
        }
    }
    Ok(())
}

#[derive(Debug)]
pub enum RulesError {
    MissingDirectory {
        path: PathBuf,
    },
    Io {
        path: PathBuf,
        source: String,
    },
    Parse {
        path: PathBuf,
        message: String,
    },
    DuplicateId {
        id: String,
        first: PathBuf,
        second: PathBuf,
    },
    DuplicateWeapon {
        id: String,
    },
    DuplicateFaction {
        id: String,
    },
    DuplicateArmourTable {
        path: PathBuf,
    },
    TooManyEntities {
        limit: usize,
    },
    Invalid {
        problems: Vec<String>,
    },
}

impl std::fmt::Display for RulesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RulesError::MissingDirectory { path } => {
                write!(f, "no rules directory at {}", path.display())
            }
            RulesError::Io { path, source } => {
                write!(f, "could not read {}: {source}", path.display())
            }
            RulesError::Parse { path, message } => {
                write!(f, "{} is malformed: {message}", path.display())
            }
            RulesError::DuplicateId { id, first, second } => write!(
                f,
                "\"{id}\" is defined twice, in {} and {}",
                first.display(),
                second.display()
            ),
            RulesError::DuplicateWeapon { id } => write!(f, "weapon \"{id}\" is defined twice"),
            RulesError::DuplicateFaction { id } => write!(f, "faction \"{id}\" is defined twice"),
            RulesError::DuplicateArmourTable { path } => write!(
                f,
                "{} declares a second armour table; there can only be one",
                path.display()
            ),
            RulesError::TooManyEntities { limit } => {
                write!(
                    f,
                    "more than {limit} entities, which the index cannot address"
                )
            }
            RulesError::Invalid { problems } => {
                writeln!(f, "the rules have {} problem(s):", problems.len())?;
                for problem in problems {
                    writeln!(f, "  - {problem}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for RulesError {}
