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

/// Serde default for flags that are true unless stated otherwise.
pub(crate) fn yes() -> bool {
    true
}

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

/// A standing effect on a player.
///
/// Usually lasting while its source stands — an ore purifier, a captured
/// machine shop. Infiltration grants some of these permanently instead, since
/// the spy is consumed and the building it entered stays the victim's.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub enum PlayerEffect {
    /// Every load of ore delivered is worth this percentage of its usual value.
    OreValue(Percent),
    /// Everything this player builds *of this category* arrives one rank
    /// higher.
    ///
    /// Keyed on a category because the original keys it on one: a spy in a
    /// barracks promotes your infantry, and a spy in a war factory promotes
    /// your vehicles and aircraft. A single flag would make either spy do both,
    /// which is a much better deal than the game offers.
    VeteranProduction(String),
    /// Every vehicle this player owns repairs itself, wherever it is.
    RepairEverywhere,
}

/// What infiltrating one building yields.
///
/// Five rows, and they are genuinely five different mechanisms rather than one
/// with a parameter — which is the finding that made this worth writing down.
/// A single "infiltration effect" number would have hidden that.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub enum InfiltrationEffect {
    /// Everything of a category the infiltrator builds from now on arrives
    /// promoted. A barracks gives infantry, a war factory vehicles.
    ///
    /// Persistent, and does not stack: this is a production modifier that lasts
    /// the rest of the match, not an event.
    Promotes { category: String },
    /// The victim loses power for a while, however many plants they own.
    ///
    /// The only row with a duration, which is why the timer lives on the
    /// player rather than on the building.
    Blackout { ticks: u32 },
    /// The infiltrator takes this percentage of the victim's funds.
    ///
    /// The only row that is a one-off with no lasting state at all.
    StealsFunds { percent: u32 },
    /// The infiltrator may now build this, whatever their own tech tree says.
    ///
    /// Keyed on the building, which is what makes an Allied lab yield a Chrono
    /// Commando and a Soviet one a Chrono Ivan. The unit is unlocked for
    /// whoever sent the spy — so a Soviet player who spies an Allied lab builds
    /// an Allied commando, which is exactly the point of doing it.
    Unlocks { unit: String },
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

    /// Carries an additional weapon.
    ///
    /// A second `Armed` would be a data error, since a unit has one primary
    /// weapon and the code needs to know which. This is the other one: an
    /// Apocalypse fires a cannon at the ground and missiles at the air, and it
    /// needs both at once rather than choosing.
    Secondary {
        weapon: String,
        turret: bool,
        turret_rate: u32,
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

    /// At most this many may exist at once, per player.
    ///
    /// A commando is unique; a superweapon is one per base. Distinct from a
    /// prerequisite, which asks what you *have* rather than how many.
    BuildLimit { max: u8 },

    /// Comes with these units when it is built.
    ///
    /// A refinery arrives with a miner, and that is not a nicety — it is why a
    /// refinery is the first thing built, and an economy balanced without it
    /// would be wrong from the start.
    Delivers { units: Vec<String> },

    /// Pays whoever destroys it.
    ///
    /// Small, and not only flavour: it is why shooting a civilian is a decision
    /// with an upside rather than only spite. Any unit may carry a payout.
    Bounty { credits: u32 },

    /// Grants a standing effect to its owner while it stands.
    ///
    /// A shape the original uses repeatedly and that has no other home: an ore
    /// purifier makes every load worth more, a machine shop repairs every
    /// vehicle anywhere, a spy in a barracks promotes everything built
    /// afterwards. None of those are events, and none belong to a unit — they
    /// are modifiers on a *player*.
    Grants { effect: PlayerEffect },

    /// Supplies power to the grid.
    PowerSupply { output: u32 },

    /// Draws power from the grid.
    ///
    /// A structure short of power **stops working**, which is the original's
    /// rule and not the same as working slowly. A radar goes dark; an
    /// anti-air gun holds its fire. That is most of what makes attacking a
    /// power plant worth doing.
    PowerDraw {
        amount: u32,
        /// Whether this keeps working in a shortage.
        ///
        /// A few structures do — a refinery still refines, a wall still
        /// blocks — and saying so per structure is what stops "low power" from
        /// meaning "the base is destroyed".
        #[serde(default)]
        works_unpowered: bool,
    },

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

    /// Can be captured by walking an engineer into it.
    Capturable,

    /// Can walk into a building to capture or repair it.
    ///
    /// One trait rather than two, because the original made it one action: the
    /// engineer enters, and what happens depends on whose building it was. A
    /// player never chooses between "capture" and "repair" — they choose a
    /// building.
    Engineer {
        /// Whether entering destroys the unit. It did in the original, which is
        /// what makes an engineer a considered purchase rather than a tool.
        #[serde(default = "crate::traits::yes")]
        consumed: bool,
    },

    /// Carries other units.
    Transport { capacity: u8, allowed: Vec<String> },

    /// Gains rank with kills.
    Veterancy {
        kills_for_veteran: u32,
        kills_for_elite: u32,
    },

    /// Hides the ground around it from everyone else.
    ///
    /// The Gap Generator, and the only thing in the game that *subtracts* from
    /// what a player can see. It reveals nothing for its owner — a structure
    /// whose entire effect is on the other side of the table.
    ///
    /// Note that it does not hide units, it hides *ground*. An enemy who walks
    /// something into the area sees around it perfectly well while it is there,
    /// which is what makes scouting the answer rather than a counter-structure.
    HidesGround { radius: Hundredths },

    /// Carries ground units over water while it stands.
    ///
    /// An entity rather than a kind of terrain, because a bridge is something
    /// you shoot: Crazy Ivan blowing one up is the usual way a player uses it,
    /// and that wants the ordinary damage path rather than a second one for
    /// terrain. Its [`Trait::Footprint`] is the span.
    ///
    /// The one thing here that is destroyed without being removed. A wrecked
    /// bridge is still visibly there and can be rebuilt, so it stays in the
    /// world at zero health rather than vanishing — which is also what makes
    /// the repair hut possible at all, since there is something left to repair.
    ///
    /// Unlike every other footprint, a bridge's cells are *opened* rather than
    /// blocked.
    Bridge,

    /// Rebuilds wrecked bridges near it when an engineer walks in.
    ///
    /// Researched, and the correction worth recording: bridges are repaired
    /// through a **hut beside them**, not by touching the bridge. That makes
    /// bridge repair the same act as capturing a tech building rather than a
    /// new mechanic — which is why there is no bridge-repair command.
    RepairsBridges {
        /// How far from the hut a wreck can be and still be served, in cells.
        /// A hut serves the bridge it was built for, and proximity is how the
        /// original expresses that.
        radius: u8,
    },

    /// Poisons the ground around it while it stands there.
    ///
    /// The deployed Desolator, and — later — anything else that leaves a place
    /// dangerous rather than damaging a thing. The distinction matters: this
    /// does not target, does not need line of sight, and keeps working after
    /// whatever laid it is gone.
    ///
    /// Note what is *not* here. There is no "immune to radiation" flag, because
    /// the armour table already answers it: give the warhead a zero against
    /// vehicle armour and infantry die on ground a tank drives across. A second
    /// mechanism for the same question would be one refactor away from
    /// disagreeing with the first.
    Contaminates {
        /// How far the contamination reaches, in hundredths of a cell.
        radius: Hundredths,
        /// Damage dealt each tick to whatever stands in it.
        damage: u32,
        /// The warhead that damage uses, so armour decides who cares.
        warhead: WarheadId,
        /// How long ground stays dangerous after it stops being poisoned.
        ///
        /// The reason this is worth doing at all: a Desolator that could be
        /// killed to make the ground safe again immediately would be a slow gun
        /// rather than an area denied.
        lingers: Ticks,
    },

    /// What a spy gets for reaching this building.
    ///
    /// Declared on the **building**, not on the spy. Infiltration is not one
    /// effect aimed at a target — it is a table keyed on what was entered, and
    /// the table's rows belong with the things they describe. A new building
    /// with a new infiltration effect is then a rules file, which is the whole
    /// of ADR 0006.
    Infiltrated { effect: InfiltrationEffect },

    /// Can walk into an enemy building for whatever that building yields.
    ///
    /// Distinct from [`Trait::Engineer`], which takes a building rather than
    /// robbing it, and which works on neutral and friendly ones too. A spy that
    /// reached a building with no [`Trait::Infiltrated`] has wasted itself,
    /// exactly as in the original.
    Infiltrator {
        #[serde(default = "crate::traits::yes")]
        consumed: bool,
    },

    /// Can be occupied by infantry, who fight from inside it.
    ///
    /// Researched, and more specific than "infantry can garrison buildings".
    /// Four rules, and three of them are easy to get wrong:
    ///
    /// - The building fires with **its own** predetermined weapon, not the
    ///   weapon of whoever is inside. This is the exact opposite of how an IFV
    ///   works, and the two are easy to confuse.
    /// - Only **basic** infantry may enter — a GI or a Conscript, not a
    ///   commando. Hence a category list rather than "anything on foot".
    /// - Capacity is a property of the **building**, since it follows from the
    ///   building's size.
    /// - The garrison is **forced out below a third health**, rather than dying
    ///   with the building. A garrisoned building is therefore not a death
    ///   trap, and clearing one means damaging it enough to evict rather than
    ///   destroying it.
    ///
    /// Only a neutral building can be occupied, and an emptied one goes back to
    /// being neutral. That is both what the original does — these are the
    /// civilian buildings scattered across a map — and what saves this from
    /// needing to remember who owned the building first.
    Garrisonable {
        /// How many may be inside at once.
        capacity: u8,
        /// Categories that may enter.
        categories: Vec<String>,
        /// The weapon the building fires while occupied. It has none while
        /// empty.
        weapon: String,
        /// The fraction of full health below which the garrison is thrown out,
        /// as a percentage.
        #[serde(default = "crate::traits::a_third")]
        evict_below_percent: u32,
    },

    /// Burrows into an enemy unit and takes it apart from inside.
    ///
    /// The Terror Drone. It is not a weapon with a strange effect — the drone
    /// stops being on the battlefield and starts being *in* something, which is
    /// why nothing can shoot it and why the counter is a building rather than a
    /// gun. See [`Trait::Repairs`] and its `cures_infestation`.
    ///
    /// A drone keeps whatever [`Trait::Armed`] it has for everything outside
    /// `categories`, which is how the original's behaves like an attack dog
    /// against infantry while being something else entirely against a tank.
    Infests {
        /// Categories it can get inside. Everything else it just shoots.
        categories: Vec<String>,
        /// Damage dealt to the host each tick, through the usual warhead and
        /// armour table — so armour still means something, and the kill is
        /// credited like any other.
        damage: u32,
        /// The warhead that damage is dealt with.
        warhead: WarheadId,
    },

    /// Repairs friendly units that are sent into it.
    ///
    /// The Service Depot, the Naval Shipyard, and Yuri's Outpost. All three
    /// are the same structure with a different list of what they will service,
    /// which is why this is a trait and not three special cases.
    ///
    /// Not the same thing as [`PlayerEffect::RepairEverywhere`]: that heals a
    /// player's vehicles wherever they happen to be, at no cost and with no
    /// decision attached. This one asks a player to pull a damaged unit out of
    /// the fight and pay for it, which is a trade rather than a gift.
    Repairs {
        /// Categories it will service. `["vehicle"]` for a Service Depot,
        /// `["ship"]` for a Naval Shipyard.
        categories: Vec<String>,
        /// Health restored per tick.
        ///
        /// Whole points rather than hundredths, unlike most rates here. A
        /// fractional rate would need somewhere to keep the remainder between
        /// ticks, and a per-unit accumulator is a lot of state to carry for a
        /// precision nothing needs: at twenty ticks a second, one point per
        /// tick is already twenty a second.
        rate: u32,
        /// What a repair from nothing to full costs, as a percentage of the
        /// unit's build cost. Charged in proportion to the damage actually
        /// undone, so half a repair costs half as much.
        ///
        /// Zero means free.
        #[serde(default)]
        cost_percent: u32,
        /// Whether arriving here removes anything that has burrowed into the
        /// unit.
        ///
        /// The Service Depot's second job, and the counter that makes a Terror
        /// Drone a problem to be solved rather than a death sentence.
        #[serde(default)]
        cures_infestation: bool,
    },

    /// Becomes something else on command.
    ///
    /// One trait covers both halves of what the original calls deploying,
    /// because they are the same act seen from two distances. An MCV becomes a
    /// Construction Yard — plainly a different thing. A GI "changes stance"
    /// into a static emplacement with a better gun — and *that* is also a
    /// different thing, once you accept that a unit which cannot move and
    /// shoots differently is not the same unit with a flag set.
    ///
    /// Modelling the deployed form as its own entity is what makes this cost
    /// one mechanism instead of two. The deployed form is an ordinary entry in
    /// the rules: it simply has no [`Trait::Mobile`], a stronger
    /// [`Trait::Armed`], and its own `Deploys` pointing back at the mobile
    /// form. Undeploying is deploying in the other direction.
    ///
    /// It also means a stance is expressible entirely in data, which is the
    /// whole of ADR 0006. Nothing in the simulation knows what an MCV is.
    Deploys {
        /// The entity id to become.
        into: String,
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
            Trait::Secondary { .. } => "Secondary",
            Trait::Vision { .. } => "Vision",
            Trait::Detector => "Detector",
            Trait::Cloakable { .. } => "Cloakable",
            Trait::Crushes { .. } => "Crushes",
            Trait::Crushable { .. } => "Crushable",
            Trait::Footprint { .. } => "Footprint",
            Trait::Buildable { .. } => "Buildable",
            Trait::Produces { .. } => "Produces",
            Trait::BuildLimit { .. } => "BuildLimit",
            Trait::Delivers { .. } => "Delivers",
            Trait::Bounty { .. } => "Bounty",
            Trait::Grants { .. } => "Grants",
            Trait::PowerSupply { .. } => "PowerSupply",
            Trait::PowerDraw { .. } => "PowerDraw",
            Trait::Harvester { .. } => "Harvester",
            Trait::Refinery { .. } => "Refinery",
            Trait::SelfHealing { .. } => "SelfHealing",
            Trait::Explodes { .. } => "Explodes",
            Trait::Capturable => "Capturable",
            Trait::Engineer { .. } => "Engineer",
            Trait::Transport { .. } => "Transport",
            Trait::Veterancy { .. } => "Veterancy",
            Trait::Deploys { .. } => "Deploys",
            Trait::Repairs { .. } => "Repairs",
            Trait::Infests { .. } => "Infests",
            Trait::Garrisonable { .. } => "Garrisonable",
            Trait::HidesGround { .. } => "HidesGround",
            Trait::Bridge => "Bridge",
            Trait::RepairsBridges { .. } => "RepairsBridges",
            Trait::Contaminates { .. } => "Contaminates",
            Trait::Infiltrated { .. } => "Infiltrated",
            Trait::Infiltrator { .. } => "Infiltrator",
            Trait::Selectable { .. } => "Selectable",
        }
    }

    /// Names of other definitions this trait refers to, so loading can check
    /// they exist rather than discovering it mid-match.
    pub fn references(&self) -> Vec<(&'static str, String)> {
        match self {
            Trait::Health { armour, .. } => vec![("armour", armour.clone())],
            Trait::Armed { weapon, .. } | Trait::Secondary { weapon, .. } => {
                vec![("weapon", weapon.clone())]
            }
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
            Trait::Delivers { units } => units.iter().map(|u| ("delivered", u.clone())).collect(),
            Trait::Transport { allowed, .. } => allowed
                .iter()
                .map(|a| ("transportable", a.clone()))
                .collect(),
            Trait::Deploys { into } => vec![("deployed form", into.clone())],
            Trait::Infests { warhead, .. } | Trait::Contaminates { warhead, .. } => {
                vec![("warhead", warhead.clone())]
            }
            Trait::Garrisonable { weapon, .. } => vec![("weapon", weapon.clone())],
            Trait::Infiltrated {
                effect: InfiltrationEffect::Unlocks { unit },
            } => vec![("unlocked unit", unit.clone())],
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
/// The researched eviction threshold: a third of full health.
fn a_third() -> u32 {
    33
}

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
    "Repairs",
    "Infests",
    "Garrisonable",
    "HidesGround",
    "Bridge",
    "RepairsBridges",
    "Contaminates",
    "Infiltrated",
    "Infiltrator",
    // A unit with two deployed forms would silently pick one, and which one
    // depends on trait order in a RON file — a desync in waiting.
    "Deploys",
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
