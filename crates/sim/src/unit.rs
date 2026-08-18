//! Units: what they are and how they move.
//!
//! Phase 0 keeps a unit deliberately thin — enough to select it, order it
//! somewhere, and watch it get there. Health, weapons, vision and the rest
//! arrive in Phase 3 as data-driven traits (see `docs/05-data-and-modding.md`).

use serde::{Deserialize, Serialize};

use redshift_data::rules::EntityKind;

use crate::command::PlayerId;
use crate::fx::{Angle, Fx};
use crate::hash::{StateHash, StateHasher};
use crate::map::{Cell, WorldPos};

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

/// A thing in the world.
///
/// Deliberately thin. A unit carries only what changes — where it is, what it
/// is doing, how much health is left. Everything constant comes from its
/// [`EntityKind`] via the resolved stat table, so a unit is small and the same
/// numbers cannot drift between two copies of the same kind.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Unit {
    pub owner: PlayerId,
    /// What this is, as defined in the rules.
    pub kind: EntityKind,
    pub pos: WorldPos,
    pub facing: Angle,
    /// Remaining health. Its maximum lives in the stat table.
    pub health: u32,
    pub order: Order,
}

impl Unit {
    pub fn new(owner: PlayerId, kind: EntityKind, pos: WorldPos, max_health: u32) -> Unit {
        Unit {
            owner,
            kind,
            pos,
            facing: Angle::ZERO,
            health: max_health,
            order: Order::Idle,
        }
    }

    pub fn cell(&self) -> Cell {
        self.pos.cell()
    }

    pub fn is_alive(&self) -> bool {
        self.health > 0
    }
}

impl StateHash for Unit {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write(&self.owner);
        h.write_u16(self.kind.0);
        h.write(&self.pos);
        h.write_u16(self.facing.raw());
        h.write_u32(self.health);
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

    fn sample(pos: WorldPos) -> Unit {
        Unit::new(PlayerId(0), EntityKind(0), pos, 100)
    }

    fn hash(u: &Unit) -> u64 {
        let mut h = StateHasher::new();
        h.write(u);
        h.finish()
    }

    #[test]
    fn a_new_unit_is_idle_at_its_position() {
        let pos = Cell::new(4, 5).centre();
        let u = sample(pos);
        assert!(u.order.is_idle());
        assert_eq!(u.cell(), Cell::new(4, 5));
        assert_eq!(u.pos, pos);
        assert!(u.is_alive());
    }

    #[test]
    fn a_unit_at_zero_health_is_not_alive() {
        let mut u = sample(WorldPos::ORIGIN);
        u.health = 0;
        assert!(!u.is_alive());
    }

    #[test]
    fn arrival_tolerance_is_reachable_in_one_step() {
        // A unit that cannot cover the tolerance in a single tick would stop
        // short of every waypoint and never finish a path. Speed now comes from
        // the rules, so this checks the shipped values rather than a constant.
        let rules = crate::sim::test_rules();
        let stats = crate::stats::StatTable::resolve(&rules, &[None]);
        let speed = stats.get(PlayerId(0), crate::sim::TEST_KIND).speed;
        assert!(
            speed > ARRIVAL_TOLERANCE,
            "speed {speed:?} must exceed the arrival tolerance {ARRIVAL_TOLERANCE:?}"
        );
    }

    #[test]
    fn hashing_covers_the_kind_and_health() {
        // Two units differing only in what they are, or in how hurt they are,
        // must not hash alike — both change how the match plays out.
        let base = sample(WorldPos::ORIGIN);

        let mut other_kind = base.clone();
        other_kind.kind = EntityKind(1);
        assert_ne!(hash(&base), hash(&other_kind));

        let mut hurt = base.clone();
        hurt.health -= 1;
        assert_ne!(hash(&base), hash(&hurt));
    }

    #[test]
    fn hashing_covers_the_order() {
        let mut a = sample(WorldPos::ORIGIN);
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
