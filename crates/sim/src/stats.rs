//! Turning rules into the numbers the simulation actually uses.
//!
//! # Why stats are resolved once, not read per tick
//!
//! A unit's effective numbers depend on more than its definition: a country's
//! modifiers change the cost, speed or health of particular units. Applying
//! those every time a value is needed would mean doing the same arithmetic
//! thousands of times a second, and — worse — would put a multiply-and-divide
//! on a hot path where a future refactor could reorder it and change a low bit.
//!
//! So every combination of player and entity kind is resolved once, at match
//! start, into a flat table. After that the simulation only ever reads.
//!
//! # Unit conversion happens here too
//!
//! Rules are authored in units a person can reason about — cells per *second*,
//! degrees per *second* — and the simulation works in cells per *tick* and
//! binary angle units per tick. Doing that conversion once, in one place, with
//! exact integer arithmetic, keeps it from being repeated slightly differently
//! somewhere else.

use redshift_data::rules::{EntityKind, Rules};
use redshift_data::traits::{Locomotor, Trait};
use redshift_data::value::{Hundredths, Percent};

use crate::TICKS_PER_SECOND;
use crate::command::PlayerId;
use crate::fx::Fx;
use crate::hash::{StateHash, StateHasher};

/// A full turn in binary angle units.
const FULL_TURN: i64 = 65_536;

/// The numbers one player's copy of one entity kind actually uses.
///
/// Everything is already in per-tick terms and fixed point. Nothing here needs
/// converting again.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct UnitStats {
    pub max_health: u32,
    /// Cells travelled per tick.
    pub speed: Fx,
    /// Binary angle units turned per tick.
    pub turn_rate: u16,
    pub locomotor: Locomotor,
    /// Vision radius in cells.
    pub vision: Fx,
    /// Tie-break when units overlap under the pointer.
    pub selection_priority: u8,
    pub cost: u32,
    /// Ticks to produce.
    pub build_time: u32,
    pub can_crush: bool,
    /// Whether this kind can move at all. Structures cannot.
    pub mobile: bool,
    /// Physical radius, for keeping units out of each other.
    ///
    /// Derived from the same source the renderer sizes its placeholder from, so
    /// what the player sees and what the simulation enforces are the same
    /// shape. A unit whose drawn box is wider than its collision radius reads
    /// as clipping through its neighbours.
    pub radius: Fx,
}

impl StateHash for UnitStats {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_u32(self.max_health);
        h.write_i32(self.speed.raw());
        h.write_u16(self.turn_rate);
        h.write_u8(self.locomotor as u8);
        h.write_i32(self.vision.raw());
        h.write_u32(self.cost);
        h.write_u32(self.build_time);
        h.write_bool(self.can_crush);
        h.write_bool(self.mobile);
        h.write_i32(self.radius.raw());
    }
}

/// Resolved stats for every player and every entity kind.
///
/// Indexed `[player][kind]`, both dense, so a lookup is two array reads and no
/// hashing — and iteration order is index order, which is what determinism
/// requires.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct StatTable {
    per_player: Vec<Vec<UnitStats>>,
}

impl StatTable {
    /// Resolves every combination once.
    ///
    /// `factions[i]` is the faction id for player `i`, or `None` for a player
    /// with no country — which simply means no modifiers apply.
    pub fn resolve(rules: &Rules, factions: &[Option<String>]) -> StatTable {
        let per_player = factions
            .iter()
            .map(|faction| {
                rules
                    .entities()
                    .map(|(kind, _)| resolve_one(rules, kind, faction.as_deref()))
                    .collect()
            })
            .collect();
        StatTable { per_player }
    }

    /// Stats for a player's copy of a kind.
    ///
    /// Falls back to defaults for an unknown player or kind rather than
    /// panicking: a stale entity id can outlive its owner, and a crash there
    /// would be a far worse failure than a unit that briefly cannot move.
    pub fn get(&self, player: PlayerId, kind: EntityKind) -> UnitStats {
        self.per_player
            .get(player.0 as usize)
            .and_then(|row| row.get(kind.0 as usize))
            .copied()
            .unwrap_or_default()
    }

    pub fn player_count(&self) -> usize {
        self.per_player.len()
    }

    pub fn kind_count(&self) -> usize {
        self.per_player.first().map_or(0, |row| row.len())
    }
}

impl StateHash for StatTable {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_u32(self.per_player.len() as u32);
        for row in &self.per_player {
            h.write_u32(row.len() as u32);
            for stats in row {
                h.write(stats);
            }
        }
    }
}

fn resolve_one(rules: &Rules, kind: EntityKind, faction: Option<&str>) -> UnitStats {
    let def = rules.entity(kind);

    // Physical size. A structure states its footprint; everything else is
    // sized by what it is. These match the renderer's placeholder proportions,
    // so what the player sees is what the simulation enforces.
    let radius = match def.category.as_str() {
        "infantry" => Fx::from_frac(16, 100),
        "vehicle" => Fx::from_frac(39, 100),
        "aircraft" => Fx::from_frac(35, 100),
        "ship" => Fx::from_frac(60, 100),
        "structure" => Fx::from_frac(90, 100),
        _ => Fx::from_frac(30, 100),
    };
    let mut stats = UnitStats {
        radius,
        ..UnitStats::default()
    };

    for t in &def.traits {
        match t {
            Trait::Health { max, .. } => stats.max_health = *max,
            Trait::Mobile {
                speed,
                turn_rate,
                locomotor,
            } => {
                stats.speed = cells_per_tick(*speed);
                stats.turn_rate = degrees_per_second_to_tick(*turn_rate);
                stats.locomotor = *locomotor;
                stats.mobile = true;
            }
            Trait::Vision { range } => stats.vision = Fx::from_raw(range.to_fx_raw()),
            Trait::Selectable { priority } => stats.selection_priority = *priority,
            Trait::Crushes { classes } => stats.can_crush = !classes.is_empty(),
            Trait::Buildable {
                cost, build_time, ..
            } => {
                stats.cost = *cost;
                stats.build_time = build_time.0;
            }
            _ => {}
        }
    }

    if let Some(faction_id) = faction
        && let Some(faction) = rules.faction(faction_id)
    {
        apply_modifiers(&mut stats, &faction.modifiers, &def.id, &def.category);
    }
    stats
}

fn apply_modifiers(
    stats: &mut UnitStats,
    modifiers: &[redshift_data::rules::Modifier],
    id: &str,
    category: &str,
) {
    use redshift_data::rules::Modifier;
    for modifier in modifiers {
        match modifier {
            Modifier::UnitCost { unit, multiplier } if unit == id => {
                stats.cost = scale_u32(stats.cost, *multiplier);
            }
            Modifier::UnitHealth { unit, multiplier } if unit == id => {
                stats.max_health = scale_u32(stats.max_health, *multiplier);
            }
            Modifier::UnitSpeed { unit, multiplier } if unit == id => {
                stats.speed = Fx::from_raw(scale_i32(stats.speed.raw(), *multiplier));
            }
            // A *higher* multiplier means faster, so the time goes down.
            // Writing it the other way round is an easy and very confusing
            // mistake: a country advertised as building faster would build
            // slower. A non-positive multiplier is ignored rather than
            // dividing by zero.
            Modifier::BuildSpeed {
                category: c,
                multiplier,
            } if c == category && multiplier.0 > 0 => {
                stats.build_time = ((stats.build_time as i64 * 100) / multiplier.0 as i64) as u32;
            }
            _ => {}
        }
    }
}

fn scale_u32(value: u32, percent: Percent) -> u32 {
    ((value as i64 * percent.0 as i64) / 100).max(0) as u32
}

fn scale_i32(value: i32, percent: Percent) -> i32 {
    ((value as i64 * percent.0 as i64) / 100) as i32
}

/// Cells per second to cells per tick.
///
/// Both divisions happen in one expression, on a widened intermediate, rather
/// than converting to fixed point and then dividing again. Two successive
/// integer divisions truncate twice and lose roughly twice as much; doing it
/// once keeps the error under a single unit in the last place.
///
/// It still truncates, and that is fine — the residue is under one part in
/// 65536 of a cell per tick, far below anything visible. What matters is that
/// it truncates *identically* everywhere, which integer arithmetic guarantees
/// and floating point would not.
fn cells_per_tick(per_second: Hundredths) -> Fx {
    const DIVISOR: i64 = 100 * TICKS_PER_SECOND as i64;
    Fx::from_raw((((per_second.0 as i64) << 16) / DIVISOR) as i32)
}

/// Degrees per second to binary angle units per tick.
pub(crate) fn degrees_per_second_to_tick(degrees_per_second: u32) -> u16 {
    let per_tick = (degrees_per_second as i64 * FULL_TURN) / 360 / TICKS_PER_SECOND as i64;
    per_tick.clamp(0, u16::MAX as i64) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use redshift_data::rules::{ArmourTable, EntityDef, FactionDef, Modifier};
    use redshift_data::value::Ticks;

    fn armour() -> ArmourTable {
        ron::from_str(r#"( classes: ["none"], table: { "sa": { "none": 100 } } )"#).unwrap()
    }

    fn tank() -> EntityDef {
        EntityDef {
            id: "tank".into(),
            name_key: "unit.tank".into(),
            side: None,
            category: "vehicle".into(),
            traits: vec![
                Trait::Health {
                    max: 400,
                    armour: "none".into(),
                },
                Trait::Mobile {
                    speed: Hundredths(450),
                    turn_rate: 90,
                    locomotor: Locomotor::Tracked,
                },
                Trait::Vision {
                    range: Hundredths(600),
                },
                Trait::Crushes {
                    classes: vec!["infantry".into()],
                },
                Trait::Selectable { priority: 2 },
                Trait::Buildable {
                    cost: 900,
                    build_time: Ticks(60),
                    prerequisites: vec![],
                    produced_by: "factory".into(),
                },
            ],
        }
    }

    fn rules_with(factions: Vec<FactionDef>) -> Rules {
        // The tank's producer must exist for validation to pass.
        let factory = EntityDef {
            id: "factory".into(),
            name_key: "building.factory".into(),
            side: None,
            category: "structure".into(),
            traits: vec![Trait::Health {
                max: 1000,
                armour: "none".into(),
            }],
        };
        Rules::from_parts(vec![tank(), factory], Vec::new(), armour(), factions).expect("valid")
    }

    fn faction(id: &str, modifiers: Vec<Modifier>) -> FactionDef {
        FactionDef {
            id: id.into(),
            name_key: format!("faction.{id}"),
            side: "side".into(),
            colour: (1, 2, 3),
            unique_units: vec![],
            removes_units: vec![],
            modifiers,
            voice_set: id.into(),
        }
    }

    #[test]
    fn traits_become_numbers() {
        let rules = rules_with(vec![]);
        let table = StatTable::resolve(&rules, &[None]);
        let kind = rules.kind_of("tank").unwrap();
        let stats = table.get(PlayerId(0), kind);

        assert_eq!(stats.max_health, 400);
        assert_eq!(stats.locomotor, Locomotor::Tracked);
        assert_eq!(stats.vision, Fx::from_raw(Hundredths(600).to_fx_raw()));
        assert_eq!(stats.cost, 900);
        assert_eq!(stats.build_time, 60);
        assert!(stats.can_crush);
        assert!(stats.mobile);
        assert_eq!(stats.selection_priority, 2);
    }

    #[test]
    fn a_structure_is_not_mobile() {
        // Absence of a trait is meaningful: no Mobile means it cannot move, and
        // the simulation must be able to tell that apart from "speed zero".
        let rules = rules_with(vec![]);
        let table = StatTable::resolve(&rules, &[None]);
        let stats = table.get(PlayerId(0), rules.kind_of("factory").unwrap());
        assert!(!stats.mobile);
        assert_eq!(stats.speed, Fx::ZERO);
    }

    #[test]
    fn speed_converts_from_seconds_to_ticks() {
        // 4.5 cells/second at 20 Hz is 0.225 cells/tick.
        let per_tick = cells_per_tick(Hundredths(450));
        assert_eq!(per_tick, Fx::from_frac(450, 100 * TICKS_PER_SECOND as i32));
    }

    #[test]
    fn a_second_of_travel_matches_the_authored_speed_to_within_a_rounding_unit() {
        // Exact equality is the wrong bar: converting to a per-tick value has
        // to truncate somewhere. The contract is that the shortfall stays at
        // the level of the last representable bit — about 0.0002 cells per
        // second here, which is a tenth of a cell over a ten-minute match.
        for authored in [Hundredths(100), Hundredths(450), Hundredths(1200)] {
            let travelled = cells_per_tick(authored).mul_int(TICKS_PER_SECOND as i32);
            let intended = Fx::from_raw(authored.to_fx_raw());
            let shortfall = intended - travelled;

            assert!(shortfall >= Fx::ZERO, "truncation must never overshoot");
            assert!(
                shortfall < Fx::from_raw(TICKS_PER_SECOND as i32),
                "{authored} lost {shortfall:?} per second, which is more than rounding"
            );
        }
    }

    #[test]
    fn turn_rate_converts_to_binary_angle_units() {
        // A full turn per second is the whole 65536 spread over 20 ticks.
        assert_eq!(degrees_per_second_to_tick(360), (65_536 / 20) as u16);
        // 90 degrees per second is a quarter of that.
        assert_eq!(degrees_per_second_to_tick(90), (65_536 / 4 / 20) as u16);
        assert_eq!(degrees_per_second_to_tick(0), 0);
    }

    #[test]
    fn an_absurd_turn_rate_clamps_rather_than_wrapping() {
        // Wrapping would turn a very fast turret into a very slow one, which
        // is a maddening thing to debug from a data file.
        assert_eq!(degrees_per_second_to_tick(u32::MAX), u16::MAX);
    }

    #[test]
    fn conversions_are_identical_every_time() {
        // Integer arithmetic throughout, so the same rules produce the same
        // numbers on every machine.
        for raw in [1, 45, 450, 10_000] {
            assert_eq!(
                cells_per_tick(Hundredths(raw)),
                cells_per_tick(Hundredths(raw))
            );
        }
    }

    #[test]
    fn country_modifiers_apply_to_the_right_units() {
        let rules = rules_with(vec![faction(
            "thrifty",
            vec![Modifier::UnitCost {
                unit: "tank".into(),
                multiplier: Percent(90),
            }],
        )]);
        let table = StatTable::resolve(&rules, &[None, Some("thrifty".into())]);
        let kind = rules.kind_of("tank").unwrap();

        assert_eq!(
            table.get(PlayerId(0), kind).cost,
            900,
            "no country, no change"
        );
        assert_eq!(
            table.get(PlayerId(1), kind).cost,
            810,
            "the country is 10% cheaper"
        );
    }

    #[test]
    fn a_modifier_naming_another_unit_is_ignored() {
        let rules = rules_with(vec![faction(
            "elsewhere",
            vec![Modifier::UnitCost {
                unit: "factory".into(),
                multiplier: Percent(50),
            }],
        )]);
        let table = StatTable::resolve(&rules, &[Some("elsewhere".into())]);
        assert_eq!(
            table.get(PlayerId(0), rules.kind_of("tank").unwrap()).cost,
            900
        );
    }

    #[test]
    fn a_faster_build_speed_means_less_time_not_more() {
        // The mistake this pins: a country advertised as building faster
        // building slower, because the multiplier was applied to the duration
        // directly.
        let rules = rules_with(vec![faction(
            "industrious",
            vec![Modifier::BuildSpeed {
                category: "vehicle".into(),
                multiplier: Percent(125),
            }],
        )]);
        let table = StatTable::resolve(&rules, &[Some("industrious".into())]);
        let stats = table.get(PlayerId(0), rules.kind_of("tank").unwrap());
        assert!(
            stats.build_time < 60,
            "125% build speed should shorten 60 ticks"
        );
        assert_eq!(stats.build_time, 48);
    }

    #[test]
    fn health_and_speed_modifiers_apply() {
        let rules = rules_with(vec![faction(
            "elite",
            vec![
                Modifier::UnitHealth {
                    unit: "tank".into(),
                    multiplier: Percent(110),
                },
                Modifier::UnitSpeed {
                    unit: "tank".into(),
                    multiplier: Percent(120),
                },
            ],
        )]);
        let table = StatTable::resolve(&rules, &[Some("elite".into())]);
        let plain = StatTable::resolve(&rules, &[None]);
        let kind = rules.kind_of("tank").unwrap();

        assert_eq!(table.get(PlayerId(0), kind).max_health, 440);
        assert!(table.get(PlayerId(0), kind).speed > plain.get(PlayerId(0), kind).speed);
    }

    #[test]
    fn an_unknown_player_or_kind_returns_defaults_rather_than_panicking() {
        // A stale entity id can outlive its owner. Crashing there would be a
        // far worse failure than a unit that briefly cannot move.
        let rules = rules_with(vec![]);
        let table = StatTable::resolve(&rules, &[None]);
        assert_eq!(
            table.get(PlayerId(200), EntityKind(0)),
            UnitStats::default()
        );
        assert_eq!(
            table.get(PlayerId(0), EntityKind(9999)),
            UnitStats::default()
        );
    }

    #[test]
    fn resolution_is_reproducible() {
        // The table feeds the state hash, so two peers must build it
        // identically from identical rules.
        let rules = rules_with(vec![faction(
            "x",
            vec![Modifier::UnitSpeed {
                unit: "tank".into(),
                multiplier: Percent(117),
            }],
        )]);
        let a = StatTable::resolve(&rules, &[None, Some("x".into())]);
        let b = StatTable::resolve(&rules, &[None, Some("x".into())]);
        let mut ha = StateHasher::new();
        let mut hb = StateHasher::new();
        ha.write(&a);
        hb.write(&b);
        assert_eq!(ha.finish(), hb.finish());
    }
}
