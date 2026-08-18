//! Building things.
//!
//! # Credits drain, they are not deducted up front
//!
//! The original charged for an item gradually as it built, and paused when the
//! player ran out. That is not an accounting detail — it is a real part of how
//! the game plays. It lets a player queue something they cannot yet afford and
//! let their harvesters catch up, and it means losing a refinery mid-build
//! stalls production rather than silently refunding it.
//!
//! Charging the whole cost at queue time would be simpler and would quietly
//! remove that decision from the game.
//!
//! # Paying exactly, in integers
//!
//! Paying `cost / total_ticks` each tick loses the remainder and undercharges.
//! Instead each tick pays *the difference between what should have been paid by
//! now and what has been paid so far*, so the running total is always exact and
//! the final instalment settles the balance to the credit.

use serde::{Deserialize, Serialize};

use redshift_data::rules::EntityKind;

use crate::hash::{StateHash, StateHasher};

/// One item being produced.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ProductionItem {
    pub kind: EntityKind,
    /// Full price.
    pub cost: u32,
    /// Paid so far.
    pub paid: u32,
    /// Ticks of work completed.
    pub progress: u32,
    /// Ticks of work required.
    pub duration: u32,
}

impl ProductionItem {
    pub fn new(kind: EntityKind, cost: u32, duration: u32) -> ProductionItem {
        ProductionItem {
            kind,
            cost,
            paid: 0,
            progress: 0,
            // A zero-tick item would finish before it could be paid for.
            duration: duration.max(1),
        }
    }

    /// What should have been paid once `progress` ticks are done.
    ///
    /// Computed from the total rather than accumulated per tick, so rounding
    /// cannot drift and the last instalment always settles the balance exactly.
    fn due_at(&self, progress: u32) -> u32 {
        ((self.cost as u64 * progress as u64) / self.duration as u64) as u32
    }

    /// The instalment owed to advance one more tick.
    pub fn next_instalment(&self) -> u32 {
        self.due_at(self.progress + 1).saturating_sub(self.paid)
    }

    pub fn is_complete(&self) -> bool {
        self.progress >= self.duration
    }

    /// Progress as a percentage, for the interface.
    pub fn percent(&self) -> u32 {
        ((self.progress as u64 * 100) / self.duration as u64) as u32
    }
}

impl StateHash for ProductionItem {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_u16(self.kind.0);
        h.write_u32(self.cost);
        h.write_u32(self.paid);
        h.write_u32(self.progress);
        h.write_u32(self.duration);
    }
}

/// A production building's queue.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct ProductionQueue {
    items: Vec<ProductionItem>,
    /// A finished structure waiting for the player to choose a site.
    ///
    /// Buildings are not delivered where they were built. The original made
    /// this a two-step act — build it, then place it — and that is most of what
    /// base layout *is* as a decision. Spawning a structure next to its
    /// construction yard would take that decision away entirely.
    pub ready: Option<EntityKind>,
    /// Whether the finished item has nowhere to go.
    ///
    /// Distinct from [`ProductionQueue::starved`]: one is fixed by earning
    /// money and the other by clearing space, and telling a player the wrong
    /// one sends them off doing something useless.
    pub blocked: bool,
    /// Whether the queue is held up for want of credits.
    ///
    /// Kept so the interface can say *why* nothing is happening. "Paused" and
    /// "working slowly" look identical otherwise, and a player who cannot tell
    /// them apart concludes the game is broken.
    pub starved: bool,
}

/// How many items may be queued at one building.
pub const MAX_QUEUE: usize = 9;

impl ProductionQueue {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn items(&self) -> &[ProductionItem] {
        &self.items
    }

    /// The item currently being worked on.
    pub fn current(&self) -> Option<&ProductionItem> {
        self.items.first()
    }

    /// Adds an item, unless the queue is full.
    pub fn enqueue(&mut self, item: ProductionItem) -> bool {
        if self.items.len() >= MAX_QUEUE {
            return false;
        }
        self.items.push(item);
        true
    }

    /// Puts a finished item back at the front, still complete.
    ///
    /// Used when there is nowhere to place what was built. The item is fully
    /// paid for, so it is held rather than refunded or discarded — losing a
    /// paid-for unit because the factory was boxed in would be an infuriating
    /// way to lose a match, and refunding it would let a player park a factory
    /// against a wall to bank production.
    pub fn hold_completed(&mut self, item: ProductionItem) {
        self.items.insert(0, item);
        self.blocked = true;
    }

    /// Removes an item by queue position, returning what has been paid for it.
    ///
    /// Cancelling refunds what was actually paid, not the full price: the
    /// player has had the benefit of the credits being committed, and refunding
    /// more than was spent would make queueing and cancelling a way to print
    /// money.
    pub fn cancel(&mut self, index: usize) -> Option<u32> {
        if index >= self.items.len() {
            return None;
        }
        let item = self.items.remove(index);
        Some(item.paid)
    }

    /// Advances the front item by one tick, given the credits available.
    ///
    /// Returns what was spent and, if one finished, what it was. Spending is
    /// reported rather than applied so the caller keeps the treasury in one
    /// place — a queue that could reach into the treasury would be a second
    /// place credits are created.
    pub fn tick(&mut self, available: u32) -> ProductionStep {
        // A structure waiting to be placed holds the queue. The original built
        // one structure at a time, and letting the next start would leave the
        // player with several finished buildings and no way to tell them apart.
        if self.ready.is_some() {
            return ProductionStep::default();
        }

        let Some(item) = self.items.first_mut() else {
            self.starved = false;
            return ProductionStep::default();
        };

        if item.is_complete() {
            // Held from a previous tick because there was nowhere to place it.
            let finished = self.items.remove(0);
            self.blocked = false;
            return ProductionStep {
                spent: 0,
                completed: Some(finished.kind),
            };
        }

        let instalment = item.next_instalment();
        if instalment > available {
            // Held, not cancelled. The player keeps their place and their
            // partial payment; harvesters can catch up.
            self.starved = true;
            return ProductionStep::default();
        }

        self.starved = false;
        item.paid += instalment;
        item.progress += 1;

        if item.is_complete() {
            let finished = self.items.remove(0);
            ProductionStep {
                spent: instalment,
                completed: Some(finished.kind),
            }
        } else {
            ProductionStep {
                spent: instalment,
                completed: None,
            }
        }
    }
}

/// What one tick of production did.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ProductionStep {
    pub spent: u32,
    pub completed: Option<EntityKind>,
}

impl StateHash for ProductionQueue {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_u32(self.items.len() as u32);
        for item in &self.items {
            h.write(item);
        }
        h.write_bool(self.starved);
        h.write_bool(self.blocked);
        match self.ready {
            Some(kind) => {
                h.write_u8(1);
                h.write_u16(kind.0);
            }
            None => h.write_u8(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(cost: u32, duration: u32) -> ProductionItem {
        ProductionItem::new(EntityKind(1), cost, duration)
    }

    /// Runs a queue to completion with unlimited funds, returning the total
    /// spent and the ticks taken.
    fn run_to_completion(queue: &mut ProductionQueue, budget: u32) -> (u32, u32) {
        let mut spent = 0;
        let mut ticks = 0;
        while !queue.is_empty() && ticks < 10_000 {
            let step = queue.tick(budget - spent);
            spent += step.spent;
            ticks += 1;
        }
        (spent, ticks)
    }

    #[test]
    fn an_item_costs_exactly_its_price() {
        // The property the instalment arithmetic exists for. Paying
        // cost/duration each tick loses the remainder and undercharges; this
        // must land on the price to the credit, whatever the numbers.
        for (cost, duration) in [(1000, 60), (900, 45), (777, 13), (100, 99), (1, 50)] {
            let mut queue = ProductionQueue::default();
            queue.enqueue(item(cost, duration));
            let (spent, ticks) = run_to_completion(&mut queue, u32::MAX);
            assert_eq!(
                spent, cost,
                "cost {cost} over {duration} ticks cost {spent}"
            );
            assert_eq!(ticks, duration, "took {ticks} ticks, expected {duration}");
        }
    }

    #[test]
    fn payment_is_spread_across_the_build_rather_than_taken_up_front() {
        // Charging up front would be simpler and would quietly remove a real
        // decision from the game: queueing something you cannot yet afford and
        // letting the harvesters catch up.
        let mut queue = ProductionQueue::default();
        queue.enqueue(item(1000, 100));

        let first = queue.tick(u32::MAX);
        assert!(first.spent > 0, "nothing was charged");
        assert!(first.spent < 1000, "the whole cost was taken at once");
        assert_eq!(queue.current().unwrap().paid, first.spent);
    }

    #[test]
    fn a_queue_holds_when_credits_run_out_and_resumes_when_they_return() {
        let mut queue = ProductionQueue::default();
        queue.enqueue(item(1000, 10));

        // Not enough for the first instalment.
        let step = queue.tick(0);
        assert_eq!(step.spent, 0);
        assert!(queue.starved, "the queue should say why it is not moving");
        assert_eq!(
            queue.current().unwrap().progress,
            0,
            "progress must not advance unpaid"
        );

        // Money arrives.
        let step = queue.tick(1000);
        assert!(step.spent > 0);
        assert!(!queue.starved);
        assert_eq!(queue.current().unwrap().progress, 1);
    }

    #[test]
    fn a_starved_queue_keeps_its_place_and_its_payment() {
        // Cancelling on the player's behalf would lose both their position and
        // their partial payment for a shortfall that might last one tick.
        let mut queue = ProductionQueue::default();
        queue.enqueue(item(1000, 10));
        queue.tick(u32::MAX);
        let paid = queue.current().unwrap().paid;

        for _ in 0..100 {
            queue.tick(0);
        }
        assert_eq!(queue.len(), 1, "the item was dropped");
        assert_eq!(
            queue.current().unwrap().paid,
            paid,
            "partial payment was lost"
        );
    }

    #[test]
    fn cancelling_refunds_what_was_paid_and_no_more() {
        // Refunding the full price would make queue-and-cancel a way to print
        // money.
        let mut queue = ProductionQueue::default();
        queue.enqueue(item(1000, 100));
        let mut spent = 0;
        for _ in 0..30 {
            spent += queue.tick(u32::MAX).spent;
        }

        let refund = queue.cancel(0).expect("there is an item to cancel");
        assert_eq!(
            refund, spent,
            "refund {refund} did not match the {spent} paid"
        );
        assert!(refund < 1000, "a partial build refunded the full price");
        assert!(queue.is_empty());
    }

    #[test]
    fn cancelling_a_position_that_is_not_there_does_nothing() {
        let mut queue = ProductionQueue::default();
        assert_eq!(queue.cancel(0), None);
        queue.enqueue(item(100, 10));
        assert_eq!(queue.cancel(5), None);
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn items_are_built_in_the_order_they_were_queued() {
        let mut queue = ProductionQueue::default();
        for k in 1..=3u16 {
            queue.enqueue(ProductionItem::new(EntityKind(k), 100, 5));
        }
        let mut finished = Vec::new();
        for _ in 0..20 {
            if let Some(kind) = queue.tick(u32::MAX).completed {
                finished.push(kind);
            }
        }
        assert_eq!(finished, vec![EntityKind(1), EntityKind(2), EntityKind(3)]);
    }

    #[test]
    fn the_queue_has_a_limit() {
        let mut queue = ProductionQueue::default();
        for _ in 0..MAX_QUEUE {
            assert!(queue.enqueue(item(10, 1)));
        }
        assert!(!queue.enqueue(item(10, 1)), "the queue should be full");
        assert_eq!(queue.len(), MAX_QUEUE);
    }

    #[test]
    fn an_instant_item_still_gets_paid_for() {
        // A zero-tick duration would otherwise finish before any instalment was
        // due, handing the player a free unit.
        let mut queue = ProductionQueue::default();
        queue.enqueue(ProductionItem::new(EntityKind(1), 500, 0));
        let (spent, ticks) = run_to_completion(&mut queue, u32::MAX);
        assert_eq!(spent, 500);
        assert_eq!(ticks, 1);
    }

    #[test]
    fn a_free_item_builds_without_charge() {
        let mut queue = ProductionQueue::default();
        queue.enqueue(item(0, 10));
        let (spent, ticks) = run_to_completion(&mut queue, 0);
        assert_eq!(spent, 0);
        assert_eq!(ticks, 10, "a free item should still take its build time");
    }

    #[test]
    fn progress_is_reportable() {
        let mut queue = ProductionQueue::default();
        queue.enqueue(item(100, 10));
        assert_eq!(queue.current().unwrap().percent(), 0);
        for _ in 0..5 {
            queue.tick(u32::MAX);
        }
        assert_eq!(queue.current().unwrap().percent(), 50);
    }

    #[test]
    fn an_empty_queue_is_not_starved() {
        // Nothing waiting is not the same as waiting for money, and the
        // interface has to be able to tell them apart.
        let mut queue = ProductionQueue::default();
        queue.tick(0);
        assert!(!queue.starved);
    }

    #[test]
    fn a_queue_hashes_its_whole_state() {
        let hash = |q: &ProductionQueue| {
            let mut h = StateHasher::new();
            h.write(q);
            h.finish()
        };
        let mut a = ProductionQueue::default();
        a.enqueue(item(100, 10));
        let base = hash(&a);

        let mut b = a.clone();
        b.tick(u32::MAX);
        assert_ne!(hash(&b), base, "progress must be visible in the hash");

        let mut c = a.clone();
        c.tick(0);
        assert_ne!(hash(&c), base, "being starved must be visible in the hash");
    }
}
