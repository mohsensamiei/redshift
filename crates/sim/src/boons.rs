//! Standing modifiers on a player.
//!
//! A shape the original uses repeatedly and that has nowhere else to live: an
//! ore purifier makes every load worth a quarter more, a captured machine shop
//! repairs every vehicle you own anywhere on the map, a spy in a barracks
//! promotes everything you build from then on.
//!
//! None of those are events, and none of them belong to a unit. They are
//! properties of a *player*, held for as long as their source stands.
//!
//! # Rebuilt, not maintained
//!
//! Recomputed from scratch each tick, for the same reason the power grid is: a
//! running total would have to be corrected on every spawn, death, capture and
//! sale, and one missed correction leaves a player permanently and invisibly
//! wrong about their own economy. Both are O(units) against a tick that already
//! walks every unit several times.

use serde::{Deserialize, Serialize};

use redshift_data::traits::PlayerEffect;
use redshift_data::value::Percent;

use crate::command::PlayerId;
use crate::hash::{StateHash, StateHasher};

/// What each player currently enjoys.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Boons {
    ore_value: Vec<Percent>,
    veteran_production: Vec<bool>,
    repair_everywhere: Vec<bool>,
}

impl Boons {
    pub fn new(player_count: usize) -> Boons {
        Boons {
            ore_value: vec![Percent::FULL; player_count],
            veteran_production: vec![false; player_count],
            repair_everywhere: vec![false; player_count],
        }
    }

    /// Resets to the baseline before rebuilding.
    ///
    /// Ore value resets to 100%, not zero. A clear that zeroed it would make
    /// every load worthless for a tick, which is the kind of thing that looks
    /// like a rounding bug in the income graph.
    pub fn clear(&mut self) {
        self.ore_value.iter_mut().for_each(|v| *v = Percent::FULL);
        self.veteran_production.iter_mut().for_each(|v| *v = false);
        self.repair_everywhere.iter_mut().for_each(|v| *v = false);
    }

    /// Applies one source's effect.
    ///
    /// Ore value multiplies, so two purifiers compound rather than the second
    /// being ignored. The flags do not stack, because "promoted" and "promoted
    /// twice" are the same thing.
    pub fn grant(&mut self, player: PlayerId, effect: PlayerEffect) {
        let Some(index) = self.index_of(player) else {
            return;
        };
        match effect {
            PlayerEffect::OreValue(percent) => {
                let current = self.ore_value[index];
                self.ore_value[index] = Percent((current.0 * percent.0) / 100);
            }
            PlayerEffect::VeteranProduction => self.veteran_production[index] = true,
            PlayerEffect::RepairEverywhere => self.repair_everywhere[index] = true,
        }
    }

    fn index_of(&self, player: PlayerId) -> Option<usize> {
        let index = player.0 as usize;
        (index < self.ore_value.len()).then_some(index)
    }

    /// What a load of ore is worth to this player, as a percentage.
    pub fn ore_value(&self, player: PlayerId) -> Percent {
        self.index_of(player)
            .map(|i| self.ore_value[i])
            .unwrap_or(Percent::FULL)
    }

    /// Whether everything this player builds arrives promoted.
    pub fn veteran_production(&self, player: PlayerId) -> bool {
        self.index_of(player)
            .is_some_and(|i| self.veteran_production[i])
    }

    /// Whether this player's vehicles repair themselves anywhere.
    pub fn repair_everywhere(&self, player: PlayerId) -> bool {
        self.index_of(player)
            .is_some_and(|i| self.repair_everywhere[i])
    }
}

impl StateHash for Boons {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_u32(self.ore_value.len() as u32);
        for v in &self.ore_value {
            h.write_i32(v.0);
        }
        for v in &self.veteran_production {
            h.write_bool(*v);
        }
        for v in &self.repair_everywhere {
            h.write_bool(*v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_player_with_nothing_gets_the_baseline() {
        // Ore at full value, not zero. A baseline of zero would make every load
        // worthless until something granted otherwise.
        let boons = Boons::new(2);
        assert_eq!(boons.ore_value(PlayerId(0)), Percent::FULL);
        assert!(!boons.veteran_production(PlayerId(0)));
        assert!(!boons.repair_everywhere(PlayerId(0)));
    }

    #[test]
    fn an_ore_bonus_multiplies_the_value() {
        let mut boons = Boons::new(1);
        boons.grant(PlayerId(0), PlayerEffect::OreValue(Percent(125)));
        assert_eq!(boons.ore_value(PlayerId(0)), Percent(125));
    }

    #[test]
    fn two_ore_bonuses_compound() {
        // Two purifiers should be better than one. Overwriting would make the
        // second free to build and useless to own.
        let mut boons = Boons::new(1);
        boons.grant(PlayerId(0), PlayerEffect::OreValue(Percent(125)));
        boons.grant(PlayerId(0), PlayerEffect::OreValue(Percent(120)));
        assert_eq!(boons.ore_value(PlayerId(0)), Percent(150));
    }

    #[test]
    fn flags_do_not_stack_because_there_is_nothing_to_stack() {
        let mut boons = Boons::new(1);
        boons.grant(PlayerId(0), PlayerEffect::VeteranProduction);
        boons.grant(PlayerId(0), PlayerEffect::VeteranProduction);
        assert!(boons.veteran_production(PlayerId(0)));
    }

    #[test]
    fn one_player_does_not_get_anothers_bonus() {
        let mut boons = Boons::new(2);
        boons.grant(PlayerId(0), PlayerEffect::RepairEverywhere);
        assert!(boons.repair_everywhere(PlayerId(0)));
        assert!(!boons.repair_everywhere(PlayerId(1)));
    }

    #[test]
    fn clearing_returns_to_the_baseline_rather_than_to_zero() {
        // The rebuild happens every tick, so a clear that zeroed ore value
        // would make every load worthless for a tick — which looks like a
        // rounding bug in the income graph rather than a reset.
        let mut boons = Boons::new(1);
        boons.grant(PlayerId(0), PlayerEffect::OreValue(Percent(200)));
        boons.grant(PlayerId(0), PlayerEffect::VeteranProduction);
        boons.clear();
        assert_eq!(boons.ore_value(PlayerId(0)), Percent::FULL);
        assert!(!boons.veteran_production(PlayerId(0)));
    }

    #[test]
    fn an_unknown_player_reads_as_the_baseline() {
        let mut boons = Boons::new(1);
        boons.grant(PlayerId(9), PlayerEffect::VeteranProduction);
        assert_eq!(boons.ore_value(PlayerId(9)), Percent::FULL);
        assert!(!boons.veteran_production(PlayerId(9)));
    }

    #[test]
    fn every_effect_is_hashed() {
        let hash = |b: &Boons| {
            let mut h = StateHasher::new();
            h.write(b);
            h.finish()
        };
        let base = Boons::new(2);
        for effect in [
            PlayerEffect::OreValue(Percent(125)),
            PlayerEffect::VeteranProduction,
            PlayerEffect::RepairEverywhere,
        ] {
            let mut changed = base.clone();
            changed.grant(PlayerId(0), effect);
            assert_ne!(hash(&changed), hash(&base), "{effect:?} is not in the hash");
        }
    }
}
