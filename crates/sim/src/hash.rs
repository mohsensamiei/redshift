//! Stable hashing for simulation state.
//!
//! # Why not `DefaultHasher`
//!
//! `std::collections::hash_map::DefaultHasher` is deterministic within a single
//! build, but its output is explicitly **not** guaranteed to be stable across
//! Rust versions. Peers in a match may be running binaries built with different
//! toolchains; a hash that changed between them would report a desync on every
//! comparison while the simulations were in perfect agreement.
//!
//! So we specify the algorithm ourselves: FNV-1a, 64-bit. It is a few integer
//! operations, has no seed, and will produce the same bytes in ten years.
//!
//! # What it is for
//!
//! Comparing whole-world state between peers once per second, to detect
//! divergence early. It is *not* a cryptographic hash and must not be used for
//! anything security-related — an opponent who wants to forge a matching hash
//! can. Detecting accidental divergence is the entire job.
//!
//! See `docs/03-networking.md`.

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// An order-sensitive, version-stable hasher.
///
/// Order sensitivity is deliberate: two peers whose unit lists are permuted
/// relative to each other *have* diverged, even if the contents match, because
/// iteration order drives every subsequent simulation decision.
#[derive(Clone, Debug)]
pub struct StateHasher {
    state: u64,
}

impl Default for StateHasher {
    fn default() -> Self {
        StateHasher::new()
    }
}

impl StateHasher {
    #[inline]
    pub const fn new() -> StateHasher {
        StateHasher { state: FNV_OFFSET }
    }

    #[inline]
    pub fn write_u8(&mut self, v: u8) {
        self.state ^= v as u64;
        self.state = self.state.wrapping_mul(FNV_PRIME);
    }

    #[inline]
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u8(b);
        }
    }

    #[inline]
    pub fn write_u16(&mut self, v: u16) {
        self.write_bytes(&v.to_le_bytes());
    }

    #[inline]
    pub fn write_u32(&mut self, v: u32) {
        self.write_bytes(&v.to_le_bytes());
    }

    #[inline]
    pub fn write_u64(&mut self, v: u64) {
        self.write_bytes(&v.to_le_bytes());
    }

    #[inline]
    pub fn write_i32(&mut self, v: i32) {
        // Little-endian explicitly, so a big-endian peer would still agree.
        self.write_bytes(&v.to_le_bytes());
    }

    #[inline]
    pub fn write_bool(&mut self, v: bool) {
        self.write_u8(v as u8);
    }

    /// Folds in a value that knows how to hash itself.
    #[inline]
    pub fn write<T: StateHash>(&mut self, value: &T) {
        value.state_hash(self);
    }

    #[inline]
    pub fn finish(&self) -> u64 {
        self.state
    }
}

/// State that contributes to the whole-world hash.
///
/// Implement this only for data that affects gameplay. Cosmetic values —
/// animation timers, particle seeds, anything the renderer owns — must be left
/// out, or peers will report desyncs over differences that cannot affect the
/// outcome.
pub trait StateHash {
    fn state_hash(&self, hasher: &mut StateHasher);
}

impl StateHash for u32 {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_u32(*self);
    }
}

impl StateHash for u64 {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_u64(*self);
    }
}

impl StateHash for i32 {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_i32(*self);
    }
}

impl StateHash for bool {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_bool(*self);
    }
}

impl<T: StateHash> StateHash for Option<T> {
    fn state_hash(&self, h: &mut StateHasher) {
        match self {
            // The tag matters: `None` and `Some(0)` must not collide.
            None => h.write_u8(0),
            Some(v) => {
                h.write_u8(1);
                v.state_hash(h);
            }
        }
    }
}

impl<T: StateHash> StateHash for [T] {
    fn state_hash(&self, h: &mut StateHasher) {
        // Length first, so [a] and [a, a] differ even under a weak element hash.
        h.write_u32(self.len() as u32);
        for item in self {
            item.state_hash(h);
        }
    }
}

impl<T: StateHash> StateHash for Vec<T> {
    fn state_hash(&self, h: &mut StateHasher) {
        self.as_slice().state_hash(h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash_of<T: StateHash>(v: &T) -> u64 {
        let mut h = StateHasher::new();
        h.write(v);
        h.finish()
    }

    #[test]
    fn empty_hash_is_the_offset_basis() {
        assert_eq!(StateHasher::new().finish(), FNV_OFFSET);
    }

    #[test]
    fn known_vector_pins_the_algorithm() {
        // FNV-1a of "redshift". If this ever changes, the algorithm changed and
        // every peer running an older build will report false desyncs.
        let mut h = StateHasher::new();
        h.write_bytes(b"redshift");
        assert_eq!(h.finish(), 0x149d_8f80_e4d6_a638);
    }

    #[test]
    fn identical_input_gives_identical_output() {
        let a = hash_of(&vec![1u32, 2, 3]);
        let b = hash_of(&vec![1u32, 2, 3]);
        assert_eq!(a, b);
    }

    #[test]
    fn order_matters() {
        // Two peers with permuted entity lists have diverged, even though the
        // contents are equal.
        assert_ne!(hash_of(&vec![1u32, 2, 3]), hash_of(&vec![3u32, 2, 1]));
    }

    #[test]
    fn length_is_folded_in() {
        // Without the length prefix these could collide under concatenation.
        assert_ne!(hash_of(&vec![0u32]), hash_of(&vec![0u32, 0]));
        assert_ne!(hash_of(&Vec::<u32>::new()), hash_of(&vec![0u32]));
    }

    #[test]
    fn option_tag_prevents_collision() {
        assert_ne!(hash_of(&None::<u32>), hash_of(&Some(0u32)));
        assert_ne!(hash_of(&Some(0u32)), hash_of(&Some(1u32)));
    }

    #[test]
    fn single_bit_changes_propagate() {
        // A one-unit position change must not be lost. This is the property the
        // desync detector depends on.
        let base = hash_of(&vec![1000u32, 2000, 3000]);
        for i in 0..3 {
            let mut v = vec![1000u32, 2000, 3000];
            v[i] += 1;
            assert_ne!(hash_of(&v), base, "a change at index {i} vanished");
        }
    }

    #[test]
    fn negative_integers_are_distinguished() {
        assert_ne!(hash_of(&-1i32), hash_of(&1i32));
        assert_ne!(hash_of(&0i32), hash_of(&-1i32));
        assert_ne!(hash_of(&i32::MIN), hash_of(&i32::MAX));
    }
}
