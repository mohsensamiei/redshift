//! Entity traits: what a thing *can do*, listed rather than inherited.
//!
//! # Composition, not inheritance
//!
//! A unit is not a subclass of a base unit. It is a list of traits, each a
//! small independent piece of behaviour. A novel unit is a novel *combination*
//! of existing traits and needs no new code; only a genuinely new capability
//! does.
//!
//! That boundary is the whole design. The project's stated goal — adding a
//! country should be a data and art task — holds exactly as long as this list
//! covers the vocabulary the original game used. Every time a unit cannot be
//! expressed here, the goal has slipped a little.
//!
//! See `docs/05-data-and-modding.md`.

use serde::{Deserialize, Serialize};

use crate::value::{Hundredths, Percent, Ticks};

/// Which layer a thing occupies for the purpose of being shot at.
///
/// Separate from its surfaces, because they answer different questions.
/// Surfaces are about where a unit may *go*; the layer is about what can
/// *reach* it. A hovercraft crosses water and is still shot at like anything
/// else on the ground.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize,
)]
pub enum Layer {
    #[default]
    Ground,
    Air,
}

/// A surface a unit may occupy.
///
/// Declared per unit rather than inferred from its locomotor. See
/// `docs/adr/0006-capability-is-data-not-category.md`: the original is full of
/// units that break their category's rule, and each of those must be a line of
/// data rather than an arm in a match.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum Surface {
    /// Open, level ground.
    Land,
    /// Water deep enough to float on.
    Water,
    /// High ground. Only flight crosses it.
    Height,
}

/// How a thing traverses terrain. Mirrors the original's "locomotor" concept.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Locomotor {
    #[default]
    Foot,
    Wheeled,
    Tracked,
    Hover,
    Ship,
    Air,
}

/// What a weapon's damage is like, for the armour table to interpret.
pub type WarheadId = String;
/// Which armour class a thing has.
pub type ArmourId = String;

/// One capability.
///
/// Deliberately flat. A nested trait hierarchy would reintroduce the
/// inheritance this design exists to avoid.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Trait {
    /// Can be damaged and destroyed.
    Health { max: u32, armour: ArmourId },

    /// Can move under its own power.
    Mobile {
        /// Cells per second.
        speed: Hundredths,
        /// Degrees per second.
        turn_rate: u32,
        /// Movement style: what it looks like, whether it crushes, how it
        /// turns. Also supplies the default surfaces when none are given.
        locomotor: Locomotor,
        /// Surfaces this unit may cross.
        ///
        /// Omitted means "whatever this locomotor usually crosses", which keeps
        /// ordinary units short. Stated means exactly this list, which is how
        /// an amphibious rifleman or a hovercraft is expressed without touching
        /// the engine.
        #[serde(default)]
        surfaces: Option<Vec<Surface>>,
        /// Physical radius in hundredths of a cell. Defaults from the
        /// locomotor.
        #[serde(default)]
        size: Option<Hundredths>,
        /// Which layer this occupies for targeting. Defaults from the
        /// locomotor: flying things are in the air, everything else is not.
        #[serde(default)]
        layer: Option<Layer>,
    },

    /// Carries a weapon.
    Armed {
        weapon: String,
        /// Whether the weapon is on a turret that can aim independently of the
        /// hull. Without one, the whole body must face the target — which is
        /// most of what makes a tank feel like a tank.
        turret: bool,
        /// Degrees per second the turret traverses. Ignored without a turret.
        turret_rate: u32,
    },

    /// Reveals the map around it.
    Vision { range: Hundredths },

    /// Reveals cloaked things within its vision.
    Detector,

    /// Invisible to anything without [`Trait::Detector`].
    Cloakable {
        /// Ticks after firing before cloak returns.
        recloak_delay: Ticks,
    },

    /// Drives over lighter things, destroying them.
    Crushes { classes: Vec<String> },

    /// Cannot be crushed, and is a member of these classes for crushing rules.
    Crushable { class: String },

    /// Occupies more than one cell. Buildings, mostly.
    Footprint { width: u8, height: u8 },

    /// Can be built, and how.
    Buildable {
        cost: u32,
        build_time: Ticks,
        /// Everything that must exist first.
        prerequisites: Vec<String>,
        /// Which production building makes it.
        produced_by: String,
    },

    /// Produces other things.
    Produces { categories: Vec<String> },

    /// Supplies power to the grid.
    PowerSupply { output: u32 },

    /// Draws power from the grid. Low power slows production and disables some
    /// structures, as in the original.
    PowerDraw { amount: u32 },

    /// Gathers resources.
    Harvester {
        capacity: u32,
        /// Units of ore gathered per tick.
        gather_rate: Hundredths,
    },

    /// Accepts harvesters and converts their load to credits.
    Refinery {
        /// Credits per unit of ore.
        value_per_unit: Percent,
    },

    /// Repairs itself over time.
    SelfHealing {
        per_tick: Hundredths,
        delay_after_damage: Ticks,
    },

    /// Damages its surroundings when destroyed.
    Explodes { warhead: WarheadId, damage: u32 },

    /// Can be captured by an engineer-like unit.
    Capturable,

    /// Carries other units.
    Transport { capacity: u8, allowed: Vec<String> },

    /// Gains rank with kills.
    Veterancy {
        kills_for_veteran: u32,
        kills_for_elite: u32,
    },

    /// Selectable by the player. Priority breaks ties when units overlap —
    /// clicking a crowd should pick the thing the player meant.
    Selectable { priority: u8 },
}

impl Trait {
    /// A short name, for error messages and tooling.
    pub fn name(&self) -> &'static str {
        match self {
            Trait::Health { .. } => "Health",
            Trait::Mobile { .. } => "Mobile",
            Trait::Armed { .. } => "Armed",
            Trait::Vision { .. } => "Vision",
            Trait::Detector => "Detector",
            Trait::Cloakable { .. } => "Cloakable",
            Trait::Crushes { .. } => "Crushes",
            Trait::Crushable { .. } => "Crushable",
            Trait::Footprint { .. } => "Footprint",
            Trait::Buildable { .. } => "Buildable",
            Trait::Produces { .. } => "Produces",
            Trait::PowerSupply { .. } => "PowerSupply",
            Trait::PowerDraw { .. } => "PowerDraw",
            Trait::Harvester { .. } => "Harvester",
            Trait::Refinery { .. } => "Refinery",
            Trait::SelfHealing { .. } => "SelfHealing",
            Trait::Explodes { .. } => "Explodes",
            Trait::Capturable => "Capturable",
            Trait::Transport { .. } => "Transport",
            Trait::Veterancy { .. } => "Veterancy",
            Trait::Selectable { .. } => "Selectable",
        }
    }

    /// Names of other definitions this trait refers to, so loading can check
    /// they exist rather than discovering it mid-match.
    pub fn references(&self) -> Vec<(&'static str, String)> {
        match self {
            Trait::Health { armour, .. } => vec![("armour", armour.clone())],
            Trait::Armed { weapon, .. } => vec![("weapon", weapon.clone())],
            Trait::Buildable {
                prerequisites,
                produced_by,
                ..
            } => {
                let mut out: Vec<(&'static str, String)> = prerequisites
                    .iter()
                    .map(|p| ("prerequisite", p.clone()))
                    .collect();
                out.push(("producer", produced_by.clone()));
                out
            }
            Trait::Explodes { warhead, .. } => vec![("warhead", warhead.clone())],
            Trait::Transport { allowed, .. } => allowed
                .iter()
                .map(|a| ("transportable", a.clone()))
                .collect(),
            _ => Vec::new(),
        }
    }
}

impl Locomotor {
    /// The surfaces a unit of this style crosses when it does not say.
    ///
    /// A *default*, not a rule: any unit may override it. That distinction is
    /// the whole of ADR 0006 — a category-based default is usually right, which
    /// is exactly what makes it dangerous to bake in.
    pub fn default_surfaces(self) -> &'static [Surface] {
        match self {
            Locomotor::Foot | Locomotor::Wheeled | Locomotor::Tracked => &[Surface::Land],
            Locomotor::Hover => &[Surface::Land, Surface::Water],
            Locomotor::Ship => &[Surface::Water],
            Locomotor::Air => &[Surface::Land, Surface::Water, Surface::Height],
        }
    }

    /// The radius a unit of this style takes up, in hundredths of a cell, when
    /// it does not say.
    /// The layer a unit of this style occupies when it does not say.
    pub fn default_layer(self) -> Layer {
        match self {
            Locomotor::Air => Layer::Air,
            _ => Layer::Ground,
        }
    }

    pub fn default_size(self) -> Hundredths {
        match self {
            Locomotor::Foot => Hundredths(16),
            Locomotor::Wheeled | Locomotor::Tracked => Hundredths(39),
            Locomotor::Hover => Hundredths(35),
            Locomotor::Air => Hundredths(35),
            Locomotor::Ship => Hundredths(60),
        }
    }
}

/// Trait names that may appear at most once on an entity.
///
/// Two `Health` traits is a data error, not an interesting combination, and the
/// simulation would silently use whichever it found first.
pub const UNIQUE_TRAITS: &[&str] = &[
    "Health",
    "Mobile",
    "Vision",
    "Footprint",
    "Buildable",
    "PowerSupply",
    "PowerDraw",
    "Harvester",
    "Refinery",
    "SelfHealing",
    "Transport",
    "Veterancy",
    "Selectable",
    "Cloakable",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_trait_names_itself() {
        // Guards the match arm being forgotten when a trait is added, which
        // would otherwise surface as a misleading error message.
        let samples = vec![
            Trait::Health {
                max: 1,
                armour: "light".into(),
            },
            Trait::Detector,
            Trait::Capturable,
            Trait::Selectable { priority: 1 },
        ];
        for t in samples {
            assert!(!t.name().is_empty());
            assert!(t.name().chars().next().unwrap().is_uppercase());
        }
    }

    #[test]
    fn references_are_reported_for_validation() {
        let health = Trait::Health {
            max: 100,
            armour: "heavy".into(),
        };
        assert_eq!(health.references(), vec![("armour", "heavy".to_string())]);

        let buildable = Trait::Buildable {
            cost: 900,
            build_time: Ticks(45),
            prerequisites: vec!["war_factory".into(), "radar".into()],
            produced_by: "war_factory".into(),
        };
        let refs = buildable.references();
        assert_eq!(refs.len(), 3, "two prerequisites and a producer");
        assert!(refs.contains(&("producer", "war_factory".to_string())));
    }

    #[test]
    fn traits_without_references_report_none() {
        assert!(Trait::Detector.references().is_empty());
        assert!(
            Trait::Mobile {
                speed: Hundredths(450),
                turn_rate: 90,
                locomotor: Locomotor::Tracked,
                surfaces: None,
                size: None,
                layer: None,
            }
            .references()
            .is_empty()
        );
    }

    #[test]
    fn a_unit_is_expressible_as_a_list() {
        // The design claim, exercised: a heavy tank is a combination of
        // existing traits, with no type of its own.
        let tank = [
            Trait::Health {
                max: 400,
                armour: "heavy".into(),
            },
            Trait::Mobile {
                speed: Hundredths(450),
                turn_rate: 90,
                locomotor: Locomotor::Tracked,
                surfaces: None,
                size: None,
                layer: None,
            },
            Trait::Armed {
                weapon: "120mm".into(),
                turret: true,
                turret_rate: 120,
            },
            Trait::Vision {
                range: Hundredths(600),
            },
            Trait::Crushes {
                classes: vec!["infantry".into()],
            },
            Trait::Selectable { priority: 2 },
        ];
        assert_eq!(tank.len(), 6);
        assert!(tank.iter().any(|t| t.name() == "Armed"));
    }

    #[test]
    fn traits_roundtrip_through_ron() {
        // Rules are hand-edited, so the serialised shape has to be something a
        // person would willingly type.
        let t = Trait::Mobile {
            speed: Hundredths(450),
            turn_rate: 90,
            locomotor: Locomotor::Tracked,
            surfaces: None,
            size: None,
            layer: None,
        };
        let text = ron::to_string(&t).unwrap();
        assert!(
            text.contains("Mobile"),
            "the variant name should be visible: {text}"
        );
        assert_eq!(ron::from_str::<Trait>(&text).unwrap(), t);
    }
}
