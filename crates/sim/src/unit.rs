//! Units: what they are and how they move.
//!
//! Phase 0 keeps a unit deliberately thin — enough to select it, order it
//! somewhere, and watch it get there. Health, weapons, vision and the rest
//! arrive in Phase 3 as data-driven traits (see `docs/05-data-and-modding.md`).

use serde::{Deserialize, Serialize};

use redshift_data::rules::EntityKind;

use crate::arena::EntityId;
use crate::command::PlayerId;
use crate::fx::{Angle, Fx};
use crate::hash::{StateHash, StateHasher};
use crate::map::{Cell, WorldPos};

/// How close to a waypoint counts as having arrived.
///
/// A unit cannot land exactly on a point in fixed-point arithmetic, so without
/// a tolerance it would oscillate around every waypoint forever. An eighth of a
/// cell is tight enough to look precise and loose enough to always be reached.
pub const ARRIVAL_TOLERANCE: Fx = Fx::from_raw(65536 / 8);

/// How closely a unit must face its heading before it will drive forward.
///
/// Roughly 22.5°. Moving before turning looks like drifting; waiting for a
/// perfect alignment looks hesitant. This matches the original's feel of units
/// pivoting, then committing.
/// How far off the mark a weapon may be and still fire, in binary angle units.
///
/// About 5°. Wide enough that a unit is not paralysed by a target strafing
/// across its arc, narrow enough that shots visibly come from a barrel pointed
/// the right way.
pub const FIRING_ARC: u16 = 1024;

/// Below this separation, two units count as exactly coincident and the push
/// direction has to come from somewhere other than their positions.
pub const SEPARATION_EPSILON: Fx = Fx::from_raw(64);

/// The most a unit may be displaced by separation in one tick.
///
/// A unit in the middle of a dense press accumulates a push from every
/// neighbour. Without a cap it would be flung clear rather than shuffling
/// aside, and the crowd would visibly explode.
pub const MAX_SEPARATION_STEP: Fx = Fx::from_raw(6553);

pub const MOVE_ALIGNMENT: u16 = 4096;

/// A harvester's cycle.
///
/// Held beside the order rather than inside it, for the same reason combat
/// state is: a harvester *moves* while it harvests, and the two answer
/// different questions. "Where am I going" is an order and goes through the
/// ordinary movement and pathfinding code, which is already tested. "What am I
/// doing when I get there" is this.
///
/// Folding them together would have meant teaching the path service a second
/// kind of destination, and duplicating the repath and partial-route handling
/// that took two rounds of bugs to get right.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct HarvestState {
    pub stage: HarvestStage,
    /// The cell being worked, once one has been chosen.
    pub field: Option<Cell>,
    /// Ore aboard.
    pub load: u32,
    /// Ticks until the next bite.
    pub gather_delay: u32,
}

impl Default for HarvestState {
    fn default() -> Self {
        HarvestState {
            stage: HarvestStage::Approaching,
            field: None,
            load: 0,
            gather_delay: 0,
        }
    }
}

impl StateHash for HarvestState {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_u8(self.stage as u8);
        match self.field {
            Some(cell) => {
                h.write_u8(1);
                h.write(&cell);
            }
            None => h.write_u8(0),
        }
        h.write_u32(self.load);
        h.write_u32(self.gather_delay);
    }
}

/// Where a harvester is in its cycle.
///
/// Explicit rather than inferred from whether the load is full: "walking to a
/// field" and "walking home" look identical from the load alone at the moment
/// the first bite lands, and a harvester that guesses wrong turns round in the
/// middle of the field.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum HarvestStage {
    /// Walking to a chosen field.
    Approaching,
    /// Standing on ore, taking bites.
    Gathering,
    /// Walking to a refinery with a load.
    Returning,
}

/// What a unit is currently trying to do.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Order {
    /// Nothing to do.
    #[default]
    Idle,
    /// Going somewhere, ignoring everything on the way.
    Move(Travel),
    /// Going somewhere, but stopping to fight whatever is met.
    ///
    /// The distinction is the whole reason both exist. A plain move is for
    /// repositioning and should not be derailed; an attack-move is for
    /// advancing into contested ground and should be.
    AttackMove(Travel),
    /// Closing on a specific target and killing it.
    Attack {
        target: EntityId,
        /// How to get within range. Recomputed as the target moves.
        approach: Travel,
    },
    /// Holding a position and engaging whatever comes near.
    Guard {
        post: Cell,
        /// Set while walking back to the post after being drawn off it.
        returning: Option<Travel>,
    },
    /// Walking to a transport in order to board it.
    ///
    /// A separate order rather than a flag on `Move`, because arriving is not
    /// the end of it: the unit has to still want to board when it gets there,
    /// and the transport may have filled up or driven off in the meantime.
    Board {
        transport: EntityId,
        approach: Travel,
    },
    /// Walking into a building to capture or repair it.
    ///
    /// One order rather than two, because the original made it one action: the
    /// engineer enters, and what happens depends on whose building it was.
    Enter { target: EntityId, approach: Travel },
}

/// Where a unit is going and how far it has got.
///
/// Shared by every order that involves movement rather than repeated three
/// times. The repetition would have been the obvious way to add attack-move,
/// and it would have meant three copies of the repath and partial-route
/// handling that took two rounds of bugs to settle.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Travel {
    pub destination: Cell,
    /// Remaining waypoints. The next one is at `path[0]`.
    pub path: Vec<Cell>,
    /// Set when the route was partial, so the unit knows to ask again on
    /// arrival rather than assuming it is done.
    pub needs_repath: bool,
    /// Node budget for the next search. Raised on each retry so a unit in
    /// awkward terrain escalates instead of livelocking.
    pub retry_budget: u32,
}

impl Travel {
    pub fn to(destination: Cell, budget: u32) -> Travel {
        Travel {
            destination,
            path: Vec::new(),
            needs_repath: true,
            retry_budget: budget,
        }
    }
}

impl StateHash for Travel {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write(&self.destination);
        h.write_u32(self.path.len() as u32);
        for c in &self.path {
            h.write(c);
        }
        h.write_bool(self.needs_repath);
        h.write_u32(self.retry_budget);
    }
}

impl Order {
    pub fn is_idle(&self) -> bool {
        matches!(self, Order::Idle)
    }

    /// The travel state this order is following, if it is going anywhere.
    pub fn travel(&self) -> Option<&Travel> {
        match self {
            Order::Move(t) | Order::AttackMove(t) => Some(t),
            Order::Attack { approach, .. }
            | Order::Board { approach, .. }
            | Order::Enter { approach, .. } => Some(approach),
            Order::Guard { returning, .. } => returning.as_ref(),
            Order::Idle => None,
        }
    }

    pub fn travel_mut(&mut self) -> Option<&mut Travel> {
        match self {
            Order::Move(t) | Order::AttackMove(t) => Some(t),
            Order::Attack { approach, .. }
            | Order::Board { approach, .. }
            | Order::Enter { approach, .. } => Some(approach),
            Order::Guard { returning, .. } => returning.as_mut(),
            Order::Idle => None,
        }
    }

    /// Whether this order lets the unit stop and fight on the way.
    ///
    /// A plain move does not: a player repositioning an army expects it to
    /// arrive, not to stop at the first thing that shoots at it.
    pub fn engages_on_the_way(&self) -> bool {
        matches!(
            self,
            Order::AttackMove(_) | Order::Attack { .. } | Order::Guard { .. } | Order::Idle
        )
    }
}

/// A thing in the world.
///
/// Deliberately thin. A unit carries only what changes — where it is, what it
/// is doing, how much health is left. Everything constant comes from its
/// [`EntityKind`] via the resolved stat table, so a unit is small and the same
/// numbers cannot drift between two copies of the same kind.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Unit {
    pub owner: PlayerId,
    /// What this is, as defined in the rules.
    pub kind: EntityKind,
    pub pos: WorldPos,
    pub facing: Angle,
    /// Remaining health. Its maximum lives in the stat table.
    pub health: u32,
    pub order: Order,
    /// The harvest cycle, for units that gather ore.
    ///
    /// `None` for everything else, so the harvester pass can skip them without
    /// consulting the rules.
    pub harvest: Option<crate::unit::HarvestState>,
    /// The build queue, for buildings that produce.
    ///
    /// `None` for everything else, so the production pass can skip most of the
    /// world without consulting the rules.
    pub production: Option<crate::production::ProductionQueue>,
    /// Kills to this unit's name.
    pub kills: u32,
    /// The transport this is riding in, if any.
    ///
    /// A unit aboard something is still in the arena — it keeps its identity,
    /// its health and its rank — but it must be skipped by everything that
    /// acts on the world: movement, targeting, vision, separation, crushing
    /// and drawing. Missing one of those is the classic bug here, and it looks
    /// like a passenger shooting from inside a sealed truck.
    pub carrier: Option<EntityId>,
    /// Who is riding in this, in the order they boarded.
    pub cargo: Vec<EntityId>,
    /// Ticks since it last took damage.
    ///
    /// Counted up like [`Unit::since_fired`], so a unit that has never been hit
    /// is not perpetually one tick away from an event that already fired.
    pub since_damaged: u32,
    /// Ticks since it last fired.
    ///
    /// Counted up rather than down, so a unit that never fires is not
    /// perpetually one tick from being uncloaked by an integer that already
    /// hit zero.
    pub since_fired: u32,
    /// Targeting and reload state.
    ///
    /// Deliberately separate from [`Order`]: a unit shoots *while* moving.
    /// Folding the two together would force a choice between advancing and
    /// firing that the original never made.
    pub combat: crate::combat::CombatState,
}

impl Unit {
    pub fn new(owner: PlayerId, kind: EntityKind, pos: WorldPos, max_health: u32) -> Unit {
        Unit {
            owner,
            kind,
            pos,
            facing: Angle::ZERO,
            health: max_health,
            order: Order::Idle,
            kills: 0,
            carrier: None,
            cargo: Vec::new(),
            since_damaged: u32::MAX,
            since_fired: u32::MAX,
            harvest: None,
            production: None,
            combat: crate::combat::CombatState::default(),
        }
    }

    pub fn cell(&self) -> Cell {
        self.pos.cell()
    }

    pub fn is_alive(&self) -> bool {
        self.health > 0
    }

    /// Whether this unit is riding inside something.
    ///
    /// The single check every pass over the world has to make. Named rather
    /// than inlined as `carrier.is_some()` so that grepping for it finds every
    /// place that remembered.
    #[inline]
    pub fn is_aboard(&self) -> bool {
        self.carrier.is_some()
    }

    /// Applies damage, saturating at zero.
    ///
    /// Saturating rather than wrapping: an overkill shot on a nearly-dead unit
    /// would otherwise wrap to enormous health, and the unit would become
    /// unkillable in a way that looks like a rendering glitch.
    pub fn take_damage(&mut self, amount: u32) {
        self.health = self.health.saturating_sub(amount);
        if amount > 0 {
            self.since_damaged = 0;
        }
    }
}

impl StateHash for Unit {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write(&self.owner);
        h.write_u16(self.kind.0);
        h.write(&self.pos);
        h.write_u16(self.facing.raw());
        h.write_u32(self.health);
        match &self.harvest {
            Some(state) => {
                h.write_u8(1);
                h.write(state);
            }
            None => h.write_u8(0),
        }
        match &self.production {
            Some(queue) => {
                h.write_u8(1);
                h.write(queue);
            }
            None => h.write_u8(0),
        }
        h.write_u32(self.kills);
        match self.carrier {
            Some(id) => {
                h.write_u8(1);
                h.write_u32(id.index());
                h.write_u32(id.generation());
            }
            None => h.write_u8(0),
        }
        h.write_u32(self.cargo.len() as u32);
        for id in &self.cargo {
            h.write_u32(id.index());
            h.write_u32(id.generation());
        }
        h.write_u32(self.since_damaged);
        h.write_u32(self.since_fired);
        h.write(&self.combat);
        match &self.order {
            Order::Idle => h.write_u8(0),
            Order::Move(t) => {
                h.write_u8(1);
                h.write(t);
            }
            Order::AttackMove(t) => {
                h.write_u8(2);
                h.write(t);
            }
            Order::Attack { target, approach } => {
                h.write_u8(3);
                h.write_u32(target.index());
                h.write_u32(target.generation());
                h.write(approach);
            }
            Order::Enter { target, approach } => {
                h.write_u8(6);
                h.write_u32(target.index());
                h.write_u32(target.generation());
                h.write(approach);
            }
            Order::Board {
                transport,
                approach,
            } => {
                h.write_u8(5);
                h.write_u32(transport.index());
                h.write_u32(transport.generation());
                h.write(approach);
            }
            Order::Guard { post, returning } => {
                h.write_u8(4);
                h.write(post);
                match returning {
                    Some(t) => {
                        h.write_u8(1);
                        h.write(t);
                    }
                    None => h.write_u8(0),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(pos: WorldPos) -> Unit {
        Unit::new(PlayerId(0), EntityKind(0), pos, 100)
    }

    fn hash(u: &Unit) -> u64 {
        let mut h = StateHasher::new();
        h.write(u);
        h.finish()
    }

    #[test]
    fn a_new_unit_is_idle_at_its_position() {
        let pos = Cell::new(4, 5).centre();
        let u = sample(pos);
        assert!(u.order.is_idle());
        assert_eq!(u.cell(), Cell::new(4, 5));
        assert_eq!(u.pos, pos);
        assert!(u.is_alive());
    }

    #[test]
    fn a_unit_at_zero_health_is_not_alive() {
        let mut u = sample(WorldPos::ORIGIN);
        u.health = 0;
        assert!(!u.is_alive());
    }

    #[test]
    fn arrival_tolerance_is_reachable_in_one_step() {
        // A unit that cannot cover the tolerance in a single tick would stop
        // short of every waypoint and never finish a path. Speed now comes from
        // the rules, so this checks the shipped values rather than a constant.
        let rules = crate::sim::test_rules();
        let stats = crate::stats::StatTable::resolve(&rules, &[None]);
        let speed = stats.get(PlayerId(0), crate::sim::TEST_KIND).speed;
        assert!(
            speed > ARRIVAL_TOLERANCE,
            "speed {speed:?} must exceed the arrival tolerance {ARRIVAL_TOLERANCE:?}"
        );
    }

    #[test]
    fn hashing_covers_the_kind_and_health() {
        // Two units differing only in what they are, or in how hurt they are,
        // must not hash alike — both change how the match plays out.
        let base = sample(WorldPos::ORIGIN);

        let mut other_kind = base.clone();
        other_kind.kind = EntityKind(1);
        assert_ne!(hash(&base), hash(&other_kind));

        let mut hurt = base.clone();
        hurt.health -= 1;
        assert_ne!(hash(&base), hash(&hurt));
    }

    #[test]
    fn hashing_covers_the_order() {
        let mut a = sample(WorldPos::ORIGIN);
        let base = hash(&a);
        a.order = Order::Move(Travel {
            destination: Cell::new(1, 1),
            path: vec![Cell::new(1, 1)],
            needs_repath: false,
            retry_budget: 100,
        });
        assert_ne!(
            hash(&a),
            base,
            "an order change must be visible in the hash"
        );
    }
}
