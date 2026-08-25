//! # redshift-ai
//!
//! Computer opponents.
//!
//! ## What this crate is allowed to do
//!
//! Read the simulation, and return commands. Nothing else. It holds a `&Sim`
//! and never a `&mut Sim`, which is the same rule the renderer obeys and for
//! the same reason: a command is the only way anything reaches the world, and
//! an opponent that reached in and moved a unit would be playing a different
//! game from the one the replay records.
//!
//! ## Determinism
//!
//! Every decision comes from simulation state and the tick counter. No
//! randomness, no wall clock, no iteration over a hash map. Two peers running
//! the same match must produce the same commands on the same ticks — an
//! opponent that did not would be a desync generator that looked like a
//! netcode bug.
//!
//! That constraint is worth stating plainly because it is tempting to break:
//! "pick a random spot to build" is the obvious way to write base layout, and
//! it is the wrong one. Every choice here is a deterministic function of what
//! is on the map.
//!
//! ## Difficulty
//!
//! See [`skill`]. The short version: no cheating. The hardest opponent sees the
//! same fog and pays the same prices as the player.

pub mod dummy;
pub mod skill;

pub use dummy::Commander;
pub use skill::Difficulty;
