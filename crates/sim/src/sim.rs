//! The simulation itself: state, the tick loop, and the read-only view.
//!
//! See the crate documentation for the rules this module lives under. The one
//! worth repeating: **the phase order inside [`Sim::tick`] is part of the
//! game's contract.** Reordering it changes behaviour and invalidates every
//! recorded replay.

use serde::{Deserialize, Serialize};

use redshift_data::rules::{EntityKind, Rules};

use crate::arena::{Arena, EntityId};
use crate::command::{Command, CommandKind, PlayerId};
use crate::fx::Fx;
use crate::hash::StateHasher;
use crate::map::{Cell, Map, WorldPos};
use crate::path::{self, DEFAULT_NODE_BUDGET, PathResult, PathWorkspace};
use crate::rng::SimRng;
use crate::stats::{StatTable, UnitStats};
use crate::unit::{ARRIVAL_TOLERANCE, MOVE_ALIGNMENT, Order, Unit};
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

        self.tick += 1;
    }

    // -- Phase 1: commands ---------------------------------------------------

    fn apply_commands(&mut self, commands: &[Command]) {
        for command in commands {
            match &command.kind {
                CommandKind::Move { units, target } => {
                    for &id in units {
                        self.order_move(command.player, id, *target);
                    }
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
