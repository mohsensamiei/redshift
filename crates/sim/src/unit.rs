//! Units: what they are and how they move.
//!
//! Phase 0 keeps a unit deliberately thin — enough to select it, order it
//! somewhere, and watch it get there. Health, weapons, vision and the rest
//! arrive in Phase 3 as data-driven traits (see `docs/05-data-and-modding.md`).

use serde::{Deserialize, Serialize};

use crate::command::PlayerId;
use crate::fx::{Angle, Fx};
use crate::hash::{StateHash, StateHasher};
use crate::map::{Cell, Locomotor, WorldPos};

/// How close to a waypoint counts as having arrived.
///
/// A unit cannot land exactly on a point in fixed-point arithmetic, so without
/// a tolerance it would oscillate around every waypoint forever. An eighth of a
/// cell is tight enough to look precise and loose enough to always be reached.
pub const ARRIVAL_TOLERANCE: Fx = Fx::from_raw(65536 / 8);

/// How closely a unit must face its heading before it will drive forward.
///
/// Roughly 22.5°. Moving before turning looks like drifting; waiting for a
/// perfect alignment looks hesitant. This matches the original's feel of units
/// pivoting, then committing.
pub const MOVE_ALIGNMENT: u16 = 4096;

/// What a unit is currently trying to do.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Order {
    /// Nothing to do.
    #[default]
    Idle,
    /// Heading for `destination`, following `path`.
    Move {
        destination: Cell,
        /// Remaining waypoints. The next one is at `path[0]`.
        path: Vec<Cell>,
        /// Set when the route was partial, so the unit knows to ask again on
        /// arrival rather than assuming it is done.
        needs_repath: bool,
        /// Node budget for the next search. Raised on each retry so a unit in
        /// awkward terrain escalates instead of livelocking.
        retry_budget: u32,
    },
}

impl Order {
    pub fn is_idle(&self) -> bool {
        matches!(self, Order::Idle)
    }
}

/// A movable thing in the world.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Unit {
    pub owner: PlayerId,
    pub pos: WorldPos,
    pub facing: Angle,
    pub locomotor: Locomotor,
    /// Cells travelled per tick.
    pub speed: Fx,
    /// Binary-angle units turned per tick.
    pub turn_rate: u16,
    pub order: Order,
}

impl Unit {
    /// A placeholder unit, standing in until data-driven definitions arrive.
    pub fn new(owner: PlayerId, pos: WorldPos) -> Unit {
        Unit {
            owner,
            pos,
            facing: Angle::ZERO,
            locomotor: Locomotor::Tracked,
            // 0.15 cells per tick at 20 Hz is 3 cells per second.
            speed: Fx::from_frac(15, 100),
            // A full turn in about a second.
            turn_rate: 3000,
            order: Order::Idle,
        }
    }

    pub fn cell(&self) -> Cell {
        self.pos.cell()
    }
}

impl StateHash for Unit {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write(&self.owner);
        h.write(&self.pos);
        h.write_u16(self.facing.raw());
        h.write_u8(self.locomotor as u8);
        h.write_i32(self.speed.raw());
        h.write_u16(self.turn_rate);
        match &self.order {
            Order::Idle => h.write_u8(0),
            Order::Move {
                destination,
                path,
                needs_repath,
                retry_budget,
            } => {
                h.write_u8(1);
                h.write(destination);
                h.write_u32(path.len() as u32);
                for c in path {
                    h.write(c);
                }
                h.write_bool(*needs_repath);
                h.write_u32(*retry_budget);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_unit_is_idle_at_its_position() {
        let pos = Cell::new(4, 5).centre();
        let u = Unit::new(PlayerId(0), pos);
        assert!(u.order.is_idle());
        assert_eq!(u.cell(), Cell::new(4, 5));
        assert_eq!(u.pos, pos);
    }

    #[test]
    fn arrival_tolerance_is_reachable_in_one_step() {
        // If a unit could not cover the tolerance in a single tick it would
        // stop short of every waypoint.
        let u = Unit::new(PlayerId(0), WorldPos::ORIGIN);
        assert!(
            u.speed > ARRIVAL_TOLERANCE,
            "speed must exceed the arrival tolerance"
        );
    }

    #[test]
    fn hashing_covers_the_order() {
        fn hash(u: &Unit) -> u64 {
            let mut h = StateHasher::new();
            h.write(u);
            h.finish()
        }
        let mut a = Unit::new(PlayerId(0), WorldPos::ORIGIN);
        let base = hash(&a);
        a.order = Order::Move {
            destination: Cell::new(1, 1),
            path: vec![Cell::new(1, 1)],
            needs_repath: false,
            retry_budget: 100,
        };
        assert_ne!(
            hash(&a),
            base,
            "an order change must be visible in the hash"
        );
    }
}
