//! How good a computer opponent is.
//!
//! # What "difficulty" actually means here
//!
//! An RTS opponent does not have an IQ, and pretending otherwise leads
//! straight to the usual dishonest answer: give the hard one more money. What
//! it has is three things that can be turned down without lying to the player:
//!
//! 1. **How often it thinks.** A human notices something and acts a fraction of
//!    a second later. A worse player takes longer — not because they are
//!    slower to click, but because they are looking somewhere else.
//! 2. **How much of its income it spends.** Floating credits is the single most
//!    reliable marker of a weak player: money in the bank is an army that does
//!    not exist.
//! 3. **How much it commits.** Attacking with six tanks when the answer is
//!    twelve, holding twelve when six would have done.
//!
//! Everything below is those three scaled by one number. **No cheating** — the
//! hardest opponent sees the same fog, pays the same prices and obeys the same
//! build radius as the player. `Hard` is meant to be about as good as a
//! competent human, not better; if it needs to be harder than that, the answer
//! is a better opponent rather than a richer one.
//!
//! # Determinism
//!
//! Everything here is derived from the difficulty and the tick. No randomness
//! and no wall clock: two peers running the same match must issue the same
//! commands on the same ticks, or the opponent becomes a desync generator.

use redshift_sim::TICKS_PER_SECOND;

/// How well a computer opponent plays, and whether it attacks at all.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub enum Difficulty {
    /// Plays as well as [`Difficulty::Easy`] and **never attacks**.
    ///
    /// Not a broken opponent — a deliberate one. It builds a base, builds an
    /// army, and defends itself if you come to it. It exists so a player can
    /// learn the game, test a build order, or try a unit against something that
    /// shoots back without being under a clock.
    #[default]
    Dummy,
    Easy,
    Medium,
    /// Roughly a competent human. The ceiling, deliberately: an opponent that
    /// beat a good player would have to cheat to do it.
    Hard,
}

impl Difficulty {
    /// Competence, as a percentage of a competent human.
    ///
    /// The steps are even rather than tuned. An even ladder is honest about
    /// being arbitrary; an uneven one implies a measurement nobody made.
    pub const fn competence(self) -> u32 {
        match self {
            // Dummy thinks exactly as well as Easy. The only difference is what
            // it is willing to do with the conclusion.
            Difficulty::Dummy | Difficulty::Easy => 40,
            Difficulty::Medium => 70,
            Difficulty::Hard => 100,
        }
    }

    /// Whether this opponent will ever leave its base to attack.
    pub const fn attacks(self) -> bool {
        !matches!(self, Difficulty::Dummy)
    }

    /// Ticks between decisions.
    ///
    /// A competent human reacts in about a quarter of a second; at a fifth of
    /// that competence they are effectively looking elsewhere for a second and
    /// a half. Scaled inversely, so the number falls as skill rises.
    pub const fn think_interval(self) -> u32 {
        let quarter_second = TICKS_PER_SECOND / 4;
        // `100 / competence` at a floor of one tick — the fastest anything is
        // allowed to be is once per simulation step.
        let interval = quarter_second * 100 / self.competence();
        if interval < 1 { 1 } else { interval }
    }

    /// The share of its income it will actually commit, as a percentage.
    ///
    /// Floating credits is the most reliable marker of a weak player: money in
    /// the bank is an army that does not exist. A weak opponent holds a reserve
    /// it never spends, and it should be visible on its face — a player who
    /// destroys a weak base should find it was sitting on money.
    pub const fn spend_share(self) -> u32 {
        // Never below half: an opponent that spent almost nothing would look
        // broken rather than bad, and "looks broken" is not a difficulty.
        50 + self.competence() / 2
    }

    /// How many harvesters it tries to keep working.
    ///
    /// The one number where a weak opponent is weak in a way that compounds:
    /// two miners instead of four is not half an economy after ten minutes, it
    /// is a quarter of one.
    pub const fn harvesters_wanted(self) -> u32 {
        2 + self.competence() / 40
    }

    /// How many fighting units it gathers before attacking.
    ///
    /// Backwards from the others on purpose. A *worse* player attacks with too
    /// few or sits on too many; this models the first, which is the one that
    /// loses games rather than merely delaying them.
    pub const fn attack_at(self) -> u32 {
        4 + self.competence() / 12
    }
}
