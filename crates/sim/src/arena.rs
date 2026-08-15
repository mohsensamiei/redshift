//! Entity storage: a generational arena.
//!
//! # Why not an off-the-shelf ECS
//!
//! Bevy's ECS is excellent and is used freely on the rendering side. It is not
//! used here, for two reasons:
//!
//! 1. Its scheduler parallelises systems. Determinism forbids that — see
//!    `docs/adr/0003-deterministic-lockstep.md`.
//! 2. It would couple the simulation to the engine, which
//!    `docs/01-architecture.md` forbids.
//!
//! # Determinism properties
//!
//! - Iteration is always in slot order, never hash order.
//! - Freed slots are reused via a free list in a fixed order, so two peers
//!   allocating the same entities get the same indices.
//! - Generations make stale handles detectable: an [`EntityId`] pointing at a
//!   reused slot resolves to `None` rather than silently addressing whatever
//!   unit now occupies it. Without this, an order targeting a destroyed unit
//!   would retarget itself at that unit's replacement — a subtle, and
//!   desync-prone, bug.

use serde::{Deserialize, Serialize};

/// A handle to an entity.
///
/// Small and `Copy`, so it can live in commands and orders freely. The
/// generation makes it safe to hold across ticks: after the entity dies, the
/// handle stops resolving instead of aliasing a new entity.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct EntityId {
    index: u32,
    generation: u32,
}

impl EntityId {
    /// A handle that never resolves. Useful as "no target".
    pub const NONE: EntityId = EntityId { index: u32::MAX, generation: u32::MAX };

    #[inline]
    pub const fn index(self) -> u32 {
        self.index
    }

    #[inline]
    pub const fn generation(self) -> u32 {
        self.generation
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Slot<T> {
    /// Even when vacant, odd when occupied. Storing it this way means the
    /// occupancy check and the staleness check are the same comparison.
    generation: u32,
    value: Option<T>,
}

/// A generational arena with deterministic allocation order.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Arena<T> {
    slots: Vec<Slot<T>>,
    /// Vacant slot indices, most recently freed first.
    free: Vec<u32>,
    len: usize,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Arena::new()
    }
}

impl<T> Arena<T> {
    pub fn new() -> Arena<T> {
        Arena { slots: Vec::new(), free: Vec::new(), len: 0 }
    }

    pub fn with_capacity(cap: usize) -> Arena<T> {
        Arena { slots: Vec::with_capacity(cap), free: Vec::new(), len: 0 }
    }

    /// Number of live entities.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Highest slot index ever used. Iteration bound, not a live count.
    #[inline]
    pub fn capacity_used(&self) -> usize {
        self.slots.len()
    }

    /// Inserts a value and returns its handle.
    ///
    /// Reuses the most recently freed slot when one is available. This keeps
    /// the arena compact and — because the free list order is itself
    /// deterministic — gives every peer the same index for the same entity.
    pub fn insert(&mut self, value: T) -> EntityId {
        self.len += 1;
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            debug_assert!(slot.value.is_none(), "free list pointed at an occupied slot");
            slot.generation = slot.generation.wrapping_add(1);
            slot.value = Some(value);
            EntityId { index, generation: slot.generation }
        } else {
            let index = self.slots.len() as u32;
            // Generation starts at 1 so that a zeroed EntityId is not
            // accidentally valid.
            self.slots.push(Slot { generation: 1, value: Some(value) });
            EntityId { index, generation: 1 }
        }
    }

    /// Removes an entity, returning its value if the handle was live.
    pub fn remove(&mut self, id: EntityId) -> Option<T> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        let value = slot.value.take()?;
        self.len -= 1;
        self.free.push(id.index);
        Some(value)
    }

    /// `true` if the handle refers to a live entity.
    #[inline]
    pub fn contains(&self, id: EntityId) -> bool {
        self.get(id).is_some()
    }

    #[inline]
    pub fn get(&self, id: EntityId) -> Option<&T> {
        let slot = self.slots.get(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.value.as_ref()
    }

    #[inline]
    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut T> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.value.as_mut()
    }

    /// Iterates live entities in slot order.
    ///
    /// Slot order is the deterministic order. All simulation passes must use
    /// this rather than collecting into some other container first.
    pub fn iter(&self) -> impl Iterator<Item = (EntityId, &T)> + '_ {
        self.slots.iter().enumerate().filter_map(|(i, slot)| {
            slot.value
                .as_ref()
                .map(|v| (EntityId { index: i as u32, generation: slot.generation }, v))
        })
    }

    /// Mutably iterates live entities in slot order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (EntityId, &mut T)> + '_ {
        self.slots.iter_mut().enumerate().filter_map(|(i, slot)| {
            let generation = slot.generation;
            slot.value.as_mut().map(|v| (EntityId { index: i as u32, generation }, v))
        })
    }

    /// Live handles in slot order.
    ///
    /// Collect this before a pass that needs to mutate the arena while walking
    /// it — spawning or destroying entities mid-iteration.
    pub fn ids(&self) -> Vec<EntityId> {
        self.iter().map(|(id, _)| id).collect()
    }

    /// Removes every entity for which `keep` returns `false`.
    ///
    /// Visits in slot order, so the resulting free list — and therefore every
    /// subsequent allocation — is deterministic.
    pub fn retain<F: FnMut(EntityId, &mut T) -> bool>(&mut self, mut keep: F) {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            let Some(value) = slot.value.as_mut() else { continue };
            let id = EntityId { index: i as u32, generation: slot.generation };
            if !keep(id, value) {
                slot.value = None;
                self.free.push(i as u32);
                self.len -= 1;
            }
        }
    }

    pub fn clear(&mut self) {
        self.slots.clear();
        self.free.clear();
        self.len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_get() {
        let mut arena: Arena<i32> = Arena::new();
        let a = arena.insert(10);
        let b = arena.insert(20);
        assert_eq!(arena.get(a), Some(&10));
        assert_eq!(arena.get(b), Some(&20));
        assert_eq!(arena.len(), 2);
    }

    #[test]
    fn remove_invalidates_the_handle() {
        let mut arena: Arena<i32> = Arena::new();
        let a = arena.insert(10);
        assert_eq!(arena.remove(a), Some(10));
        assert_eq!(arena.get(a), None);
        assert!(!arena.contains(a));
        assert_eq!(arena.remove(a), None, "double remove must be a no-op");
        assert_eq!(arena.len(), 0);
    }

    #[test]
    fn stale_handle_does_not_alias_a_reused_slot() {
        // The bug this test exists to prevent: a unit is destroyed, a new unit
        // takes its slot, and an old order silently retargets onto it.
        let mut arena: Arena<&str> = Arena::new();
        let old = arena.insert("destroyed tank");
        arena.remove(old);
        let new = arena.insert("fresh tank");

        assert_eq!(old.index(), new.index(), "slot should have been reused");
        assert_ne!(old.generation(), new.generation());
        assert_eq!(arena.get(old), None, "stale handle must not resolve");
        assert_eq!(arena.get(new), Some(&"fresh tank"));
    }

    #[test]
    fn none_handle_never_resolves() {
        let mut arena: Arena<i32> = Arena::new();
        arena.insert(1);
        assert_eq!(arena.get(EntityId::NONE), None);
        assert!(!arena.contains(EntityId::NONE));
    }

    #[test]
    fn zeroed_handle_never_resolves() {
        let mut arena: Arena<i32> = Arena::new();
        arena.insert(1);
        let zeroed = EntityId { index: 0, generation: 0 };
        assert_eq!(arena.get(zeroed), None, "generation starts at 1 for this reason");
    }

    #[test]
    fn iteration_is_in_slot_order() {
        let mut arena: Arena<i32> = Arena::new();
        for i in 0..10 {
            arena.insert(i);
        }
        let seen: Vec<i32> = arena.iter().map(|(_, v)| *v).collect();
        assert_eq!(seen, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn allocation_order_is_reproducible() {
        // Two arenas driven by the same sequence of operations must produce
        // identical handles. This is what keeps peers agreeing on entity ids.
        fn run() -> Vec<EntityId> {
            let mut arena: Arena<i32> = Arena::new();
            let mut ids = Vec::new();
            for i in 0..20 {
                ids.push(arena.insert(i));
            }
            for i in (0..20).step_by(3) {
                arena.remove(ids[i]);
            }
            let mut fresh = Vec::new();
            for i in 100..110 {
                fresh.push(arena.insert(i));
            }
            fresh
        }
        assert_eq!(run(), run());
    }

    #[test]
    fn retain_keeps_slot_order_and_frees_correctly() {
        let mut arena: Arena<i32> = Arena::new();
        for i in 0..10 {
            arena.insert(i);
        }
        arena.retain(|_, v| *v % 2 == 0);
        assert_eq!(arena.len(), 5);
        let seen: Vec<i32> = arena.iter().map(|(_, v)| *v).collect();
        assert_eq!(seen, vec![0, 2, 4, 6, 8]);

        // Freed slots come back into use in a defined order.
        let a = arena.insert(100);
        let b = arena.insert(200);
        assert_ne!(a.index(), b.index());
        assert_eq!(arena.len(), 7);
    }

    #[test]
    fn mutation_through_iter_mut() {
        let mut arena: Arena<i32> = Arena::new();
        for i in 0..5 {
            arena.insert(i);
        }
        for (_, v) in arena.iter_mut() {
            *v *= 10;
        }
        let seen: Vec<i32> = arena.iter().map(|(_, v)| *v).collect();
        assert_eq!(seen, vec![0, 10, 20, 30, 40]);
    }

    #[test]
    fn serialisation_roundtrips_including_generations() {
        let mut arena: Arena<i32> = Arena::new();
        let a = arena.insert(1);
        arena.insert(2);
        arena.remove(a);
        let c = arena.insert(3);

        let encoded = ron::to_string(&arena).unwrap();
        let decoded: Arena<i32> = ron::from_str(&encoded).unwrap();

        assert_eq!(decoded.len(), arena.len());
        assert_eq!(decoded.get(c), Some(&3));
        assert_eq!(decoded.get(a), None, "stale handles stay stale across a save");
        assert_eq!(
            decoded.iter().map(|(_, v)| *v).collect::<Vec<_>>(),
            arena.iter().map(|(_, v)| *v).collect::<Vec<_>>()
        );
    }
}
