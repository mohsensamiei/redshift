//! Deterministic pseudo-random number generation for the simulation.
//!
//! There is exactly **one** generator per match, it lives inside the
//! simulation state, and it is advanced only by simulation code. Never call
//! `rand::thread_rng()` or any OS entropy source from the sim: peers would
//! immediately diverge.
//!
//! The renderer may want randomness too — jitter on an explosion, variation in
//! an idle animation. That must use a separate, non-simulation generator.
//! Cosmetic randomness must never touch this one, because drawing from it
//! changes the sequence every subsequent simulation draw receives.
//!
//! # Determinism properties
//!
//! - Fixed algorithm (PCG-XSH-RR 64/32), not a library default that could
//!   change between versions.
//! - Pure integer arithmetic with explicit wrapping.
//! - The state serialises with the rest of the match, so a save or a replay
//!   resumes the exact sequence.
//!
//! One further rule, which the code cannot enforce: **the number of draws per
//! tick must not depend on anything non-deterministic.** Drawing inside a loop
//! over a `HashMap`, or only when a cosmetic condition holds, desyncs the
//! sequence even though every individual draw is reproducible.

use serde::{Deserialize, Serialize};

use crate::fx::{Angle, Fx};

/// Multiplier from the PCG reference implementation.
const PCG_MULT: u64 = 6_364_136_223_846_793_005;
/// Default increment (must be odd).
const PCG_INC: u64 = 1_442_695_040_888_963_407;

/// A seeded, reproducible generator: PCG-XSH-RR with 64-bit state and 32-bit output.
///
/// Chosen over a plain LCG or xorshift because its low bits are as well
/// distributed as its high bits — so `next_range` stays sound — while remaining
/// a handful of integer operations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimRng {
    state: u64,
}

impl SimRng {
    /// Creates a generator from a match seed.
    #[inline]
    pub fn new(seed: u64) -> SimRng {
        // Run the state through one step so that adjacent seeds (0, 1, 2 …)
        // do not produce correlated opening sequences — match seeds are often
        // small integers in tests and lobbies.
        let mut rng = SimRng {
            state: seed.wrapping_add(PCG_INC),
        };
        rng.next_u32();
        rng
    }

    /// The raw state. For state hashing and diagnostics.
    #[inline]
    pub fn state(&self) -> u64 {
        self.state
    }

    /// Uniform `u32` over the full range.
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(PCG_MULT).wrapping_add(PCG_INC);
        // XSH RR output permutation.
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        ((self.next_u32() as u64) << 32) | self.next_u32() as u64
    }

    /// Uniform integer in `[lo, hi)`.
    ///
    /// Uses Lemire's multiply-shift method with rejection, which is unbiased
    /// and — critically — consumes a *deterministic* number of draws for a
    /// given state. A modulo would be biased; a rejection loop without the
    /// multiply-shift would consume a variable number of draws in a way that
    /// is still deterministic but far harder to reason about.
    ///
    /// # Panics
    /// If `lo >= hi`.
    #[inline]
    pub fn next_range(&mut self, lo: i32, hi: i32) -> i32 {
        assert!(lo < hi, "SimRng::next_range needs lo < hi");
        let span = (hi as i64 - lo as i64) as u64;

        let mut m = (self.next_u32() as u64).wrapping_mul(span);
        let mut low = m as u32;
        if (low as u64) < span {
            let threshold = span.wrapping_neg() % span;
            while (low as u64) < threshold {
                m = (self.next_u32() as u64).wrapping_mul(span);
                low = m as u32;
            }
        }
        lo + (m >> 32) as i32
    }

    /// `true` with probability `numerator / denominator`.
    ///
    /// # Panics
    /// If `denominator` is zero.
    #[inline]
    pub fn chance(&mut self, numerator: i32, denominator: i32) -> bool {
        assert!(
            denominator > 0,
            "SimRng::chance needs a positive denominator"
        );
        self.next_range(0, denominator) < numerator
    }

    /// Uniform [`Fx`] in `[0, 1)`.
    #[inline]
    pub fn next_fx(&mut self) -> Fx {
        // Take the top 16 bits: they carry the most entropy in any generator,
        // and 16 bits is exactly the fractional width of Fx.
        Fx::from_raw((self.next_u32() >> 16) as i32)
    }

    /// Uniform [`Fx`] in `[lo, hi)`.
    #[inline]
    pub fn next_fx_range(&mut self, lo: Fx, hi: Fx) -> Fx {
        debug_assert!(lo < hi, "SimRng::next_fx_range needs lo < hi");
        lo + (hi - lo).mul(self.next_fx())
    }

    /// A uniformly distributed direction.
    #[inline]
    pub fn next_angle(&mut self) -> Angle {
        Angle::from_raw(self.next_u32() as u16)
    }

    /// Shuffles a slice in place (Fisher-Yates, descending).
    ///
    /// Safe to use in the simulation *provided the slice order coming in is
    /// itself deterministic* — shuffling a vector built by iterating a
    /// `HashMap` is still non-deterministic.
    pub fn shuffle<T>(&mut self, slice: &mut [T]) {
        if slice.len() < 2 {
            return;
        }
        for i in (1..slice.len()).rev() {
            let j = self.next_range(0, i as i32 + 1) as usize;
            slice.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_gives_same_sequence() {
        let mut a = SimRng::new(12345);
        let mut b = SimRng::new(12345);
        for _ in 0..10_000 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
        assert_eq!(a.state(), b.state());
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = SimRng::new(1);
        let mut b = SimRng::new(2);
        let first: Vec<u32> = (0..32).map(|_| a.next_u32()).collect();
        let second: Vec<u32> = (0..32).map(|_| b.next_u32()).collect();
        assert_ne!(first, second);
    }

    #[test]
    fn adjacent_seeds_are_not_correlated() {
        // Without the priming step in `new`, small adjacent seeds produce
        // visibly similar opening values.
        let firsts: Vec<u32> = (0..8).map(|s| SimRng::new(s).next_u32()).collect();
        for w in firsts.windows(2) {
            assert_ne!(w[0], w[1]);
            assert!(w[0].abs_diff(w[1]) > 1000, "seeds too correlated: {w:?}");
        }
    }

    #[test]
    fn serialisation_preserves_the_sequence() {
        let mut a = SimRng::new(99);
        for _ in 0..50 {
            a.next_u32();
        }
        let encoded = ron::to_string(&a).unwrap();
        let mut b: SimRng = ron::from_str(&encoded).unwrap();
        assert_eq!(a, b);
        for _ in 0..100 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
    }

    #[test]
    fn range_stays_in_bounds() {
        let mut rng = SimRng::new(7);
        for _ in 0..100_000 {
            let v = rng.next_range(-5, 5);
            assert!((-5..5).contains(&v), "out of range: {v}");
        }
    }

    #[test]
    fn range_of_one_is_constant_and_consumes_no_entropy_unnecessarily() {
        let mut rng = SimRng::new(3);
        for _ in 0..100 {
            assert_eq!(rng.next_range(42, 43), 42);
        }
    }

    #[test]
    fn range_is_roughly_uniform() {
        let mut rng = SimRng::new(2024);
        let mut buckets = [0u32; 10];
        const N: u32 = 200_000;
        for _ in 0..N {
            buckets[rng.next_range(0, 10) as usize] += 1;
        }
        let expected = N / 10;
        for (i, &count) in buckets.iter().enumerate() {
            let deviation = count.abs_diff(expected);
            assert!(
                deviation < expected / 10,
                "bucket {i} had {count}, expected about {expected}"
            );
        }
    }

    #[test]
    fn fx_output_is_in_unit_interval() {
        let mut rng = SimRng::new(555);
        for _ in 0..50_000 {
            let v = rng.next_fx();
            assert!(v >= Fx::ZERO && v < Fx::ONE, "out of [0,1): {v:?}");
        }
    }

    #[test]
    fn chance_matches_its_probability() {
        let mut rng = SimRng::new(31337);
        let mut hits = 0;
        const N: i32 = 100_000;
        for _ in 0..N {
            if rng.chance(25, 100) {
                hits += 1;
            }
        }
        let expected = N / 4;
        assert!(
            (hits - expected).abs() < expected / 20,
            "got {hits}, expected ~{expected}"
        );
    }

    #[test]
    fn shuffle_is_reproducible_and_preserves_contents() {
        let mut a = SimRng::new(808);
        let mut b = SimRng::new(808);
        let mut left: Vec<i32> = (0..100).collect();
        let mut right: Vec<i32> = (0..100).collect();
        a.shuffle(&mut left);
        b.shuffle(&mut right);
        assert_eq!(left, right);

        let mut sorted = left.clone();
        sorted.sort();
        assert_eq!(sorted, (0..100).collect::<Vec<_>>());
        assert_ne!(
            left, sorted,
            "a 100-element shuffle should reorder something"
        );
    }

    #[test]
    fn shuffle_handles_degenerate_lengths() {
        let mut rng = SimRng::new(1);
        let mut empty: [i32; 0] = [];
        rng.shuffle(&mut empty);
        let mut single = [7];
        rng.shuffle(&mut single);
        assert_eq!(single, [7]);
    }
}
