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
use redshift_data::traits::{Layer, Locomotor, Trait};
use redshift_data::value::{Hundredths, Percent};

use crate::TICKS_PER_SECOND;
use crate::command::PlayerId;
use crate::fx::Fx;
use crate::hash::{StateHash, StateHasher};
use crate::map::SurfaceMask;
use crate::map::Terrain;

/// A full turn in binary angle units.
const FULL_TURN: i64 = 65_536;

/// The numbers one player's copy of one entity kind actually uses.
///
/// Everything is already in per-tick terms and fixed point. Nothing here needs
/// converting again.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct UnitStats {
    pub max_health: u32,
    /// Cells travelled per tick.
    pub speed: Fx,
    /// Binary angle units turned per tick.
    pub turn_rate: u16,
    pub locomotor: Locomotor,
    /// Surfaces this unit may cross. The authority on where it can go — the
    /// locomotor only supplied the default.
    pub movement: SurfaceMask,
    /// Which layer this occupies for targeting — what can reach it.
    pub layer: Layer,
    /// Vision radius in cells.
    pub vision: Fx,
    /// Tie-break when units overlap under the pointer.
    pub selection_priority: u8,
    pub cost: u32,
    /// Ticks to produce.
    pub build_time: u32,
    /// Classes this can drive over and kill.
    pub crushes: u8,
    /// The class this belongs to for crushing. Zero means it cannot be crushed.
    pub crush_class: u8,
    /// Health regained per tick, in hundredths.
    pub self_heal: u32,
    /// Ticks of quiet before self-healing starts.
    pub heal_delay: u32,
    /// Damage dealt to its surroundings when destroyed.
    ///
    /// The warhead lives in the combat table, where warhead names are interned.
    /// Resolving it here would need a second, independent index, and two
    /// indexes that disagreed would be far worse than the small split.
    pub death_damage: u32,
    /// How many passengers this can carry.
    pub capacity: u8,
    /// At most this many may exist at once, per player. Zero means no limit.
    pub build_limit: u8,
    /// Credits paid to whoever destroys this.
    pub bounty: u32,
    /// Whether this can be taken over by walking an engineer into it.
    pub capturable: bool,
    /// Whether this can enter a building to capture or repair it.
    pub is_engineer: bool,
    /// Whether entering destroys it.
    pub consumed_on_use: bool,
    /// Whether this kind can move at all. Structures cannot.
    pub mobile: bool,
    /// How much ore this can carry, if it harvests at all.
    ///
    /// `None` rather than zero: "cannot harvest" and "can harvest nothing" are
    /// different, and the harvester loop needs to skip the first entirely
    /// rather than run a cycle that can never make progress.
    pub harvest_capacity: Option<u32>,
    /// Ore gathered per bite, in hundredths of a unit.
    pub gather_rate: u32,
    /// Cells this occupies, as `(width, height)`.
    ///
    /// `(1, 1)` for anything without a declared footprint. Buildings state
    /// theirs; units are a single cell and are kept apart by their radius
    /// instead, since they move and a building does not.
    pub footprint: (u8, u8),
    /// Kills needed to reach each rank. `None` if this kind never promotes.
    pub veterancy: Option<(u32, u32)>,
    /// How far a supporter may stand. Zero for anything nothing supports.
    pub support_radius: Fx,
    /// Extra damage per supporter, as a percentage.
    pub support_bonus_percent: u32,
    /// The most supporters that count.
    pub max_supporters: u8,
    /// Supporters needed before this works with no power. Zero means never.
    pub self_powered_at: u8,
    /// Whether this vehicle takes its weapon from whoever is riding inside.
    pub weapon_from_cargo: bool,
    /// Whether this plants charges rather than shooting.
    pub plants_charges: bool,
    /// What this appears to be to everyone else, if it is disguised.
    pub disguised_as: Option<EntityKind>,
    /// Terrain this must be placed beside, if any.
    ///
    /// `None` for almost everything. A Naval Shipyard says `Water`, and the
    /// placement rule falls out of the data rather than out of a case in the
    /// placement code.
    pub needs_adjacent: Option<Terrain>,
    /// Whether this joins up with its own kind. For the renderer.
    pub connects: bool,
    /// Ticks a superweapon here takes to charge. Zero for everything else.
    pub charge_time: u32,
    /// Whether this can be demolished for money.
    pub sellable: bool,
    /// How far from home this strays when idle. Zero for anything that stands
    /// still.
    pub wander_radius: Fx,
    /// Roughly how many ticks between one stroll and the next.
    pub wander_interval: u32,
    /// Whether anything is left where this falls.
    ///
    /// Only the flag, not the list. `UnitStats` is `Copy` — every hot path
    /// takes one by value — and a `Vec` here would cost that for the sake of
    /// something read a handful of times a match. What is actually left is read
    /// from the rules at the moment of death, the same way a producer's
    /// categories are.
    pub leaves_something: bool,
    /// How far this grows ore around itself. Zero for anything that does not.
    pub grow_radius: Fx,
    /// Ticks between one unit of ore appearing and the next.
    pub grow_interval: u32,
    /// The most ore a grown cell reaches.
    pub grow_cell_limit: u16,
    /// How far this hides ground from other players. Zero for everything that
    /// does not.
    pub hides_ground: Fx,
    /// Whether this carries ground units over water — and so opens its
    /// footprint instead of blocking it, and survives being destroyed.
    pub is_bridge: bool,
    /// How far from this a wrecked bridge can be and still be rebuilt here.
    /// Zero for anything that is not a repair hut.
    pub bridge_repair_radius: u8,
    /// How many infantry may occupy this. Zero for anything that cannot be
    /// garrisoned, which is nearly everything.
    pub garrison_capacity: u8,
    /// The fraction of full health below which a garrison is thrown out.
    pub evict_below_percent: u32,
    /// Health per tick this restores to units sent into it, in hundredths.
    /// Zero for anything that is not a repair structure.
    pub repair_rate: u32,
    /// What a full repair here costs, as a percentage of the unit's build cost.
    pub repair_cost_percent: u32,
    /// Whether arriving here removes an infestation.
    pub cures_infestation: bool,
    /// What this becomes when deployed, if anything.
    ///
    /// Resolved to a kind at load, so the simulation never looks up a name
    /// mid-match. `None` means the deploy command does nothing to this unit,
    /// which is most of them.
    pub deploys_into: Option<EntityKind>,
    /// Whether this can hide from anything without a detector.
    pub cloakable: bool,
    /// Ticks after firing before the cloak returns.
    pub recloak_delay: u32,
    /// Whether this travels below the surface.
    pub submersible: bool,
    /// Ticks after firing or being hit before it submerges again.
    pub resurface_delay: u32,
    /// Whether this hears submerged things within its vision.
    pub sonar: bool,
    /// Whether this reveals cloaked things within its vision.
    pub detector: bool,
    /// Power this supplies to its owner's grid.
    pub power_supply: u32,
    /// Power this draws. Anything with a draw stops working in a shortage.
    pub power_draw: u32,
    /// Whether this keeps working when its owner is short of power.
    pub works_unpowered: bool,
    /// Whether harvesters can unload here.
    pub is_refinery: bool,
    /// Physical radius, for keeping units out of each other.
    ///
    /// Derived from the same source the renderer sizes its placeholder from, so
    /// what the player sees and what the simulation enforces are the same
    /// shape. A unit whose drawn box is wider than its collision radius reads
    /// as clipping through its neighbours.
    pub radius: Fx,
}

impl Default for UnitStats {
    fn default() -> Self {
        UnitStats {
            max_health: 0,
            speed: Fx::ZERO,
            turn_rate: 0,
            locomotor: Locomotor::default(),
            movement: SurfaceMask::NONE,
            layer: Layer::Ground,
            vision: Fx::ZERO,
            selection_priority: 0,
            cost: 0,
            build_time: 0,
            crushes: 0,
            crush_class: 0,
            self_heal: 0,
            heal_delay: 0,
            death_damage: 0,
            capacity: 0,
            build_limit: 0,
            bounty: 0,
            capturable: false,
            is_engineer: false,
            consumed_on_use: false,
            mobile: false,
            harvest_capacity: None,
            gather_rate: 0,
            is_refinery: false,
            veterancy: None,
            support_radius: Fx::ZERO,
            support_bonus_percent: 0,
            max_supporters: 0,
            self_powered_at: 0,
            weapon_from_cargo: false,
            plants_charges: false,
            disguised_as: None,
            needs_adjacent: None,
            connects: false,
            charge_time: 0,
            sellable: true,
            wander_radius: Fx::ZERO,
            wander_interval: 0,
            leaves_something: false,
            grow_radius: Fx::ZERO,
            grow_interval: 0,
            grow_cell_limit: 0,
            hides_ground: Fx::ZERO,
            is_bridge: false,
            bridge_repair_radius: 0,
            garrison_capacity: 0,
            evict_below_percent: 0,
            repair_rate: 0,
            repair_cost_percent: 0,
            cures_infestation: false,
            deploys_into: None,
            cloakable: false,
            recloak_delay: 0,
            submersible: false,
            resurface_delay: 0,
            sonar: false,
            detector: false,
            power_supply: 0,
            power_draw: 0,
            works_unpowered: false,
            // One cell, not zero. A zero footprint would make a thing occupy
            // nothing and stack invisibly with everything else.
            footprint: (1, 1),
            radius: Fx::ZERO,
        }
    }
}

impl StateHash for UnitStats {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_u32(self.max_health);
        h.write_i32(self.speed.raw());
        h.write_u16(self.turn_rate);
        h.write_u8(self.locomotor as u8);
        h.write_u8(self.movement.raw());
        h.write_u8(self.layer as u8);
        h.write_i32(self.vision.raw());
        h.write_u32(self.cost);
        h.write_u32(self.build_time);
        h.write_u8(self.crushes);
        h.write_u8(self.crush_class);
        h.write_u32(self.self_heal);
        h.write_u32(self.heal_delay);
        h.write_u32(self.death_damage);
        h.write_u8(self.capacity);
        h.write_u8(self.build_limit);
        h.write_u32(self.bounty);
        h.write_bool(self.capturable);
        h.write_bool(self.is_engineer);
        h.write_bool(self.consumed_on_use);
        h.write_bool(self.mobile);
        h.write_i32(self.radius.raw());
        h.write_u32(self.harvest_capacity.unwrap_or(u32::MAX));
        h.write_u32(self.gather_rate);
        h.write_bool(self.is_refinery);
        let (vet, elite) = self.veterancy.unwrap_or((u32::MAX, u32::MAX));
        h.write_u32(vet);
        h.write_u32(elite);
        h.write_bool(self.cloakable);
        h.write_u32(self.recloak_delay);
        h.write_bool(self.detector);
        h.write_u32(self.power_supply);
        h.write_u32(self.power_draw);
        h.write_bool(self.works_unpowered);
        h.write_u8(self.footprint.0);
        h.write_u8(self.footprint.1);
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
    /// Stats for the neutral side.
    ///
    /// A row of its own rather than a slot in `per_player`, because the neutral
    /// player's id is deliberately out of the way of real slots and sizing a
    /// dense vector to reach it would allocate 255 unused rows.
    ///
    /// Its absence was a real bug: a neutral unit resolved to `UnitStats`
    /// default, which has zero maximum health, so every civilian and tech
    /// building died on the tick it was created.
    neutral: Vec<UnitStats>,
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
        // The neutral side takes no faction modifiers — it has no country.
        let neutral = rules
            .entities()
            .map(|(kind, _)| resolve_one(rules, kind, None))
            .collect();

        StatTable {
            per_player,
            neutral,
        }
    }

    /// Stats for a player's copy of a kind.
    ///
    /// Falls back to defaults for an unknown player or kind rather than
    /// panicking: a stale entity id can outlive its owner, and a crash there
    /// would be a far worse failure than a unit that briefly cannot move.
    pub fn get(&self, player: PlayerId, kind: EntityKind) -> UnitStats {
        if player.is_neutral() {
            return self
                .neutral
                .get(kind.0 as usize)
                .copied()
                .unwrap_or_default();
        }
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
        h.write_u32(self.neutral.len() as u32);
        for stats in &self.neutral {
            h.write(stats);
        }
    }
}

/// Interns crush class names to a bitmask.
///
/// The original's classes are few and fixed, so a bitmask is both faster than
/// string comparison on the movement path and free of the iteration-order
/// hazard a per-unit `Vec<String>` would carry.
fn crush_mask(classes: &[String]) -> u8 {
    classes.iter().fold(0u8, |acc, c| {
        acc | match c.as_str() {
            "infantry" => 1,
            "light" => 2,
            "heavy" => 4,
            _ => 8,
        }
    })
}

fn resolve_one(rules: &Rules, kind: EntityKind, faction: Option<&str>) -> UnitStats {
    let def = rules.entity(kind);

    // Physical size and passability both come from the unit now, not from its
    // category. See docs/adr/0006-capability-is-data-not-category.md.
    let mut stats = UnitStats {
        ..UnitStats::default()
    };

    for t in &def.traits {
        match t {
            Trait::Health { max, .. } => stats.max_health = *max,
            Trait::Mobile {
                speed,
                turn_rate,
                locomotor,
                surfaces,
                size,
                layer,
            } => {
                stats.speed = cells_per_tick(*speed);
                stats.turn_rate = degrees_per_second_to_tick(*turn_rate);
                stats.locomotor = *locomotor;
                stats.mobile = true;
                // The unit's own list wins; the locomotor is only a default.
                stats.movement = SurfaceMask::from_surfaces(
                    surfaces.as_deref().unwrap_or(locomotor.default_surfaces()),
                );
                stats.layer = layer.unwrap_or(locomotor.default_layer());
                stats.radius = Fx::from_raw(size.unwrap_or(locomotor.default_size()).to_fx_raw());
            }
            Trait::Vision { range } => stats.vision = Fx::from_raw(range.to_fx_raw()),
            Trait::Selectable { priority } => stats.selection_priority = *priority,
            // Crush classes are interned to a bitmask at load. The alternative
            // is comparing strings on the movement hot path, which is both slow
            // and — because the comparison order would come from a Vec built
            // per unit — a determinism hazard waiting to be introduced.
            Trait::Crushes { classes } => {
                stats.crushes = crush_mask(classes);
            }
            Trait::Crushable { class } => {
                stats.crush_class = crush_mask(std::slice::from_ref(class))
            }
            Trait::SelfHealing {
                per_tick,
                delay_after_damage,
            } => {
                stats.self_heal = per_tick.0.max(0) as u32;
                stats.heal_delay = delay_after_damage.0;
            }
            Trait::Explodes { damage, .. } => stats.death_damage = *damage,
            Trait::Transport { capacity, .. } => stats.capacity = *capacity,
            // Validated at load, so a missing entity is a rules error rather
            // than a unit that silently refuses to deploy.
            Trait::HidesGround { radius } => stats.hides_ground = Fx::from_raw(radius.to_fx_raw()),
            Trait::Supported {
                radius,
                bonus_percent,
                max_supporters,
                self_powered_at,
                ..
            } => {
                stats.support_radius = Fx::from_raw(radius.to_fx_raw());
                stats.support_bonus_percent = *bonus_percent;
                stats.max_supporters = *max_supporters;
                stats.self_powered_at = *self_powered_at;
            }
            Trait::WeaponFromCargo => stats.weapon_from_cargo = true,
            Trait::Grows {
                radius,
                interval,
                cell_limit,
            } => {
                stats.grow_radius = Fx::from_raw(radius.to_fx_raw());
                stats.grow_interval = interval.0;
                stats.grow_cell_limit = *cell_limit;
            }
            Trait::Leaves { units, .. } => stats.leaves_something = !units.is_empty(),
            Trait::Wanders { radius, interval } => {
                stats.wander_radius = Fx::from_raw(radius.to_fx_raw());
                stats.wander_interval = interval.0;
            }
            Trait::Superweapon { charge, .. } => stats.charge_time = charge.0,
            Trait::NeedsAdjacent { terrain } => {
                stats.needs_adjacent = Some(match terrain {
                    redshift_data::map::Ground::Land => Terrain::Ground,
                    redshift_data::map::Ground::Water => Terrain::Water,
                    redshift_data::map::Ground::Rock => Terrain::Rock,
                })
            }
            Trait::Connects => stats.connects = true,
            Trait::PlantsCharge { .. } => stats.plants_charges = true,
            Trait::Disguised { looks_like } => stats.disguised_as = rules.kind_of(looks_like),
            Trait::Unsellable => stats.sellable = false,
            Trait::Bridge => stats.is_bridge = true,
            Trait::RepairsBridges { radius } => stats.bridge_repair_radius = *radius,
            Trait::Deploys { into } => stats.deploys_into = rules.kind_of(into),
            Trait::Garrisonable {
                capacity,
                evict_below_percent,
                ..
            } => {
                stats.garrison_capacity = *capacity;
                stats.evict_below_percent = *evict_below_percent;
            }
            Trait::Repairs {
                rate,
                cost_percent,
                cures_infestation,
                ..
            } => {
                stats.repair_rate = *rate;
                stats.repair_cost_percent = *cost_percent;
                stats.cures_infestation = *cures_infestation;
            }
            Trait::BuildLimit { max } => stats.build_limit = *max,
            Trait::Bounty { credits } => stats.bounty = *credits,
            Trait::Capturable => stats.capturable = true,
            Trait::Engineer { consumed } => {
                stats.is_engineer = true;
                stats.consumed_on_use = *consumed;
            }
            Trait::Harvester {
                capacity,
                gather_rate,
            } => {
                stats.harvest_capacity = Some(*capacity);
                stats.gather_rate = gather_rate.0.max(0) as u32;
            }
            Trait::Refinery { .. } => stats.is_refinery = true,
            Trait::Veterancy {
                kills_for_veteran,
                kills_for_elite,
            } => stats.veterancy = Some((*kills_for_veteran, *kills_for_elite)),
            Trait::Cloakable { recloak_delay } => {
                stats.cloakable = true;
                stats.recloak_delay = recloak_delay.0;
            }
            Trait::Submersible { resurface_delay } => {
                stats.submersible = true;
                stats.resurface_delay = resurface_delay.0;
            }
            Trait::Sonar => stats.sonar = true,
            Trait::Detector => stats.detector = true,
            Trait::PowerSupply { output } => stats.power_supply = *output,
            Trait::PowerDraw {
                amount,
                works_unpowered,
            } => {
                stats.power_draw = *amount;
                stats.works_unpowered = *works_unpowered;
            }
            Trait::Footprint { width, height } => {
                stats.footprint = ((*width).max(1), (*height).max(1))
            }
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
                    surfaces: None,
                    size: None,
                    layer: None,
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
        // The bitmask is the authority; there used to be a `can_crush` bool
        // beside it that nothing ever read.
        assert!(stats.crushes != 0);
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
