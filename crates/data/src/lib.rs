//! # redshift-data
//!
//! Rules as data: units, structures, weapons, armour and factions, defined in
//! files rather than in code.
//!
//! ## The design goal
//!
//! **Adding a unit, a structure or a country must require no Rust.** That is a
//! stated project goal, and it is what makes new content an art-and-data task
//! instead of an engineering one. The trait system in [`traits`] is what buys
//! it: an entity is a *list of capabilities*, so a novel unit is a novel
//! combination rather than a new type.
//!
//! When something cannot be expressed as a combination of existing traits, the
//! honest answer is to add a trait — a small, testable piece of simulation
//! behaviour — not to special-case a unit.
//!
//! ## No floating point here either
//!
//! This crate is a dependency of `redshift-sim`, and its values feed the
//! simulation directly. A float among them would desync a match exactly as
//! surely as one written in the simulation itself, so fractional values are
//! written as integers and converted by exact integer arithmetic. See
//! [`value`].
//!
//! See `docs/05-data-and-modding.md`.

pub mod map;
pub mod rules;
pub mod traits;
pub mod value;

pub use rules::{
    ArmourTable, EntityDef, EntityKind, FactionDef, Modifier, Rules, RulesError, WeaponDef,
};
pub use traits::{Locomotor, Trait};
pub use value::{Hundredths, Percent, Ticks};
