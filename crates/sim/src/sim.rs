//! The simulation itself: state, the tick loop, and the read-only view.
//!
//! See the crate documentation for the rules this module lives under. The one
//! worth repeating: **the phase order inside [`Sim::tick`] is part of the
//! game's contract.** Reordering it changes behaviour and invalidates every
//! recorded replay.

use serde::{Deserialize, Serialize};

use redshift_data::rules::{EntityKind, Rules};

use crate::arena::{Arena, EntityId};
use crate::combat::{self, CombatTable, PendingHit};
use crate::command::{Command, CommandKind, PlayerId};
use crate::economy::{self, Treasury};
use crate::fx::{Angle, Fx};
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
            claim_footprint(&mut map, unit.cell(), spawn_stats.footprint, true);
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
            visibility: Visibility::new(map_width, map_height, factions.len()),
            rules: setup.rules,
            stats,
            players: setup.players,
        };
        // Built before the world is handed out, not on the first tick. A base
        // that reported no power until something ticked would show the player
        // a blackout on the frame they started the match.
        sim.recompute_power();
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
        claim_footprint(&mut self.map, unit.cell(), stats.footprint, true);
        self.units.insert(unit)
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
        let landed = self.advance_projectiles();
        let hits = self.acquire_and_fire();
        self.resolve_damage(&landed);
        self.resolve_damage(&hits);
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

        self.tick += 1;
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
        for (_, unit) in self.units.iter() {
            if !unit.is_alive() {
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
            self.visibility
                .reveal(unit.owner, unit.cell(), stats.vision);
            // A detector reveals cloaked things across the same ground it
            // watches. A separate layer, because a player can see a patch of
            // ground perfectly well and still not see what is standing on it.
            if stats.detector {
                self.visibility
                    .reveal_cloaked(unit.owner, unit.cell(), stats.vision);
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
    }

    /// Shots currently in the air.
    pub fn projectiles(&self) -> &[Projectile] {
        &self.projectiles
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
                self.spawn_unit(owner, kind, cell.centre());
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
                        self.treasury
                            .deposit(owner, state.load * economy::CREDITS_PER_ORE);
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
            if self.map.is_passable(target_cell, stats.movement) {
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

        // A snapshot for targeting, so every attacker sees the same world.
        // Cloning the arena each tick would be wasteful; the targeting pass
        // reads it immutably and the firing pass writes only to the attacker.
        let ids = self.units.ids();

        for attacker in ids {
            let Some(unit) = self.units.get(attacker) else {
                continue;
            };
            if !unit.is_alive() {
                continue;
            }
            let Some(weapon) = self.combat.weapon(unit.kind).copied() else {
                continue;
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
            let target = keep.or_else(|| {
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

            let Some(target) = target else { continue };
            let owner = unit.owner;
            let firing_from = unit.pos;
            unit.combat.reload_remaining = weapon.reload;
            // Firing gives a cloaked unit away.
            unit.since_fired = 0;
            let at = self.units.get(target).map(|t| t.pos);

            if let Some(at) = at {
                if weapon.projectile_speed > Fx::ZERO {
                    // Slow enough to be worth watching fly. Launched now and
                    // resolved when it lands, which is what lets it be dodged.
                    launched.push(Projectile {
                        attacker,
                        owner,
                        target: weapon.homing.then_some(target),
                        aim: at,
                        pos: firing_from,
                        speed: weapon.projectile_speed,
                        damage: weapon.damage,
                        warhead: weapon.warhead,
                        splash_radius: weapon.splash_radius,
                        fuse: crate::projectile::MAX_FLIGHT_TICKS,
                    });
                } else {
                    // A rifle. Instant, exactly as before.
                    hits.push(PendingHit {
                        attacker,
                        target,
                        damage: weapon.damage,
                        warhead: weapon.warhead,
                        splash_radius: weapon.splash_radius,
                        at,
                    });
                }
            }
        }
        self.projectiles.extend(launched);
        hits
    }

    /// Moves everything in flight and resolves what lands.
    ///
    /// Run before targeting, so a shot fired last tick lands before this tick's
    /// shots are chosen — otherwise a unit would keep firing at something the
    /// shell already in the air is about to kill.
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
                let killed = target.is_alive() && target.health <= damage;

                if let Some(target) = self.units.get_mut(hit.target) {
                    target.take_damage(damage);
                }
                // Credited on the killing blow only. Anything else and a unit
                // promotes for as long as it keeps firing at a body.
                if killed
                    && hit.attacker != hit.target
                    && let Some(attacker) = self.units.get_mut(hit.attacker)
                {
                    attacker.kills = attacker.kills.saturating_add(1);
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
        // A destroyed building has to release its footprint, or it leaves a
        // permanent hole in the map that nothing can walk through and nothing
        // can build on — invisible, and impossible to explain to a player.
        let released: Vec<(Cell, (u8, u8))> = self
            .units
            .iter()
            .filter(|(_, u)| !u.is_alive())
            .filter_map(|(_, u)| {
                let stats = self.stats.get(u.owner, u.kind);
                (stats.footprint != (1, 1)).then(|| (u.cell(), stats.footprint))
            })
            .collect();
        for (centre, footprint) in released {
            claim_footprint(&mut self.map, centre, footprint, false);
        }

        self.units.retain(|_, unit| unit.is_alive());
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

                Order::AttackMove(_) | Order::Move(_) | Order::Idle => {}
            }
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
                // A guard keeps its post rather than forgetting it.
                unit.order = match &unit.order {
                    Order::Guard { post, .. } => Order::Guard {
                        post: *post,
                        returning: None,
                    },
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
