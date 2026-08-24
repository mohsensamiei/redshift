//! The simulation itself: state, the tick loop, and the read-only view.
//!
//! See the crate documentation for the rules this module lives under. The one
//! worth repeating: **the phase order inside [`Sim::tick`] is part of the
//! game's contract.** Reordering it changes behaviour and invalidates every
//! recorded replay.

use serde::{Deserialize, Serialize};

use redshift_data::rules::{EntityKind, Rules};

use crate::arena::{Arena, EntityId};
use crate::boons::Boons;
use crate::combat::{self, CombatTable, PendingHit, WeaponStats};
use crate::command::{Command, CommandKind, PlayerId};
use crate::economy::{self, Treasury};
use crate::fx::{Angle, Fx, FxWide};
use crate::hash::StateHasher;
use crate::map::{Cell, Locomotor, Map, SurfaceMask, WorldPos};
use crate::path::{self, DEFAULT_NODE_BUDGET, PathResult, PathWorkspace};
use crate::power::PowerGrid;
use crate::production::{ProductionItem, ProductionQueue};
use crate::projectile::{IMPACT_TOLERANCE, Projectile};
use crate::rank::Rank;
use crate::rng::SimRng;
use crate::stats::{StatTable, UnitStats};
use crate::unit::{
    ARRIVAL_TOLERANCE, FIRING_ARC, HarvestStage, MAX_SEPARATION_STEP, MOVE_ALIGNMENT, Order,
    SEPARATION_EPSILON, Travel, Unit,
};
use crate::vision::Visibility;
use crate::{TICKS_PER_SECOND, Tick};

/// Node expansions all pathfinding may spend in a single tick, across every
/// unit.
///
/// A shared per-tick ceiling is what keeps tick cost bounded no matter how many
/// units are given orders at once. Requests that do not fit are served on later
/// ticks, in entity order, so the outcome does not depend on how the budget
/// happened to divide.
pub const TICK_PATH_BUDGET: u32 = 20_000;

/// Minimal rules for tests and scenarios: one generic mobile unit.
///
/// Real matches load rules from `rules/`. This exists so a test about
/// pathfinding or netcode does not have to define a whole game first — and so
/// that when such a test fails, it is failing about the thing it is named
/// after.
pub fn test_rules() -> Rules {
    use redshift_data::rules::EntityDef;
    use redshift_data::traits::{Locomotor, Trait};
    use redshift_data::value::Hundredths;

    let armour: redshift_data::rules::ArmourTable =
        ron::from_str(r#"( classes: ["none"], table: { "generic": { "none": 100 } } )"#)
            .expect("the built-in test armour table should parse");

    let unit = EntityDef {
        id: "test_unit".into(),
        name_key: "unit.test".into(),
        side: None,
        category: "vehicle".into(),
        traits: vec![
            Trait::Health {
                max: 100,
                armour: "none".into(),
            },
            Trait::Mobile {
                // Three cells a second, and a full turn in about a second —
                // the same feel the hard-coded placeholder unit had, so tests
                // written against it keep their timings.
                speed: Hundredths(300),
                turn_rate: 330,
                locomotor: Locomotor::Tracked,
                surfaces: None,
                size: None,
                layer: None,
            },
            Trait::Vision {
                range: Hundredths(500),
            },
            Trait::Selectable { priority: 1 },
        ],
    };
    Rules::from_parts(vec![unit], Vec::new(), armour, Vec::new())
        .expect("the built-in test rules should validate")
}

/// The kind every [`test_rules`] spawn uses.
pub const TEST_KIND: EntityKind = EntityKind(0);

/// A player in the match.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayerSetup {
    pub id: PlayerId,
    /// Which country, if any. `None` simply means no modifiers apply.
    pub faction: Option<String>,
}

/// One starting unit.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Spawn {
    pub owner: PlayerId,
    pub kind: EntityKind,
    pub pos: WorldPos,
}

impl MatchSetup {
    /// A setup using [`test_rules`], with spawns given as `(owner, position)`.
    ///
    /// For tests and scenarios that care about movement or networking rather
    /// than about content.
    pub fn for_test(seed: u64, map: Map, spawns: Vec<(PlayerId, WorldPos)>) -> MatchSetup {
        let mut players: Vec<PlayerId> = spawns.iter().map(|(owner, _)| *owner).collect();
        players.sort();
        players.dedup();
        MatchSetup {
            seed,
            map,
            rules: test_rules(),
            players: players
                .into_iter()
                .map(|id| PlayerSetup { id, faction: None })
                .collect(),
            spawns: spawns
                .into_iter()
                .map(|(owner, pos)| Spawn {
                    owner,
                    kind: TEST_KIND,
                    pos,
                })
                .collect(),
        }
    }
}

/// How a match starts.
///
/// Carries the rules rather than referring to them. Every peer must simulate
/// against byte-identical rules, and owning them makes a saved match
/// self-contained — a replay from six months ago still describes the game it
/// was recorded from.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MatchSetup {
    pub seed: u64,
    pub map: Map,
    pub rules: Rules,
    pub players: Vec<PlayerSetup>,
    pub spawns: Vec<Spawn>,
}

/// One cell of ground made dangerous.
///
/// The damage and warhead are carried per cell rather than looked up from
/// whatever laid it, deliberately: contamination outlives its source, and a
/// patch that stopped hurting the moment the Desolator died would be a slow gun
/// rather than an area denied.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Hazard {
    pub cell: Cell,
    /// The tick this stops being dangerous.
    pub until: Tick,
    pub damage: u32,
    pub warhead: crate::combat::WarheadId,
    /// Who laid it, so a kill is credited. A stale id once the source is dead
    /// is fine — the rank lookup simply finds nothing.
    pub source: EntityId,
}

/// The 3×3 block of cells a unit can overlap something in.
///
/// A fixed array rather than a computed range, so the visiting order is part of
/// the code and cannot drift.
const NEIGHBOURHOOD: [(i32, i32); 9] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (0, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// How far a formation may spread from the ordered target, in cells.
///
/// Generous enough for a large selection, bounded so a group ordered into a
/// pocket does not scatter across the map looking for room.
const FORMATION_MAX_RADIUS: i32 = 6;

/// Offsets forming the ring at `radius`, in a fixed order.
///
/// Built rather than stored because the outer rings are large, but the order is
/// deterministic: top edge left to right, then right edge, then bottom right to
/// left, then left edge. Two peers walking this produce the same formation.
pub(crate) fn ring_offsets(radius: i32) -> Vec<(i32, i32)> {
    if radius == 0 {
        return vec![(0, 0)];
    }
    let mut out = Vec::with_capacity((radius as usize * 8).max(8));
    for x in -radius..=radius {
        out.push((x, -radius));
    }
    for y in (-radius + 1)..=radius {
        out.push((radius, y));
    }
    for x in (-radius..radius).rev() {
        out.push((x, radius));
    }
    for y in (-radius + 1..radius).rev() {
        out.push((-radius, y));
    }
    out
}

/// How close a harvester must get to a refinery to unload.
///
/// Generous, because a refinery occupies several cells and a harvester that
/// has to reach the exact centre would push through the building.
const UNLOAD_RANGE: Fx = Fx::from_frac(250, 100);

/// Credits every player starts with.
///
/// A placeholder until match settings are data. Enough to build an opening base
/// without being able to skip the early economy.
const STARTING_CREDITS: u32 = 5_000;

/// Builds a unit with whatever state its kind implies.
///
/// Shared by match setup and runtime spawning. They used to be separate, and
/// diverged exactly as you would expect: a building placed by the player did
/// not claim its ground, so units walked straight through it while a building
/// that started the match blocked correctly.
fn new_unit(owner: PlayerId, kind: EntityKind, pos: WorldPos, stats: UnitStats) -> Unit {
    let mut unit = Unit::new(owner, kind, pos, stats.max_health);
    // A harvester begins its cycle the moment it exists. Waiting for an order
    // would mean telling each one to start working, which the original never
    // did.
    if stats.harvest_capacity.is_some() {
        unit.harvest = Some(crate::unit::HarvestState::default());
    }
    unit
}

/// Claims or releases the ground under a building.
///
/// A no-op for anything a single cell in size, since units are kept apart by
/// their radius rather than by occupancy.
fn claim_footprint(map: &mut Map, centre: Cell, footprint: (u8, u8), claim: bool) {
    if footprint == (1, 1) {
        return;
    }
    let origin = footprint_origin(centre, footprint);
    map.set_blocked(origin, footprint.0, footprint.1, claim);
}

/// The top-left cell of a footprint centred on `centre`.
///
/// A building's position is its centre, but its footprint is laid out from a
/// corner. Deriving one from the other in a single place keeps placement,
/// occupancy and release from disagreeing — three sites that must all pick the
/// same cells or a building leaves a permanent hole in the map when it dies.
pub fn footprint_origin(centre: Cell, footprint: (u8, u8)) -> Cell {
    Cell::new(
        centre.x - (footprint.0 as i32 - 1) / 2,
        centre.y - (footprint.1 as i32 - 1) / 2,
    )
}

/// How far around a building to look for somewhere to stand while entering it.
///
/// Wider than the largest footprint's half-width, so a unit can always find the
/// edge of the building it was sent to.
const ENTRY_SEARCH_RADIUS: i32 = 4;

/// How far from a factory a new unit will look for somewhere to stand.
const EXIT_SEARCH_RADIUS: i32 = 5;

/// How far from an existing structure a player may build, in cells.
///
/// The constraint that makes base layout a decision: expanding means building
/// outward step by step rather than dropping a barracks wherever it would be
/// most annoying. Not verified against the original's exact figure — flagged
/// in TODO.md alongside the low-power ratio.
const BUILD_RADIUS: i32 = 8;

/// How far a guarding unit may be drawn from its post before walking back.
///
/// Without a leash a guard would chase a target across the map and the order
/// would be indistinguishable from attack-move.
const GUARD_LEASH: i32 = 4;

/// How close a passenger must get to a transport to climb aboard.
const BOARDING_RANGE: i32 = 2;

/// How close an engineer must get to a building to enter it.
const ENTRY_RANGE: i32 = 2;

/// What proportion of a structure's cost selling returns, as a percentage.
///
/// Not verified against the original, and flagged in TODO.md with the other
/// unverified rates.
const SELL_REFUND_PERCENT: u32 = 50;

/// Health per tick, in hundredths, granted by a repair-everywhere effect.
///
/// Not verified against the original, and flagged in TODO.md with the other
/// unverified rates.
const BOON_REPAIR_RATE: u32 = 50;

/// How far around a transport passengers are set down.
const UNLOAD_SPREAD: i32 = 4;

/// How far a death explosion reaches, in cells.
///
/// A single figure for now. The original varies it by warhead, which is where
/// it belongs once weapons carry their own blast radius for this purpose.
const DEATH_BLAST_RADIUS: Fx = Fx::from_frac(150, 100);

/// How far around a refinery a harvester will look for somewhere to pull up.
///
/// Wider than the largest footprint, so a harvester can always find the edge of
/// the building it is aiming for.
const UNLOAD_APPROACH: i32 = 4;

/// The simulated world.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sim {
    tick: Tick,
    map: Map,
    units: Arena<Unit>,
    rng: SimRng,
    /// Units waiting for a path, in the order they asked. A plain queue rather
    /// than a set, so servicing order is defined.
    path_queue: Vec<EntityId>,
    /// Scratch space for pathfinding. Excluded from the state hash: it is a
    /// cache, not game state.
    #[serde(skip, default = "empty_workspace")]
    workspace: PathWorkspace,
    rules: Rules,
    /// Stats resolved once per player and kind, so nothing recomputes them.
    ///
    /// Serialised rather than skipped: a loaded match must be able to move its
    /// units, and skipping this would leave every unit with default stats —
    /// stationary, and with no indication why.
    stats: StatTable,
    treasury: Treasury,
    /// Shots between firing and landing.
    projectiles: Vec<Projectile>,
    power: PowerGrid,
    boons: Boons,
    /// Ground that is dangerous to stand on.
    ///
    /// Sparse and sorted by cell, not a layer on the map. Contamination covers
    /// a handful of cells and expires; a dense array would be one `u32` per
    /// cell of a large map, hashed every tick, to describe almost nothing.
    ///
    /// On the simulation rather than on [`Map`] because it is dynamic and
    /// tick-bound. The map describes what the ground *is*; this describes what
    /// has happened to it.
    hazards: Vec<Hazard>,
    /// Effects a player keeps for the rest of the match.
    ///
    /// [`Boons`] is rebuilt from scratch every tick from the buildings a player
    /// owns, which is right for a machine shop and wrong for a spy: the spy is
    /// consumed and the barracks stays the victim's, so there is no standing
    /// source to rebuild from. These are replayed into the boons after each
    /// rebuild.
    ///
    /// A `Vec` per player, kept sorted and deduplicated, because it is hashed.
    standing: Vec<Vec<redshift_data::traits::PlayerEffect>>,
    /// The tick each player's blackout ends. Zero means none.
    blackout_until: Vec<Tick>,
    /// Kinds each player may build regardless of their own tech tree, having
    /// stolen the technology. Sorted, and hashed.
    unlocked: Vec<Vec<EntityKind>>,
    visibility: Visibility,
    /// Reused between ticks so separation does not allocate every frame.
    #[serde(skip)]
    separation_buckets: Vec<Vec<EntityId>>,
    /// Weapons, armour classes and the damage table, resolved once at load for
    /// the same reason the stat table is: this is read on the hot path.
    ///
    /// Serialised with the rest of the world rather than skipped. Skipping it
    /// would leave a restored match with no weapons at all — every unit would
    /// quietly stop shooting, with nothing to say why. It also means a desync
    /// dump carries the exact tables that were in play.
    combat: CombatTable,
    players: Vec<PlayerSetup>,
}

fn empty_workspace() -> PathWorkspace {
    PathWorkspace::new(0)
}

impl Sim {
    pub fn new(setup: MatchSetup) -> Sim {
        let factions: Vec<Option<String>> = {
            // Indexed by player id, so a table lookup is an array read. Players
            // need not be contiguous, so the vector is sized to the highest id.
            let highest = setup
                .players
                .iter()
                .map(|p| p.id.0 as usize)
                .max()
                .unwrap_or(0);
            let mut out = vec![None; highest + 1];
            for player in &setup.players {
                out[player.id.0 as usize] = player.faction.clone();
            }
            out
        };
        let stats = StatTable::resolve(&setup.rules, &factions);

        let mut map = setup.map;
        let mut units = Arena::with_capacity(setup.spawns.len());
        for spawn in &setup.spawns {
            let spawn_stats = stats.get(spawn.owner, spawn.kind);
            let unit = new_unit(
                spawn.owner,
                spawn.kind,
                map.clamp_pos(spawn.pos),
                spawn_stats,
            );
            // The same split as `spawn_unit`. These two paths diverged once
            // before — a building placed mid-match did not claim its ground
            // while one that started the match did — so any rule about
            // footprints has to be applied in both.
            if spawn_stats.is_bridge {
                let origin = footprint_origin(unit.cell(), spawn_stats.footprint);
                map.set_bridged(
                    origin,
                    spawn_stats.footprint.0,
                    spawn_stats.footprint.1,
                    true,
                );
            } else {
                claim_footprint(&mut map, unit.cell(), spawn_stats.footprint, true);
            }
            units.insert(unit);
        }
        let workspace = PathWorkspace::new(map.cell_count());
        let (map_width, map_height) = (map.width() as u16, map.height() as u16);
        let mut sim = Sim {
            tick: 0,
            map,
            units,
            rng: SimRng::new(setup.seed),
            path_queue: Vec::new(),
            workspace,
            combat: CombatTable::build(&setup.rules),
            separation_buckets: Vec::new(),
            treasury: Treasury::new(factions.len(), STARTING_CREDITS),
            projectiles: Vec::new(),
            power: PowerGrid::new(factions.len()),
            boons: Boons::new(factions.len()),
            hazards: Vec::new(),
            standing: vec![Vec::new(); factions.len()],
            blackout_until: vec![0; factions.len()],
            unlocked: vec![Vec::new(); factions.len()],
            visibility: Visibility::new(map_width, map_height, factions.len()),
            rules: setup.rules,
            stats,
            players: setup.players,
        };
        // Built before the world is handed out, not on the first tick. A base
        // that reported no power until something ticked would show the player
        // a blackout on the frame they started the match.
        sim.recompute_power();
        sim.recompute_boons();
        sim.update_vision();
        sim
    }

    pub fn rules(&self) -> &Rules {
        &self.rules
    }

    pub fn players(&self) -> &[PlayerSetup] {
        &self.players
    }

    /// The resolved stats for a unit, as its owner fields it.
    pub fn stats_of(&self, unit: &Unit) -> UnitStats {
        self.stats.get(unit.owner, unit.kind)
    }

    pub fn stats(&self) -> &StatTable {
        &self.stats
    }

    #[inline]
    pub fn tick_number(&self) -> Tick {
        self.tick
    }

    /// Seconds of simulated time elapsed.
    #[inline]
    pub fn elapsed_seconds(&self) -> u32 {
        self.tick / TICKS_PER_SECOND
    }

    #[inline]
    pub fn map(&self) -> &Map {
        &self.map
    }

    #[inline]
    pub fn units(&self) -> &Arena<Unit> {
        &self.units
    }

    #[inline]
    pub fn unit(&self, id: EntityId) -> Option<&Unit> {
        self.units.get(id)
    }

    /// Spawns a unit. Test and scenario setup only — in a match, units arrive
    /// through production, which is a command.
    pub fn spawn_unit(&mut self, owner: PlayerId, kind: EntityKind, pos: WorldPos) -> EntityId {
        let stats = self.stats.get(owner, kind);
        let unit = new_unit(owner, kind, self.map.clamp_pos(pos), stats);
        let cell = unit.cell();
        // A bridge is the one footprint that *opens* ground rather than closing
        // it. Everything else about it is an ordinary entity, which is what
        // lets Crazy Ivan blow one up through the usual damage path.
        if stats.is_bridge {
            self.open_span(cell, stats.footprint, true);
        } else {
            claim_footprint(&mut self.map, cell, stats.footprint, true);
        }
        self.units.insert(unit)
    }

    /// Opens or closes a bridge's span.
    fn open_span(&mut self, centre: Cell, footprint: (u8, u8), open: bool) {
        let origin = footprint_origin(centre, footprint);
        self.map.set_bridged(origin, footprint.0, footprint.1, open);
    }

    /// Advances the world by exactly one tick.
    ///
    /// `commands` must already be in total order — the network layer sorts by
    /// `(tick, player, sequence)` before handing them over. Each phase runs to
    /// completion for every unit before the next begins.
    pub fn tick(&mut self, commands: &[Command]) {
        debug_assert!(
            commands
                .windows(2)
                .all(|w| w[0].order_key() <= w[1].order_key()),
            "commands reached the simulation out of order"
        );

        self.apply_commands(commands);
        self.run_production();
        self.run_harvesters();
        self.update_engagements();
        self.service_path_requests();
        self.move_units();
        self.separate_units();
        self.intercept_projectiles();
        let landed = self.advance_projectiles();
        let hits = self.acquire_and_fire();
        // Parasites bite through the ordinary damage path, so armour still
        // means something and the kill is credited to the drone like any other.
        let bites = self.run_infestations();
        self.spread_contamination();
        let poison = self.run_hazards();
        self.resolve_damage(&landed);
        self.resolve_damage(&hits);
        self.resolve_damage(&bites);
        self.resolve_damage(&poison);
        self.crush_underfoot();
        // After damage and before deaths, so a building shot below the
        // threshold this tick throws its garrison out this tick.
        self.evict_broken_garrisons();
        self.regenerate();
        let blasts = self.detonate_the_dying();
        self.resolve_damage(&blasts);
        self.remove_the_dead();
        // After deaths, for the same reason the power grid is: a destroyed
        // scout should stop revealing ground on the tick it dies, not the one
        // after.
        self.update_vision();
        // After the deaths, so the grid describes the world as it stands at the
        // end of the tick — which is the world anyone looking at it will see.
        // Recomputing at the start instead left it a tick stale: a base whose
        // only plant had just been destroyed still reported itself powered.
        self.recompute_power();
        self.recompute_boons();

        self.tick += 1;
    }

    // -- Crushing, healing and death throes ----------------------------------

    /// Kills anything driven over by something heavy enough.
    ///
    /// Run after movement, so a unit is crushed where the tank ended up rather
    /// than where it started. Only enemies are crushed: driving over your own
    /// infantry would make large armies unmanageable, and the original did not
    /// do it either.
    fn crush_underfoot(&mut self) {
        let mut crushed: Vec<EntityId> = Vec::new();

        for (id, unit) in self.units.iter() {
            if unit.is_aboard() {
                continue;
            }
            let stats = self.stats.get(unit.owner, unit.kind);
            if stats.crushes == 0 || !stats.mobile {
                continue;
            }
            let cell = unit.cell();

            for (other_id, other) in self.units.iter() {
                if other_id == id || !other.is_alive() {
                    continue;
                }
                if Self::are_allied(unit.owner, other.owner) {
                    continue;
                }
                let other_stats = self.stats.get(other.owner, other.kind);
                // Nothing to crush, or not a class this can crush.
                if other_stats.crush_class == 0 || stats.crushes & other_stats.crush_class == 0 {
                    continue;
                }
                // Underfoot means the same cell, not merely nearby: a tank
                // squeezing past should not kill what it brushed.
                if other.cell() == cell {
                    crushed.push(other_id);
                }
            }
        }

        for id in crushed {
            if let Some(unit) = self.units.get_mut(id) {
                unit.health = 0;
            }
        }
    }

    /// Refreshes the ground under everything that poisons it, and forgets
    /// patches that have gone cold.
    ///
    /// Refreshed every tick rather than laid once, so a Desolator that walks
    /// away leaves a trail that expires behind it instead of a permanent scar.
    fn spread_contamination(&mut self) {
        let tick = self.tick;
        // Pruned first, so a patch that expired this tick is not immediately
        // re-laid by a source that is no longer there.
        self.hazards.retain(|h| h.until > tick);

        let mut laid: Vec<Hazard> = Vec::new();
        for (id, unit) in self.units.iter() {
            if !unit.is_alive() || unit.is_aboard() {
                continue;
            }
            let Some(c) = self.combat.contamination(unit.kind) else {
                continue;
            };
            let centre = unit.cell();
            let reach = c.radius.floor_int();
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let cell = Cell::new(centre.x + dx, centre.y + dy);
                    if !self.map.contains(cell) {
                        continue;
                    }
                    // Round, not square. A square patch would be visibly a
                    // square, and the radius would mean two different things
                    // along an axis and along a diagonal.
                    if Fx::dist_sq(Fx::from_int(dx), Fx::from_int(dy)) > c.radius.sq() {
                        continue;
                    }
                    laid.push(Hazard {
                        cell,
                        until: tick + c.lingers,
                        damage: c.damage,
                        warhead: c.warhead,
                        source: id,
                    });
                }
            }
        }

        for hazard in laid {
            match self.hazards.binary_search_by_key(&hazard.cell, |h| h.cell) {
                // Already dangerous. The later expiry wins rather than the
                // damage stacking: two Desolators standing together make an
                // area denied for longer, not a cell that kills twice as fast.
                Ok(at) => {
                    let existing = &mut self.hazards[at];
                    if hazard.until > existing.until {
                        *existing = hazard;
                    }
                }
                Err(at) => self.hazards.insert(at, hazard),
            }
        }
    }

    /// One tick of standing on poisoned ground.
    ///
    /// Returns hits rather than applying them, like every other source of
    /// damage, so armour decides who cares. That is the whole reason there is
    /// no "immune to radiation" flag: give the warhead a zero against vehicle
    /// armour and infantry die on ground a tank drives across, from the table
    /// that already exists.
    fn run_hazards(&mut self) -> Vec<PendingHit> {
        if self.hazards.is_empty() {
            return Vec::new();
        }
        let mut hits = Vec::new();
        for (id, unit) in self.units.iter() {
            if !unit.is_alive() || unit.is_aboard() {
                continue;
            }
            // Anything off the ground is out of it. Flight is the one thing
            // that crosses contaminated ground without touching it.
            if self.stats.get(unit.owner, unit.kind).layer == redshift_data::traits::Layer::Air {
                continue;
            }
            let Ok(at) = self.hazards.binary_search_by_key(&unit.cell(), |h| h.cell) else {
                continue;
            };
            let hazard = self.hazards[at];
            hits.push(PendingHit {
                attacker: hazard.source,
                instant_kill: false,
                target: id,
                damage: hazard.damage,
                warhead: hazard.warhead,
                splash_radius: Fx::ZERO,
                at: unit.pos,
            });
        }
        hits
    }

    /// Ground currently dangerous to stand on. For the renderer.
    pub fn hazards(&self) -> &[Hazard] {
        &self.hazards
    }

    /// One tick of every parasite eating its host.
    ///
    /// Returns hits rather than applying them, like every other source of
    /// damage, so a host killed by a drone dies through the same path as one
    /// killed by a tank — bounty, veterancy credit, death explosion and all.
    fn run_infestations(&mut self) -> Vec<PendingHit> {
        let mut bites = Vec::new();
        for (host_id, host) in self.units.iter() {
            let Some(parasite_id) = host.infestation else {
                continue;
            };
            // The parasite may have been killed by something that reached
            // inside — a cure, or its owner losing the match. A host still
            // pointing at a dead one would be eaten by a ghost.
            let Some(parasite) = self.units.get(parasite_id) else {
                continue;
            };
            if !parasite.is_alive() || !host.is_alive() {
                continue;
            }
            let Some(bite) = self.combat.infestation(parasite.kind) else {
                continue;
            };
            bites.push(PendingHit {
                attacker: parasite_id,
                instant_kill: false,
                target: host_id,
                damage: bite.damage,
                warhead: bite.warhead,
                splash_radius: Fx::ZERO,
                at: host.pos,
            });
        }
        bites
    }

    /// Whether a parasite can get inside this target.
    ///
    /// Category-matched like production and repair. A drone that cannot get
    /// into a thing still has its ordinary weapon for it, which is how the
    /// original's behaves like an attack dog against infantry and like
    /// something else entirely against a tank.
    fn may_infest(&self, parasite: EntityId, host: EntityId) -> bool {
        let (Some(p), Some(h)) = (self.units.get(parasite), self.units.get(host)) else {
            return false;
        };
        if !p.is_alive() || !h.is_alive() || p.is_aboard() || Self::are_allied(p.owner, h.owner) {
            return false;
        }
        // One at a time. A second drone piling into an occupied tank would
        // overwrite the first, quietly deleting a unit from the match.
        if h.infestation.is_some() {
            return false;
        }
        let host_category = &self.rules.entity(h.kind).category;
        self.rules.entity(p.kind).traits.iter().any(|t| match t {
            redshift_data::traits::Trait::Infests { categories, .. } => {
                categories.contains(host_category)
            }
            _ => false,
        })
    }

    /// A parasite gets inside its host.
    fn infest(&mut self, parasite: EntityId, host: EntityId) {
        if let Some(p) = self.units.get_mut(parasite) {
            // `carrier` is what takes it off the field: `is_aboard` already
            // means "not here to be shot at", which is exactly the state a
            // burrowed drone is in and the reason the counter has to be a
            // building rather than a gun.
            p.carrier = Some(host);
            p.order = Order::Idle;
            p.combat.target = None;
        }
        if let Some(h) = self.units.get_mut(host) {
            h.infestation = Some(parasite);
        }
    }

    /// Removes whatever has burrowed into a unit, killing it.
    ///
    /// The Service Depot's second job. Not an eviction — the drone does not get
    /// to crawl out and try again, or a player could farm one drone across a
    /// whole armoured column.
    fn cure_infestation(&mut self, host: EntityId) {
        let Some(parasite) = self.units.get(host).and_then(|h| h.infestation) else {
            return;
        };
        if let Some(p) = self.units.get_mut(parasite) {
            p.carrier = None;
            p.health = 0;
        }
        if let Some(h) = self.units.get_mut(host) {
            h.infestation = None;
        }
    }

    /// Regenerates units that have been left alone.
    ///
    /// The delay is what makes it a recovery mechanic rather than an armour
    /// bonus: a unit under fire heals nothing, and one pulled out of a fight
    /// comes back.
    fn regenerate(&mut self) {
        // Advanced here rather than in the firing pass, which only visits armed
        // units — an unarmed thing that healed only when something nearby
        // happened to be shooting would be a memorable bug.
        for (_, unit) in self.units.iter_mut() {
            unit.since_damaged = unit.since_damaged.saturating_add(1);
        }

        let healing: Vec<(EntityId, u32, u32)> = self
            .units
            .iter()
            .filter_map(|(id, unit)| {
                let stats = self.stats.get(unit.owner, unit.kind);
                if !unit.is_alive() {
                    return None;
                }
                // A player holding the repair boon heals everything they own,
                // whether or not the unit can normally repair itself. That is
                // the whole point of a captured machine shop.
                let rate = if stats.self_heal > 0 {
                    stats.self_heal
                } else if self.boons.repair_everywhere(unit.owner) {
                    BOON_REPAIR_RATE
                } else {
                    return None;
                };
                if unit.since_damaged < stats.heal_delay {
                    return None;
                }
                Some((id, rate, stats.max_health))
            })
            .collect();

        for (id, per_tick, max) in healing {
            if let Some(unit) = self.units.get_mut(id) {
                // Hundredths, accumulated by integer division rather than a
                // fractional carry: a unit healing 0.5 a tick gains 1 every
                // other tick, identically on every peer.
                let gain = (per_tick + (unit.since_damaged % 100)) / 100;
                unit.health = (unit.health + gain).min(max);
            }
        }
    }

    /// Collects the blasts from everything that is about to be removed.
    ///
    /// Chain reactions resolve one tick at a time: a unit killed by a blast
    /// detonates on the *next* tick rather than immediately. That is both
    /// bounded — no recursion to run away with — and visibly correct, since a
    /// chain of explosions should be a chain rather than a single event.
    fn detonate_the_dying(&mut self) -> Vec<PendingHit> {
        self.units
            .iter()
            .filter(|(_, u)| !u.is_alive())
            .filter_map(|(id, unit)| {
                let stats = self.stats.get(unit.owner, unit.kind);
                // Every other dead thing is removed at the end of this tick, so
                // it detonates exactly once. A wrecked bridge stays, and would
                // otherwise explode again every tick for the rest of the match.
                // This is the price of the one entity that outlives its own
                // death, and it is worth paying explicitly rather than by
                // hoping nobody gives a bridge a death charge.
                (stats.death_damage > 0 && !stats.is_bridge).then(|| PendingHit {
                    attacker: id,
                    instant_kill: false,
                    // No primary target: a blast is entirely splash, and hits
                    // whatever happens to be standing there.
                    target: EntityId::NONE,
                    damage: stats.death_damage,
                    warhead: self.combat.death_warhead(unit.kind),
                    splash_radius: DEATH_BLAST_RADIUS,
                    at: unit.pos,
                })
            })
            .collect()
    }

    // -- Vision --------------------------------------------------------------

    /// Rebuilds what each player can see.
    ///
    /// From scratch each tick rather than updated as units move. Incremental
    /// updates would mean clearing an old vision circle and stamping a new one
    /// on every step of every unit, and one missed clear leaves a permanently
    /// visible patch that nobody can account for. Explored ground is cumulative
    /// and never cleared, so only the visible layer is rebuilt.
    fn update_vision(&mut self) {
        if !self.visibility.is_enabled() {
            return;
        }
        self.visibility.begin_tick();

        // Gathered now and applied in two passes below, after the ordinary
        // revealing.
        let concealments: Vec<(PlayerId, Cell, Fx)> = self
            .units
            .iter()
            .filter(|(_, u)| u.is_alive() && !u.is_aboard())
            .filter(|(_, u)| self.stats.get(u.owner, u.kind).hides_ground > Fx::ZERO)
            // A generator with no power conceals nothing, like every other
            // structure that draws from the grid.
            .filter(|(_, u)| !self.is_unpowered(u))
            .map(|(_, u)| {
                (
                    u.owner,
                    u.cell(),
                    self.stats.get(u.owner, u.kind).hides_ground,
                )
            })
            .collect();
        for (_, unit) in self.units.iter() {
            if !unit.is_alive() {
                continue;
            }
            if unit.is_aboard() {
                continue;
            }
            let stats = self.stats.get(unit.owner, unit.kind);
            if stats.vision <= Fx::ZERO {
                continue;
            }
            // A radar with no power goes dark. Without this, losing a reactor
            // costs a player nothing they can see, which removes most of the
            // reason to attack one.
            if self.is_unpowered(unit) {
                continue;
            }
            // You can see further from a hill. Applied here as well as to
            // weapon range because the two have to agree: a unit that shot
            // further than it could see would be firing into fog, and one that
            // saw further than it shot would just be a spotter.
            let vision = Fx::from_raw(
                ((stats.vision.raw() as i64 * self.map.elevation_bonus(unit.cell()) as i64) / 100)
                    as i32,
            );
            self.visibility.reveal(unit.owner, unit.cell(), vision);
            // A detector reveals cloaked things across the same ground it
            // watches. A separate layer, because a player can see a patch of
            // ground perfectly well and still not see what is standing on it.
            if stats.detector {
                self.visibility
                    .reveal_cloaked(unit.owner, unit.cell(), vision);
            }
        }

        if concealments.is_empty() {
            return;
        }

        // Pass two: take the concealed ground back, after everyone has looked.
        // Before, and a watchtower far outside the area would simply see
        // straight in and the generator would do nothing at all.
        for (owner, at, radius) in &concealments {
            for index in 0..self.players.len() {
                let other = self.players[index].id;
                if Self::are_allied(*owner, other) {
                    continue;
                }
                self.visibility.hide(other, *at, *radius);
            }
        }

        // Pass three: anything actually standing inside the area sees around
        // itself again. That is what makes scouting the answer to a Gap
        // Generator rather than a counter-structure — you cannot look in from
        // outside, but you can walk in.
        //
        // A third pass rather than a cleverer hide, because "which cells would
        // this unit have seen" is a question `reveal` already answers, and a
        // second implementation of it would be one change away from disagreeing.
        let inside: Vec<(PlayerId, Cell, Fx, bool)> = self
            .units
            .iter()
            .filter(|(_, u)| u.is_alive() && !u.is_aboard() && !self.is_unpowered(u))
            .filter_map(|(_, u)| {
                let stats = self.stats.get(u.owner, u.kind);
                if stats.vision <= Fx::ZERO {
                    return None;
                }
                let cell = u.cell();
                let within = concealments.iter().any(|(owner, at, radius)| {
                    !Self::are_allied(*owner, u.owner)
                        && Fx::dist_sq(Fx::from_int(cell.x - at.x), Fx::from_int(cell.y - at.y))
                            <= radius.sq()
                });
                within.then_some((u.owner, cell, stats.vision, stats.detector))
            })
            .collect();
        for (owner, cell, vision, detector) in inside {
            self.visibility.reveal(owner, cell, vision);
            if detector {
                self.visibility.reveal_cloaked(owner, cell, vision);
            }
        }
    }

    /// What each player can see.
    pub fn visibility(&self) -> &Visibility {
        &self.visibility
    }

    /// Reveals the whole map to everyone.
    ///
    /// For replays and spectators, which should not be watching through one
    /// player's eyes.
    pub fn reveal_all(&mut self) {
        self.visibility.reveal_all();
    }

    /// Whether `player` can currently see `unit`.
    ///
    /// Used by targeting as well as by the interface, which is why it lives
    /// here: a unit that fired at something it could not see would be both
    /// unfair and — since the interface and the simulation would be working
    /// from different answers — a source of desyncs.
    pub fn can_see(&self, player: PlayerId, unit: &Unit) -> bool {
        // Inside a transport is not somewhere anyone can see. This one check
        // keeps a passenger out of targeting, out of the renderer, and out of
        // every "nearest enemy" search, which is why it lives here rather than
        // being repeated at each of them.
        if unit.is_aboard() {
            return false;
        }
        if !self.visibility.is_visible(player, unit.cell()) {
            return false;
        }
        // A cloaked unit stands in plain sight and still cannot be seen. Its
        // own side always sees it, or a player could not command their own
        // units — a rule the original never needed to state either.
        if unit.owner == player || !self.is_cloaked(unit) {
            return true;
        }
        self.visibility.is_detected(player, unit.cell())
    }

    /// Whether a unit is currently hidden.
    ///
    /// Cloak lapses when a unit fires and returns after a delay. That is the
    /// whole tension of the mechanic: staying hidden and doing something are
    /// mutually exclusive, so a cloaked unit is a threat rather than an
    /// invulnerability.
    pub fn is_cloaked(&self, unit: &Unit) -> bool {
        let stats = self.stats.get(unit.owner, unit.kind);
        stats.cloakable && unit.since_fired >= stats.recloak_delay
    }

    /// A unit's earned rank.
    pub fn rank_of(&self, unit: &Unit) -> Rank {
        Rank::for_kills(unit.kills, self.stats.get(unit.owner, unit.kind).veterancy)
    }

    // -- Power ---------------------------------------------------------------

    /// Rebuilds the power grid from what is standing.
    ///
    /// From scratch every tick rather than maintained incrementally. Keeping a
    /// running total would be faster and would have to be corrected on every
    /// spawn, death, capture and sale — and a single missed correction leaves a
    /// base permanently and invisibly wrong about its own power, which is close
    /// to impossible to notice from a bug report.
    fn recompute_power(&mut self) {
        self.power.clear();
        for (_, unit) in self.units.iter() {
            if !unit.is_alive() {
                continue;
            }
            let stats = self.stats.get(unit.owner, unit.kind);
            if stats.power_supply > 0 {
                self.power.add_supply(unit.owner, stats.power_supply);
            }
            if stats.power_draw > 0 {
                self.power.add_draw(unit.owner, stats.power_draw);
            }
        }

        // Last, because it cuts supply to nothing and adding plants afterwards
        // would undo it. A sabotaged player is short of power however many
        // reactors they own, which is what makes a spy in a power plant worth
        // the thousand credits.
        for (index, until) in self.blackout_until.iter().enumerate() {
            if self.tick < *until {
                self.power.black_out(PlayerId(index as u8));
            }
        }
    }

    /// Shots currently in the air.
    pub fn projectiles(&self) -> &[Projectile] {
        &self.projectiles
    }

    /// Rebuilds each player's standing modifiers from what is standing.
    ///
    /// From scratch every tick, like the power grid and for the same reason: a
    /// running total would need correcting on every spawn, death, capture and
    /// sale, and one missed correction leaves a player permanently and
    /// invisibly wrong about their own economy.
    fn recompute_boons(&mut self) {
        self.boons.clear();
        for (_, unit) in self.units.iter() {
            if !unit.is_alive() || unit.is_aboard() {
                continue;
            }
            // A source with no power grants nothing. An ore purifier that keeps
            // paying while blacked out would make cutting an enemy's power much
            // less worth doing.
            if self.is_unpowered(unit) {
                continue;
            }
            for effect in self
                .rules
                .entity(unit.kind)
                .traits
                .iter()
                .filter_map(|t| match t {
                    redshift_data::traits::Trait::Grants { effect } => Some(effect.clone()),
                    _ => None,
                })
            {
                self.boons.grant(unit.owner, effect);
            }
        }

        // Replayed after the rebuild, because these have no standing source to
        // be rebuilt from — the spy that won them is gone and the building it
        // entered is still the victim's.
        for (index, effects) in self.standing.iter().enumerate() {
            for effect in effects {
                self.boons.grant(PlayerId(index as u8), effect.clone());
            }
        }
    }

    /// Grants a player an effect for the rest of the match.
    fn grant_standing(&mut self, player: PlayerId, effect: redshift_data::traits::PlayerEffect) {
        let Some(list) = self.standing.get_mut(player.0 as usize) else {
            return;
        };
        // Sorted and deduplicated, because it is hashed and because "promoted"
        // and "promoted twice" are the same thing.
        if let Err(at) = list.binary_search(&effect) {
            list.insert(at, effect);
        }
    }

    /// The resolved weapon and armour tables.
    pub fn combat(&self) -> &CombatTable {
        &self.combat
    }

    /// Each player's standing modifiers.
    pub fn boons(&self) -> &Boons {
        &self.boons
    }

    /// The power grid, for the interface.
    pub fn power(&self) -> &PowerGrid {
        &self.power
    }

    /// Whether a unit is drawing power it is not getting.
    ///
    /// Anything that consumes power stops working in a shortage. Structures
    /// that consume none — a wall, a refinery — carry on regardless, which is
    /// what makes a power plant worth attacking rather than merely worth owning.
    pub fn is_unpowered(&self, unit: &Unit) -> bool {
        let stats = self.stats.get(unit.owner, unit.kind);
        stats.power_draw > 0 && !stats.works_unpowered && !self.power.is_satisfied(unit.owner)
    }

    // -- Production ----------------------------------------------------------

    /// Advances every build queue by one tick.
    ///
    /// Queues are visited in arena order and each is offered the player's whole
    /// remaining balance in turn, so with two factories and not enough money
    /// the lower-indexed one is funded first. That is arbitrary but it has to
    /// be *decided*: splitting the balance evenly would be equally arbitrary
    /// and would stall both instead of finishing one.
    fn run_production(&mut self) {
        for id in self.units.ids() {
            let Some(unit) = self.units.get(id) else {
                continue;
            };
            if unit.production.as_ref().is_none_or(|q| q.is_empty()) || !unit.is_alive() {
                continue;
            }
            let owner = unit.owner;

            // Production is the one thing the original degrades rather than
            // stopping: a base that simply froze would end the match on the
            // spot, whereas one that slows gives the player a chance to notice
            // and build a plant.
            //
            // Everything *else* that draws power stops outright, handled where
            // each capability is used — "disabled" means something different
            // for a radar than for a gun.
            if !self.power.is_satisfied(owner)
                && !self.tick.is_multiple_of(crate::power::LOW_POWER_DIVISOR)
            {
                continue;
            }

            let available = self.treasury.credits(owner);

            let Some(queue) = self.units.get_mut(id).and_then(|u| u.production.as_mut()) else {
                continue;
            };
            let step = queue.tick(available);

            if step.spent > 0 {
                // Reported by the queue and applied here, so credits are only
                // ever created or destroyed in one place.
                let paid = self.treasury.try_spend(owner, step.spent);
                debug_assert!(paid, "the queue spent more than it was offered");
            }

            if let Some(kind) = step.completed {
                self.deliver_produced(id, owner, kind);
            }
        }
    }

    /// Places a newly built unit next to the building that made it.
    ///
    /// If there is nowhere to put it, the unit is *not* lost: the item goes
    /// back to the front of the queue, fully paid, and is delivered as soon as
    /// room appears. Destroying a paid-for unit because a factory was boxed in
    /// would be an infuriating way to lose a match.
    fn deliver_produced(&mut self, building: EntityId, owner: PlayerId, kind: EntityKind) {
        let Some(unit) = self.units.get(building) else {
            return;
        };
        let origin = unit.cell();
        let produced = self.stats.get(owner, kind);

        // A structure is not delivered anywhere: it waits for the player to
        // choose a site. Placing it automatically would remove base layout as a
        // decision, which is a large part of what the genre is.
        // Anything the new thing brings with it. A refinery arrives with a
        // miner, which is why a refinery is the first thing built.
        let delivered_with: Vec<EntityKind> = self
            .rules
            .entity(kind)
            .traits
            .iter()
            .filter_map(|t| match t {
                redshift_data::traits::Trait::Delivers { units } => Some(units.clone()),
                _ => None,
            })
            .flatten()
            .filter_map(|id| self.rules.kind_of(&id))
            .collect();

        if !produced.mobile {
            if let Some(queue) = self
                .units
                .get_mut(building)
                .and_then(|u| u.production.as_mut())
            {
                queue.ready = Some(kind);
            }
            return;
        }

        let movement = produced.movement;

        let spot = self.free_cell_near(origin, movement, EXIT_SEARCH_RADIUS);

        match spot {
            Some(cell) => {
                let delivered = self.spawn_unit(owner, kind, cell.centre());
                self.deliver_extras(owner, cell, &delivered_with);
                // Off to the rally point, if one is set. Without this a factory
                // builds a wall of its own units in front of its exit.
                if let Some(rally) = self.units.get(building).and_then(|b| b.rally)
                    && self.stats.get(owner, kind).mobile
                {
                    self.order_move(owner, delivered, rally);
                }
                // A player whose barracks has been infiltrated, or who holds
                // whatever else grants it, gets everything promoted on arrival
                // rather than having to earn it.
                // Keyed on category, because the original keys it on one: a
                // spy in a barracks promotes infantry, a spy in a war factory
                // promotes vehicles. One flag for both would be a better deal
                // than the game offers.
                if self
                    .boons
                    .veteran_production(owner, &self.rules.entity(kind).category)
                    && let Some(unit) = self.units.get_mut(delivered)
                {
                    let veteran_at = self
                        .stats
                        .get(owner, kind)
                        .veterancy
                        .map(|(v, _)| v)
                        .unwrap_or(0);
                    unit.kills = unit.kills.max(veteran_at);
                }
            }
            None => {
                if let Some(queue) = self
                    .units
                    .get_mut(building)
                    .and_then(|u| u.production.as_mut())
                {
                    let mut finished = ProductionItem::new(kind, 0, 1);
                    finished.progress = finished.duration;
                    queue.hold_completed(finished);
                }
            }
        }
    }

    /// Where a unit should walk to in order to reach a building.
    ///
    /// Beside it, not on it. A building's footprint blocks its own cells, so
    /// aiming at its centre asks pathfinding for a route that genuinely does
    /// not exist — and the honest "no route" answer looks, from the outside,
    /// exactly like the order being ignored.
    ///
    /// A single-cell building needs no adjustment, which is why this went
    /// unnoticed: every test that entered a building used one.
    fn approach_cell(&self, unit: EntityId, building: EntityId) -> Option<Cell> {
        let (u, b) = (self.units.get(unit)?, self.units.get(building)?);
        let centre = b.cell();
        let movement = self.stats.get(u.owner, u.kind).movement;
        if self.map.is_passable(centre, movement) {
            return Some(centre);
        }
        self.free_cell_near(centre, movement, ENTRY_SEARCH_RADIUS)
    }

    /// The nearest cell to `origin` that `locomotor` can stand on.
    ///
    /// Needed because a building's own cells are impassable once it occupies
    /// them. Anything that wants to *reach* a building — a harvester coming to
    /// unload, a freshly built unit driving out — has to aim beside it rather
    /// than at it, or pathfinding correctly reports no route and the unit
    /// simply gives up.
    fn free_cell_near(&self, origin: Cell, movement: SurfaceMask, radius: i32) -> Option<Cell> {
        (0..=radius).find_map(|r| {
            ring_offsets(r).into_iter().find_map(|(dx, dy)| {
                let cell = Cell::new(origin.x + dx, origin.y + dy);
                self.map.is_passable(cell, movement).then_some(cell)
            })
        })
    }

    // -- Economy -------------------------------------------------------------

    /// Advances every harvester through its cycle.
    ///
    /// The cycle sets ordinary [`Order::Move`] orders and watches for them to
    /// finish, so it inherits the pathfinding, repathing and partial-route
    /// handling that movement already has rather than growing a second copy.
    ///
    /// Every step has to survive the next one failing: a field can run dry
    /// while a harvester walks to it, and a refinery can be destroyed while it
    /// walks home. Keeping the whole cycle in one place means each of those has
    /// an obvious recovery rather than leaving a harvester wedged.
    fn run_harvesters(&mut self) {
        // Cells already being worked, so two harvesters do not pile onto one
        // square while the rest of a field sits untouched. Built in arena
        // order, so every peer builds the same list.
        let claimed: Vec<Cell> = self
            .units
            .iter()
            .filter_map(|(_, u)| u.harvest.and_then(|h| h.field))
            .collect();

        for id in self.units.ids() {
            let Some(unit) = self.units.get(id) else {
                continue;
            };
            let Some(state) = unit.harvest else {
                continue;
            };
            let stats = self.stats.get(unit.owner, unit.kind);
            let Some(capacity) = stats.harvest_capacity else {
                continue;
            };

            let owner = unit.owner;
            let position = unit.pos;
            let cell = unit.cell();
            let travelling = !unit.order.is_idle();

            match state.stage {
                HarvestStage::Approaching => {
                    // The field may have been mined out by someone else while
                    // this harvester was walking to it.
                    let target_gone = state.field.is_none_or(|c| !self.map.has_ore(c));
                    if target_gone {
                        self.assign_field(id, cell, &claimed);
                        continue;
                    }
                    if state.field == Some(cell) {
                        self.set_harvest(id, |h| {
                            h.stage = HarvestStage::Gathering;
                            h.gather_delay = 0;
                        });
                    } else if !travelling {
                        // Arrived somewhere that is not the field: the route
                        // was partial or blocked. Ask again from here.
                        self.assign_field(id, cell, &claimed);
                    }
                }

                HarvestStage::Gathering => {
                    if state.gather_delay > 0 {
                        self.set_harvest(id, |h| h.gather_delay = h.gather_delay.saturating_sub(1));
                        continue;
                    }

                    let wanted = (capacity - state.load).min(economy::ORE_PER_BITE as u32) as u16;
                    let taken = self.map.take_ore(cell, wanted) as u32;
                    let load = state.load + taken;

                    if taken == 0 || load >= capacity {
                        if load > 0 {
                            self.send_harvester_home(id, load, owner, position);
                        } else {
                            self.assign_field(id, cell, &claimed);
                        }
                    } else {
                        self.set_harvest(id, |h| {
                            h.load = load;
                            h.gather_delay = economy::GATHER_INTERVAL;
                        });
                    }
                }

                HarvestStage::Returning => {
                    let refinery = economy::nearest_refinery(
                        &self.units,
                        owner,
                        position.x,
                        position.y,
                        &|u| self.stats.get(u.owner, u.kind).is_refinery,
                    );
                    let Some(refinery) = refinery else {
                        // Nowhere to unload. The load is kept rather than
                        // dropped: silently destroying a player's income when a
                        // refinery is lost mid-run would be very hard to notice
                        // and very annoying to diagnose.
                        continue;
                    };
                    let Some(target) = self.units.get(refinery) else {
                        continue;
                    };
                    let target_cell = target.cell();
                    let reached = Fx::dist_sq(position.x - target.pos.x, position.y - target.pos.y)
                        <= UNLOAD_RANGE.sq();

                    if reached {
                        self.treasury.deposit(
                            owner,
                            self.boons
                                .ore_value(owner)
                                .apply((state.load * economy::CREDITS_PER_ORE) as i32)
                                .max(0) as u32,
                        );
                        self.set_harvest(id, |h| h.load = 0);
                        self.assign_field(id, cell, &claimed);
                    } else if !travelling {
                        // Beside the refinery, not into it: its own cells are
                        // impassable, so aiming at the centre would correctly
                        // find no route and the harvester would give up.
                        if let Some(approach) =
                            self.free_cell_near(target_cell, stats.movement, UNLOAD_APPROACH)
                        {
                            self.order_move(owner, id, approach);
                        }
                    }
                }
            }
        }
    }

    /// Sends a harvester to the nearest unworked ore, or idles it if there is
    /// none within reach.
    fn assign_field(&mut self, id: EntityId, from: Cell, claimed: &[Cell]) {
        let field = economy::nearest_ore(&self.map, from, economy::ORE_SEARCH_RADIUS, claimed);
        let Some(field) = field else {
            // Nothing left within reach. Idling is visible and diagnosable;
            // rescanning the whole map every tick would not be.
            self.set_harvest(id, |h| {
                h.stage = HarvestStage::Approaching;
                h.field = None;
            });
            if let Some(unit) = self.units.get_mut(id) {
                unit.order = Order::Idle;
            }
            return;
        };

        self.set_harvest(id, |h| {
            h.stage = HarvestStage::Approaching;
            h.field = Some(field);
            h.gather_delay = 0;
        });
        let owner = self.units.get(id).map(|u| u.owner);
        if let Some(owner) = owner {
            self.order_move(owner, id, field);
        }
    }

    fn send_harvester_home(&mut self, id: EntityId, load: u32, owner: PlayerId, at: WorldPos) {
        self.set_harvest(id, |h| {
            h.stage = HarvestStage::Returning;
            h.field = None;
            h.load = load;
            h.gather_delay = 0;
        });
        let refinery = economy::nearest_refinery(&self.units, owner, at.x, at.y, &|u| {
            self.stats.get(u.owner, u.kind).is_refinery
        });
        if let Some(refinery) = refinery
            && let Some(cell) = self.units.get(refinery).map(|u| u.cell())
        {
            let movement = self
                .units
                .get(id)
                .map(|u| self.stats.get(u.owner, u.kind).movement)
                .unwrap_or_default();
            if let Some(approach) = self.free_cell_near(cell, movement, UNLOAD_APPROACH) {
                self.order_move(owner, id, approach);
            }
        }
    }

    /// Edits a harvester's cycle state in place.
    fn set_harvest<F: FnOnce(&mut crate::unit::HarvestState)>(&mut self, id: EntityId, edit: F) {
        if let Some(unit) = self.units.get_mut(id)
            && let Some(state) = unit.harvest.as_mut()
        {
            edit(state);
        }
    }

    /// Credits available to each player.
    pub fn treasury(&self) -> &Treasury {
        &self.treasury
    }

    // -- Separation ----------------------------------------------------------

    /// Pushes overlapping units apart.
    ///
    /// # Why a push rather than a hard block
    ///
    /// Refusing to enter an occupied cell sounds tidier and is much worse in
    /// practice: a unit arriving at a crowded destination has nowhere legal to
    /// stand, so it stops early, re-paths, and jitters against the crowd
    /// forever. Letting units overlap briefly and pressing them apart lets a
    /// group settle into a formation on its own, which is what the original
    /// did and what reads as natural.
    ///
    /// # Determinism
    ///
    /// Three things would break this if written carelessly:
    ///
    /// - **Order dependence.** Displacements are accumulated for every unit
    ///   first and applied afterwards, so no unit sees a neighbour that has
    ///   already been nudged this tick. Applying as we went would make the
    ///   result depend on arena order in a way that shifts when slots are
    ///   reused.
    /// - **Exactly coincident units.** Two units at the same point have no
    ///   direction between them. Falling back to an arbitrary direction would
    ///   be fine locally and catastrophic across peers, so the fallback is
    ///   derived from their entity ids.
    /// - **Cost.** Checking every pair is quadratic, and 400 units is 80,000
    ///   pairs a tick. Units are bucketed by cell and only neighbouring
    ///   buckets are consulted.
    fn separate_units(&mut self) {
        // Bucket by cell. A dense vector indexed by cell rather than a map:
        // lookup is an array read, and there is no hashed iteration order to
        // worry about.
        let cell_count = self.map.cell_count();
        if self.separation_buckets.len() != cell_count {
            self.separation_buckets = vec![Vec::new(); cell_count];
        }
        for bucket in &mut self.separation_buckets {
            bucket.clear();
        }

        for (id, unit) in self.units.iter() {
            if unit.is_aboard() {
                continue;
            }
            let stats = self.stats.get(unit.owner, unit.kind);
            // Aircraft fly over everything, each other included.
            if stats.locomotor == Locomotor::Air {
                continue;
            }
            if let Some(index) = self.map.index(unit.cell()) {
                self.separation_buckets[index as usize].push(id);
            }
        }

        // Accumulated displacement per unit, in arena order.
        let mut pushes: Vec<(EntityId, Fx, Fx)> = Vec::new();

        for (id, unit) in self.units.iter() {
            let stats = self.stats.get(unit.owner, unit.kind);
            if stats.locomotor == Locomotor::Air || !stats.mobile {
                continue;
            }
            let cell = unit.cell();
            let mut dx_total = Fx::ZERO;
            let mut dy_total = Fx::ZERO;

            for (ox, oy) in NEIGHBOURHOOD {
                let neighbour = Cell::new(cell.x + ox, cell.y + oy);
                let Some(index) = self.map.index(neighbour) else {
                    continue;
                };
                for &other_id in &self.separation_buckets[index as usize] {
                    if other_id == id {
                        continue;
                    }
                    let Some(other) = self.units.get(other_id) else {
                        continue;
                    };
                    let other_stats = self.stats.get(other.owner, other.kind);
                    let wanted = stats.radius + other_stats.radius;

                    let dx = unit.pos.x - other.pos.x;
                    let dy = unit.pos.y - other.pos.y;
                    let gap_sq = Fx::dist_sq(dx, dy);
                    if gap_sq >= wanted.sq() {
                        continue;
                    }

                    let gap = gap_sq.sqrt();
                    let (nx, ny) = if gap <= SEPARATION_EPSILON {
                        // Exactly on top of each other. There is no direction
                        // between them, so one is derived from the entity ids —
                        // arbitrary, but identically arbitrary on every peer.
                        let spread =
                            Angle::from_raw((id.index().wrapping_mul(2654435761) >> 16) as u16);
                        (spread.cos(), spread.sin())
                    } else {
                        (dx.div(gap), dy.div(gap))
                    };

                    // Half the overlap each: the neighbour is doing the same
                    // sum from its own side, so together they close the gap.
                    let push = (wanted - gap).div_int(2);
                    dx_total += nx.mul(push);
                    dy_total += ny.mul(push);
                }
            }

            if dx_total != Fx::ZERO || dy_total != Fx::ZERO {
                // Capped so a unit caught in a dense press is nudged rather
                // than flung across the map.
                let magnitude = Fx::dist(dx_total, dy_total);
                if magnitude > MAX_SEPARATION_STEP {
                    let scale = MAX_SEPARATION_STEP.div(magnitude);
                    dx_total = dx_total.mul(scale);
                    dy_total = dy_total.mul(scale);
                }
                pushes.push((id, dx_total, dy_total));
            }
        }

        for (id, dx, dy) in pushes {
            let Some(unit) = self.units.get_mut(id) else {
                continue;
            };
            let moved = WorldPos {
                x: unit.pos.x + dx,
                y: unit.pos.y + dy,
            };
            // A push must never shove a unit somewhere it could not walk —
            // into a lake, or through a cliff.
            let stats = self.stats.get(unit.owner, unit.kind);
            let target_cell = moved.cell();
            if self.map.is_passable(target_cell, stats.movement)
                && self
                    .map
                    .step_is_climbable(unit.cell(), target_cell, stats.movement)
            {
                unit.pos = self.map.clamp_pos(moved);
            }
        }
    }

    // -- Combat --------------------------------------------------------------

    /// Whether two players are on the same side.
    ///
    /// Trivial while every player is an enemy of every other. It exists as a
    /// function so that teams and alliances slot in here later rather than
    /// being threaded through targeting after the fact.
    fn are_allied(a: PlayerId, b: PlayerId) -> bool {
        // Neutral things are nobody's enemy. Civilians standing next to an army
        // must not start a battle, and a tech building must not be shot at
        // simply for being there.
        //
        // This governs *automatic* targeting only. A player who deliberately
        // orders an attack on a civilian gets one, which is the distinction the
        // original drew and the reason this is not an ownership rule.
        if a.is_neutral() || b.is_neutral() {
            return true;
        }
        a == b
    }

    /// Targeting and firing.
    ///
    /// Returns the shots to apply rather than applying them, so every unit
    /// chooses its target against the *same* world. Applying damage as each
    /// unit fires would let a unit early in the arena kill a target before a
    /// later unit considered it — an outcome that depends on arena order, and
    /// that shifts whenever slots are reused.
    fn acquire_and_fire(&mut self) -> Vec<PendingHit> {
        let mut hits = Vec::new();
        let mut launched: Vec<Projectile> = Vec::new();
        let mut burrows: Vec<(EntityId, EntityId)> = Vec::new();

        // A snapshot for targeting, so every attacker sees the same world.
        // Cloning the arena each tick would be wasteful; the targeting pass
        // reads it immutably and the firing pass writes only to the attacker.
        let ids = self.units.ids();

        for attacker in ids {
            let Some(unit) = self.units.get(attacker) else {
                continue;
            };
            if !unit.is_alive() || unit.is_aboard() {
                continue;
            }
            // Targeting considers *both* weapons: a unit with a ground cannon
            // and an anti-air missile looks for anything either can reach, and
            // then uses whichever one reaches it. Consulting only the primary
            // would leave the secondary resolved and never fired.
            // An occupied building shoots with *its own* weapon rather than
            // its occupants' — the exact opposite of a vehicle whose gun
            // changes with its passenger, and the thing most easily got
            // backwards. An empty one has no weapon at all, which is what makes
            // garrisoning a building do something.
            let garrison = self
                .combat
                .garrison_weapon(unit.kind)
                .filter(|_| !unit.cargo.is_empty())
                .copied();

            let reach = if let Some(w) = garrison {
                w.targets
            } else {
                let primary = self.combat.weapon(unit.kind).map(|w| w.targets.raw());
                let secondary = self.combat.secondary(unit.kind).map(|w| w.targets.raw());
                match (primary, secondary) {
                    (None, None) => continue,
                    (a, b) => combat::LayerMask::from_raw(a.unwrap_or(0) | b.unwrap_or(0)),
                }
            };
            // The longer of the two, for the search radius.
            let kind_of_attacker = unit.kind;
            let ground_bonus = self.map.elevation_bonus(unit.cell());
            let Some(weapon) = garrison.or_else(|| {
                self.combat
                    .weapon(unit.kind)
                    .into_iter()
                    .chain(self.combat.secondary(unit.kind))
                    .max_by_key(|w| w.range_sq)
                    .copied()
            }) else {
                continue;
            };
            // The search radius carries the hill's advantage too. If only the
            // firing check did, a unit on high ground would never look far
            // enough to find what its extended reach could hit.
            let weapon = WeaponStats {
                targets: reach,
                ..weapon.with_range_percent(ground_bonus)
            };
            // A defence with no power does not shoot. This is most of what
            // makes cutting an enemy's power worth doing.
            if self.is_unpowered(unit) {
                continue;
            }

            // Keep the current target if it is still worth shooting at, so a
            // unit does not flicker between two equally close enemies.
            // Only what the owner can see. Firing through fog would be both
            // unfair and a source of desyncs, since the interface and the
            // simulation would be working from different answers about what is
            // there.
            let visible = |other: &Unit| self.can_see(unit.owner, other);
            let layer_of = |other: &Unit| self.stats.get(other.owner, other.kind).layer;

            // An explicit attack order names its target, and that target stands
            // even when automatic acquisition would refuse it — which is how a
            // player kills a civilian. Without this the order is accepted, the
            // unit walks over, and then never fires, because auto-targeting
            // skips neutrals.
            let ordered = match unit.order {
                Order::Attack { target, .. } => self
                    .units
                    .get(target)
                    .filter(|v| v.is_alive() && !v.is_aboard())
                    .filter(|v| {
                        let dx = v.pos.x - unit.pos.x;
                        let dy = v.pos.y - unit.pos.y;
                        Fx::dist_sq(dx, dy) <= weapon.range_sq
                            && weapon.targets.engages(layer_of(v))
                            && self.can_see(unit.owner, v)
                    })
                    .map(|_| target),
                _ => None,
            };

            let keep = unit.combat.target.filter(|t| {
                combat::target_is_valid(
                    unit,
                    *t,
                    &weapon,
                    &self.units,
                    &Self::are_allied,
                    &layer_of,
                ) && self.units.get(*t).is_some_and(&visible)
            });
            let target = ordered.or(keep).or_else(|| {
                combat::choose_target_where(
                    attacker,
                    unit,
                    &weapon,
                    &self.units,
                    &Self::are_allied,
                    &visible,
                    &layer_of,
                )
            });

            // Aim before firing. A turret traverses on its own; without one the
            // hull must come round, which is most of what makes a tank feel
            // like a tank.
            let aim = target.and_then(|t| self.units.get(t)).and_then(|other| {
                self.units
                    .get(attacker)
                    .and_then(|a| a.pos.heading_to(other.pos))
            });

            // Which of the unit's weapons actually engages the chosen target,
            // resolved before the mutable borrow below. A unit with a cannon
            // and a missile fires whichever reaches, and the reload and
            // ammunition it spends belong to that weapon rather than to the
            // primary.
            // High ground reaches further. Applied to the resolved weapon rather
            // than to the range check alone, so a unit on a hill both acquires
            // and engages at the longer distance — checking only one would let
            // it lock onto something it then refused to shoot.
            let elevation = self.map.elevation_bonus(unit.cell());

            let firing_weapon = if garrison.is_some() {
                // A garrison has one weapon and no second choice to make.
                weapon
            } else {
                target
                    .and_then(|t| self.units.get(t))
                    .map(|t| self.stats.get(t.owner, t.kind).layer)
                    .and_then(|layer| self.combat.weapon_for(kind_of_attacker, layer).copied())
                    .unwrap_or(weapon)
            };
            let firing_weapon = firing_weapon.with_range_percent(elevation);
            // Resolved here, before the mutable borrow below, for the same
            // reason `firing_weapon` is.
            let can_burrow = target.is_some_and(|t| self.may_infest(attacker, t));

            let Some(unit) = self.units.get_mut(attacker) else {
                continue;
            };
            unit.combat.target = target;
            unit.combat.reload_remaining = unit.combat.reload_remaining.saturating_sub(1);
            unit.since_fired = unit.since_fired.saturating_add(1);

            let Some(heading) = aim else {
                // Nothing to shoot at: let the turret settle back to the hull
                // so an idle unit is not left staring at where an enemy was.
                unit.combat.turret_facing = unit
                    .combat
                    .turret_facing
                    .rotate_toward(unit.facing, weapon.turret_rate);
                continue;
            };

            if weapon.turret {
                unit.combat.turret_facing = unit
                    .combat
                    .turret_facing
                    .rotate_toward(heading, weapon.turret_rate);
            } else {
                unit.combat.turret_facing = unit.facing;
            }

            // Too far off the mark to shoot yet.
            if unit.combat.turret_facing.delta(heading).unsigned_abs() > FIRING_ARC as u32 {
                continue;
            }
            if unit.combat.reload_remaining > 0 {
                continue;
            }
            // Out of ammunition. The unit holds fire until it rearms, which is
            // what makes an aircraft a sortie rather than a flying gun — and
            // the rule is general, not an aircraft special case.
            if weapon.ammo > 0 && unit.combat.shots_fired >= weapon.ammo {
                continue;
            }

            let Some(target) = target else { continue };
            // A parasite that has closed on something it can get inside gets
            // inside it instead of shooting it. Triggered at weapon range
            // rather than at a special burrowing distance, so the drone's own
            // short reach is what makes it run up to a tank — a number in the
            // rules, not a movement rule in here.
            //
            // Collected and applied after the loop, like damage, so every unit
            // this tick chooses against the same world.
            if can_burrow {
                burrows.push((attacker, target));
                continue;
            }
            let owner = unit.owner;
            let firing_from = unit.pos;
            unit.combat.reload_remaining = firing_weapon.reload;
            unit.combat.shots_fired = unit.combat.shots_fired.saturating_add(1);
            // Firing gives a cloaked unit away.
            unit.since_fired = 0;
            let at = self.units.get(target).map(|t| t.pos);

            if let Some(at) = at {
                if firing_weapon.projectile_speed > Fx::ZERO {
                    // Slow enough to be worth watching fly. Launched now and
                    // resolved when it lands, which is what lets it be dodged.
                    launched.push(Projectile {
                        attacker,
                        owner,
                        target: firing_weapon.homing.then_some(target),
                        aim: at,
                        pos: firing_from,
                        speed: firing_weapon.projectile_speed,
                        damage: firing_weapon.damage,
                        warhead: firing_weapon.warhead,
                        splash_radius: firing_weapon.splash_radius,
                        instant_kill: firing_weapon.instant_kill,
                        fuse: crate::projectile::MAX_FLIGHT_TICKS,
                    });
                } else {
                    // A rifle. Instant, exactly as before.
                    hits.push(PendingHit {
                        attacker,
                        instant_kill: firing_weapon.instant_kill,
                        target,
                        damage: firing_weapon.damage,
                        warhead: firing_weapon.warhead,
                        splash_radius: firing_weapon.splash_radius,
                        at,
                    });
                }
            }
        }
        self.projectiles.extend(launched);
        for (parasite, host) in burrows {
            // Re-checked, because an earlier burrow this tick may have taken
            // the host: two drones reaching the same tank on the same tick must
            // not both get in, and the loop above could not have known.
            if self.may_infest(parasite, host) {
                self.infest(parasite, host);
            }
        }
        hits
    }

    /// Moves everything in flight and resolves what lands.
    ///
    /// Run before targeting, so a shot fired last tick lands before this tick's
    /// shots are chosen — otherwise a unit would keep firing at something the
    /// shell already in the air is about to kill.
    /// Shoots down projectiles that pass within reach of an interceptor.
    ///
    /// Runs before flight, so a shot is stopped where it is rather than after
    /// it has moved. Three units in the original exist largely to do this, and
    /// two exist to fire the missiles they stop.
    fn intercept_projectiles(&mut self) {
        if self.projectiles.is_empty() {
            return;
        }
        let interceptors: Vec<(PlayerId, WorldPos, FxWide)> = self
            .units
            .iter()
            .filter(|(_, u)| u.is_alive() && !u.is_aboard())
            .filter_map(|(_, u)| {
                let weapon = self.combat.weapon(u.kind)?;
                weapon
                    .intercepts
                    .then_some((u.owner, u.pos, weapon.range_sq))
            })
            .collect();
        if interceptors.is_empty() {
            return;
        }

        self.projectiles.retain(|shot| {
            !interceptors.iter().any(|(owner, pos, range_sq)| {
                // Nobody shoots down their own shots.
                *owner != shot.owner
                    && Fx::dist_sq(shot.pos.x - pos.x, shot.pos.y - pos.y) <= *range_sq
            })
        });
    }

    fn advance_projectiles(&mut self) -> Vec<PendingHit> {
        let mut hits = Vec::new();
        let mut still_flying = Vec::with_capacity(self.projectiles.len());

        // Taken out so the aim can be refreshed against the arena while the
        // projectiles are mutated.
        let flying = std::mem::take(&mut self.projectiles);
        for mut shot in flying {
            // A homing shot follows its target. One whose target has died
            // carries on to where it was last seen, which is both simpler and
            // more truthful than making the shot vanish in mid-air.
            if let Some(target) = shot.target
                && let Some(unit) = self.units.get(target)
                && unit.is_alive()
            {
                shot.aim = unit.pos;
            }

            let arrived = shot.advance();
            if arrived {
                hits.push(PendingHit {
                    attacker: shot.attacker,
                    instant_kill: shot.instant_kill,
                    // A homing shot damages what it was following; a ballistic
                    // one damages whatever is standing where it landed, which
                    // the splash pass works out.
                    target: shot.target.unwrap_or(EntityId::NONE),
                    damage: shot.damage,
                    warhead: shot.warhead,
                    splash_radius: shot.splash_radius.max(IMPACT_TOLERANCE),
                    at: shot.pos,
                });
            } else if !shot.is_spent() {
                still_flying.push(shot);
            }
        }
        self.projectiles = still_flying;
        hits
    }

    /// Applies the shots collected this tick.
    fn resolve_damage(&mut self, hits: &[PendingHit]) {
        for hit in hits {
            // The primary target takes the shot even if it has moved, since the
            // shot was already committed this tick.
            if let Some(target) = self.units.get(hit.target) {
                let armour = self.combat.armour(target.kind);
                let base =
                    self.combat
                        .damage_table()
                        .damage_against(hit.damage, hit.warhead, armour);

                // Both ranks matter: the attacker's experience makes the shot
                // hit harder, and the defender's makes it land softer.
                let attacker_rank = self.units.get(hit.attacker).map(|u| self.rank_of(u));
                let damage = self
                    .rank_of(target)
                    .resist(attacker_rank.map_or(base, |r| r.scale(base)));
                // An instant-kill weapon kills whatever it can hurt at all, and
                // does nothing to what it cannot. Expressing it as enormous
                // damage would make a sniper excellent against tanks, which is
                // exactly wrong.
                let damage = if hit.instant_kill && damage > 0 {
                    target.health
                } else {
                    damage
                };
                let killed = target.is_alive() && target.health <= damage;
                // Read before the mutable borrow below, since the target is
                // about to be modified and may then be gone.
                let bounty = self.stats.get(target.owner, target.kind).bounty;

                if let Some(target) = self.units.get_mut(hit.target) {
                    target.take_damage(damage);
                }
                // Credited on the killing blow only. Anything else and a unit
                // promotes for as long as it keeps firing at a body.
                if killed && hit.attacker != hit.target {
                    // The bounty goes to the owner, and is read before the
                    // attacker is looked up: a shell already in the air still
                    // pays out even if the unit that fired it has since died.
                    if bounty > 0
                        && let Some(owner) = self.units.get(hit.attacker).map(|a| a.owner)
                    {
                        self.treasury.deposit(owner, bounty);
                    }
                    if let Some(attacker) = self.units.get_mut(hit.attacker) {
                        attacker.kills = attacker.kills.saturating_add(1);
                    }
                }
            }

            if hit.splash_radius <= Fx::ZERO {
                continue;
            }

            // Splash catches everything nearby, friend included. Sparing
            // friendly units would make artillery strictly better than it
            // should be, and the original did not spare them either.
            let radius_sq = hit.splash_radius.sq();
            let splashed: Vec<(EntityId, u32)> = self
                .units
                .iter()
                .filter(|(id, other)| *id != hit.target && other.is_alive())
                .filter_map(|(id, other)| {
                    let dx = other.pos.x - hit.at.x;
                    let dy = other.pos.y - hit.at.y;
                    if Fx::dist_sq(dx, dy) > radius_sq {
                        return None;
                    }
                    let armour = self.combat.armour(other.kind);
                    Some((
                        id,
                        self.combat
                            .damage_table()
                            .damage_against(hit.damage, hit.warhead, armour),
                    ))
                })
                .collect();

            for (id, damage) in splashed {
                if let Some(unit) = self.units.get_mut(id) {
                    unit.take_damage(damage);
                }
            }
        }
    }

    /// Removes destroyed units.
    ///
    /// After all damage, so a unit that dies this tick still got to fire —
    /// which is what makes two evenly matched units able to destroy each other.
    fn remove_the_dead(&mut self) {
        // A wrecked bridge drops its span, and the water underneath is back.
        // Nothing has to remember what the cell used to be, because the terrain
        // was never rewritten in the first place.
        let dropped: Vec<(Cell, (u8, u8))> = self
            .units
            .iter()
            .filter(|(_, u)| !u.is_alive())
            .filter_map(|(_, u)| {
                let stats = self.stats.get(u.owner, u.kind);
                stats.is_bridge.then(|| (u.cell(), stats.footprint))
            })
            .collect();
        for (centre, footprint) in dropped {
            self.open_span(centre, footprint, false);
        }

        // A destroyed building has to release its footprint, or it leaves a
        // permanent hole in the map that nothing can walk through and nothing
        // can build on — invisible, and impossible to explain to a player.
        let released: Vec<(Cell, (u8, u8))> = self
            .units
            .iter()
            .filter(|(_, u)| !u.is_alive())
            .filter_map(|(_, u)| {
                let stats = self.stats.get(u.owner, u.kind);
                (stats.footprint != (1, 1) && !stats.is_bridge).then(|| (u.cell(), stats.footprint))
            })
            .collect();
        for (centre, footprint) in released {
            claim_footprint(&mut self.map, centre, footprint, false);
        }

        // Passengers die with the transport that was carrying them. The
        // alternative — spilling them out — would make a loaded transport safer
        // than an empty one, which is exactly backwards.
        let doomed: Vec<EntityId> = self
            .units
            .iter()
            .filter(|(_, u)| !u.is_alive())
            .flat_map(|(_, u)| u.cargo.clone())
            .collect();
        for id in doomed {
            if let Some(unit) = self.units.get_mut(id) {
                unit.health = 0;
            }
        }

        // A parasite outlives its host and crawls back out where the host
        // died. It killed something and should get to do it again — that is
        // what makes one drone worth spending, and what makes the depot worth
        // building. Killing it with the wreck would quietly make the drone a
        // one-shot weapon.
        let freed: Vec<(EntityId, WorldPos)> = self
            .units
            .iter()
            .filter(|(_, u)| !u.is_alive())
            .filter_map(|(_, u)| u.infestation.map(|p| (p, u.pos)))
            .collect();
        for (parasite, at) in freed {
            if let Some(p) = self.units.get_mut(parasite) {
                p.carrier = None;
                p.pos = at;
            }
        }

        // A wrecked bridge stays. It is the one thing destroyed without being
        // removed, and for a plain reason: the ruined span is still visibly
        // there, and an engineer at the hut beside it puts it back. Take the
        // entity away and there is nothing left to repair.
        self.units
            .retain(|_, unit| unit.is_alive() || self.stats.get(unit.owner, unit.kind).is_bridge);
    }

    // -- Phase 1: commands ---------------------------------------------------

    fn apply_commands(&mut self, commands: &[Command]) {
        for command in commands {
            match &command.kind {
                CommandKind::Move { units, target } => {
                    self.order_group_move(command.player, units, *target);
                }
                CommandKind::Stop { units } => {
                    for &id in units {
                        if self.owned_by(id, command.player)
                            && let Some(unit) = self.units.get_mut(id)
                        {
                            unit.order = Order::Idle;
                        }
                    }
                }
                CommandKind::Produce { building, kind } => {
                    self.order_produce(command.player, *building, *kind);
                }
                CommandKind::CancelProduction { building, index } => {
                    self.order_cancel_production(command.player, *building, *index as usize);
                }
                CommandKind::AttackMove { units, target } => {
                    self.order_group_move(command.player, units, *target);
                    // Reissued as attack-move: the group move already worked
                    // out a formation and a route for each unit, and only the
                    // kind of order differs.
                    for &id in units {
                        if self.owned_by(id, command.player)
                            && let Some(unit) = self.units.get_mut(id)
                            && let Order::Move(travel) = unit.order.clone()
                        {
                            unit.order = Order::AttackMove(travel);
                        }
                    }
                }
                CommandKind::Attack { units, target } => {
                    for &id in units {
                        self.order_attack(command.player, id, *target);
                    }
                }
                CommandKind::Guard { units } => {
                    for &id in units {
                        if self.owned_by(id, command.player)
                            && let Some(unit) = self.units.get_mut(id)
                        {
                            let post = unit.cell();
                            unit.order = Order::Guard {
                                post,
                                returning: None,
                            };
                        }
                    }
                }
                CommandKind::SetRally { building, at } => {
                    if self.owned_by(*building, command.player)
                        && self.map.contains(*at)
                        && let Some(unit) = self.units.get_mut(*building)
                    {
                        unit.rally = Some(*at);
                    }
                }
                CommandKind::Sell { building } => {
                    self.sell(command.player, *building);
                }
                CommandKind::Deploy { units } => {
                    for &id in units {
                        if self.owned_by(id, command.player) {
                            self.deploy(id);
                        }
                    }
                }
                CommandKind::EnterBuilding { units, target } => {
                    for &id in units {
                        if !self.owned_by(id, command.player) {
                            continue;
                        }
                        // A building in fog cannot be entered, for the same
                        // reason it cannot be attacked: a client with the fog
                        // switched off would otherwise be issuing orders its
                        // peers would refuse.
                        let visible = self
                            .units
                            .get(*target)
                            .is_some_and(|b| b.is_alive() && self.can_see(command.player, b));
                        if !visible {
                            continue;
                        }
                        let Some(cell) = self.approach_cell(id, *target) else {
                            continue;
                        };
                        if let Some(unit) = self.units.get_mut(id) {
                            unit.order = Order::Enter {
                                target: *target,
                                approach: Travel::to(cell, DEFAULT_NODE_BUDGET),
                            };
                        }
                        if !self.path_queue.contains(&id) {
                            self.path_queue.push(id);
                        }
                    }
                }
                CommandKind::Load { units, transport } => {
                    for &id in units {
                        if !self.owned_by(id, command.player)
                            || !self.owned_by(*transport, command.player)
                            || !self.may_board(*transport, id)
                        {
                            continue;
                        }
                        let Some(cell) = self.units.get(*transport).map(|t| t.cell()) else {
                            continue;
                        };
                        if let Some(unit) = self.units.get_mut(id) {
                            unit.order = Order::Board {
                                transport: *transport,
                                approach: Travel::to(cell, DEFAULT_NODE_BUDGET),
                            };
                        }
                        if !self.path_queue.contains(&id) {
                            self.path_queue.push(id);
                        }
                    }
                }
                CommandKind::Unload { transport, at } => {
                    if self.owned_by(*transport, command.player) {
                        self.unload(*transport, *at);
                    }
                }
                CommandKind::PlaceBuilding { producer, at } => {
                    self.order_place_building(command.player, *producer, *at);
                }
            }
        }
    }

    /// Queues an item at a production building.
    ///
    /// Every condition is checked here rather than in the interface. A modified
    /// client could send anything, and — more importantly — every peer has to
    /// reach the same answer about whether a queue happened, or their
    /// simulations diverge on the next tick.
    fn order_produce(&mut self, player: PlayerId, building: EntityId, kind: EntityKind) {
        if !self.owned_by(building, player) {
            return;
        }
        let Some(unit) = self.units.get(building) else {
            return;
        };
        if !unit.is_alive() {
            return;
        }
        // The building must actually make this sort of thing.
        if !self.can_produce(unit.kind, kind) {
            return;
        }
        if !self.prerequisites_met(player, kind) {
            return;
        }
        // A commando is unique; a superweapon is one per base. Queued items
        // count, or a player could fill a queue with commandos and get every
        // one of them — the limit would bite only on the last.
        if !self.within_build_limit(player, kind) {
            return;
        }
        // A country's roster. Another country's unique unit is not available,
        // and a country that gives one up does not get it back.
        if !self.available_to(player, kind) {
            return;
        }

        let stats = self.stats.get(player, kind);
        let item = ProductionItem::new(kind, stats.cost, stats.build_time);

        if let Some(unit) = self.units.get_mut(building) {
            unit.production
                .get_or_insert_with(ProductionQueue::default)
                .enqueue(item);
        }
    }

    fn order_cancel_production(&mut self, player: PlayerId, building: EntityId, index: usize) {
        if !self.owned_by(building, player) {
            return;
        }
        let refund = self
            .units
            .get_mut(building)
            .and_then(|u| u.production.as_mut())
            .and_then(|q| q.cancel(index));
        if let Some(refund) = refund {
            self.treasury.deposit(player, refund);
        }
    }

    /// Orders a unit to attack a specific target.
    ///
    /// Refused for a target the player cannot see. Allowing it would let a
    /// client with the fog switched off pick out units it has no business
    /// knowing about — and, since the simulation and the interface would then
    /// disagree about which orders are legal, desync the match.
    fn order_attack(&mut self, player: PlayerId, id: EntityId, target: EntityId) {
        if !self.owned_by(id, player) || id == target {
            return;
        }
        let Some(victim) = self.units.get(target) else {
            return;
        };
        if !victim.is_alive() || !self.can_see(player, victim) {
            return;
        }
        let goal = victim.cell();
        if let Some(unit) = self.units.get_mut(id) {
            unit.order = Order::Attack {
                target,
                approach: Travel::to(goal, DEFAULT_NODE_BUDGET),
            };
        }
        if !self.path_queue.contains(&id) {
            self.path_queue.push(id);
        }
    }

    /// Keeps attack, attack-move and guard orders honest.
    ///
    /// Run before movement so that a unit which should be standing still this
    /// tick has already stopped, rather than taking one more step and then
    /// noticing.
    fn update_engagements(&mut self) {
        for id in self.units.ids() {
            let Some(unit) = self.units.get(id) else {
                continue;
            };
            match unit.order.clone() {
                Order::Attack { target, .. } => {
                    let gone = self
                        .units
                        .get(target)
                        .is_none_or(|v| !v.is_alive() || !self.can_see(unit.owner, v));
                    if gone {
                        // The target died or slipped into fog. Stopping is the
                        // honest answer: chasing a remembered position would
                        // walk the unit into an ambush it cannot see.
                        if let Some(unit) = self.units.get_mut(id) {
                            unit.order = Order::Idle;
                        }
                        continue;
                    }

                    // Close only until the weapon reaches, then hold. Walking
                    // all the way onto the target would push it about instead
                    // of shooting it.
                    let in_range = self
                        .combat
                        .weapon(unit.kind)
                        .zip(self.units.get(target))
                        .is_some_and(|(w, v)| {
                            Fx::dist_sq(v.pos.x - unit.pos.x, v.pos.y - unit.pos.y) <= w.range_sq
                        });
                    let moved = self.units.get(target).map(|v| v.cell());
                    if let Some(unit) = self.units.get_mut(id)
                        && let Order::Attack { approach, .. } = &mut unit.order
                    {
                        if in_range {
                            approach.path.clear();
                            approach.needs_repath = false;
                        } else if let Some(cell) = moved
                            && approach.destination != cell
                        {
                            // The target moved: aim at where it is now.
                            *approach = Travel::to(cell, DEFAULT_NODE_BUDGET);
                            if !self.path_queue.contains(&id) {
                                self.path_queue.push(id);
                            }
                        }
                    }
                }

                Order::Guard { post, returning } => {
                    let has_target = unit.combat.target.is_some();
                    let drifted = unit.cell().chebyshev_to(post) > GUARD_LEASH;

                    if has_target {
                        // Fighting takes priority; walking home mid-fight would
                        // present the unit's back to whatever it is shooting.
                        if returning.is_some()
                            && let Some(unit) = self.units.get_mut(id)
                        {
                            unit.order = Order::Guard {
                                post,
                                returning: None,
                            };
                        }
                    } else if drifted && returning.is_none() {
                        if let Some(unit) = self.units.get_mut(id) {
                            unit.order = Order::Guard {
                                post,
                                returning: Some(Travel::to(post, DEFAULT_NODE_BUDGET)),
                            };
                        }
                        if !self.path_queue.contains(&id) {
                            self.path_queue.push(id);
                        }
                    }
                }

                Order::Board {
                    transport,
                    approach,
                } => {
                    // The transport may have filled up, driven off or been
                    // destroyed while this unit was walking to it. Arriving is
                    // not the same as boarding, which is why this is its own
                    // order rather than a flag on a move.
                    let target = self.units.get(transport);
                    let gone = target.is_none_or(|t| !t.is_alive());
                    let full = target.is_some_and(|t| {
                        t.cargo.len() >= self.stats.get(t.owner, t.kind).capacity as usize
                    });
                    if gone || full {
                        if let Some(unit) = self.units.get_mut(id) {
                            unit.order = Order::Idle;
                        }
                        continue;
                    }

                    let Some(target_cell) = target.map(|t| t.cell()) else {
                        continue;
                    };
                    let close = unit.cell().chebyshev_to(target_cell) <= BOARDING_RANGE;

                    if close {
                        self.board(id, transport);
                    } else if approach.destination != target_cell {
                        // The transport moved: follow it.
                        if let Some(unit) = self.units.get_mut(id)
                            && let Order::Board { approach, .. } = &mut unit.order
                        {
                            *approach = Travel::to(target_cell, DEFAULT_NODE_BUDGET);
                        }
                        if !self.path_queue.contains(&id) {
                            self.path_queue.push(id);
                        }
                    }
                }

                Order::Enter { target, approach } => {
                    let building = self.units.get(target);
                    if building.is_none_or(|b| !b.is_alive()) {
                        if let Some(unit) = self.units.get_mut(id) {
                            unit.order = Order::Idle;
                        }
                        continue;
                    }
                    let Some(target_cell) = building.map(|b| b.cell()) else {
                        continue;
                    };
                    // Where to *walk*, which is not where the building is. A
                    // building's own cells are impassable once it occupies
                    // them, so a route to its centre correctly does not exist
                    // and the unit gives up on the spot. Invisible until now
                    // only because no test had given a building a footprint.
                    let Some(walk_to) = self.approach_cell(id, target) else {
                        continue;
                    };

                    if unit.cell().chebyshev_to(target_cell) <= ENTRY_RANGE {
                        // Two quite different things share the arrival, because
                        // from the player's side they share a gesture: select
                        // something, right-click a friendly building. An
                        // engineer walking in is one act that resolves at once;
                        // a damaged tank driving on is a stay, resolved a
                        // little each tick until it is whole or the money runs
                        // out.
                        // Four things arriving at a building can mean, tried
                        // in order of how specific they are. Each declines by
                        // returning false, which keeps the decision in the data
                        // — a spy is a spy because it says so, not because of
                        // where this branch happens to sit.
                        if !self.garrison(id, target)
                            && !self.infiltrate(id, target)
                            && !self.repair_bridges(id, target)
                            && !self.service_repair(id, target)
                        {
                            self.enter_building(id, target);
                        }
                    } else if approach.destination != walk_to {
                        if let Some(unit) = self.units.get_mut(id)
                            && let Order::Enter { approach, .. } = &mut unit.order
                        {
                            *approach = Travel::to(walk_to, DEFAULT_NODE_BUDGET);
                        }
                        if !self.path_queue.contains(&id) {
                            self.path_queue.push(id);
                        }
                    }
                }

                Order::AttackMove(_) | Order::Move(_) | Order::Idle => {}
            }
        }
    }

    /// Turns a unit into its deployed form, or back out of it.
    ///
    /// One direction, not two. The deployed form is an ordinary entity in the
    /// rules whose own `Deploys` points back at the mobile one, so undeploying
    /// is this same function run again. Nothing here knows what an MCV is.
    ///
    /// The unit keeps its `EntityId`. Removing it and inserting a replacement
    /// would have been simpler to write and wrong in three visible ways: the
    /// player's selection would empty the instant they deployed, shots already
    /// in flight would lose their target, and a transport holding the unit
    /// would be left with a dangling reference. Deploying is a change of form,
    /// not a death and a birth, and the identity should say so.
    ///
    /// Returns whether anything happened, so a caller can tell a refusal from a
    /// success. Refusals are quiet by design: a player who presses deploy with
    /// a mixed group selected means it for the units that can.
    fn deploy(&mut self, id: EntityId) -> bool {
        let Some(unit) = self.units.get(id) else {
            return false;
        };
        if !unit.is_alive() || unit.is_aboard() {
            return false;
        }
        let (owner, from_kind, centre) = (unit.owner, unit.kind, unit.cell());
        let from = self.stats.get(owner, from_kind);
        let Some(to_kind) = from.deploys_into else {
            return false;
        };
        let to = self.stats.get(owner, to_kind);

        // The deployed form is usually larger than the unit — a Construction
        // Yard where an MCV stood. Its ground has to be clear, and the unit's
        // own footprint must not count against it, so the old claim is dropped
        // before the new one is tested.
        claim_footprint(&mut self.map, centre, from.footprint, false);
        let origin = footprint_origin(centre, to.footprint);
        let fits = self.map.can_place(origin, to.footprint.0, to.footprint.1);
        if !fits {
            claim_footprint(&mut self.map, centre, from.footprint, true);
            return false;
        }

        // Deliberately *not* checked against the build radius. An MCV deploying
        // is how a player gets their first building and how they expand to a
        // second base; requiring an existing structure nearby would make the
        // first one impossible.
        claim_footprint(&mut self.map, centre, to.footprint, true);

        let Some(unit) = self.units.get_mut(id) else {
            return false;
        };
        // Health carries across as a fraction rather than a number. The two
        // forms rarely have the same maximum — a Construction Yard is far
        // tougher than the MCV that became it — so copying the raw value would
        // either heal a damaged unit for free or leave a full-health one
        // apparently wounded.
        //
        // Rounded up, so a unit clinging to one point of health survives
        // deploying. Rounding down could kill it, which is a strange thing for
        // a player's own command to do.
        let fraction = (unit.health as u64 * to.max_health as u64)
            .div_ceil((from.max_health as u64).max(1)) as u32;
        unit.health = fraction.clamp(1, to.max_health);
        unit.kind = to_kind;

        // State that belonged to the old form and means nothing in the new one.
        // Left behind, a harvest cycle or a half-finished build queue would
        // keep running against stats that no longer describe it.
        unit.order = Order::Idle;
        unit.harvest = to.harvest_capacity.is_some().then(Default::default);
        unit.production = None;
        unit.rally = None;
        unit.combat = crate::combat::CombatState::default();
        // Kills are the unit's own record and survive: a veteran MCV that
        // deploys should still be a veteran when it undeploys. Everything above
        // is machinery; this is history.
        true
    }

    /// Demolishes a structure for part of its cost back.
    ///
    /// Only structures. Selling a tank would be an odd thing to allow and a
    /// very easy way to convert an army into cash mid-battle.
    fn sell(&mut self, player: PlayerId, building: EntityId) {
        if !self.owned_by(building, player) {
            return;
        }
        let Some(unit) = self.units.get(building) else {
            return;
        };
        let stats = self.stats.get(unit.owner, unit.kind);
        if stats.mobile || stats.cost == 0 {
            return;
        }

        // Paid on the building's condition, not its full price. A wreck is
        // worth less than a fresh one, which stops selling from being a way to
        // launder damage into money.
        let condition = if stats.max_health > 0 {
            (unit.health as u64 * 100) / stats.max_health as u64
        } else {
            0
        };
        let refund = ((stats.cost as u64 * SELL_REFUND_PERCENT as u64 * condition) / 10_000) as u32;
        self.treasury.deposit(player, refund);

        if let Some(unit) = self.units.get_mut(building) {
            unit.health = 0;
        }
    }

    /// An engineer walks into a repair hut and the bridge beside it comes back.
    ///
    /// Returns whether this pairing was a bridge repair. The correction worth
    /// recording is that bridges are repaired through a **hut**, not by
    /// touching the bridge — which makes this the same act as capturing a tech
    /// building rather than a new mechanic, and is why there is no
    /// bridge-repair command anywhere.
    fn repair_bridges(&mut self, engineer: EntityId, hut: EntityId) -> bool {
        let (Some(u), Some(h)) = (self.units.get(engineer), self.units.get(hut)) else {
            return false;
        };
        if !u.is_alive() || !h.is_alive() || u.is_aboard() {
            return false;
        }
        let stats = self.stats.get(u.owner, u.kind);
        let hut_stats = self.stats.get(h.owner, h.kind);
        if !stats.is_engineer || hut_stats.bridge_repair_radius == 0 {
            return false;
        }
        let (at, radius) = (h.cell(), hut_stats.bridge_repair_radius as i32);

        // Every wreck the hut serves, not just the nearest. A hut beside a
        // four-lane crossing should rebuild the crossing, and "which of these
        // spans is *the* bridge" is not a question with an answer.
        let wrecks: Vec<EntityId> = self
            .units
            .iter()
            .filter(|(_, b)| {
                let s = self.stats.get(b.owner, b.kind);
                s.is_bridge && !b.is_alive() && b.cell().chebyshev_to(at) <= radius
            })
            .map(|(id, _)| id)
            .collect();
        if wrecks.is_empty() {
            // Nothing to do, so the engineer is not spent. Walking one into a
            // hut beside an intact bridge should cost the player nothing.
            if let Some(u) = self.units.get_mut(engineer) {
                u.order = Order::Idle;
            }
            return true;
        }

        for wreck in wrecks {
            let Some(b) = self.units.get(wreck) else {
                continue;
            };
            let (centre, s) = (b.cell(), self.stats.get(b.owner, b.kind));
            if let Some(b) = self.units.get_mut(wreck) {
                b.health = s.max_health;
            }
            self.open_span(centre, s.footprint, true);
        }

        if stats.consumed_on_use {
            if let Some(u) = self.units.get_mut(engineer) {
                u.health = 0;
            }
        } else if let Some(u) = self.units.get_mut(engineer) {
            u.order = Order::Idle;
        }
        true
    }

    /// A spy reaches an enemy building and takes what it holds.
    ///
    /// Returns whether this pairing was an infiltration, so the caller can fall
    /// through to the other things arriving at a building can mean.
    ///
    /// The effect is read from the *building*, not from the spy. Infiltration
    /// is not one effect aimed at a target — it is a table keyed on what was
    /// entered, which is why an Allied lab yields something different from a
    /// Soviet one with no code knowing either exists.
    fn infiltrate(&mut self, spy: EntityId, building: EntityId) -> bool {
        use redshift_data::traits::{InfiltrationEffect, PlayerEffect, Trait};

        let (Some(u), Some(b)) = (self.units.get(spy), self.units.get(building)) else {
            return false;
        };
        if !u.is_alive() || !b.is_alive() || u.is_aboard() {
            return false;
        }
        let consumed = self
            .rules
            .entity(u.kind)
            .traits
            .iter()
            .find_map(|t| match t {
                Trait::Infiltrator { consumed } => Some(*consumed),
                _ => None,
            });
        let Some(consumed) = consumed else {
            return false;
        };
        // Somebody else's. Infiltrating your own barracks would be a strange
        // way to promote your infantry, and infiltrating a neutral building is
        // what an engineer is for.
        if b.owner == u.owner || b.owner.is_neutral() {
            return false;
        }
        let (owner, victim) = (u.owner, b.owner);
        let effect = self
            .rules
            .entity(b.kind)
            .traits
            .iter()
            .find_map(|t| match t {
                Trait::Infiltrated { effect } => Some(effect.clone()),
                _ => None,
            });
        // A spy that reached a building with nothing to steal has wasted
        // itself, exactly as in the original. Returning true rather than
        // falling through matters: without it the spy would try to capture the
        // building instead.
        let Some(effect) = effect else {
            if let Some(u) = self.units.get_mut(spy) {
                u.order = Order::Idle;
            }
            return true;
        };

        match effect {
            InfiltrationEffect::Promotes { category } => {
                self.grant_standing(owner, PlayerEffect::VeteranProduction(category));
            }
            InfiltrationEffect::Blackout { ticks } => {
                if let Some(until) = self.blackout_until.get_mut(victim.0 as usize) {
                    // Extended from now rather than added to what is left, so
                    // a second spy during a blackout is worth sending and yet
                    // two spies are not worth twice one.
                    *until = (*until).max(self.tick + ticks);
                }
            }
            InfiltrationEffect::StealsFunds { percent } => {
                let held = self.treasury.credits(victim);
                let taken = (held as u64 * percent.min(100) as u64 / 100) as u32;
                if self.treasury.try_spend(victim, taken) {
                    self.treasury.deposit(owner, taken);
                }
            }
            InfiltrationEffect::Unlocks { unit } => {
                if let Some(kind) = self.rules.kind_of(&unit)
                    && let Some(list) = self.unlocked.get_mut(owner.0 as usize)
                    && let Err(at) = list.binary_search(&kind)
                {
                    list.insert(at, kind);
                }
            }
        }

        if consumed {
            if let Some(u) = self.units.get_mut(spy) {
                u.health = 0;
            }
        } else if let Some(u) = self.units.get_mut(spy) {
            u.order = Order::Idle;
        }
        true
    }

    /// Infantry occupy a building and fight from inside it.
    ///
    /// Returns whether this pairing was a garrison at all, so the caller can
    /// fall through to the other two things that arriving at a building can
    /// mean.
    ///
    /// Only a *neutral* building can be occupied — the civilian ones scattered
    /// across a map — and an emptied one goes back to neutral. That is what the
    /// original does, and it is also what saves this from having to remember
    /// who owned the building first: there is only ever one answer.
    fn garrison(&mut self, occupant: EntityId, building: EntityId) -> bool {
        if !self.may_garrison(occupant, building) {
            return false;
        }
        let Some(owner) = self.units.get(occupant).map(|u| u.owner) else {
            return false;
        };
        // The building fights for whoever is inside it. Ownership carries the
        // vision, the targeting and the colour with it, which is why this is a
        // transfer rather than a flag saying who to shoot for.
        self.change_owner(building, owner);
        self.board(occupant, building);
        true
    }

    /// Whether this unit may occupy this building.
    fn may_garrison(&self, occupant: EntityId, building: EntityId) -> bool {
        let (Some(u), Some(b)) = (self.units.get(occupant), self.units.get(building)) else {
            return false;
        };
        if !u.is_alive() || !b.is_alive() || u.is_aboard() {
            return false;
        }
        let stats = self.stats.get(b.owner, b.kind);
        if stats.garrison_capacity == 0 || b.cargo.len() >= stats.garrison_capacity as usize {
            return false;
        }
        // Neutral, or already held by this player's own garrison. Anything else
        // is somebody's building and is captured or shot, not moved into.
        if !b.owner.is_neutral() && b.owner != u.owner {
            return false;
        }
        // A GI or a Conscript, not a commando. Category-matched like everything
        // else that asks "what sort of thing is this".
        let category = &self.rules.entity(u.kind).category;
        self.rules.entity(b.kind).traits.iter().any(|t| match t {
            redshift_data::traits::Trait::Garrisonable { categories, .. } => {
                categories.contains(category)
            }
            _ => false,
        })
    }

    /// Throws a garrison out of a building that has been shot up badly enough.
    ///
    /// The rule that makes a garrisoned building worth attacking rather than
    /// avoiding: clearing one means damaging it enough to evict, not destroying
    /// it. Killing the occupants with the building would make a garrison a
    /// death trap and nobody would ever use one.
    fn evict_broken_garrisons(&mut self) {
        let broken: Vec<(EntityId, Cell)> = self
            .units
            .iter()
            .filter(|(_, u)| {
                let stats = self.stats.get(u.owner, u.kind);
                if stats.garrison_capacity == 0 || u.cargo.is_empty() || !u.is_alive() {
                    return false;
                }
                // Multiplied out rather than divided, so a small building with
                // an odd maximum evicts at the same fraction as a large one.
                u.health as u64 * 100 < stats.max_health as u64 * stats.evict_below_percent as u64
            })
            .map(|(id, u)| (id, u.cell()))
            .collect();
        for (building, at) in broken {
            self.unload(building, at);
        }
    }

    /// One tick of repair for a unit sitting on a repair structure.
    ///
    /// Returns whether this pairing is a repair at all, so the caller can fall
    /// through to the engineer's very different behaviour. A structure that
    /// repairs nothing, or a unit of a category it does not service, is simply
    /// not this — and an engineer sent to a Service Depot should still capture
    /// or repair it rather than be serviced by it.
    ///
    /// Deliberately incremental. The original does not restore a tank the
    /// instant it touches the depot; it pulls the unit out of the fight for a
    /// while and charges for the privilege, and both halves of that are what
    /// makes it a decision.
    fn service_repair(&mut self, unit_id: EntityId, depot: EntityId) -> bool {
        let (Some(unit), Some(building)) = (self.units.get(unit_id), self.units.get(depot)) else {
            return false;
        };
        let owner = unit.owner;
        // Only your own. Driving a damaged tank into an enemy depot to be
        // patched up would be a strange thing for the enemy to allow.
        if building.owner != owner || !building.is_alive() {
            return false;
        }
        let depot_stats = self.stats.get(building.owner, building.kind);
        if depot_stats.repair_rate == 0 || !self.services(building.kind, unit.kind) {
            return false;
        }
        // A depot with no power does no work, like every other structure that
        // draws from the grid.
        if self.is_unpowered(building) {
            return true;
        }

        let stats = self.stats.get(owner, unit.kind);
        let health = unit.health;

        // The depot's other job, and the one that makes a Terror Drone a
        // problem to be solved rather than a sentence. Done on arrival rather
        // than over time, and before the health check: a unit at full health
        // with a drone in it still has a reason to be here.
        if depot_stats.cures_infestation {
            self.cure_infestation(unit_id);
        }

        if health >= stats.max_health {
            // Whole, and free to go. Left on an Enter order it would sit on the
            // depot forever, blocking the next customer.
            if let Some(unit) = self.units.get_mut(unit_id) {
                unit.order = Order::Idle;
            }
            return true;
        }

        // Charged in proportion to the damage actually undone, so a scratch
        // costs a little and a wreck costs a lot. The alternative — a flat fee
        // on arrival — would make it cheaper to let a unit die and rebuild it.
        let missing = stats.max_health - health;
        let step = depot_stats.repair_rate.min(missing);

        // Billed on the running total rather than per step. Pricing each step
        // on its own needs a rounding decision, and either answer is wrong:
        // rounding down makes a slow depot free, and rounding up overcharged by
        // a fifth here — a tank advertised at 200 credits to restore actually
        // cost 264, because three-credit steps were being taken to undo two and
        // a half credits of damage.
        //
        // The difference between two cumulative figures has no such drift. The
        // total is exact whatever the step size, and no accumulator has to be
        // carried on the unit.
        let owed = |health: u32| -> u32 {
            if depot_stats.repair_cost_percent == 0 || stats.max_health == 0 {
                return 0;
            }
            let undone = stats.max_health.saturating_sub(health) as u64;
            (stats.cost as u64 * depot_stats.repair_cost_percent as u64 * undone
                / (100 * stats.max_health as u64)) as u32
        };
        let price = owed(health).saturating_sub(owed(health + step));
        if price > 0 && !self.treasury.try_spend(owner, price) {
            // Broke. The unit waits rather than leaving: a player who is about
            // to sell something should not have to re-issue the order.
            return true;
        }

        if let Some(unit) = self.units.get_mut(unit_id) {
            unit.health = (unit.health + step).min(stats.max_health);
        }
        true
    }

    /// Whether a repair structure services a unit's category.
    ///
    /// Matched the same way production is: on the serviced thing's *category*
    /// against the structure's declared list. A Naval Shipyard is a Service
    /// Depot that says "ship" instead of "vehicle" — no code between them.
    fn services(&self, depot_kind: EntityKind, kind: EntityKind) -> bool {
        let serviced = &self.rules.entity(kind).category;
        self.rules
            .entity(depot_kind)
            .traits
            .iter()
            .any(|t| match t {
                redshift_data::traits::Trait::Repairs { categories, .. } => {
                    categories.contains(serviced)
                }
                _ => false,
            })
    }

    /// An engineer enters a building.
    ///
    /// One action with three outcomes, decided by whose building it is. The
    /// original never asked the player to choose between "capture" and
    /// "repair" — they chose a building, and the engineer did the appropriate
    /// thing. Splitting it into two commands would invent a decision the game
    /// does not have.
    fn enter_building(&mut self, engineer: EntityId, target: EntityId) {
        let (Some(unit), Some(building)) = (self.units.get(engineer), self.units.get(target))
        else {
            return;
        };
        let stats = self.stats.get(unit.owner, unit.kind);
        if !stats.is_engineer {
            return;
        }

        let owner = unit.owner;
        let building_owner = building.owner;
        let building_stats = self.stats.get(building_owner, building.kind);
        let damaged = building.health < building_stats.max_health;

        let acted = if building_owner == owner {
            // Own building: repaired outright, and only if it needed it.
            // Consuming an engineer on an undamaged building would be a pure
            // loss with nothing to show for it.
            if damaged {
                if let Some(b) = self.units.get_mut(target) {
                    b.health = building_stats.max_health;
                }
                true
            } else {
                false
            }
        } else if building_stats.capturable {
            // Anyone else's, including a neutral tech building. Health carries
            // over: capturing a wreck gives a wreck.
            self.change_owner(target, owner);
            true
        } else {
            false
        };

        if !acted {
            if let Some(unit) = self.units.get_mut(engineer) {
                unit.order = Order::Idle;
            }
            return;
        }

        // Consumed. That is what makes an engineer a considered purchase rather
        // than a tool you keep.
        if stats.consumed_on_use {
            if let Some(unit) = self.units.get_mut(engineer) {
                unit.health = 0;
            }
        } else if let Some(unit) = self.units.get_mut(engineer) {
            unit.order = Order::Idle;
        }
    }

    /// Transfers a unit to another player.
    ///
    /// Ownership is read from a great many places — targeting, vision, power,
    /// the stat table — so this exists to make the transfer one operation
    /// rather than a field assignment that someone forgets to accompany.
    ///
    /// Power and vision are rebuilt from scratch every tick, so both follow a
    /// capture on their own. A captured power plant supplies its new owner on
    /// the next tick with nothing here to arrange it.
    pub fn change_owner(&mut self, id: EntityId, owner: PlayerId) {
        let Some(unit) = self.units.get_mut(id) else {
            return;
        };
        unit.owner = owner;
        // Orders and targets belonged to the previous owner's intent.
        unit.order = Order::Idle;
        unit.combat.target = None;
        // Anything it was carrying changes hands with it, since a transport's
        // passengers cannot be somewhere else.
        let cargo = unit.cargo.clone();
        for passenger in cargo {
            if let Some(p) = self.units.get_mut(passenger) {
                p.owner = owner;
            }
        }
    }

    /// Puts a unit inside a transport.
    ///
    /// The passenger stays in the arena — it keeps its identity, health and
    /// rank — and is skipped by every pass that acts on the world.
    fn board(&mut self, passenger: EntityId, transport: EntityId) {
        // Releases the ground it was standing on, so nothing paths around a
        // unit that is no longer there.
        if let Some(unit) = self.units.get_mut(passenger) {
            unit.carrier = Some(transport);
            unit.order = Order::Idle;
            unit.combat.target = None;
        }
        if let Some(t) = self.units.get_mut(transport) {
            t.cargo.push(passenger);
        }
    }

    /// Puts every passenger back on the ground near the transport.
    fn unload(&mut self, transport: EntityId, at: Cell) {
        let Some(t) = self.units.get(transport) else {
            return;
        };
        let cargo = t.cargo.clone();
        let owner = t.owner;

        let mut placed: Vec<Cell> = Vec::new();
        for passenger in cargo {
            let Some(unit) = self.units.get(passenger) else {
                continue;
            };
            let movement = self.stats.get(unit.owner, unit.kind).movement;

            // Each passenger gets its own cell, or they would all be unloaded
            // on top of each other and spend the next second shoving apart.
            let spot = (0..=UNLOAD_SPREAD).find_map(|radius| {
                ring_offsets(radius).into_iter().find_map(|(dx, dy)| {
                    let cell = Cell::new(at.x + dx, at.y + dy);
                    (!placed.contains(&cell) && self.map.is_passable(cell, movement))
                        .then_some(cell)
                })
            });
            let Some(cell) = spot else {
                // Nowhere to stand. The passenger stays aboard rather than
                // being destroyed or dropped into a wall.
                continue;
            };
            placed.push(cell);

            if let Some(unit) = self.units.get_mut(passenger) {
                unit.carrier = None;
                unit.pos = cell.centre();
                unit.order = Order::Idle;
            }
            if let Some(t) = self.units.get_mut(transport) {
                t.cargo.retain(|p| *p != passenger);
            }
        }
        let _ = owner;

        // An emptied garrison goes back to being nobody's. Here rather than at
        // each call site, so an explicit "get out" order and a building shot
        // below the eviction threshold reach the same end — a burnt-out
        // civilian building must not keep fighting for whoever held it last.
        //
        // A transport has no garrison capacity, so this passes it by.
        let emptied = self.units.get(transport).is_some_and(|t| {
            t.cargo.is_empty() && self.stats.get(t.owner, t.kind).garrison_capacity > 0
        });
        if emptied {
            self.change_owner(transport, PlayerId::NEUTRAL);
        }
    }

    /// Whether a transport may take this passenger.
    fn may_board(&self, transport: EntityId, passenger: EntityId) -> bool {
        let (Some(t), Some(p)) = (self.units.get(transport), self.units.get(passenger)) else {
            return false;
        };
        if t.owner != p.owner || transport == passenger {
            return false;
        }
        if !t.is_alive() || !p.is_alive() || p.is_aboard() {
            return false;
        }
        let stats = self.stats.get(t.owner, t.kind);
        if stats.capacity == 0 || t.cargo.len() >= stats.capacity as usize {
            return false;
        }
        // The allowed list names entity ids, so a transport can carry infantry
        // and refuse tanks without either of them knowing about the other.
        let allowed = self
            .rules
            .entity(t.kind)
            .traits
            .iter()
            .find_map(|trait_| match trait_ {
                redshift_data::traits::Trait::Transport { allowed, .. } => Some(allowed),
                _ => None,
            });
        match allowed {
            Some(list) if !list.is_empty() => list.contains(&self.rules.entity(p.kind).id),
            _ => true,
        }
    }

    /// Sites a structure that has finished building.
    ///
    /// Every condition is checked here, not in the interface. A modified client
    /// could send anything, and — more to the point — every peer must reach the
    /// same answer about whether a building appeared, or their worlds diverge
    /// on the next tick.
    fn order_place_building(&mut self, player: PlayerId, producer: EntityId, at: Cell) {
        if !self.owned_by(producer, player) {
            return;
        }
        let Some(kind) = self
            .units
            .get(producer)
            .and_then(|u| u.production.as_ref())
            .and_then(|q| q.ready)
        else {
            return;
        };

        let footprint = self.stats.get(player, kind).footprint;
        if !self.can_build_at(player, at, footprint) {
            return;
        }

        // The stored position is the centre, but placement is given as the
        // footprint's corner, so the two have to be reconciled in exactly the
        // way `footprint_origin` reverses.
        let centre = Cell::new(
            at.x + (footprint.0 as i32 - 1) / 2,
            at.y + (footprint.1 as i32 - 1) / 2,
        );
        self.spawn_unit(player, kind, centre.centre());
        // Whatever the structure brings with it — a refinery's miner.
        let extras: Vec<EntityKind> = self
            .rules
            .entity(kind)
            .traits
            .iter()
            .filter_map(|t| match t {
                redshift_data::traits::Trait::Delivers { units } => Some(units.clone()),
                _ => None,
            })
            .flatten()
            .filter_map(|id| self.rules.kind_of(&id))
            .collect();
        self.deliver_extras(player, centre, &extras);

        if let Some(queue) = self
            .units
            .get_mut(producer)
            .and_then(|u| u.production.as_mut())
        {
            queue.ready = None;
        }
    }

    /// Whether a player may put a building of this size here.
    ///
    /// Two conditions. The ground has to be clear — that is
    /// [`Map::can_place`]. And the site has to be near something the player
    /// already owns, which is what stops a player dropping a barracks in the
    /// enemy's base and is most of what makes expanding a decision rather than
    /// a formality.
    pub fn can_build_at(&self, player: PlayerId, at: Cell, footprint: (u8, u8)) -> bool {
        if !self.map.can_place(at, footprint.0, footprint.1) {
            return false;
        }
        self.within_build_radius(player, at, footprint)
    }

    /// Whether a site is close enough to one of the player's existing buildings.
    fn within_build_radius(&self, player: PlayerId, at: Cell, footprint: (u8, u8)) -> bool {
        // Measured from the nearest corner of the proposed footprint, so a
        // large building is not penalised for being large.
        self.units.iter().any(|(_, unit)| {
            if unit.owner != player || !unit.is_alive() {
                return false;
            }
            let stats = self.stats.get(unit.owner, unit.kind);
            // Only structures anchor a build area. A tank parked in the enemy
            // base must not become a foothold.
            if stats.mobile {
                return false;
            }
            let theirs = unit.cell();
            let nearest_x = at.x.max(theirs.x).min(at.x + footprint.0 as i32 - 1);
            let nearest_y = at.y.max(theirs.y).min(at.y + footprint.1 as i32 - 1);
            (theirs.x - nearest_x)
                .abs()
                .max((theirs.y - nearest_y).abs())
                <= BUILD_RADIUS
        })
    }

    /// The structure a player has finished and not yet placed, if any.
    pub fn ready_to_place(&self, player: PlayerId) -> Option<(EntityId, EntityKind)> {
        self.units.iter().find_map(|(id, unit)| {
            if unit.owner != player {
                return None;
            }
            unit.production.as_ref()?.ready.map(|kind| (id, kind))
        })
    }

    /// A building of this player's that can produce `kind`.
    ///
    /// Exposed because the alternative is the interface deciding for itself
    /// which building makes what — a second copy of a rule that lives in the
    /// data, and one that would drift the moment a factory's categories
    /// changed. Ties break on the lowest entity id.
    pub fn producer_for(&self, player: PlayerId, kind: EntityKind) -> Option<EntityId> {
        self.units.iter().find_map(|(id, unit)| {
            (unit.owner == player && unit.is_alive() && self.can_produce(unit.kind, kind))
                .then_some(id)
        })
    }

    /// Places whatever a newly built thing arrives with.
    ///
    /// Beside it rather than inside it, and skipped if there is nowhere to
    /// stand — a free miner is a bonus, not a reason to fail a build.
    fn deliver_extras(&mut self, owner: PlayerId, near: Cell, kinds: &[EntityKind]) {
        for kind in kinds {
            let movement = self.stats.get(owner, *kind).movement;
            if let Some(cell) = self.free_cell_near(near, movement, EXIT_SEARCH_RADIUS) {
                self.spawn_unit(owner, *kind, cell.centre());
            }
        }
    }

    /// Whether a player's country lets them build this at all.
    ///
    /// `unique_units` and `removes_units` were declared in the data and checked
    /// at load, and nothing read them — so every country could build every
    /// other country's unique unit. This is what they were for.
    ///
    /// A unit named as unique by *any* country is available only to that
    /// country. That is what makes it unique, and it means a country needs no
    /// list of things it cannot have.
    pub fn available_to(&self, player: PlayerId, kind: EntityKind) -> bool {
        let id = &self.rules.entity(kind).id;
        let faction = self
            .players
            .iter()
            .find(|p| p.id == player)
            .and_then(|p| p.faction.as_deref());

        for other in self.rules.factions() {
            if other.unique_units.iter().any(|u| u == id) {
                // Someone's unique unit: only theirs.
                return faction == Some(other.id.as_str());
            }
        }

        // Not unique to anyone. Available unless this player's country gave it
        // up in exchange for something else.
        match faction.and_then(|f| self.rules.faction(f)) {
            Some(mine) => !mine.removes_units.iter().any(|u| u == id),
            None => true,
        }
    }

    /// How many of a kind a player has, counting what is already queued.
    ///
    /// Queued items count, or a player could fill a queue with commandos and
    /// get every one of them — the limit would only bite on the last.
    pub fn count_of(&self, player: PlayerId, kind: EntityKind) -> usize {
        let standing = self
            .units
            .iter()
            .filter(|(_, u)| u.owner == player && u.is_alive() && u.kind == kind)
            .count();
        let queued: usize = self
            .units
            .iter()
            .filter(|(_, u)| u.owner == player)
            .filter_map(|(_, u)| u.production.as_ref())
            .map(|q| {
                q.items().iter().filter(|i| i.kind == kind).count()
                    + usize::from(q.ready == Some(kind))
            })
            .sum();
        standing + queued
    }

    /// Whether a player may build another of this kind.
    pub fn within_build_limit(&self, player: PlayerId, kind: EntityKind) -> bool {
        let limit = self.stats.get(player, kind).build_limit;
        limit == 0 || self.count_of(player, kind) < limit as usize
    }

    /// Whether a producer of `producer_kind` makes things of `kind`.
    ///
    /// Matched on the produced thing's *category* against the producer's
    /// declared list, so a new unit slots into an existing factory by naming
    /// its category — no code, which is the whole point of the data layer.
    fn can_produce(&self, producer_kind: EntityKind, kind: EntityKind) -> bool {
        let producer = self.rules.entity(producer_kind);
        let produced = self.rules.entity(kind);
        producer.traits.iter().any(|t| match t {
            redshift_data::traits::Trait::Produces { categories } => {
                categories.contains(&produced.category)
            }
            _ => false,
        })
    }

    /// Whether a player owns everything an item requires.
    ///
    /// Prerequisites name entity ids, and a player satisfies one by owning a
    /// living entity of that kind. Buildings under construction do not count —
    /// they are not yet entities.
    pub fn prerequisites_met(&self, player: PlayerId, kind: EntityKind) -> bool {
        // Stolen technology answers for itself. A Soviet player who spied an
        // Allied lab builds an Allied commando, and no arrangement of their own
        // buildings could ever satisfy its prerequisites — that is exactly why
        // it is worth stealing.
        if self
            .unlocked
            .get(player.0 as usize)
            .is_some_and(|list| list.binary_search(&kind).is_ok())
        {
            return true;
        }
        let needed: Vec<&String> = self
            .rules
            .entity(kind)
            .traits
            .iter()
            .filter_map(|t| match t {
                redshift_data::traits::Trait::Buildable { prerequisites, .. } => {
                    Some(prerequisites)
                }
                _ => None,
            })
            .flatten()
            .collect();

        needed.iter().all(|required| {
            self.rules.kind_of(required).is_some_and(|required_kind| {
                self.units
                    .iter()
                    .any(|(_, u)| u.owner == player && u.is_alive() && u.kind == required_kind)
            })
        })
    }

    /// Rejects a command for a unit the issuing player does not own.
    ///
    /// Checked in the simulation rather than only in the interface: a modified
    /// client could send anything, and every peer must reach the same answer
    /// about whether an order was legal.
    fn owned_by(&self, id: EntityId, player: PlayerId) -> bool {
        self.units.get(id).is_some_and(|u| u.owner == player)
    }

    fn order_move(&mut self, player: PlayerId, id: EntityId, target: Cell) {
        if !self.owned_by(id, player) {
            return;
        }
        let Some(unit) = self.units.get_mut(id) else {
            return;
        };
        unit.order = Order::Move(Travel {
            destination: target,
            path: Vec::new(),
            needs_repath: true,
            retry_budget: DEFAULT_NODE_BUDGET,
        });
        if !self.path_queue.contains(&id) {
            self.path_queue.push(id);
        }
    }

    /// Orders a group to a target, giving each unit its own place to stand.
    ///
    /// # Why the destinations are spread
    ///
    /// Sending every unit to the same cell means only one of them can be right.
    /// The rest pile onto the same point and the separation pass shoves them
    /// apart — and because a unit that has arrived goes idle and never comes
    /// back, each new arrival ratchets the crowd wider. Nine units ordered to
    /// one cell settled into a blob four cells across, which is both ugly and
    /// nothing like a formation.
    ///
    /// Giving each unit a distinct nearby cell fixes it at the source. The
    /// separation pass then only has to handle incidental overlaps on the way,
    /// which is what it is good at.
    ///
    /// Assignment is deterministic: units are taken in entity-id order and
    /// given cells from a fixed outward spiral, so every peer builds the same
    /// formation.
    fn order_group_move(&mut self, player: PlayerId, units: &[EntityId], target: Cell) {
        // Entity-id order, so the assignment does not depend on the order the
        // player's client happened to list the selection in.
        let mut ordered: Vec<EntityId> = units
            .iter()
            .copied()
            .filter(|id| self.owned_by(*id, player))
            .collect();
        ordered.sort();
        ordered.dedup();

        if ordered.len() == 1 {
            self.order_move(player, ordered[0], target);
            return;
        }

        let mut taken: Vec<Cell> = Vec::with_capacity(ordered.len());
        for id in ordered {
            let movement = self
                .units
                .get(id)
                .map(|u| self.stats.get(u.owner, u.kind).movement)
                .unwrap_or_default();

            let spot = self
                .nearest_free_cell(target, movement, &taken)
                .unwrap_or(target);
            taken.push(spot);
            self.order_move(player, id, spot);
        }
    }

    /// The closest cell to `target` that is passable and not already claimed.
    ///
    /// Walks a fixed outward spiral, so two peers considering the same
    /// candidates in the same order arrive at the same formation.
    fn nearest_free_cell(
        &self,
        target: Cell,
        movement: SurfaceMask,
        taken: &[Cell],
    ) -> Option<Cell> {
        for radius in 0..=FORMATION_MAX_RADIUS {
            for (dx, dy) in ring_offsets(radius) {
                let cell = Cell::new(target.x + dx, target.y + dy);
                if !self.map.is_passable(cell, movement) {
                    continue;
                }
                if taken.contains(&cell) {
                    continue;
                }
                return Some(cell);
            }
        }
        None
    }

    // -- Phase 2: pathfinding ------------------------------------------------

    /// Spends this tick's path budget on queued requests, in queue order.
    ///
    /// The budget is counted in node expansions, never milliseconds. A
    /// time-based cutoff would give different results on a fast and a slow
    /// machine, which is the most common way an RTS desyncs.
    fn service_path_requests(&mut self) {
        let mut spent = 0u32;
        let mut deferred = Vec::new();

        // Drain rather than iterate: servicing may re-queue a unit that got a
        // partial route, and it must land at the back rather than be retried
        // immediately.
        let queue = std::mem::take(&mut self.path_queue);
        for id in queue {
            if spent >= TICK_PATH_BUDGET {
                deferred.push(id);
                continue;
            }
            let Some(unit) = self.units.get(id) else {
                continue;
            };
            let Some(travel) = unit.order.travel() else {
                continue;
            };
            if !travel.needs_repath {
                continue;
            }

            let start = unit.cell();
            let goal = travel.destination;
            let movement = self.stats.get(unit.owner, unit.kind).movement;
            let budget = travel.retry_budget.min(TICK_PATH_BUDGET - spent);

            let result = path::find_path(
                &self.map,
                &mut self.workspace,
                start,
                goal,
                movement,
                budget,
            );
            spent += self.workspace.last_expansions();

            let Some(unit) = self.units.get_mut(id) else {
                continue;
            };
            // The travel state is updated in place rather than the order being
            // replaced. Replacing it would quietly turn an attack-move into a
            // plain move the first time a route was recomputed, and the unit
            // would stop engaging without anything having obviously changed.
            if unit.is_aboard() {
                continue;
            }

            // A unit that is fighting stands and fights, provided its order
            // allows it to. This is the entire difference between a move and an
            // attack-move: a player repositioning an army expects it to arrive,
            // not to stop at the first thing that shoots at it.
            if unit.combat.target.is_some() && unit.order.engages_on_the_way() {
                continue;
            }

            let Some(travel) = unit.order.travel_mut() else {
                continue;
            };
            match result {
                PathResult::Found(cells) => {
                    travel.path = cells;
                    travel.needs_repath = false;
                    travel.retry_budget = DEFAULT_NODE_BUDGET;
                }
                PathResult::Partial(cells) => {
                    let empty = cells.is_empty();
                    travel.path = cells;
                    // Ask again on arrival: the goal may still be reachable, we
                    // just could not afford to prove it.
                    travel.needs_repath = true;
                    travel.retry_budget = budget.saturating_mul(2).max(DEFAULT_NODE_BUDGET);
                    if empty {
                        // No progress possible at this budget. Raise it and try
                        // again next tick rather than spinning forever.
                        deferred.push(id);
                    }
                }
                PathResult::Unreachable => {
                    // Proved unreachable, so stop. Retrying would burn the
                    // budget every tick for a route that does not exist.
                    unit.order = Order::Idle;
                }
            }
        }

        self.path_queue = deferred;
    }

    // -- Phase 3: movement ---------------------------------------------------

    fn move_units(&mut self) {
        let mut arrived = Vec::new();

        // Collected first so the stat table can be read while units are being
        // mutated. The table never changes during a match, so this is a borrow
        // concern rather than a semantic one.
        // Destructured so the arena and the stat table are borrowed
        // separately. Collecting the stats into a parallel vector first would
        // also satisfy the borrow checker, but it allocates every tick and —
        // worse — the two sequences could fall out of step, silently giving a
        // unit another unit's speed.
        let Sim { units, stats, .. } = self;

        for (id, unit) in units.iter_mut() {
            let unit_stats = stats.get(unit.owner, unit.kind);

            let Some(travel) = unit.order.travel_mut() else {
                continue;
            };
            let Some(&waypoint) = travel.path.first() else {
                // An empty route means the unit has nothing left to walk,
                // whether it finished one or never received one. Either way the
                // order is resolved below — leaving it in `Move` with no path
                // would strand the unit in a permanently busy state.
                arrived.push(id);
                continue;
            };

            let target = waypoint.centre();

            // Turn first, drive second. A unit that moves while badly
            // misaligned looks like it is sliding rather than steering.
            if let Some(heading) = unit.pos.heading_to(target) {
                unit.facing = unit.facing.rotate_toward(heading, unit_stats.turn_rate);
                if unit.facing.delta(heading).unsigned_abs() > MOVE_ALIGNMENT as u32 {
                    continue;
                }
            }

            let remaining = unit.pos.dist(target);
            if remaining <= unit_stats.speed.max(ARRIVAL_TOLERANCE) {
                // Snap to the waypoint rather than overshooting and correcting
                // next tick, which would read as jitter.
                unit.pos = target;
                travel.path.remove(0);
                if travel.path.is_empty() {
                    arrived.push(id);
                }
            } else {
                let step = unit.pos.step(unit.facing, unit_stats.speed);
                unit.pos = step;
            }
        }

        // Units that ran out of route are resolved after the movement pass, so
        // every unit moves before any re-path is queued and the phases stay
        // cleanly separated.
        for id in arrived {
            let Some(unit) = self.units.get(id) else {
                continue;
            };
            let Some(travel) = unit.order.travel() else {
                continue;
            };
            let (destination, needs_repath) = (&travel.destination, &travel.needs_repath);
            let destination = *destination;
            let needs_repath = *needs_repath;
            let at_destination = unit.cell() == destination;

            let Some(unit) = self.units.get_mut(id) else {
                continue;
            };
            if at_destination || !needs_repath {
                // Arrived, or walked a route that was known to be complete.
                //
                // Only a plain move is *finished* by arriving. The others own
                // their own lifetimes and are ended in `update_engagements`
                // when their reason to exist goes away — the target dies, the
                // transport fills up, the building is entered.
                //
                // Getting this wrong was subtle: holding position by clearing
                // the path made the movement pass think the unit had arrived,
                // so an attack order was wiped the instant it came into range
                // and the unit stood there having been told to shoot something.
                unit.order = match &unit.order {
                    Order::Guard { post, .. } => Order::Guard {
                        post: *post,
                        returning: None,
                    },
                    Order::Attack { .. } | Order::Board { .. } | Order::Enter { .. } => {
                        unit.order.clone()
                    }
                    _ => Order::Idle,
                };
            } else if !self.path_queue.contains(&id) {
                // The route was partial: ask for the next leg.
                self.path_queue.push(id);
            }
        }

        // Positions can only be nudged out of bounds by future collision
        // handling, but clamping here keeps the invariant local to movement.
        for (_, unit) in self.units.iter_mut() {
            unit.pos = self.map.clamp_pos(unit.pos);
        }
    }

    // -- State hashing -------------------------------------------------------

    /// A hash over everything that affects gameplay.
    ///
    /// Peers exchange this once per second; a mismatch means the simulations
    /// have diverged and the match must stop. Deliberately excludes the
    /// pathfinding workspace, which is a cache and can legitimately differ.
    pub fn state_hash(&self) -> u64 {
        let mut h = StateHasher::new();
        h.write_u32(self.tick);
        h.write_u64(self.rng.state());
        h.write(&self.map);
        // Rules are part of the world. Two peers with different unit stats have
        // already diverged, whatever their unit positions say — folding the
        // hash in here catches that on the very first comparison rather than
        // whenever the difference happens to matter.
        h.write_u64(self.rules.hash());
        h.write(&self.stats);
        h.write(&self.boons);
        // The three pieces of player state that are *not* rebuilt each tick,
        // and so are not implied by anything else already hashed. A blackout
        // that had run out on one peer and not the other would be a divergence
        // nothing else could see.
        for until in &self.blackout_until {
            h.write_u32(*until);
        }
        h.write_u32(self.hazards.len() as u32);
        for hazard in &self.hazards {
            h.write(&hazard.cell);
            h.write_u32(hazard.until);
            h.write_u32(hazard.damage);
            h.write_u32(hazard.warhead.0 as u32);
        }
        for effects in &self.standing {
            h.write_u32(effects.len() as u32);
            for effect in effects {
                // Written out by hand rather than through `Debug`. A derived
                // format is not a contract: a field renamed for clarity would
                // silently change every peer's hash, and the desync would
                // arrive with nothing in the diff to explain it.
                match effect {
                    redshift_data::traits::PlayerEffect::OreValue(percent) => {
                        h.write_u8(0);
                        h.write_i32(percent.0);
                    }
                    redshift_data::traits::PlayerEffect::VeteranProduction(category) => {
                        h.write_u8(1);
                        h.write_bytes(category.as_bytes());
                    }
                    redshift_data::traits::PlayerEffect::RepairEverywhere => h.write_u8(2),
                }
            }
        }
        for kinds in &self.unlocked {
            h.write_u32(kinds.len() as u32);
            for kind in kinds {
                h.write_u32(kind.0 as u32);
            }
        }
        // Shots in flight are world state: two peers that disagree about a
        // shell already in the air will disagree about who is alive a second
        // later, and the divergence would surface with nothing visibly wrong
        // at the moment it happened.
        h.write_u32(self.projectiles.len() as u32);
        for shot in &self.projectiles {
            h.write(shot);
        }

        // Slot order, including empty slots, so that two peers whose arenas
        // differ only in which slots are free still register as divergent —
        // because their next spawn would land in different places.
        h.write_u32(self.units.capacity_used() as u32);
        h.write_u32(self.units.len() as u32);
        for (id, unit) in self.units.iter() {
            h.write_u32(id.index());
            h.write_u32(id.generation());
            h.write(unit);
        }

        h.write_u32(self.path_queue.len() as u32);
        for id in &self.path_queue {
            h.write_u32(id.index());
            h.write_u32(id.generation());
        }
        h.finish()
    }

    /// A read-only view for renderers.
    pub fn view(&self) -> WorldView<'_> {
        WorldView { sim: self }
    }
}

/// The renderer's window onto the world.
///
/// Read-only by construction. The renderer never writes back — player input
/// becomes a [`Command`] and enters through the network layer instead. See
/// `docs/01-architecture.md`.
pub struct WorldView<'a> {
    sim: &'a Sim,
}

impl<'a> WorldView<'a> {
    #[inline]
    pub fn tick(&self) -> Tick {
        self.sim.tick
    }

    #[inline]
    pub fn map(&self) -> &'a Map {
        &self.sim.map
    }

    /// Live units, in slot order.
    pub fn units(&self) -> impl Iterator<Item = (EntityId, &'a Unit)> {
        self.sim.units.iter()
    }

    #[inline]
    pub fn unit(&self, id: EntityId) -> Option<&'a Unit> {
        self.sim.units.get(id)
    }

    #[inline]
    /// A unit's resolved stats, as its owner fields them.
    ///
    /// The renderer needs the maximum health to draw a health bar, and the
    /// radius to size a selection ring. Both live in the stat table rather than
    /// on the unit, so the view has to reach them.
    pub fn stats_of(&self, id: EntityId) -> Option<UnitStats> {
        self.unit(id).map(|u| self.sim.stats.get(u.owner, u.kind))
    }

    /// How hurt a unit is, as a percentage of its maximum.
    ///
    /// Returned as an integer percentage rather than a fraction: the caller is
    /// the renderer, which will turn it into a float anyway, and keeping the
    /// simulation side integral means this can be used in simulation code too
    /// without introducing the one float that ruins everything.
    pub fn health_percent(&self, id: EntityId) -> Option<u32> {
        let unit = self.unit(id)?;
        let max = self.sim.stats.get(unit.owner, unit.kind).max_health;
        if max == 0 {
            return None;
        }
        Some(((unit.health as u64 * 100) / max as u64) as u32)
    }

    pub fn unit_count(&self) -> usize {
        self.sim.units.len()
    }

    /// Units waiting on a path. Diagnostics and the performance overlay.
    #[inline]
    pub fn pending_paths(&self) -> usize {
        self.sim.path_queue.len()
    }
}

/// Sum of the distances every unit still has to travel. Test helper.
#[doc(hidden)]
pub fn total_remaining_distance(sim: &Sim) -> Fx {
    sim.units
        .iter()
        .filter_map(|(_, u)| match &u.order {
            Order::Move(t) => Some(u.pos.dist(t.destination.centre())),
            _ => None,
        })
        .sum()
}
