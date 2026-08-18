//! Veterancy.
//!
//! Units that survive get better at their job. The original used three ranks
//! and awarded them on kills, and the effect is less about the numbers than
//! about what it does to a player's decisions: a veteran squad is worth pulling
//! out of a losing fight, which is a decision that does not exist otherwise.
//!
//! # A caution about the numbers
//!
//! The *mechanism* is faithful — kills promote, promotion helps. The exact
//! bonuses are placeholders chosen to feel right and are flagged in TODO.md
//! alongside the other unverified ratios. They are named constants precisely so
//! that correcting them later is a one-line change.

use serde::{Deserialize, Serialize};

use crate::hash::{StateHash, StateHasher};

/// How much better a rank makes a unit, as a percentage.
///
/// **Not verified against the original.**
pub const VETERAN_BONUS: u32 = 115;
pub const ELITE_BONUS: u32 = 135;

/// A unit's experience.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum Rank {
    #[default]
    Rookie = 0,
    Veteran = 1,
    Elite = 2,
}

impl Rank {
    /// The rank a unit with this many kills has earned.
    ///
    /// Takes the thresholds rather than knowing them, because they are per-kind
    /// data: a unit that promotes on one kill and one that never promotes are
    /// both expressible, and neither needs code.
    pub fn for_kills(kills: u32, thresholds: Option<(u32, u32)>) -> Rank {
        let Some((veteran, elite)) = thresholds else {
            return Rank::Rookie;
        };
        if kills >= elite {
            Rank::Elite
        } else if kills >= veteran {
            Rank::Veteran
        } else {
            Rank::Rookie
        }
    }

    /// Multiplier applied to damage dealt and damage resisted, as a percentage.
    pub fn bonus_percent(self) -> u32 {
        match self {
            Rank::Rookie => 100,
            Rank::Veteran => VETERAN_BONUS,
            Rank::Elite => ELITE_BONUS,
        }
    }

    /// Scales a value by this rank's bonus.
    pub fn scale(self, value: u32) -> u32 {
        ((value as u64 * self.bonus_percent() as u64) / 100) as u32
    }

    /// Reduces incoming damage by this rank's bonus.
    ///
    /// The inverse of [`Rank::scale`], so a rank that deals 15% more also takes
    /// about 13% less. Expressed as a division rather than a second constant,
    /// so the two can never drift apart into a rank that is good at attacking
    /// and mysteriously bad at defending.
    pub fn resist(self, damage: u32) -> u32 {
        ((damage as u64 * 100) / self.bonus_percent() as u64) as u32
    }

    pub fn name(self) -> &'static str {
        match self {
            Rank::Rookie => "rookie",
            Rank::Veteran => "veteran",
            Rank::Elite => "elite",
        }
    }
}

impl StateHash for Rank {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_u8(*self as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unit_that_never_promotes_stays_a_rookie() {
        // Most things in the game have no veterancy at all, and a missing trait
        // must mean "never promotes" rather than "promotes immediately".
        assert_eq!(Rank::for_kills(0, None), Rank::Rookie);
        assert_eq!(Rank::for_kills(1_000, None), Rank::Rookie);
    }

    #[test]
    fn kills_earn_rank_at_the_thresholds() {
        let t = Some((3, 8));
        assert_eq!(Rank::for_kills(0, t), Rank::Rookie);
        assert_eq!(Rank::for_kills(2, t), Rank::Rookie);
        assert_eq!(Rank::for_kills(3, t), Rank::Veteran, "the threshold counts");
        assert_eq!(Rank::for_kills(7, t), Rank::Veteran);
        assert_eq!(Rank::for_kills(8, t), Rank::Elite);
        assert_eq!(Rank::for_kills(99, t), Rank::Elite);
    }

    #[test]
    fn rank_makes_a_unit_better_at_both_halves_of_a_fight() {
        // A rank that dealt more and took the same would be a different, and
        // much weaker, mechanic.
        assert!(Rank::Veteran.scale(100) > Rank::Rookie.scale(100));
        assert!(Rank::Elite.scale(100) > Rank::Veteran.scale(100));
        assert!(Rank::Veteran.resist(100) < Rank::Rookie.resist(100));
        assert!(Rank::Elite.resist(100) < Rank::Veteran.resist(100));
    }

    #[test]
    fn a_rookie_is_unchanged_in_both_directions() {
        // The common case must be exactly neutral, or every unit in the game
        // quietly has a rounding error applied to it.
        for value in [1, 7, 50, 999, 100_000] {
            assert_eq!(Rank::Rookie.scale(value), value);
            assert_eq!(Rank::Rookie.resist(value), value);
        }
    }

    #[test]
    fn resistance_is_derived_from_the_same_number_as_the_bonus() {
        // Two independent constants would drift into a rank that is good at
        // attacking and mysteriously bad at defending.
        for rank in [Rank::Rookie, Rank::Veteran, Rank::Elite] {
            let dealt = rank.scale(1000);
            let taken = rank.resist(1000);
            assert!(
                dealt >= 1000 && taken <= 1000,
                "{} deals {dealt} and takes {taken}",
                rank.name()
            );
        }
    }

    #[test]
    fn ranks_order_from_worst_to_best() {
        // Compared directly in places, so the ordering has to be the obvious
        // one rather than declaration order that happens to look right.
        assert!(Rank::Rookie < Rank::Veteran);
        assert!(Rank::Veteran < Rank::Elite);
    }

    #[test]
    fn scaling_does_not_overflow_at_absurd_values() {
        assert!(Rank::Elite.scale(u32::MAX) > 0);
        assert!(Rank::Elite.resist(u32::MAX) > 0);
    }
}
