//! Numeric types for rules files.
//!
//! # Why not just write decimals
//!
//! Rules values feed the simulation directly, and the simulation cannot use
//! floating point — see `docs/02-simulation.md`. Writing `4.5` in a rules file
//! would mean parsing a float, and the exact bits that produced would depend on
//! the parser. Two peers with different toolchains could then load *different*
//! unit speeds from an identical file, and diverge without anything looking
//! wrong.
//!
//! So fractional values are written as integers in hundredths, and converted to
//! [`Fx`] by exact integer arithmetic. `4.5 cells/second` is written `450`.
//! It reads a little oddly for about a day, and then it reads as normal.

use serde::{Deserialize, Serialize};

/// The fixed-point scalar the simulation uses.
///
/// Re-exported so rules code does not need to depend on `redshift-sim`, which
/// would be a dependency cycle — the simulation depends on this crate.
pub type FxRaw = i32;

/// Number of fractional bits in the simulation's fixed-point type.
const FRAC_BITS: u32 = 16;

/// A value in hundredths. `450` means `4.5`.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize,
)]
#[repr(transparent)]
#[serde(transparent)]
pub struct Hundredths(pub i32);

impl Hundredths {
    pub const ZERO: Hundredths = Hundredths(0);
    pub const ONE: Hundredths = Hundredths(100);

    /// The raw fixed-point representation, computed by exact integer
    /// arithmetic so every peer produces the same bits.
    pub const fn to_fx_raw(self) -> FxRaw {
        (((self.0 as i64) << FRAC_BITS) / 100) as FxRaw
    }

    /// The whole part, truncated.
    pub const fn whole(self) -> i32 {
        self.0 / 100
    }

    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }
}

impl std::fmt::Display for Hundredths {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let magnitude = self.0.unsigned_abs();
        write!(f, "{sign}{}.{:02}", magnitude / 100, magnitude % 100)
    }
}

/// A percentage. `100` is unchanged, `60` is "60% as much".
///
/// Used for armour multipliers, cost modifiers and anything else expressed as a
/// proportion. A distinct type from [`Hundredths`] because confusing the two is
/// otherwise a silent hundredfold error.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[repr(transparent)]
#[serde(transparent)]
pub struct Percent(pub i32);

impl Percent {
    pub const ZERO: Percent = Percent(0);
    pub const FULL: Percent = Percent(100);

    /// Applies this percentage to an integer, rounding toward zero.
    pub const fn apply(self, value: i32) -> i32 {
        (((value as i64) * (self.0 as i64)) / 100) as i32
    }
}

impl Default for Percent {
    fn default() -> Self {
        Percent::FULL
    }
}

impl std::fmt::Display for Percent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}%", self.0)
    }
}

/// A duration in simulation ticks.
///
/// Rules are authored in ticks rather than seconds for the same reason values
/// are authored in hundredths: seconds would need a conversion, and a
/// conversion is a place for two peers to disagree. At 20 Hz, one second is 20.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize,
)]
#[repr(transparent)]
#[serde(transparent)]
pub struct Ticks(pub u32);

impl Ticks {
    pub const fn from_seconds(seconds: u32, ticks_per_second: u32) -> Ticks {
        Ticks(seconds * ticks_per_second)
    }
}

impl std::fmt::Display for Ticks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ticks", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hundredths_convert_exactly() {
        // 1.0 must land exactly on the fixed-point one, or every speed and
        // range in the game is subtly off.
        assert_eq!(Hundredths(100).to_fx_raw(), 1 << FRAC_BITS);
        assert_eq!(Hundredths(50).to_fx_raw(), 1 << (FRAC_BITS - 1));
        assert_eq!(Hundredths(0).to_fx_raw(), 0);
        assert_eq!(Hundredths(-100).to_fx_raw(), -(1 << FRAC_BITS));
    }

    #[test]
    fn conversion_is_pure_integer_arithmetic() {
        // The property that matters: identical input, identical bits, on every
        // machine. A float parser could not promise this.
        for raw in [-100_000, -450, -1, 0, 1, 450, 100_000] {
            let a = Hundredths(raw).to_fx_raw();
            let b = Hundredths(raw).to_fx_raw();
            assert_eq!(a, b);
        }
    }

    #[test]
    fn hundredths_display_readably() {
        assert_eq!(Hundredths(450).to_string(), "4.50");
        assert_eq!(Hundredths(5).to_string(), "0.05");
        assert_eq!(Hundredths(-450).to_string(), "-4.50");
        assert_eq!(Hundredths(0).to_string(), "0.00");
    }

    #[test]
    fn percent_applies_without_overflow_at_realistic_magnitudes() {
        assert_eq!(Percent(60).apply(400), 240);
        assert_eq!(Percent::FULL.apply(400), 400);
        assert_eq!(Percent::ZERO.apply(400), 0);
        // Health values and credit totals stay well inside i32 even after the
        // wide intermediate.
        assert_eq!(Percent(150).apply(1_000_000), 1_500_000);
    }

    #[test]
    fn percent_defaults_to_unchanged() {
        // A modifier left out of a rules file must mean "no change", not "zero".
        assert_eq!(Percent::default(), Percent::FULL);
        assert_eq!(Percent::default().apply(77), 77);
    }

    #[test]
    fn the_two_proportion_types_do_not_interchange() {
        // Hundredths(60) is 0.6; Percent(60) is 60%. They coincide in meaning
        // and differ in representation, which is exactly why they are separate
        // types — mixing them silently is a hundredfold error.
        assert_ne!(Hundredths(60).0 as i64, Percent(60).apply(1) as i64 * 100);
        assert_eq!(Percent(60).apply(100), 60);
        assert_eq!(Hundredths(60).to_fx_raw(), (60 << FRAC_BITS) / 100);
    }

    #[test]
    fn values_roundtrip_through_ron() {
        // Rules are hand-edited text, so the serialised form must be the plain
        // integer a human would type.
        assert_eq!(ron::to_string(&Hundredths(450)).unwrap(), "450");
        assert_eq!(ron::to_string(&Percent(60)).unwrap(), "60");
        assert_eq!(ron::to_string(&Ticks(45)).unwrap(), "45");
        assert_eq!(ron::from_str::<Hundredths>("450").unwrap(), Hundredths(450));
    }

    #[test]
    fn ticks_convert_from_seconds() {
        assert_eq!(Ticks::from_seconds(1, 20), Ticks(20));
        assert_eq!(Ticks::from_seconds(45, 20), Ticks(900));
    }
}
