//! Weapons, targeting and damage.
//!
//! # The damage model
//!
//! Faithful to the original: a weapon carries a **warhead**, a thing has an
//! **armour class**, and a table gives the multiplier between them. That single
//! indirection is what produces the whole rock-paper-scissors structure the
//! genre runs on — rifles shred infantry and scratch tanks, shells do the
//! reverse — and it stays entirely in data.
//!
//! # Determinism notes
//!
//! Two things here would desync if written carelessly:
//!
//! - **Target selection.** "Nearest enemy" must break ties the same way on
//!   every machine, so distance ties fall back to entity id. Iterating in arena
//!   order and keeping the first strict improvement gets that for free.
//! - **Damage arithmetic.** The multiplier is applied on a widened intermediate
//!   in one expression. Splitting it into a multiply and a divide would round
//!   twice, and two peers whose compilers ordered those differently would
//!   disagree by one point of damage — which is enough to change who dies.

use serde::{Deserialize, Serialize};

use redshift_data::rules::{EntityKind, Rules, WeaponDef};
use redshift_data::traits::{Layer, Trait};
use redshift_data::value::Percent;

use crate::arena::EntityId;
use crate::command::PlayerId;
use crate::fx::{Fx, FxWide};
use crate::hash::{StateHash, StateHasher};
use crate::map::WorldPos;
use crate::unit::Unit;

/// A unit's weapon, resolved from the rules.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct WeaponStats {
    pub damage: u32,
    /// Index into the armour table's warhead list, resolved once.
    pub warhead: WarheadId,
    /// Ticks between shots.
    pub reload: u32,
    /// Maximum engagement distance.
    pub range: Fx,
    /// Squared range, precomputed so the hot path never takes a square root.
    pub range_sq: FxWide,
    pub splash_radius: Fx,
    /// Cells the shot travels per tick. Zero means it lands instantly.
    pub projectile_speed: Fx,
    /// Layers this weapon can engage.
    pub targets: LayerMask,
    /// Kills outright rather than dealing damage.
    pub instant_kill: bool,
    /// Shots before rearming. Zero means unlimited.
    pub ammo: u32,
    /// Whether this can shoot down projectiles in flight.
    pub intercepts: bool,
    /// Whether the shot follows its target.
    ///
    /// A missile hits what it was aimed at; a shell flies to where the target
    /// *was*. That difference is most of what separates artillery from a tank
    /// gun, and it is a weapon property rather than a global rule.
    pub homing: bool,
    pub turret: bool,
    /// Binary angle units the turret traverses per tick.
    pub turret_rate: u16,
}

/// The set of layers a weapon can engage.
///
/// A bitmask for the same reason [`crate::map::SurfaceMask`] is: it is read on
/// every targeting decision and has to be `Copy`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[repr(transparent)]
pub struct LayerMask(u8);

impl LayerMask {
    pub const GROUND: LayerMask = LayerMask(1);
    pub const AIR: LayerMask = LayerMask(2);
    pub const BOTH: LayerMask = LayerMask(3);

    const fn bit(layer: Layer) -> u8 {
        match layer {
            Layer::Ground => 1,
            Layer::Air => 2,
        }
    }

    /// Wraps a raw mask. For combining two weapons' reach.
    #[inline]
    pub const fn from_raw(raw: u8) -> LayerMask {
        LayerMask(raw)
    }

    pub fn from_layers(layers: &[Layer]) -> LayerMask {
        LayerMask(layers.iter().fold(0, |acc, l| acc | Self::bit(*l)))
    }

    #[inline]
    pub fn engages(self, layer: Layer) -> bool {
        self.0 & Self::bit(layer) != 0
    }

    #[inline]
    pub fn raw(self) -> u8 {
        self.0
    }
}

impl WeaponStats {
    /// The same weapon with its reach scaled by a percentage.
    ///
    /// Returns a copy rather than mutating, because the resolved stats are
    /// shared across every unit of a kind — scaling in place would give the
    /// whole army a hill's advantage the moment one unit climbed one.
    pub fn with_range_percent(mut self, percent: u32) -> WeaponStats {
        if percent == 100 {
            return self;
        }
        let scaled = ((self.range.raw() as i64 * percent as i64) / 100) as i32;
        self.range = Fx::from_raw(scaled);
        self.range_sq = self.range.sq();
        self
    }
}

/// A warhead, interned to an index at load so the hot path compares integers
/// rather than strings.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize,
)]
pub struct WarheadId(pub u16);

/// An armour class, interned the same way.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize,
)]
pub struct ArmourId(pub u16);

/// The warhead-versus-armour multiplier table, flattened.
///
/// A dense `Vec` indexed by `warhead * armour_count + armour` rather than a map
/// of strings: this is read once per shot, and a string hash per shot would be
/// both slower and — if the map were ever iterated — a determinism hazard.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DamageTable {
    armour_count: usize,
    multipliers: Vec<Percent>,
}

impl DamageTable {
    /// Flattens the rules' armour table.
    pub fn build(rules: &Rules) -> DamageTable {
        let armour = rules.armour();
        let armour_classes = armour.classes();
        let warheads = armour.warheads();

        let mut multipliers = Vec::with_capacity(warheads.len() * armour_classes.len());
        for warhead in &warheads {
            for class in armour_classes {
                multipliers.push(armour.multiplier(warhead, class));
            }
        }
        DamageTable {
            armour_count: armour_classes.len(),
            multipliers,
        }
    }

    /// The multiplier for a warhead against an armour class.
    ///
    /// An out-of-range pairing yields full damage, matching the rules layer's
    /// "unlisted means nothing special" rule. Immunity by omission would be
    /// invisible until a match hinged on it.
    #[inline]
    pub fn multiplier(&self, warhead: WarheadId, armour: ArmourId) -> Percent {
        self.multipliers
            .get(warhead.0 as usize * self.armour_count + armour.0 as usize)
            .copied()
            .unwrap_or(Percent::FULL)
    }

    /// Damage after armour.
    ///
    /// One expression on a widened intermediate. Splitting the multiply and the
    /// divide would round twice, and two peers whose compilers ordered them
    /// differently could disagree by a point — enough to change who survives.
    #[inline]
    pub fn damage_against(&self, base: u32, warhead: WarheadId, armour: ArmourId) -> u32 {
        let percent = self.multiplier(warhead, armour);
        ((base as i64 * percent.0 as i64) / 100).clamp(0, u32::MAX as i64) as u32
    }
}

impl StateHash for DamageTable {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_u32(self.armour_count as u32);
        for m in &self.multipliers {
            h.write_i32(m.0);
        }
    }
}

/// What a unit is doing about shooting.
///
/// Kept separate from [`crate::unit::Order`] because a unit shoots *while*
/// moving. Folding the two together would force a choice between advancing and
/// firing that the original never made.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct CombatState {
    /// Ticks until the weapon can fire again.
    pub reload_remaining: u32,
    /// Current target, if any.
    pub target: Option<EntityId>,
    /// Where the turret points. Equals the hull facing when there is no turret.
    pub turret_facing: crate::fx::Angle,
    /// Shots fired since last rearming.
    ///
    /// Counted up rather than down so that a unit created before its kind had
    /// an ammunition limit is not born empty.
    pub shots_fired: u32,
}

impl StateHash for CombatState {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_u32(self.reload_remaining);
        match self.target {
            Some(id) => {
                h.write_u8(1);
                h.write_u32(id.index());
                h.write_u32(id.generation());
            }
            None => h.write_u8(0),
        }
        h.write_u16(self.turret_facing.raw());
        h.write_u32(self.shots_fired);
    }
}

/// One resolved hit, produced by the targeting pass and applied by the damage
/// pass.
///
/// Shots are collected and applied in two phases so that every unit chooses its
/// target against the *same* world. Applying damage as each unit fires would
/// mean a unit early in the arena could kill a target before a later unit got
/// to consider it — making the outcome depend on arena order in a way that is
/// deterministic but arbitrary, and that changes when slots are reused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PendingHit {
    pub attacker: EntityId,
    /// Whether this shot kills outright rather than dealing damage.
    pub instant_kill: bool,
    pub target: EntityId,
    pub damage: u32,
    pub warhead: WarheadId,
    pub splash_radius: Fx,
    pub at: WorldPos,
}

/// Resolves a unit's weapon from the rules.
pub fn weapon_of(
    rules: &Rules,
    kind: EntityKind,
    warhead_index: &dyn Fn(&str) -> WarheadId,
) -> Option<WeaponStats> {
    let def = rules.entity(kind);
    let (weapon_id, turret, turret_rate) = def.traits.iter().find_map(|t| match t {
        Trait::Armed {
            weapon,
            turret,
            turret_rate,
        } => Some((weapon, *turret, *turret_rate)),
        _ => None,
    })?;
    build_weapon(rules.weapon(weapon_id)?, turret, turret_rate, warhead_index)
}

/// Turns a weapon definition into resolved stats.
///
/// Shared by the primary and secondary lookups rather than written twice: two
/// copies would be one edit away from a unit's second weapon behaving subtly
/// differently from its first.
fn build_weapon(
    weapon: &WeaponDef,
    turret: bool,
    turret_rate: u32,
    warhead_index: &dyn Fn(&str) -> WarheadId,
) -> Option<WeaponStats> {
    let range = Fx::from_raw(weapon.range.to_fx_raw());
    Some(WeaponStats {
        damage: weapon.damage,
        warhead: warhead_index(&weapon.warhead),
        reload: weapon.reload.0,
        range,
        range_sq: range.sq(),
        splash_radius: Fx::from_raw(weapon.splash_radius.to_fx_raw()),
        projectile_speed: Fx::from_raw(
            weapon.projectile_speed.to_fx_raw() / crate::TICKS_PER_SECOND as i32,
        ),
        homing: weapon.homing,
        instant_kill: weapon.instant_kill,
        ammo: weapon.ammo,
        intercepts: weapon.intercepts,
        targets: if weapon.targets.is_empty() {
            // Ground only. The default almost every weapon wants, and the one
            // that keeps every existing rules file working unchanged.
            LayerMask::GROUND
        } else {
            LayerMask::from_layers(&weapon.targets)
        },
        turret,
        turret_rate: crate::stats::degrees_per_second_to_tick(turret_rate),
    })
}

/// Resolves a kind's secondary weapon, if it declares one.
pub fn secondary_of(
    rules: &Rules,
    kind: EntityKind,
    warhead_index: &dyn Fn(&str) -> WarheadId,
) -> Option<WeaponStats> {
    let def = rules.entity(kind);
    let (weapon_id, turret, turret_rate) = def.traits.iter().find_map(|t| match t {
        Trait::Secondary {
            weapon,
            turret,
            turret_rate,
        } => Some((weapon, *turret, *turret_rate)),
        _ => None,
    })?;
    build_weapon(rules.weapon(weapon_id)?, turret, turret_rate, warhead_index)
}

/// Picks a target for one unit.
///
/// Nearest hostile in range, breaking ties by entity id. The tie-break comes
/// free from iterating in arena order and only accepting a *strict*
/// improvement — but it has to be strict, or two equidistant enemies would be
/// chosen by whichever the loop saw last, which changes as slots are reused.
pub fn choose_target(
    attacker: EntityId,
    attacker_unit: &Unit,
    weapon: &WeaponStats,
    units: &crate::arena::Arena<Unit>,
    alliance: &dyn Fn(PlayerId, PlayerId) -> bool,
) -> Option<EntityId> {
    choose_target_where(
        attacker,
        attacker_unit,
        weapon,
        units,
        alliance,
        &|_| true,
        &|_| Layer::Ground,
    )
}

/// As [`choose_target`], but only considering targets `can_see` accepts.
///
/// The filter is a parameter rather than being folded in, so that the tie-break
/// logic has exactly one implementation. A second copy that also checked
/// visibility would be one edit away from breaking ties differently — and a
/// tie-break that differs between two code paths is a desync waiting for the
/// right battle.
pub fn choose_target_where(
    attacker: EntityId,
    attacker_unit: &Unit,
    weapon: &WeaponStats,
    units: &crate::arena::Arena<Unit>,
    alliance: &dyn Fn(PlayerId, PlayerId) -> bool,
    can_see: &dyn Fn(&Unit) -> bool,
    layer_of: &dyn Fn(&Unit) -> Layer,
) -> Option<EntityId> {
    let mut best: Option<(EntityId, FxWide)> = None;

    for (id, other) in units.iter() {
        if id == attacker || !other.is_alive() {
            continue;
        }
        if alliance(attacker_unit.owner, other.owner) {
            continue;
        }
        if !can_see(other) {
            continue;
        }
        // A weapon that cannot reach this layer does not acquire the target at
        // all. Leaving it out only of the damage table would let a tank lock
        // onto an aircraft and fire at it uselessly for the rest of the match,
        // while ignoring the enemy walking past.
        if !weapon.targets.engages(layer_of(other)) {
            continue;
        }
        let dx = other.pos.x - attacker_unit.pos.x;
        let dy = other.pos.y - attacker_unit.pos.y;
        let distance_sq = Fx::dist_sq(dx, dy);
        if distance_sq > weapon.range_sq {
            continue;
        }
        if best.is_none_or(|(_, d)| distance_sq < d) {
            best = Some((id, distance_sq));
        }
    }
    best.map(|(id, _)| id)
}

/// Whether a target is still valid: alive, hostile, and in range.
pub fn target_is_valid(
    attacker_unit: &Unit,
    target: EntityId,
    weapon: &WeaponStats,
    units: &crate::arena::Arena<Unit>,
    alliance: &dyn Fn(PlayerId, PlayerId) -> bool,
    layer_of: &dyn Fn(&Unit) -> Layer,
) -> bool {
    let Some(other) = units.get(target) else {
        return false;
    };
    if !other.is_alive() || alliance(attacker_unit.owner, other.owner) {
        return false;
    }
    if !weapon.targets.engages(layer_of(other)) {
        return false;
    }
    let dx = other.pos.x - attacker_unit.pos.x;
    let dy = other.pos.y - attacker_unit.pos.y;
    Fx::dist_sq(dx, dy) <= weapon.range_sq
}

/// The armour class of a kind, resolved once at load.
pub fn armour_of(
    rules: &Rules,
    kind: EntityKind,
    armour_index: &dyn Fn(&str) -> ArmourId,
) -> ArmourId {
    rules
        .entity(kind)
        .traits
        .iter()
        .find_map(|t| match t {
            Trait::Health { armour, .. } => Some(armour_index(armour)),
            _ => None,
        })
        .unwrap_or_default()
}

/// Resolved combat data for every entity kind.
///
/// Built alongside [`StatTable`] and for the same reason: this is read on the
/// hot path, and resolving strings there would be both slow and a determinism
/// hazard.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CombatTable {
    weapons: Vec<Option<WeaponStats>>,
    armour: Vec<ArmourId>,
    /// A second weapon, for units that need one for ground and one for air.
    secondary: Vec<Option<WeaponStats>>,
    /// Warhead of each kind's death explosion, resolved here because this is
    /// where warhead names are interned. A second index built elsewhere could
    /// disagree with this one, and a damage lookup that silently used the wrong
    /// warhead would be very hard to see.
    death_warhead: Vec<WarheadId>,
    /// What each kind does once it has burrowed into something. Here rather
    /// than in [`crate::stats::UnitStats`] for the same reason as
    /// `death_warhead`: this is where warhead names become indices, and a
    /// second index built somewhere else could quietly disagree with this one.
    infestation: Vec<Option<Infestation>>,
    /// The weapon an occupied building fires with. Its own, not its occupants'
    /// — the exact opposite of how a transport that changes weapon by cargo
    /// would work, and the thing most easily got backwards.
    garrison_weapon: Vec<Option<WeaponStats>>,
    /// The weapon each kind hands to a transport it rides in. Resolved here
    /// with every other weapon, so a turret mode is the same sort of thing as a
    /// turret.
    crew_weapon: Vec<Option<WeaponStats>>,
    /// What each kind does to the ground it stands on. Interned here with the
    /// other warheads, for the same reason as all of them.
    contamination: Vec<Option<Contamination>>,
    damage: DamageTable,
}

/// How fast a crewed turret traverses.
///
/// A full turn a second. The passenger's own rules say nothing about turrets —
/// it is infantry — so the mode it grants needs a figure from somewhere, and a
/// named constant is better than the same magic number in two places.
const DEFAULT_TURRET_RATE: u32 = 3600;

/// What a contaminating unit does to the ground around it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Contamination {
    pub radius: Fx,
    pub damage: u32,
    pub warhead: WarheadId,
    pub lingers: u32,
}

/// What a parasite does to its host, per tick.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Infestation {
    pub damage: u32,
    pub warhead: WarheadId,
}

impl CombatTable {
    pub fn build(rules: &Rules) -> CombatTable {
        let armour_table = rules.armour();
        let classes = armour_table.classes().to_vec();
        let warheads = armour_table.warheads();

        let armour_index =
            |name: &str| ArmourId(classes.iter().position(|c| c == name).unwrap_or(0) as u16);
        let warhead_index =
            |name: &str| WarheadId(warheads.iter().position(|w| w == name).unwrap_or(0) as u16);

        let mut weapons = Vec::with_capacity(rules.entity_count());
        let mut armour = Vec::with_capacity(rules.entity_count());
        let mut secondary = Vec::with_capacity(rules.entity_count());
        let mut death_warhead = Vec::with_capacity(rules.entity_count());
        let mut infestation: Vec<Option<Infestation>> = Vec::with_capacity(rules.entity_count());
        let mut garrison_weapon: Vec<Option<WeaponStats>> =
            Vec::with_capacity(rules.entity_count());
        let mut contamination: Vec<Option<Contamination>> =
            Vec::with_capacity(rules.entity_count());
        let mut crew_weapon: Vec<Option<WeaponStats>> = Vec::with_capacity(rules.entity_count());
        for (kind, def) in rules.entities() {
            weapons.push(weapon_of(rules, kind, &warhead_index));
            armour.push(armour_of(rules, kind, &armour_index));
            secondary.push(secondary_of(rules, kind, &warhead_index));
            death_warhead.push(
                def.traits
                    .iter()
                    .find_map(|t| match t {
                        Trait::Explodes { warhead, .. } => Some(warhead_index(warhead)),
                        _ => None,
                    })
                    .unwrap_or_default(),
            );
            infestation.push(def.traits.iter().find_map(|t| match t {
                Trait::Infests {
                    damage, warhead, ..
                } => Some(Infestation {
                    damage: *damage,
                    warhead: warhead_index(warhead),
                }),
                _ => None,
            }));
            garrison_weapon.push(def.traits.iter().find_map(|t| match t {
                Trait::Garrisonable { weapon, .. } => {
                    // No turret: a building does not traverse, it just shoots
                    // out of whichever window faces the target.
                    build_weapon(rules.weapon(weapon)?, false, 0, &warhead_index)
                }
                _ => None,
            }));
            crew_weapon.push(def.traits.iter().find_map(|t| match t {
                // A crewed weapon always has a turret: it is a turret mode.
                Trait::Crews { weapon } => build_weapon(
                    rules.weapon(weapon)?,
                    true,
                    DEFAULT_TURRET_RATE,
                    &warhead_index,
                ),
                _ => None,
            }));
            contamination.push(def.traits.iter().find_map(|t| match t {
                Trait::Contaminates {
                    radius,
                    damage,
                    warhead,
                    lingers,
                } => Some(Contamination {
                    radius: Fx::from_raw(radius.to_fx_raw()),
                    damage: *damage,
                    warhead: warhead_index(warhead),
                    lingers: lingers.0,
                }),
                _ => None,
            }));
        }

        CombatTable {
            weapons,
            armour,
            secondary,
            death_warhead,
            infestation,
            garrison_weapon,
            contamination,
            crew_weapon,
            damage: DamageTable::build(rules),
        }
    }

    #[inline]
    pub fn weapon(&self, kind: EntityKind) -> Option<&WeaponStats> {
        self.weapons.get(kind.0 as usize).and_then(|w| w.as_ref())
    }

    /// The warhead of a kind's death explosion.
    #[inline]
    pub fn death_warhead(&self, kind: EntityKind) -> WarheadId {
        self.death_warhead
            .get(kind.0 as usize)
            .copied()
            .unwrap_or_default()
    }

    /// The weapon this fires while occupied, if it can be occupied at all.
    pub fn garrison_weapon(&self, kind: EntityKind) -> Option<&WeaponStats> {
        self.garrison_weapon.get(kind.0 as usize)?.as_ref()
    }

    /// The weapon a passenger of this kind hands to a transport built to take
    /// one.
    pub fn crew_weapon(&self, kind: EntityKind) -> Option<&WeaponStats> {
        self.crew_weapon
            .get(kind.0 as usize)
            .and_then(|w| w.as_ref())
    }

    /// What this kind does to the ground around it, if it does anything.
    pub fn contamination(&self, kind: EntityKind) -> Option<Contamination> {
        self.contamination.get(kind.0 as usize).copied().flatten()
    }

    /// What this kind does to a host it has burrowed into, if it does that.
    pub fn infestation(&self, kind: EntityKind) -> Option<Infestation> {
        self.infestation.get(kind.0 as usize).copied().flatten()
    }

    /// A kind's second weapon, if it has one.
    #[inline]
    pub fn secondary(&self, kind: EntityKind) -> Option<&WeaponStats> {
        self.secondary.get(kind.0 as usize).and_then(|w| w.as_ref())
    }

    /// The weapon this kind would use against a target in `layer`.
    ///
    /// Primary first, then the secondary. A unit with an anti-air missile and a
    /// ground cannon uses whichever reaches, rather than being asked to choose
    /// a stance.
    pub fn weapon_for(&self, kind: EntityKind, layer: Layer) -> Option<&WeaponStats> {
        self.weapon(kind)
            .filter(|w| w.targets.engages(layer))
            .or_else(|| self.secondary(kind).filter(|w| w.targets.engages(layer)))
    }

    #[inline]
    pub fn armour(&self, kind: EntityKind) -> ArmourId {
        self.armour
            .get(kind.0 as usize)
            .copied()
            .unwrap_or_default()
    }

    #[inline]
    pub fn damage_table(&self) -> &DamageTable {
        &self.damage
    }

    /// Damage one kind's weapon does to another kind.
    pub fn damage_between(&self, attacker: EntityKind, target: EntityKind) -> u32 {
        match self.weapon(attacker) {
            Some(w) => self
                .damage
                .damage_against(w.damage, w.warhead, self.armour(target)),
            None => 0,
        }
    }

    pub fn is_armed(&self, kind: EntityKind) -> bool {
        self.weapon(kind).is_some()
    }
}

impl StateHash for CombatTable {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_u32(self.weapons.len() as u32);
        for weapon in &self.weapons {
            match weapon {
                Some(w) => {
                    h.write_u8(1);
                    h.write_u32(w.damage);
                    h.write_u16(w.warhead.0);
                    h.write_u32(w.reload);
                    h.write_i32(w.range.raw());
                    h.write_i32(w.splash_radius.raw());
                    h.write_i32(w.projectile_speed.raw());
                    h.write_bool(w.homing);
                    h.write_u8(w.targets.raw());
                    h.write_bool(w.instant_kill);
                    h.write_u32(w.ammo);
                    h.write_bool(w.intercepts);
                    h.write_bool(w.turret);
                    h.write_u16(w.turret_rate);
                }
                None => h.write_u8(0),
            }
        }
        for a in &self.armour {
            h.write_u16(a.0);
        }
        for w in &self.death_warhead {
            h.write_u16(w.0);
        }
        h.write(&self.damage);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::Arena;
    use redshift_data::rules::{ArmourTable, EntityDef, WeaponDef};
    use redshift_data::traits::{Locomotor, Trait};
    use redshift_data::value::{Hundredths, Ticks};

    /// A miniature rules set with the counterplay the genre runs on: rifles
    /// shred infantry and scratch tanks, shells do the reverse.
    fn combat_rules() -> Rules {
        let armour: ArmourTable = ron::from_str(
            r#"(
                classes: ["none", "light", "heavy"],
                table: {
                    "small_arms": { "none": 100, "light": 60, "heavy": 10 },
                    "ap_shell":   { "none": 40,  "light": 90, "heavy": 100 },
                },
            )"#,
        )
        .expect("armour table");

        let weapons = vec![
            WeaponDef {
                id: "rifle".into(),
                damage: 20,
                warhead: "small_arms".into(),
                reload: Ticks(10),
                range: Hundredths(400),
                splash_radius: Hundredths::ZERO,
                projectile_speed: Hundredths::ZERO,
                homing: false,
                targets: vec![],
                instant_kill: false,
                ammo: 0,
                intercepts: false,
            },
            WeaponDef {
                id: "cannon".into(),
                damage: 50,
                warhead: "ap_shell".into(),
                reload: Ticks(40),
                range: Hundredths(600),
                splash_radius: Hundredths(30),
                projectile_speed: Hundredths(2000),
                homing: false,
                targets: vec![],
                instant_kill: false,
                ammo: 0,
                intercepts: false,
            },
        ];

        let entities = vec![
            EntityDef {
                id: "rifleman".into(),
                name_key: "unit.rifleman".into(),
                side: None,
                category: "infantry".into(),
                traits: vec![
                    Trait::Health {
                        max: 100,
                        armour: "none".into(),
                    },
                    Trait::Mobile {
                        speed: Hundredths(200),
                        turn_rate: 360,
                        locomotor: Locomotor::Foot,
                        surfaces: None,
                        size: None,
                        layer: None,
                    },
                    Trait::Vision {
                        range: Hundredths(800),
                    },
                    Trait::Armed {
                        weapon: "rifle".into(),
                        turret: false,
                        turret_rate: 0,
                    },
                ],
            },
            EntityDef {
                id: "tank".into(),
                name_key: "unit.tank".into(),
                side: None,
                category: "vehicle".into(),
                traits: vec![
                    Trait::Health {
                        max: 400,
                        armour: "heavy".into(),
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
                        range: Hundredths(800),
                    },
                    Trait::Armed {
                        weapon: "cannon".into(),
                        turret: true,
                        turret_rate: 120,
                    },
                ],
            },
            EntityDef {
                id: "crate".into(),
                name_key: "prop.crate".into(),
                side: None,
                category: "structure".into(),
                // Deliberately unarmed, to pin what happens with no weapon.
                traits: vec![Trait::Health {
                    max: 50,
                    armour: "light".into(),
                }],
            },
        ];

        Rules::from_parts(entities, weapons, armour, Vec::new()).expect("valid rules")
    }

    fn kinds(rules: &Rules) -> (EntityKind, EntityKind, EntityKind) {
        (
            rules.kind_of("rifleman").unwrap(),
            rules.kind_of("tank").unwrap(),
            rules.kind_of("crate").unwrap(),
        )
    }

    /// Everyone is hostile to everyone else.
    fn all_hostile(_: PlayerId, _: PlayerId) -> bool {
        false
    }

    fn unit_at(owner: u8, kind: EntityKind, x: i32, y: i32, health: u32) -> Unit {
        Unit::new(
            PlayerId(owner),
            kind,
            crate::map::Cell::new(x, y).centre(),
            health,
        )
    }

    #[test]
    fn the_armour_table_produces_counterplay() {
        // The whole reason for the warhead indirection. If these numbers ever
        // flatten out, the game loses its structure.
        let rules = combat_rules();
        let table = CombatTable::build(&rules);
        let (rifleman, tank, _) = kinds(&rules);

        let rifle_vs_infantry = table.damage_between(rifleman, rifleman);
        let rifle_vs_tank = table.damage_between(rifleman, tank);
        let cannon_vs_infantry = table.damage_between(tank, rifleman);
        let cannon_vs_tank = table.damage_between(tank, tank);

        assert_eq!(rifle_vs_infantry, 20, "rifles do full damage to infantry");
        assert_eq!(rifle_vs_tank, 2, "rifles barely scratch armour");
        assert_eq!(cannon_vs_infantry, 20, "shells overpenetrate infantry");
        assert_eq!(cannon_vs_tank, 50, "shells are what armour fears");

        assert!(
            rifle_vs_infantry > rifle_vs_tank,
            "the rifle must prefer soft targets"
        );
        assert!(
            cannon_vs_tank > cannon_vs_infantry,
            "the cannon must prefer hard targets"
        );
    }

    #[test]
    fn an_unlisted_pairing_does_full_damage_rather_than_none() {
        // Immunity by omission is invisible until a match hinges on it.
        let rules = combat_rules();
        let table = CombatTable::build(&rules);
        let damage = table.damage_table();
        assert_eq!(
            damage.damage_against(100, WarheadId(99), ArmourId(99)),
            100,
            "an unknown pairing must not silently confer immunity"
        );
    }

    #[test]
    fn an_unarmed_thing_has_no_weapon_and_does_no_damage() {
        let rules = combat_rules();
        let table = CombatTable::build(&rules);
        let (_, _, crate_kind) = kinds(&rules);

        assert!(!table.is_armed(crate_kind));
        assert!(table.weapon(crate_kind).is_none());
        assert_eq!(table.damage_between(crate_kind, crate_kind), 0);
    }

    #[test]
    fn weapon_stats_come_across_from_the_rules() {
        let rules = combat_rules();
        let table = CombatTable::build(&rules);
        let (rifleman, tank, _) = kinds(&rules);

        let rifle = table.weapon(rifleman).expect("the rifleman is armed");
        assert_eq!(rifle.damage, 20);
        assert_eq!(rifle.reload, 10);
        assert_eq!(rifle.range, Fx::from_raw(Hundredths(400).to_fx_raw()));
        assert!(!rifle.turret, "infantry have no turret");

        let cannon = table.weapon(tank).expect("the tank is armed");
        assert!(cannon.turret);
        assert!(cannon.turret_rate > 0);
        assert!(cannon.range > rifle.range, "the cannon outranges the rifle");
    }

    #[test]
    fn range_is_precomputed_as_a_square() {
        // The hot path compares squared distances, so a square root per shot
        // would be pure waste — and `Fx::sqrt` truncates, so comparing rounded
        // distances could disagree with comparing exact squares at the boundary.
        let rules = combat_rules();
        let table = CombatTable::build(&rules);
        let (rifleman, _, _) = kinds(&rules);
        let rifle = table.weapon(rifleman).unwrap();
        assert_eq!(rifle.range_sq, rifle.range.sq());
    }

    #[test]
    fn the_nearest_hostile_in_range_is_chosen() {
        let rules = combat_rules();
        let table = CombatTable::build(&rules);
        let (rifleman, _, _) = kinds(&rules);
        let weapon = *table.weapon(rifleman).unwrap();

        let mut units = Arena::new();
        let attacker = units.insert(unit_at(0, rifleman, 0, 0, 100));
        let far = units.insert(unit_at(1, rifleman, 3, 0, 100));
        let near = units.insert(unit_at(1, rifleman, 1, 0, 100));

        let chosen = choose_target(
            attacker,
            units.get(attacker).unwrap(),
            &weapon,
            &units,
            &all_hostile,
        );
        assert_eq!(chosen, Some(near), "the nearer enemy should be chosen");
        assert_ne!(chosen, Some(far));
    }

    #[test]
    fn equidistant_targets_break_the_tie_by_arena_order() {
        // Two enemies at exactly the same distance must be resolved the same
        // way on every machine. Accepting only a *strict* improvement means the
        // first in arena order wins — and arena order is itself deterministic.
        let rules = combat_rules();
        let table = CombatTable::build(&rules);
        let (rifleman, _, _) = kinds(&rules);
        let weapon = *table.weapon(rifleman).unwrap();

        let mut units = Arena::new();
        let attacker = units.insert(unit_at(0, rifleman, 5, 5, 100));
        let first = units.insert(unit_at(1, rifleman, 3, 5, 100));
        let second = units.insert(unit_at(1, rifleman, 7, 5, 100));

        let a = units.get(attacker).unwrap();
        let dx1 = units.get(first).unwrap().pos.x - a.pos.x;
        let dx2 = units.get(second).unwrap().pos.x - a.pos.x;
        assert_eq!(
            Fx::dist_sq(dx1, Fx::ZERO),
            Fx::dist_sq(dx2, Fx::ZERO),
            "the test needs them genuinely equidistant"
        );

        for _ in 0..20 {
            assert_eq!(
                choose_target(attacker, a, &weapon, &units, &all_hostile),
                Some(first),
                "the tie must resolve the same way every time"
            );
        }
    }

    #[test]
    fn nothing_out_of_range_is_targeted() {
        let rules = combat_rules();
        let table = CombatTable::build(&rules);
        let (rifleman, _, _) = kinds(&rules);
        let weapon = *table.weapon(rifleman).unwrap();

        let mut units = Arena::new();
        let attacker = units.insert(unit_at(0, rifleman, 0, 0, 100));
        units.insert(unit_at(1, rifleman, 30, 30, 100));

        assert_eq!(
            choose_target(
                attacker,
                units.get(attacker).unwrap(),
                &weapon,
                &units,
                &all_hostile
            ),
            None
        );
    }

    #[test]
    fn allies_the_dead_and_oneself_are_never_targeted() {
        let rules = combat_rules();
        let table = CombatTable::build(&rules);
        let (rifleman, _, _) = kinds(&rules);
        let weapon = *table.weapon(rifleman).unwrap();

        let mut units = Arena::new();
        let attacker = units.insert(unit_at(0, rifleman, 0, 0, 100));
        // Same owner, adjacent.
        units.insert(unit_at(0, rifleman, 1, 0, 100));
        // Hostile but already dead.
        let mut corpse = unit_at(1, rifleman, 1, 1, 100);
        corpse.health = 0;
        units.insert(corpse);

        let same_owner = |a: PlayerId, b: PlayerId| a == b;
        assert_eq!(
            choose_target(
                attacker,
                units.get(attacker).unwrap(),
                &weapon,
                &units,
                &same_owner
            ),
            None,
            "only a hostile, living, in-range unit is a target"
        );
    }

    #[test]
    fn a_target_stops_being_valid_when_it_dies_or_walks_away() {
        let rules = combat_rules();
        let table = CombatTable::build(&rules);
        let (rifleman, _, _) = kinds(&rules);
        let weapon = *table.weapon(rifleman).unwrap();

        let mut units = Arena::new();
        let attacker = units.insert(unit_at(0, rifleman, 0, 0, 100));
        let target = units.insert(unit_at(1, rifleman, 2, 0, 100));

        let attacker_unit = units.get(attacker).unwrap().clone();
        assert!(target_is_valid(
            &attacker_unit,
            target,
            &weapon,
            &units,
            &all_hostile,
            &|_| Layer::Ground,
        ));

        // Out of range.
        units.get_mut(target).unwrap().pos = crate::map::Cell::new(40, 0).centre();
        assert!(!target_is_valid(
            &attacker_unit,
            target,
            &weapon,
            &units,
            &all_hostile,
            &|_| Layer::Ground,
        ));

        // Dead.
        units.get_mut(target).unwrap().pos = crate::map::Cell::new(2, 0).centre();
        units.get_mut(target).unwrap().health = 0;
        assert!(!target_is_valid(
            &attacker_unit,
            target,
            &weapon,
            &units,
            &all_hostile,
            &|_| Layer::Ground,
        ));

        // Gone entirely — a stale handle must not resolve to whatever now
        // occupies the slot.
        units.remove(target);
        assert!(!target_is_valid(
            &attacker_unit,
            target,
            &weapon,
            &units,
            &all_hostile,
            &|_| Layer::Ground,
        ));
    }

    #[test]
    fn the_tables_are_built_identically_every_time() {
        // They feed the state hash, so two peers must agree byte for byte.
        let rules = combat_rules();
        let mut a = StateHasher::new();
        let mut b = StateHasher::new();
        a.write(&CombatTable::build(&rules));
        b.write(&CombatTable::build(&rules));
        assert_eq!(a.finish(), b.finish());
    }

    #[test]
    fn warhead_order_does_not_depend_on_insertion() {
        // Warheads are interned to indices. A `BTreeMap` gives a sorted, stable
        // order; a `HashMap` would give a different index per process, and
        // every damage lookup after that would disagree between peers.
        let rules = combat_rules();
        let first = rules.armour().warheads();
        let second = rules.armour().warheads();
        assert_eq!(first, second);
        let mut sorted = first.clone();
        sorted.sort();
        assert_eq!(first, sorted, "warhead order must be the sorted one");
    }
}
