//! Fixed-point arithmetic for the simulation.
//!
//! # Why this exists
//!
//! Multiplayer is deterministic lockstep: every peer simulates the whole match
//! locally and must reach a bit-identical result. Floating point cannot be
//! relied upon for that — results vary with architecture, compiler and
//! optimisation level, and we explicitly target both ARM (Apple Silicon) and
//! x86. A single `f32` in a simulation path desyncs matches, typically several
//! minutes after the actual divergence.
//!
//! So the simulation uses [`Fx`]: a signed 32-bit integer with 16 fractional
//! bits, where `1.0` means one map cell.
//!
//! | Property | Value |
//! |---|---|
//! | Resolution | 1/65536 of a cell |
//! | Range | ±32768 cells |
//!
//! # Rules
//!
//! - Multiplication and division go through `i64` intermediates. Two `Fx`
//!   values multiplied as `i32` overflow at surprisingly small magnitudes.
//! - There is deliberately **no** `From<f32>`. Converting from a float is legal
//!   only in the renderer, and only in the other direction.
//! - Prefer [`Fx::dist_sq`] over [`Fx::dist`]; comparing squared distances
//!   avoids a square root entirely.
//!
//! See `docs/02-simulation.md`.

use core::fmt;
use core::ops::{Add, AddAssign, Div, Mul, Neg, Rem, Sub, SubAssign};

use serde::{Deserialize, Serialize};

use crate::trig_table::{SIN_TABLE, SIN_TABLE_LEN};

/// Number of fractional bits in [`Fx`].
pub const FRAC_BITS: u32 = 16;

/// Raw value of `Fx::ONE`.
const ONE_RAW: i32 = 1 << FRAC_BITS;

/// A fixed-point scalar: `i32` with 16 fractional bits. `1.0` is one map cell.
///
/// Ordering, equality and hashing are the integer ones, so they are exact and
/// identical on every platform — unlike floats, where `NaN` breaks `Ord` and
/// `-0.0 == 0.0` complicates hashing.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[repr(transparent)]
#[serde(transparent)]
pub struct Fx(i32);

impl Fx {
    pub const ZERO: Fx = Fx(0);
    pub const ONE: Fx = Fx(ONE_RAW);
    pub const HALF: Fx = Fx(ONE_RAW / 2);
    pub const MIN: Fx = Fx(i32::MIN);
    pub const MAX: Fx = Fx(i32::MAX);

    /// The smallest representable positive value: 1/65536 of a cell.
    pub const EPSILON: Fx = Fx(1);

    /// Wraps a raw fixed-point value.
    ///
    /// Prefer [`Fx::from_int`] or [`Fx::from_frac`]; this is for serialisation
    /// and for the generated tables.
    #[inline]
    pub const fn from_raw(raw: i32) -> Fx {
        Fx(raw)
    }

    /// The underlying integer. Only for serialisation, hashing and tests.
    #[inline]
    pub const fn raw(self) -> i32 {
        self.0
    }

    /// Exact conversion from a whole number of cells.
    ///
    /// # Panics
    /// In debug builds, if `n` is outside ±32768.
    #[inline]
    pub const fn from_int(n: i32) -> Fx {
        debug_assert!(n >= -32768 && n <= 32767, "Fx::from_int out of range");
        Fx(n << FRAC_BITS)
    }

    /// Exact conversion from the rational `num / den`.
    ///
    /// This is how fractional constants are written — `Fx::from_frac(3, 2)`
    /// rather than a float literal, which the crate forbids.
    ///
    /// # Panics
    /// If `den` is zero.
    #[inline]
    pub const fn from_frac(num: i32, den: i32) -> Fx {
        assert!(den != 0, "Fx::from_frac divide by zero");
        Fx((((num as i64) << FRAC_BITS) / den as i64) as i32)
    }

    /// Truncates toward zero, discarding the fractional part.
    #[inline]
    pub const fn to_int(self) -> i32 {
        self.0 >> FRAC_BITS
    }

    /// Rounds down toward negative infinity.
    ///
    /// This is the correct choice for mapping a world position to a grid cell:
    /// unlike [`Fx::to_int`], it does not fold `-0.5` and `0.5` onto the same
    /// cell, which would make the origin two cells wide.
    #[inline]
    pub const fn floor_int(self) -> i32 {
        self.0 >> FRAC_BITS
    }

    /// Rounds to the nearest whole number, halfway cases away from zero.
    #[inline]
    pub const fn round_int(self) -> i32 {
        if self.0 >= 0 {
            (self.0 + ONE_RAW / 2) >> FRAC_BITS
        } else {
            -((-self.0 + ONE_RAW / 2) >> FRAC_BITS)
        }
    }

    /// The fractional part, always in `[0, 1)` — even for negative values.
    #[inline]
    pub const fn frac(self) -> Fx {
        Fx(self.0 & (ONE_RAW - 1))
    }

    #[inline]
    pub const fn abs(self) -> Fx {
        Fx(self.0.wrapping_abs())
    }

    #[inline]
    pub const fn signum(self) -> i32 {
        if self.0 > 0 {
            1
        } else if self.0 < 0 {
            -1
        } else {
            0
        }
    }

    #[inline]
    pub fn min(self, other: Fx) -> Fx {
        if self.0 < other.0 { self } else { other }
    }

    #[inline]
    pub fn max(self, other: Fx) -> Fx {
        if self.0 > other.0 { self } else { other }
    }

    #[inline]
    pub fn clamp(self, lo: Fx, hi: Fx) -> Fx {
        debug_assert!(lo.0 <= hi.0, "Fx::clamp inverted bounds");
        self.max(lo).min(hi)
    }

    /// Saturating multiply. Overflow clamps to [`Fx::MIN`]/[`Fx::MAX`] rather
    /// than wrapping, so a runaway value degrades predictably instead of
    /// flipping sign — and does so identically on every peer.
    #[inline]
    pub const fn mul(self, rhs: Fx) -> Fx {
        let wide = ((self.0 as i64) * (rhs.0 as i64)) >> FRAC_BITS;
        Fx(saturate(wide))
    }

    /// Saturating divide.
    ///
    /// # Panics
    /// If `rhs` is zero. Division by zero is a bug, not a case to handle:
    /// silently returning a sentinel would hide the defect until it desynced a
    /// match.
    #[inline]
    pub const fn div(self, rhs: Fx) -> Fx {
        assert!(rhs.0 != 0, "Fx::div divide by zero");
        let wide = ((self.0 as i64) << FRAC_BITS) / (rhs.0 as i64);
        Fx(saturate(wide))
    }

    /// Multiplies by a plain integer.
    #[inline]
    pub const fn mul_int(self, n: i32) -> Fx {
        Fx(saturate((self.0 as i64) * (n as i64)))
    }

    /// Divides by a plain integer, truncating toward zero.
    ///
    /// # Panics
    /// If `n` is zero.
    #[inline]
    pub const fn div_int(self, n: i32) -> Fx {
        assert!(n != 0, "Fx::div_int divide by zero");
        Fx(((self.0 as i64) / (n as i64)) as i32)
    }

    /// `self * num / den`, keeping full precision in the intermediate.
    ///
    /// Use this for percentage-style modifiers — armour multipliers, cost
    /// modifiers — where doing the multiply and divide separately would lose
    /// low bits, and where those lost bits would differ between peers if the
    /// operations were ever reordered.
    ///
    /// # Panics
    /// If `den` is zero.
    #[inline]
    pub const fn mul_ratio(self, num: i32, den: i32) -> Fx {
        assert!(den != 0, "Fx::mul_ratio divide by zero");
        Fx(saturate((self.0 as i64) * (num as i64) / (den as i64)))
    }

    /// Square root. Exact floor semantics, integer-only.
    ///
    /// # Panics
    /// If `self` is negative.
    #[inline]
    pub fn sqrt(self) -> Fx {
        assert!(self.0 >= 0, "Fx::sqrt of a negative value");
        // sqrt(x / 2^16) * 2^16 == sqrt(x * 2^16). `isqrt` is exact integer
        // floor-sqrt, so this is deterministic by specification.
        Fx(((self.0 as u64) << FRAC_BITS).isqrt() as i32)
    }

    /// Linear interpolation. `t` is clamped to `[0, 1]`.
    #[inline]
    pub fn lerp(self, to: Fx, t: Fx) -> Fx {
        let t = t.clamp(Fx::ZERO, Fx::ONE);
        self + (to - self).mul(t)
    }

    /// Euclidean distance between two points.
    ///
    /// Prefer comparing [`Fx::dist_sq`] against [`Fx::sq`] when you only need
    /// to know which of two distances is larger — it avoids the square root.
    #[inline]
    pub fn dist(dx: Fx, dy: Fx) -> Fx {
        Fx::dist_sq(dx, dy).sqrt()
    }

    /// Squared distance between two points, as a [`FxWide`].
    ///
    /// Returns the wide type rather than [`Fx`] deliberately. A squared
    /// distance exceeds the `Fx` range beyond about 181 cells of separation,
    /// and saturating there would make every far-away target compare *equal* —
    /// so "nearest enemy" searches across a large map would pick arbitrarily.
    /// The wide type has no such limit at any plausible map size.
    ///
    /// Compare against [`Fx::sq`]:
    ///
    /// ```
    /// # use redshift_sim::fx::Fx;
    /// let (dx, dy) = (Fx::from_int(3), Fx::from_int(4));
    /// let range = Fx::from_int(6);
    /// assert!(Fx::dist_sq(dx, dy) <= range.sq());
    /// ```
    #[inline]
    pub fn dist_sq(dx: Fx, dy: Fx) -> FxWide {
        dx.sq().add(dy.sq())
    }

    /// This value squared, without loss of range.
    #[inline]
    pub const fn sq(self) -> FxWide {
        let v = self.0 as i64;
        FxWide((v * v) >> FRAC_BITS)
    }
}

/// A fixed-point value with 16 fractional bits held in an `i64`.
///
/// Exists for intermediate results that overflow [`Fx`] — squared distances
/// above all. Deliberately minimal: it is a comparison and accumulation type,
/// not a general-purpose scalar. Convert back with [`FxWide::sqrt`] or
/// [`FxWide::narrow`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[repr(transparent)]
#[serde(transparent)]
pub struct FxWide(i64);

impl FxWide {
    pub const ZERO: FxWide = FxWide(0);

    #[inline]
    pub const fn from_raw(raw: i64) -> FxWide {
        FxWide(raw)
    }

    #[inline]
    pub const fn raw(self) -> i64 {
        self.0
    }

    #[inline]
    pub const fn from_fx(v: Fx) -> FxWide {
        FxWide(v.0 as i64)
    }

    #[inline]
    pub const fn add(self, rhs: FxWide) -> FxWide {
        FxWide(self.0 + rhs.0)
    }

    #[inline]
    pub const fn sub(self, rhs: FxWide) -> FxWide {
        FxWide(self.0 - rhs.0)
    }

    /// Narrows back to [`Fx`], saturating if out of range.
    #[inline]
    pub const fn narrow(self) -> Fx {
        Fx(saturate(self.0))
    }

    /// Square root, narrowing to [`Fx`]. Exact floor semantics.
    ///
    /// # Panics
    /// If negative.
    #[inline]
    pub fn sqrt(self) -> Fx {
        assert!(self.0 >= 0, "FxWide::sqrt of a negative value");
        Fx(saturate((((self.0 as u64) << FRAC_BITS).isqrt()) as i64))
    }
}

impl Add for FxWide {
    type Output = FxWide;
    #[inline]
    fn add(self, rhs: FxWide) -> FxWide {
        FxWide::add(self, rhs)
    }
}

impl Sub for FxWide {
    type Output = FxWide;
    #[inline]
    fn sub(self, rhs: FxWide) -> FxWide {
        FxWide::sub(self, rhs)
    }
}

impl core::iter::Sum for FxWide {
    fn sum<I: Iterator<Item = FxWide>>(iter: I) -> FxWide {
        iter.fold(FxWide::ZERO, |a, b| a + b)
    }
}

impl fmt::Debug for FxWide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let neg = self.0 < 0;
        let mag = self.0.unsigned_abs();
        let whole = mag >> FRAC_BITS;
        let frac = ((mag & (ONE_RAW as u64 - 1)) * 1_000_000) >> FRAC_BITS;
        let sign = if neg { "-" } else { "" };
        write!(f, "{sign}{whole}.{frac:06}w")
    }
}

/// Clamps a wide intermediate into `i32`, saturating rather than wrapping.
#[inline]
const fn saturate(wide: i64) -> i32 {
    if wide > i32::MAX as i64 {
        i32::MAX
    } else if wide < i32::MIN as i64 {
        i32::MIN
    } else {
        wide as i32
    }
}

// ---------------------------------------------------------------------------
// Angles
// ---------------------------------------------------------------------------

/// A binary angle: a full turn is exactly 65536 units.
///
/// Using a `u16` means angles wrap naturally on overflow with no normalisation
/// step, and there is no irrational constant anywhere — so no rounding
/// disagreement between peers. Turning is plain integer addition.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[repr(transparent)]
#[serde(transparent)]
pub struct Angle(u16);

impl Angle {
    pub const ZERO: Angle = Angle(0);
    pub const QUARTER: Angle = Angle(16384);
    pub const HALF: Angle = Angle(32768);

    #[inline]
    pub const fn from_raw(raw: u16) -> Angle {
        Angle(raw)
    }

    #[inline]
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// From degrees. Exact for multiples of 45°; rounds otherwise.
    #[inline]
    pub const fn from_degrees(deg: i32) -> Angle {
        Angle((((deg as i64) * 65536 + 180) / 360) as u16)
    }

    #[inline]
    pub const fn to_degrees(self) -> i32 {
        ((self.0 as i64 * 360) / 65536) as i32
    }

    /// Shortest signed difference from `self` to `other`, in `[-32768, 32767]`.
    ///
    /// Wrapping subtraction on `u16` reinterpreted as `i16` gives the short way
    /// round for free — no branching on which direction is closer, and so no
    /// tie-breaking to get wrong.
    #[inline]
    pub const fn delta(self, other: Angle) -> i32 {
        (other.0.wrapping_sub(self.0)) as i16 as i32
    }

    /// Rotates toward `target` by at most `max_step` units.
    ///
    /// This is the primitive behind unit and turret turn rates.
    #[inline]
    pub const fn rotate_toward(self, target: Angle, max_step: u16) -> Angle {
        let d = self.delta(target);
        let step = max_step as i32;
        if d.abs() <= step {
            target
        } else if d > 0 {
            Angle(self.0.wrapping_add(max_step))
        } else {
            Angle(self.0.wrapping_sub(max_step))
        }
    }

    /// Sine, from the committed lookup table.
    ///
    /// The low 4 bits of the angle are truncated — the table holds 4096 entries
    /// for a full turn, a resolution of about 0.088°, which is far finer than
    /// any gameplay decision depends on.
    #[inline]
    pub fn sin(self) -> Fx {
        let idx = (self.0 as usize >> 4) & (SIN_TABLE_LEN - 1);
        Fx(SIN_TABLE[idx])
    }

    /// Cosine, as sine a quarter turn ahead.
    #[inline]
    pub fn cos(self) -> Fx {
        Angle(self.0.wrapping_add(Angle::QUARTER.0)).sin()
    }
}

impl fmt::Debug for Angle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}°", self.to_degrees())
    }
}

// ---------------------------------------------------------------------------
// Operators
// ---------------------------------------------------------------------------

// Addition and subtraction use the checked-in-debug, wrapping-in-release
// behaviour of plain `i32` arithmetic. `Mul` and `Div` intentionally route
// through the saturating methods above rather than the raw integer operators,
// so that `a * b` in sim code always means fixed-point multiply.

impl Add for Fx {
    type Output = Fx;
    #[inline]
    fn add(self, rhs: Fx) -> Fx {
        Fx(self.0 + rhs.0)
    }
}

impl Sub for Fx {
    type Output = Fx;
    #[inline]
    fn sub(self, rhs: Fx) -> Fx {
        Fx(self.0 - rhs.0)
    }
}

impl Mul for Fx {
    type Output = Fx;
    #[inline]
    fn mul(self, rhs: Fx) -> Fx {
        Fx::mul(self, rhs)
    }
}

impl Div for Fx {
    type Output = Fx;
    #[inline]
    fn div(self, rhs: Fx) -> Fx {
        Fx::div(self, rhs)
    }
}

impl Rem for Fx {
    type Output = Fx;
    #[inline]
    fn rem(self, rhs: Fx) -> Fx {
        assert!(rhs.0 != 0, "Fx::rem divide by zero");
        Fx(self.0 % rhs.0)
    }
}

impl Neg for Fx {
    type Output = Fx;
    #[inline]
    fn neg(self) -> Fx {
        Fx(-self.0)
    }
}

impl AddAssign for Fx {
    #[inline]
    fn add_assign(&mut self, rhs: Fx) {
        self.0 += rhs.0;
    }
}

impl SubAssign for Fx {
    #[inline]
    fn sub_assign(&mut self, rhs: Fx) {
        self.0 -= rhs.0;
    }
}

impl core::iter::Sum for Fx {
    fn sum<I: Iterator<Item = Fx>>(iter: I) -> Fx {
        iter.fold(Fx::ZERO, |a, b| a + b)
    }
}

impl fmt::Debug for Fx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Render as a decimal without touching floating point: the fractional
        // part is scaled into millionths by integer arithmetic.
        let neg = self.0 < 0;
        let mag = (self.0 as i64).unsigned_abs();
        let whole = mag >> FRAC_BITS;
        let frac = ((mag & (ONE_RAW as u64 - 1)) * 1_000_000) >> FRAC_BITS;
        let sign = if neg { "-" } else { "" };
        write!(f, "{sign}{whole}.{frac:06}")
    }
}

impl fmt::Display for Fx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// Shorthand for whole-number [`Fx`] literals in sim code and tests.
#[macro_export]
macro_rules! fx {
    ($n:expr) => {
        $crate::fx::Fx::from_int($n)
    };
    ($num:expr, $den:expr) => {
        $crate::fx::Fx::from_frac($num, $den)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction_and_roundtrip() {
        assert_eq!(Fx::from_int(0), Fx::ZERO);
        assert_eq!(Fx::from_int(1), Fx::ONE);
        assert_eq!(Fx::from_int(5).to_int(), 5);
        assert_eq!(Fx::from_int(-5).to_int(), -5);
        assert_eq!(Fx::from_frac(1, 2), Fx::HALF);
        assert_eq!(Fx::from_frac(-3, 2), -Fx::from_frac(3, 2));
        assert_eq!(Fx::from_raw(Fx::ONE.raw()), Fx::ONE);
    }

    #[test]
    fn add_sub_are_exact() {
        let a = Fx::from_frac(1, 3);
        let b = Fx::from_frac(2, 3);
        // Exactness matters: repeated accumulation must not drift, or two peers
        // taking different code paths to the same total would disagree.
        assert_eq!(a + b - b, a);
        let mut acc = Fx::ZERO;
        for _ in 0..1000 {
            acc += a;
        }
        assert_eq!(acc, a.mul_int(1000));
    }

    #[test]
    fn mul_div_roundtrip() {
        let a = Fx::from_int(7);
        let b = Fx::from_int(3);
        assert_eq!(a.mul(b), Fx::from_int(21));
        assert_eq!(Fx::from_int(21).div(b), a);
        assert_eq!(Fx::ONE.mul(a), a);
        assert_eq!(a.div(Fx::ONE), a);
        assert_eq!(Fx::HALF.mul(Fx::HALF), Fx::from_frac(1, 4));
    }

    #[test]
    fn mul_uses_wide_intermediate() {
        // In raw terms this is 9_830_400 * 13_107_200, which overflows i32 by
        // six orders of magnitude. This is the exact bug the i64 intermediate
        // exists to prevent — and the result still fits comfortably in Fx.
        assert_eq!(Fx::from_int(150).mul(Fx::from_int(200)), Fx::from_int(30_000));
        assert_eq!(Fx::from_int(-150).mul(Fx::from_int(200)), Fx::from_int(-30_000));
    }

    #[test]
    fn mul_saturates_rather_than_wrapping() {
        let big = Fx::from_int(30000);
        assert_eq!(big.mul(big), Fx::MAX);
        assert_eq!(big.mul(-big), Fx::MIN);
    }

    #[test]
    fn negative_rounding_is_symmetric() {
        assert_eq!(Fx::from_frac(3, 2).round_int(), 2);
        assert_eq!(Fx::from_frac(-3, 2).round_int(), -2);
        assert_eq!(Fx::from_frac(1, 4).round_int(), 0);
        assert_eq!(Fx::from_frac(-1, 4).round_int(), 0);
    }

    #[test]
    fn floor_and_frac_agree_for_negatives() {
        // The invariant grid code depends on: floor + frac reconstructs the
        // original, on both sides of zero.
        for raw in [-200_000i32, -65_537, -65_536, -1, 0, 1, 65_536, 200_000] {
            let v = Fx::from_raw(raw);
            let rebuilt = Fx::from_int(v.floor_int()) + v.frac();
            assert_eq!(rebuilt, v, "floor/frac mismatch for raw {raw}");
            assert!(v.frac() >= Fx::ZERO && v.frac() < Fx::ONE);
        }
    }

    #[test]
    fn sqrt_is_exact_for_squares() {
        // n² must stay within Fx range, so n ≤ 181.
        for n in [0i32, 1, 2, 3, 4, 5, 10, 12, 25, 50, 100, 144, 181] {
            assert_eq!(Fx::from_int(n * n).sqrt(), Fx::from_int(n), "sqrt of {n}²");
        }
    }

    #[test]
    fn sqrt_obeys_the_floor_contract() {
        // The defining property: r is the largest value whose square does not
        // exceed x. Checked in exact integer arithmetic rather than through
        // `sq()`, which floors its own result — composing two floors would make
        // the test unable to distinguish "correct" from "one unit low".
        let mut rng = crate::rng::SimRng::new(4242);
        for _ in 0..20_000 {
            let x = Fx::from_raw(rng.next_range(0, i32::MAX));
            let r = x.sqrt().raw() as i64;
            let scaled = (x.raw() as i64) << FRAC_BITS;
            assert!(r * r <= scaled, "sqrt({x:?}) is too large");
            assert!((r + 1) * (r + 1) > scaled, "sqrt({x:?}) is too small");
        }
    }

    #[test]
    fn sq_floors_consistently() {
        // `sq` discards the low fractional bits, like every Fx multiply. That
        // is expected — but it must never round *up*, or range checks would
        // report a target as in range when it is a fraction outside.
        let mut rng = crate::rng::SimRng::new(99);
        for _ in 0..20_000 {
            let v = Fx::from_raw(rng.next_range(-40_000_000, 40_000_000));
            let exact = (v.raw() as i64) * (v.raw() as i64);
            assert_eq!(v.sq().raw(), exact >> FRAC_BITS);
            assert!(v.sq() >= FxWide::ZERO, "a square is never negative");
        }
    }

    #[test]
    fn sqrt_of_two_lands_where_expected() {
        // 1.4142 ≤ sqrt(2) < 1.4143, with floor semantics allowing the lower bound.
        let two = Fx::from_int(2).sqrt();
        assert!(two >= Fx::from_frac(14142, 10000));
        assert!(two < Fx::from_frac(14143, 10000));
    }

    #[test]
    fn distance_is_a_right_triangle() {
        assert_eq!(Fx::dist(Fx::from_int(3), Fx::from_int(4)), Fx::from_int(5));
        assert_eq!(
            Fx::dist_sq(Fx::from_int(3), Fx::from_int(4)),
            FxWide::from_fx(Fx::from_int(25))
        );
    }

    #[test]
    fn squared_distance_stays_ordered_beyond_the_fx_range() {
        // The reason dist_sq returns FxWide. In Fx these would all saturate to
        // MAX and compare equal, so a "nearest enemy" search across a large map
        // would pick arbitrarily — and two peers could pick differently.
        let near = Fx::dist_sq(Fx::from_int(200), Fx::from_int(200));
        let mid = Fx::dist_sq(Fx::from_int(400), Fx::from_int(400));
        let far = Fx::dist_sq(Fx::from_int(800), Fx::from_int(800));

        assert!(near < mid && mid < far, "ordering must survive at map scale");
        assert!(near.narrow() == Fx::MAX, "these would all be equal in Fx");
        assert!(mid.narrow() == Fx::MAX);

        // And the actual distance is still recoverable.
        assert_eq!(far.sqrt(), Fx::dist(Fx::from_int(800), Fx::from_int(800)));
        assert!(far.sqrt() > Fx::from_int(1131) && far.sqrt() < Fx::from_int(1132));
    }

    #[test]
    fn range_checks_read_naturally() {
        let (dx, dy) = (Fx::from_int(3), Fx::from_int(4));
        assert!(Fx::dist_sq(dx, dy) <= Fx::from_int(5).sq());
        assert!(Fx::dist_sq(dx, dy) <= Fx::from_int(6).sq());
        assert!(Fx::dist_sq(dx, dy) > Fx::from_int(4).sq());
    }

    #[test]
    fn mul_ratio_keeps_precision() {
        let hp = Fx::from_int(400);
        assert_eq!(hp.mul_ratio(60, 100), Fx::from_int(240));
        // Doing this as two operations would truncate; one wide intermediate
        // does not.
        assert_eq!(Fx::ONE.mul_ratio(1, 3).mul_int(3), Fx::from_raw(65535));
    }

    #[test]
    fn lerp_clamps_and_hits_endpoints() {
        let a = Fx::from_int(10);
        let b = Fx::from_int(20);
        assert_eq!(a.lerp(b, Fx::ZERO), a);
        assert_eq!(a.lerp(b, Fx::ONE), b);
        assert_eq!(a.lerp(b, Fx::HALF), Fx::from_int(15));
        assert_eq!(a.lerp(b, Fx::from_int(5)), b);
        assert_eq!(a.lerp(b, Fx::from_int(-5)), a);
    }

    #[test]
    fn ordering_is_total() {
        let mut v = [Fx::from_int(3), Fx::from_int(-1), Fx::ZERO, Fx::from_int(2)];
        v.sort();
        assert_eq!(v, [Fx::from_int(-1), Fx::ZERO, Fx::from_int(2), Fx::from_int(3)]);
    }

    #[test]
    #[should_panic(expected = "divide by zero")]
    fn division_by_zero_panics() {
        let _ = Fx::ONE.div(Fx::ZERO);
    }

    #[test]
    #[should_panic(expected = "negative")]
    fn sqrt_of_negative_panics() {
        let _ = Fx::from_int(-1).sqrt();
    }

    #[test]
    fn debug_formatting_is_readable() {
        assert_eq!(format!("{:?}", Fx::from_int(3)), "3.000000");
        assert_eq!(format!("{:?}", Fx::HALF), "0.500000");
        assert_eq!(format!("{:?}", -Fx::from_frac(3, 2)), "-1.500000");
    }

    // -- Angles --------------------------------------------------------------

    #[test]
    fn angle_cardinals_are_exact() {
        assert_eq!(Angle::ZERO.sin(), Fx::ZERO);
        assert_eq!(Angle::QUARTER.sin(), Fx::ONE);
        assert_eq!(Angle::HALF.sin(), Fx::ZERO);
        assert_eq!(Angle::from_raw(49152).sin(), -Fx::ONE);

        assert_eq!(Angle::ZERO.cos(), Fx::ONE);
        assert_eq!(Angle::QUARTER.cos(), Fx::ZERO);
        assert_eq!(Angle::HALF.cos(), -Fx::ONE);
    }

    #[test]
    fn angle_degrees_roundtrip() {
        for deg in [0, 45, 90, 135, 180, 225, 270, 315] {
            assert_eq!(Angle::from_degrees(deg).to_degrees(), deg);
        }
    }

    #[test]
    fn pythagorean_identity_holds_within_tolerance() {
        // sin² + cos² should be 1. The table is quantised, so allow a small
        // slack — but it must be *deterministically* small everywhere.
        for i in 0..360 {
            let a = Angle::from_degrees(i);
            let s = a.sin();
            let c = a.cos();
            let sum = s.mul(s) + c.mul(c);
            let err = (sum - Fx::ONE).abs();
            assert!(err < Fx::from_frac(1, 1000), "sin²+cos² off by {err:?} at {i}°");
        }
    }

    #[test]
    fn angle_delta_takes_the_short_way() {
        let a = Angle::from_degrees(10);
        let b = Angle::from_degrees(350);
        // 10° → 350° is -20°, not +340°.
        assert!(a.delta(b) < 0);
        assert!(b.delta(a) > 0);
        assert_eq!(a.delta(a), 0);
    }

    #[test]
    fn angle_wraps_without_normalisation() {
        let a = Angle::from_raw(65500);
        let stepped = a.rotate_toward(Angle::from_raw(100), 200);
        // Crossing zero must not need a special case.
        assert_eq!(stepped, Angle::from_raw(100));
    }

    #[test]
    fn rotate_toward_never_overshoots() {
        let target = Angle::from_degrees(90);
        let mut a = Angle::ZERO;
        for _ in 0..1000 {
            a = a.rotate_toward(target, 64);
        }
        assert_eq!(a, target, "should settle exactly on the target and stay");
    }
}
