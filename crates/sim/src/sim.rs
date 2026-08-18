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
use crate::fx::{Angle, Fx};
use crate::hash::StateHasher;
use crate::map::{Cell, Locomotor, Map, WorldPos};
use crate::path::{self, DEFAULT_NODE_BUDGET, PathResult, PathWorkspace};
use crate::rng::SimRng;
use crate::stats::{StatTable, UnitStats};
use crate::unit::{
    ARRIVAL_TOLERANCE, FIRING_ARC, MAX_SEPARATION_STEP, MOVE_ALIGNMENT, Order, SEPARATION_EPSILON,
    Unit,
};
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
fn ring_offsets(radius: i32) -> Vec<(i32, i32)> {
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

        let mut units = Arena::with_capacity(setup.spawns.len());
        for spawn in &setup.spawns {
            let max_health = stats.get(spawn.owner, spawn.kind).max_health;
            units.insert(Unit::new(
                spawn.owner,
                spawn.kind,
                setup.map.clamp_pos(spawn.pos),
                max_health,
            ));
        }
        let workspace = PathWorkspace::new(setup.map.cell_count());
        Sim {
            tick: 0,
            map: setup.map,
            units,
            rng: SimRng::new(setup.seed),
            path_queue: Vec::new(),
            workspace,
            combat: CombatTable::build(&setup.rules),
            separation_buckets: Vec::new(),
            rules: setup.rules,
            stats,
            players: setup.players,
        }
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
        let max_health = self.stats.get(owner, kind).max_health;
        self.units
            .insert(Unit::new(owner, kind, self.map.clamp_pos(pos), max_health))
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
        self.service_path_requests();
        self.move_units();
        self.separate_units();
        let hits = self.acquire_and_fire();
        self.resolve_damage(&hits);
        self.remove_the_dead();

        self.tick += 1;
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
            if self.map.is_passable(target_cell, stats.locomotor) {
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

            // Keep the current target if it is still worth shooting at, so a
            // unit does not flicker between two equally close enemies.
            let keep = unit.combat.target.filter(|t| {
                combat::target_is_valid(unit, *t, &weapon, &self.units, &Self::are_allied)
            });
            let target = keep.or_else(|| {
                combat::choose_target(attacker, unit, &weapon, &self.units, &Self::are_allied)
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
            unit.combat.reload_remaining = weapon.reload;
            let at = self.units.get(target).map(|t| t.pos);

            if let Some(at) = at {
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
        hits
    }

    /// Applies the shots collected this tick.
    fn resolve_damage(&mut self, hits: &[PendingHit]) {
        for hit in hits {
            // The primary target takes the shot even if it has moved, since the
            // shot was already committed this tick.
            if let Some(target) = self.units.get(hit.target) {
                let armour = self.combat.armour(target.kind);
                let damage =
                    self.combat
                        .damage_table()
                        .damage_against(hit.damage, hit.warhead, armour);
                if let Some(target) = self.units.get_mut(hit.target) {
                    target.take_damage(damage);
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
            }
        }
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
        unit.order = Order::Move {
            destination: target,
            path: Vec::new(),
            needs_repath: true,
            retry_budget: DEFAULT_NODE_BUDGET,
        };
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
            let locomotor = self
                .units
                .get(id)
                .map(|u| self.stats.get(u.owner, u.kind).locomotor)
                .unwrap_or_default();

            let spot = self
                .nearest_free_cell(target, locomotor, &taken)
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
        locomotor: Locomotor,
        taken: &[Cell],
    ) -> Option<Cell> {
        for radius in 0..=FORMATION_MAX_RADIUS {
            for (dx, dy) in ring_offsets(radius) {
                let cell = Cell::new(target.x + dx, target.y + dy);
                if !self.map.is_passable(cell, locomotor) {
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
            let Order::Move {
                destination,
                needs_repath,
                retry_budget,
                ..
            } = &unit.order
            else {
                continue;
            };
            if !needs_repath {
                continue;
            }

            let start = unit.cell();
            let goal = *destination;
            let locomotor = self.stats.get(unit.owner, unit.kind).locomotor;
            let budget = (*retry_budget).min(TICK_PATH_BUDGET - spent);

            let result = path::find_path(
                &self.map,
                &mut self.workspace,
                start,
                goal,
                locomotor,
                budget,
            );
            spent += self.workspace.last_expansions();

            let Some(unit) = self.units.get_mut(id) else {
                continue;
            };
            match result {
                PathResult::Found(cells) => {
                    unit.order = Order::Move {
                        destination: goal,
                        path: cells,
                        needs_repath: false,
                        retry_budget: DEFAULT_NODE_BUDGET,
                    };
                }
                PathResult::Partial(cells) => {
                    if cells.is_empty() {
                        // No progress possible at this budget. Raise it and try
                        // again next tick rather than spinning on the same
                        // search forever.
                        unit.order = Order::Move {
                            destination: goal,
                            path: Vec::new(),
                            needs_repath: true,
                            retry_budget: budget.saturating_mul(2).max(DEFAULT_NODE_BUDGET),
                        };
                        deferred.push(id);
                    } else {
                        unit.order = Order::Move {
                            destination: goal,
                            path: cells,
                            // Ask again on arrival; the goal may still be
                            // reachable, we just could not afford to prove it.
                            needs_repath: true,
                            retry_budget: budget.saturating_mul(2).max(DEFAULT_NODE_BUDGET),
                        };
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

            let Order::Move { path, .. } = &mut unit.order else {
                continue;
            };
            let Some(&waypoint) = path.first() else {
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
                path.remove(0);
                if path.is_empty() {
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
            let Order::Move {
                destination,
                needs_repath,
                ..
            } = &unit.order
            else {
                continue;
            };
            let destination = *destination;
            let needs_repath = *needs_repath;
            let at_destination = unit.cell() == destination;

            let Some(unit) = self.units.get_mut(id) else {
                continue;
            };
            if at_destination || !needs_repath {
                // Arrived, or walked a route that was known to be complete.
                unit.order = Order::Idle;
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
            Order::Move { destination, .. } => Some(u.pos.dist(destination.centre())),
            Order::Idle => None,
        })
        .sum()
}
