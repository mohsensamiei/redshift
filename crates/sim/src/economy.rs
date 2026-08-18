//! Credits, ore, and the harvester cycle.
//!
//! # The first autonomous behaviour
//!
//! Everything a unit has done so far came from a player command. A harvester
//! does not: it picks a field, walks to it, works, finds a refinery, unloads,
//! and goes back — thousands of decisions with nothing anchoring them.
//!
//! That makes it the first real test of determinism under free choice. Every
//! decision here has to break ties the same way on every machine, because there
//! is no command stream to correct a peer that chose differently. Two rules
//! follow from that:
//!
//! - **Choices are made over the arena in index order**, and ties fall back to
//!   the lower entity id or the lower cell index. Never "whichever was closest"
//!   without saying what happens when two are equally close.
//! - **Nothing consults the clock or the RNG.** A harvester that jittered its
//!   search would be indistinguishable from one that desynced.

use serde::{Deserialize, Serialize};

use crate::arena::{Arena, EntityId};
use crate::command::PlayerId;
use crate::fx::{Fx, FxWide};
use crate::hash::{StateHash, StateHasher};
use crate::map::{Cell, Map};
use crate::unit::Unit;

/// Credits each player has to spend.
///
/// A dense vector indexed by player rather than a map: players are numbered
/// from zero and there are never many, so this is an array read, and there is
/// no iteration order to get wrong.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Treasury {
    credits: Vec<u32>,
}

impl Treasury {
    pub fn new(player_count: usize, starting_credits: u32) -> Treasury {
        Treasury {
            credits: vec![starting_credits; player_count],
        }
    }

    #[inline]
    pub fn credits(&self, player: PlayerId) -> u32 {
        self.credits.get(player.0 as usize).copied().unwrap_or(0)
    }

    /// Adds credits, saturating.
    pub fn deposit(&mut self, player: PlayerId, amount: u32) {
        if let Some(slot) = self.credits.get_mut(player.0 as usize) {
            *slot = slot.saturating_add(amount);
        }
    }

    /// Spends credits if there are enough, reporting whether it happened.
    ///
    /// Returns a bool rather than panicking or going negative: an order that
    /// arrives a tick after the money ran out is ordinary, not exceptional, and
    /// the caller has to handle it either way.
    #[must_use]
    pub fn try_spend(&mut self, player: PlayerId, amount: u32) -> bool {
        let Some(slot) = self.credits.get_mut(player.0 as usize) else {
            return false;
        };
        if *slot < amount {
            return false;
        }
        *slot -= amount;
        true
    }

    pub fn player_count(&self) -> usize {
        self.credits.len()
    }
}

impl StateHash for Treasury {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_u32(self.credits.len() as u32);
        for c in &self.credits {
            h.write_u32(*c);
        }
    }
}

/// The nearest cell with ore, searched outward from `from`.
///
/// # Determinism
///
/// Ring by ring outward, and within a ring in a fixed order, so the first hit
/// is the same on every machine. A "nearest by distance" scan over all cells
/// would be equivalent in spirit and would have to define what happens when two
/// cells tie — this way the ordering *is* the tie-break.
///
/// `claimed` holds cells other harvesters are already working, so two of them
/// do not pile onto the same square while a field sits untouched beside them.
pub fn nearest_ore(map: &Map, from: Cell, max_radius: i32, claimed: &[Cell]) -> Option<Cell> {
    for radius in 0..=max_radius {
        for (dx, dy) in crate::sim::ring_offsets(radius) {
            let cell = Cell::new(from.x + dx, from.y + dy);
            if !map.has_ore(cell) {
                continue;
            }
            if claimed.contains(&cell) {
                continue;
            }
            return Some(cell);
        }
    }
    None
}

/// The closest refinery belonging to `player`.
///
/// Ties break on the lower entity id, which comes free from scanning the arena
/// in index order and accepting only a strict improvement.
pub fn nearest_refinery(
    units: &Arena<Unit>,
    player: PlayerId,
    from: Fx,
    from_y: Fx,
    is_refinery: &dyn Fn(&Unit) -> bool,
) -> Option<EntityId> {
    let mut best: Option<(EntityId, FxWide)> = None;
    for (id, unit) in units.iter() {
        if unit.owner != player || !unit.is_alive() || !is_refinery(unit) {
            continue;
        }
        let distance = Fx::dist_sq(unit.pos.x - from, unit.pos.y - from_y);
        if best.is_none_or(|(_, d)| distance < d) {
            best = Some((id, distance));
        }
    }
    best.map(|(id, _)| id)
}

/// How much ore a harvester takes in one bite.
///
/// Bites rather than a continuous trickle, so a field visibly thins in steps
/// and a half-full harvester is a meaningful state. Also keeps the arithmetic
/// integral without a fractional accumulator per unit.
pub const ORE_PER_BITE: u16 = 25;

/// Ticks between bites.
pub const GATHER_INTERVAL: u32 = 8;

/// How far a harvester will look for a field before giving up.
///
/// Bounded so a harvester on a mined-out map does not scan the whole world
/// every tick. It goes idle instead, which is visible and diagnosable.
pub const ORE_SEARCH_RADIUS: i32 = 40;

/// Credits paid per unit of ore delivered.
pub const CREDITS_PER_ORE: u32 = 1;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::Terrain;

    #[test]
    fn credits_start_where_they_are_set_and_add_up() {
        let mut treasury = Treasury::new(2, 5000);
        assert_eq!(treasury.credits(PlayerId(0)), 5000);
        assert_eq!(treasury.credits(PlayerId(1)), 5000);

        treasury.deposit(PlayerId(0), 700);
        assert_eq!(treasury.credits(PlayerId(0)), 5700);
        assert_eq!(treasury.credits(PlayerId(1)), 5000, "players are separate");
    }

    #[test]
    fn spending_more_than_you_have_fails_rather_than_going_negative() {
        // An order arriving a tick after the money ran out is ordinary, not
        // exceptional. Going negative would let a player build on credit.
        let mut treasury = Treasury::new(1, 100);
        assert!(!treasury.try_spend(PlayerId(0), 101));
        assert_eq!(
            treasury.credits(PlayerId(0)),
            100,
            "a failed spend takes nothing"
        );
        assert!(treasury.try_spend(PlayerId(0), 100));
        assert_eq!(treasury.credits(PlayerId(0)), 0);
        assert!(!treasury.try_spend(PlayerId(0), 1));
    }

    #[test]
    fn an_unknown_player_has_nothing_and_cannot_spend() {
        let mut treasury = Treasury::new(1, 100);
        assert_eq!(treasury.credits(PlayerId(9)), 0);
        assert!(!treasury.try_spend(PlayerId(9), 1));
    }

    #[test]
    fn deposits_saturate_rather_than_wrapping() {
        // Wrapping would turn a very rich player into a bankrupt one, which is
        // a memorable bug to hit in a long match.
        let mut treasury = Treasury::new(1, u32::MAX - 5);
        treasury.deposit(PlayerId(0), 100);
        assert_eq!(treasury.credits(PlayerId(0)), u32::MAX);
    }

    #[test]
    fn the_nearest_ore_is_found_outward() {
        let mut map = Map::new(40, 40);
        map.set_ore(Cell::new(20, 25), 100);
        map.set_ore(Cell::new(22, 20), 100);

        // (22, 20) is two cells away; (20, 25) is five.
        assert_eq!(
            nearest_ore(&map, Cell::new(20, 20), 20, &[]),
            Some(Cell::new(22, 20))
        );
    }

    #[test]
    fn a_claimed_cell_is_skipped_so_harvesters_spread_out() {
        // Two harvesters piling onto one square while the rest of a field sits
        // untouched is both slower and visibly silly.
        let mut map = Map::new(40, 40);
        map.set_ore(Cell::new(21, 20), 100);
        map.set_ore(Cell::new(22, 20), 100);

        let first = nearest_ore(&map, Cell::new(20, 20), 20, &[]).expect("ore nearby");
        let second = nearest_ore(&map, Cell::new(20, 20), 20, &[first]).expect("more ore");
        assert_ne!(first, second);
    }

    #[test]
    fn an_empty_map_reports_no_ore_rather_than_searching_forever() {
        let map = Map::new(40, 40);
        assert_eq!(
            nearest_ore(&map, Cell::new(20, 20), ORE_SEARCH_RADIUS, &[]),
            None
        );
    }

    #[test]
    fn the_search_is_reproducible() {
        // Free choice with no command stream to correct it: two peers must
        // pick the same cell every time.
        let mut map = Map::new(40, 40);
        map.add_ore_field(Cell::new(25, 25), 4, 300);
        for _ in 0..50 {
            assert_eq!(
                nearest_ore(&map, Cell::new(20, 20), 30, &[]),
                nearest_ore(&map, Cell::new(20, 20), 30, &[])
            );
        }
    }

    #[test]
    fn taking_ore_never_credits_more_than_was_there() {
        let mut map = Map::new(10, 10);
        map.set_ore(Cell::new(5, 5), 30);

        assert_eq!(map.take_ore(Cell::new(5, 5), 25), 25);
        assert_eq!(map.ore(Cell::new(5, 5)), 5);
        // Asking for more than remains yields only what is left.
        assert_eq!(map.take_ore(Cell::new(5, 5), 25), 5);
        assert_eq!(map.ore(Cell::new(5, 5)), 0);
        assert_eq!(map.take_ore(Cell::new(5, 5), 25), 0);
    }

    #[test]
    fn taking_from_outside_the_map_yields_nothing() {
        let mut map = Map::new(10, 10);
        assert_eq!(map.take_ore(Cell::new(-1, -1), 25), 0);
        assert_eq!(map.take_ore(Cell::new(99, 99), 25), 0);
    }

    #[test]
    fn an_ore_field_is_richest_in_the_middle() {
        let mut map = Map::new(40, 40);
        map.add_ore_field(Cell::new(20, 20), 3, 400);

        let centre = map.ore(Cell::new(20, 20));
        let edge = map.ore(Cell::new(23, 20));
        assert!(centre > edge, "centre {centre} should beat edge {edge}");
        assert!(edge > 0, "the edge should still hold something");
        assert_eq!(map.ore(Cell::new(25, 20)), 0, "beyond the radius is bare");
    }

    #[test]
    fn ore_is_never_scattered_into_water() {
        // A field no harvester could reach would look like a bug in the
        // harvester rather than in the map.
        let mut map = Map::new(40, 40);
        map.fill_rect(Cell::new(18, 18), Cell::new(22, 22), Terrain::Water);
        map.add_ore_field(Cell::new(20, 20), 5, 400);

        for y in 18..=22 {
            for x in 18..=22 {
                assert_eq!(map.ore(Cell::new(x, y)), 0, "ore in water at {x},{y}");
            }
        }
        assert!(
            map.total_ore() > 0,
            "the field should exist outside the lake"
        );
    }

    #[test]
    fn a_cell_cannot_hold_more_than_the_cap() {
        let mut map = Map::new(10, 10);
        map.set_ore(Cell::new(5, 5), u16::MAX);
        assert_eq!(map.ore(Cell::new(5, 5)), crate::map::MAX_ORE_PER_CELL);

        // Overlapping fields must not exceed it either.
        let mut map = Map::new(40, 40);
        for _ in 0..10 {
            map.add_ore_field(Cell::new(20, 20), 3, 400);
        }
        assert_eq!(map.ore(Cell::new(20, 20)), crate::map::MAX_ORE_PER_CELL);
    }
}
