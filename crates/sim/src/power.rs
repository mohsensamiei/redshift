//! The power grid.
//!
//! # What low power actually does
//!
//! In the original, running short of power does not stop a base — it degrades
//! it. Production slows, and structures that need power to operate stop
//! operating. That distinction matters: a base that simply froze would end the
//! match on the spot, whereas one that slows down gives the player a chance to
//! notice and build a power plant.
//!
//! It also makes power plants a real target. Killing one does not destroy
//! anything, but it takes the defences offline and halves the rate the enemy
//! can replace them.
//!
//! # A caution about the numbers
//!
//! The *mechanism* here is faithful. The exact slowdown ratio is not verified
//! against the original — [`LOW_POWER_DIVISOR`] is a placeholder chosen to feel
//! right, and is flagged in TODO.md as needing checking. Everything else about
//! the model is structural and should not need to change when that number does.

use serde::{Deserialize, Serialize};

use crate::command::PlayerId;
use crate::hash::{StateHash, StateHasher};

/// How much slower production runs when a base is short of power.
///
/// **Not verified against the original.** The mechanism is faithful; this
/// number is a placeholder. Kept as a named constant precisely so that
/// correcting it later is a one-line change rather than an archaeology
/// exercise.
pub const LOW_POWER_DIVISOR: u32 = 4;

/// Supply and demand, per player.
///
/// Recomputed from scratch every tick rather than maintained incrementally.
/// Incremental bookkeeping would be faster and would have to be updated on
/// every spawn, death, capture and sale — and a single missed update leaves a
/// base permanently and invisibly wrong about its own power. Recomputing is
/// O(units) against a tick that already walks every unit several times.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerGrid {
    supply: Vec<u32>,
    draw: Vec<u32>,
}

impl PowerGrid {
    pub fn new(player_count: usize) -> PowerGrid {
        PowerGrid {
            supply: vec![0; player_count],
            draw: vec![0; player_count],
        }
    }

    pub fn clear(&mut self) {
        self.supply.iter_mut().for_each(|v| *v = 0);
        self.draw.iter_mut().for_each(|v| *v = 0);
    }

    pub fn add_supply(&mut self, player: PlayerId, amount: u32) {
        if let Some(slot) = self.supply.get_mut(player.0 as usize) {
            *slot = slot.saturating_add(amount);
        }
    }

    pub fn add_draw(&mut self, player: PlayerId, amount: u32) {
        if let Some(slot) = self.draw.get_mut(player.0 as usize) {
            *slot = slot.saturating_add(amount);
        }
    }

    /// Cuts a player's supply to nothing, whatever they are generating.
    ///
    /// Sabotage, expressed as a fact about the grid rather than as an exception
    /// at each of the dozen places that ask "is this powered". A sabotaged base
    /// then behaves exactly like one whose reactors were bombed, which is the
    /// whole point of sabotaging it.
    pub fn black_out(&mut self, player: PlayerId) {
        if let Some(slot) = self.supply.get_mut(player.0 as usize) {
            *slot = 0;
        }
    }

    #[inline]
    pub fn supply(&self, player: PlayerId) -> u32 {
        self.supply.get(player.0 as usize).copied().unwrap_or(0)
    }

    #[inline]
    pub fn draw(&self, player: PlayerId) -> u32 {
        self.draw.get(player.0 as usize).copied().unwrap_or(0)
    }

    /// Whether a player is generating at least what they consume.
    ///
    /// Equal counts as satisfied: a base drawing exactly what it makes is
    /// running on the edge, not failing.
    #[inline]
    pub fn is_satisfied(&self, player: PlayerId) -> bool {
        self.supply(player) >= self.draw(player)
    }

    /// Power available as a percentage of what is wanted, capped at 100.
    ///
    /// For the interface. A base with no draw at all reads as fully powered
    /// rather than as a division by zero.
    pub fn percent(&self, player: PlayerId) -> u32 {
        let draw = self.draw(player);
        if draw == 0 {
            return 100;
        }
        (((self.supply(player) as u64 * 100) / draw as u64) as u32).min(100)
    }

    /// How much more is being drawn than supplied.
    pub fn shortfall(&self, player: PlayerId) -> u32 {
        self.draw(player).saturating_sub(self.supply(player))
    }
}

impl StateHash for PowerGrid {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_u32(self.supply.len() as u32);
        for v in &self.supply {
            h.write_u32(*v);
        }
        for v in &self.draw {
            h.write_u32(*v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_base_with_no_buildings_is_satisfied() {
        // Nothing drawing is not a shortage, and must not read as one — a new
        // player would otherwise start the match in a brownout.
        let grid = PowerGrid::new(2);
        assert!(grid.is_satisfied(PlayerId(0)));
        assert_eq!(grid.percent(PlayerId(0)), 100);
        assert_eq!(grid.shortfall(PlayerId(0)), 0);
    }

    #[test]
    fn supply_and_draw_are_tracked_per_player() {
        let mut grid = PowerGrid::new(2);
        grid.add_supply(PlayerId(0), 100);
        grid.add_draw(PlayerId(0), 40);
        grid.add_draw(PlayerId(1), 40);

        assert!(grid.is_satisfied(PlayerId(0)));
        assert!(
            !grid.is_satisfied(PlayerId(1)),
            "one player's plant must not power another"
        );
    }

    #[test]
    fn drawing_exactly_what_is_supplied_counts_as_satisfied() {
        // Running on the edge is not failing, and a base that browned out at
        // exactly 100% would be maddening to plan around.
        let mut grid = PowerGrid::new(1);
        grid.add_supply(PlayerId(0), 100);
        grid.add_draw(PlayerId(0), 100);
        assert!(grid.is_satisfied(PlayerId(0)));
        assert_eq!(grid.percent(PlayerId(0)), 100);
        assert_eq!(grid.shortfall(PlayerId(0)), 0);
    }

    #[test]
    fn a_shortfall_is_reported_as_a_proportion_and_an_amount() {
        let mut grid = PowerGrid::new(1);
        grid.add_supply(PlayerId(0), 50);
        grid.add_draw(PlayerId(0), 200);

        assert!(!grid.is_satisfied(PlayerId(0)));
        assert_eq!(grid.percent(PlayerId(0)), 25);
        assert_eq!(grid.shortfall(PlayerId(0)), 150);
    }

    #[test]
    fn losing_the_only_plant_takes_the_base_offline() {
        let mut grid = PowerGrid::new(1);
        grid.add_supply(PlayerId(0), 100);
        grid.add_draw(PlayerId(0), 80);
        assert!(grid.is_satisfied(PlayerId(0)));

        // The plant is destroyed and the grid is rebuilt from what is left.
        grid.clear();
        grid.add_draw(PlayerId(0), 80);
        assert!(!grid.is_satisfied(PlayerId(0)));
        assert_eq!(grid.percent(PlayerId(0)), 0);
    }

    #[test]
    fn an_unknown_player_reads_as_empty_rather_than_panicking() {
        let mut grid = PowerGrid::new(1);
        grid.add_supply(PlayerId(9), 100);
        grid.add_draw(PlayerId(9), 100);
        assert_eq!(grid.supply(PlayerId(9)), 0);
        assert!(grid.is_satisfied(PlayerId(9)));
    }

    #[test]
    fn totals_saturate_rather_than_wrapping() {
        // Wrapping would turn a very well powered base into a blacked-out one.
        let mut grid = PowerGrid::new(1);
        grid.add_supply(PlayerId(0), u32::MAX);
        grid.add_supply(PlayerId(0), 1000);
        assert_eq!(grid.supply(PlayerId(0)), u32::MAX);
    }

    #[test]
    fn clearing_resets_both_sides() {
        // The grid is rebuilt from scratch each tick; a clear that missed one
        // side would leave a base permanently wrong about its own power.
        let mut grid = PowerGrid::new(1);
        grid.add_supply(PlayerId(0), 100);
        grid.add_draw(PlayerId(0), 100);
        grid.clear();
        assert_eq!(grid.supply(PlayerId(0)), 0);
        assert_eq!(grid.draw(PlayerId(0)), 0);
    }

    #[test]
    fn the_grid_hashes_both_sides() {
        let hash = |g: &PowerGrid| {
            let mut h = StateHasher::new();
            h.write(g);
            h.finish()
        };
        let base = PowerGrid::new(2);
        let mut supplied = base.clone();
        supplied.add_supply(PlayerId(0), 100);
        let mut drawn = base.clone();
        drawn.add_draw(PlayerId(0), 100);

        assert_ne!(hash(&supplied), hash(&base));
        assert_ne!(hash(&drawn), hash(&base));
        assert_ne!(
            hash(&supplied),
            hash(&drawn),
            "supply and draw must not alias"
        );
    }
}
