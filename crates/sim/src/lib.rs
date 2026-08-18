//! # redshift-sim
//!
//! The deterministic game simulation — the heart of the project.
//!
//! ## The contract
//!
//! > Given the same initial state and the same command sequence, every peer
//! > must produce a **bit-identical** world state, on every platform, forever.
//!
//! Multiplayer is deterministic lockstep: peers exchange only player commands,
//! never unit positions, and each simulates the whole match locally. If two
//! peers ever diverge, the match desyncs — and it will surface minutes after
//! the actual divergence, which is why the rules below are strict.
//!
//! ## Rules for code in this crate
//!
//! - **No floating point.** Use [`fx::Fx`]. Enforced by lint.
//! - **No engine dependency.** This crate must build and run headless.
//!   `cargo tree -p redshift-sim | grep -i bevy` must print nothing.
//! - **No `HashMap`/`HashSet` iteration.** Their order is randomised per
//!   process. Use [`arena::Arena`], `BTreeMap`, or `Vec`.
//! - **No wall-clock time.** The tick counter is the only clock.
//! - **No threads.** Single-threaded by design.
//! - **One RNG** ([`rng::SimRng`]), living in simulation state.
//! - **Budget expensive work in units of work, never milliseconds.** A
//!   time-based cutoff is the most common cause of desyncs in RTS codebases.
//!
//! See `docs/02-simulation.md` and `docs/adr/0003-deterministic-lockstep.md`.

pub mod arena;
pub mod combat;
pub mod command;
pub mod economy;
pub mod fx;
pub mod hash;
pub mod map;
pub mod path;
pub mod rng;
pub mod sim;
pub mod stats;
pub mod unit;

mod trig_table;

pub use arena::{Arena, EntityId};
pub use combat::{ArmourId, CombatState, CombatTable, DamageTable, WarheadId, WeaponStats};
pub use command::{Command, CommandKind, PlayerId};
pub use economy::Treasury;
pub use fx::{Angle, Fx, FxWide};
pub use hash::{StateHash, StateHasher};
pub use map::{Cell, Locomotor, Map, Terrain, WorldPos};
pub use rng::SimRng;
pub use stats::{StatTable, UnitStats};

// Re-exported so the renderer and other consumers can read rules without
// depending on redshift-data directly. The data crate is an implementation
// detail of what the simulation is made from, not a second public interface.
pub use redshift_data::rules::{EntityDef, EntityKind, Rules};
pub use redshift_data::traits::Trait;
pub use sim::{MatchSetup, Sim, WorldView};
pub use unit::{Order, Unit};

/// Simulation ticks per second.
///
/// Deliberately coarse. Low enough to keep lockstep bandwidth small and
/// latency tolerance comfortable; high enough that this genre feels
/// responsive. The renderer runs at 60 Hz and interpolates between ticks, so
/// this rate is not visible as choppy motion.
///
/// Changing this changes every speed and rate in the game, and invalidates
/// every recorded replay.
pub const TICKS_PER_SECOND: u32 = 20;

/// Duration of one tick in milliseconds.
pub const TICK_MS: u32 = 1000 / TICKS_PER_SECOND;

/// A tick number. Wraps after roughly six years of continuous play.
pub type Tick = u32;
