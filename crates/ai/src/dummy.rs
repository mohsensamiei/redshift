//! An opponent that builds, defends, and never attacks.
//!
//! Deliberate rather than broken. It exists so a player can learn the game,
//! test a build order, or try a unit against something that shoots back without
//! being under a clock. It thinks exactly as well as [`Difficulty::Easy`] —
//! the only difference is what it is willing to do with the conclusion.
//!
//! Which makes it the right first opponent to write, because it forces every
//! piece except target selection: economy, power, prerequisites, placement,
//! production, and defending what it has built.

use redshift_data::traits::Trait;
use redshift_sim::command::{CommandKind, PlayerId};
use redshift_sim::map::Cell;
use redshift_sim::sim::Sim;
use redshift_sim::{EntityId, EntityKind, Tick};

use crate::skill::Difficulty;

/// How far from its construction yard an opponent considers "its base".
///
/// Everything it owns inside this is worth defending; anything of the enemy's
/// inside this is worth shooting. A dummy never goes further than this, which
/// is the whole of "does not attack".
const BASE_RADIUS: i32 = 20;

/// How far out from the yard to look for somewhere to put a building.
///
/// Smaller than the build radius, so a base stays compact and the opponent does
/// not creep a power plant across the map one placement at a time.
const LAYOUT_RADIUS: i32 = 7;

/// One computer opponent.
pub struct Commander {
    player: PlayerId,
    difficulty: Difficulty,
    /// The last tick it thought on, so it thinks at its own pace rather than
    /// every tick. Its whole reaction time lives in this one number.
    last_thought: Tick,
}

impl Commander {
    pub fn new(player: PlayerId, difficulty: Difficulty) -> Commander {
        Commander {
            player,
            difficulty,
            // Deliberately not zero: an opponent that thought on tick zero would
            // act before the first frame is drawn, and a player watching would
            // see a building appear out of nothing.
            last_thought: 0,
        }
    }

    pub fn player(&self) -> PlayerId {
        self.player
    }

    pub fn difficulty(&self) -> Difficulty {
        self.difficulty
    }

    /// Decides what to do this tick.
    ///
    /// Returns commands rather than issuing them, so the caller puts them
    /// through the same ordered queue a human's go through.
    ///
    /// One decision per call at most. An opponent that queued a building, three
    /// units and four attack orders in one tick would spend its whole income in
    /// a single frame and then stand idle — which is not how a hard opponent
    /// should feel, let alone an easy one.
    pub fn think(&mut self, sim: &Sim) -> Vec<CommandKind> {
        let tick = sim.tick_number();
        if tick.saturating_sub(self.last_thought) < self.difficulty.think_interval() {
            return Vec::new();
        }
        self.last_thought = tick;

        // Defence first, always. An opponent that finished its build order
        // while its base burned would be a worse opponent than one that cannot
        // build at all.
        let mut orders = self.defend(sim);

        // A finished building waiting to be sited blocks the queue, so placing
        // it comes before deciding what to make next.
        if let Some(order) = self.place_ready_building(sim) {
            orders.push(order);
            return orders;
        }
        if let Some(order) = self.build_something(sim) {
            orders.push(order);
        }
        orders
    }

    /// Where its base is.
    ///
    /// The construction yard, or failing that whatever immobile thing it owns
    /// with the lowest entity id — deterministic, and it survives the yard
    /// being destroyed, which is when an opponent most needs to know where it
    /// lives.
    fn base(&self, sim: &Sim) -> Option<Cell> {
        let mut fallback = None;
        for (_, unit) in sim.view().units() {
            if unit.owner != self.player || !unit.is_alive() {
                continue;
            }
            let stats = sim.stats().get(unit.owner, unit.kind);
            if stats.mobile {
                continue;
            }
            if sim.producer_for(self.player, unit.kind).is_some() || stats.footprint.0 >= 3 {
                return Some(unit.cell());
            }
            if fallback.is_none() {
                fallback = Some(unit.cell());
            }
        }
        fallback
    }

    /// Sends the standing army at anything hostile inside the base.
    ///
    /// This is the whole of a dummy's aggression, and the reason it is not a
    /// punching bag: walk into its base and it fights. It simply will not come
    /// to you.
    fn defend(&self, sim: &Sim) -> Vec<CommandKind> {
        let Some(base) = self.base(sim) else {
            return Vec::new();
        };

        // The nearest intruder to the base, not to any particular defender:
        // every defender goes to the same one, which is both the right answer
        // and a deterministic one.
        let mut intruder: Option<(EntityId, i32)> = None;
        for (id, unit) in sim.view().units() {
            if unit.owner == self.player || unit.owner.is_neutral() || !unit.is_alive() {
                continue;
            }
            if !sim.can_see(self.player, unit) {
                continue;
            }
            let distance = unit.cell().chebyshev_to(base);
            if distance > BASE_RADIUS {
                continue;
            }
            if intruder.is_none_or(|(_, best)| distance < best) {
                intruder = Some((id, distance));
            }
        }
        let Some((target, _)) = intruder else {
            return Vec::new();
        };

        // Everything that can shoot and is not already shooting at it. Reissuing
        // to units that already have the order would flood the command queue
        // with no effect at all.
        let defenders: Vec<EntityId> = sim
            .view()
            .units()
            .filter(|(_, u)| u.owner == self.player && u.is_alive() && !u.is_aboard())
            .filter(|(_, u)| {
                let stats = sim.stats().get(u.owner, u.kind);
                stats.mobile && sim.combat().weapon(u.kind).is_some()
            })
            .filter(|(_, u)| u.combat.target != Some(target))
            .map(|(id, _)| id)
            .collect();

        if defenders.is_empty() {
            Vec::new()
        } else {
            vec![CommandKind::Attack {
                units: defenders,
                target,
            }]
        }
    }

    /// Sites a building that has finished and is waiting for somewhere to go.
    fn place_ready_building(&self, sim: &Sim) -> Option<CommandKind> {
        let (producer, kind) = sim.ready_to_place(self.player)?;
        let base = self.base(sim)?;
        let at = self.site_for(sim, base, kind)?;
        Some(CommandKind::PlaceBuilding { producer, at })
    }

    /// Somewhere legal to put a footprint, spiralling out from the base.
    ///
    /// A deterministic scan rather than a random spot. Random placement is the
    /// obvious way to write this and it is wrong twice over: it desyncs, and it
    /// produces bases that look like somebody spilled them.
    fn site_for(&self, sim: &Sim, base: Cell, kind: EntityKind) -> Option<Cell> {
        for ring in 2..=LAYOUT_RADIUS {
            for dy in -ring..=ring {
                for dx in -ring..=ring {
                    // Only the edge of each ring, so the search really does
                    // work outwards rather than re-testing the middle.
                    if dx.abs() != ring && dy.abs() != ring {
                        continue;
                    }
                    let at = Cell::new(base.x + dx, base.y + dy);
                    if sim.can_place_kind(self.player, kind, at) {
                        return Some(at);
                    }
                }
            }
        }
        None
    }

    /// Picks the next thing to build, and queues it.
    ///
    /// In priority order, and the order is the opinion: power before economy
    /// before army. An opponent that built tanks it could not power would look
    /// exactly like a bug.
    fn build_something(&self, sim: &Sim) -> Option<CommandKind> {
        let credits = sim.treasury().credits(self.player);
        // A weak opponent sits on a reserve it never spends, which is the most
        // reliable marker of a weak player and should be visible on its face.
        let spendable = credits * self.difficulty.spend_share() / 100;

        let wants = [
            self.wants_power(sim),
            self.wants_economy(sim),
            self.wants_production(sim),
            self.wants_army(sim),
        ];
        // The first thing it wants, not the first it can afford. Saving up
        // rather than skipping to something cheaper: an opponent that always
        // bought whatever fitted in its wallet would never build anything
        // expensive, and would drown a power shortage in infantry.
        //
        // This reads like a loop and is not one — it takes the highest priority
        // want and stops. Written as a loop first, and clippy was right to
        // point out that the later entries were unreachable: every branch
        // returned. Saying `next()` says what it does.
        let kind = wants.into_iter().flatten().next()?;
        if sim.stats().get(self.player, kind).cost > spendable {
            return None;
        }
        let building = sim.producer_for(self.player, kind)?;
        Some(CommandKind::Produce { building, kind })
    }

    /// A power plant, if the grid is short and none is already coming.
    fn wants_power(&self, sim: &Sim) -> Option<EntityKind> {
        if sim.power().is_satisfied(self.player) {
            return None;
        }
        let wanted = self.cheapest(sim, |sim, kind| {
            sim.rules()
                .entity(kind)
                .traits
                .iter()
                .any(|t| matches!(t, Trait::PowerSupply { .. }))
        })?;
        // `count_of` counts what is queued as well as what is standing, which
        // is the whole point of asking it. Without that this queues a plant on
        // every think until the first one finishes — and a base full of half
        // the power plants it can afford is what that looks like from outside.
        (!self.already_coming(sim, wanted)).then_some(wanted)
    }

    /// Whether one of these is standing or on its way.
    ///
    /// The single most important question an opponent asks itself. Judging by
    /// what is *standing* alone means queueing a second refinery while the
    /// first is still being built, and a third while the second is — an
    /// opponent that empties its bank into four copies of the same building
    /// looks broken rather than bad.
    fn already_coming(&self, sim: &Sim, kind: EntityKind) -> bool {
        sim.count_of(self.player, kind) > 0
    }

    /// A refinery if it has none, or a harvester if it is short of them.
    fn wants_economy(&self, sim: &Sim) -> Option<EntityKind> {
        let has = |f: &dyn Fn(&Sim, EntityKind) -> bool| {
            sim.view()
                .units()
                .filter(|(_, u)| u.owner == self.player && u.is_alive())
                .any(|(_, u)| f(sim, u.kind))
        };
        let is_refinery =
            |sim: &Sim, kind: EntityKind| sim.stats().get(self.player, kind).is_refinery;
        let is_harvester = |sim: &Sim, kind: EntityKind| {
            sim.stats()
                .get(self.player, kind)
                .harvest_capacity
                .is_some()
        };

        if !has(&is_refinery) {
            let wanted = self.cheapest(sim, is_refinery)?;
            return (!self.already_coming(sim, wanted)).then_some(wanted);
        }
        // Counted across kinds and including the queue: two different miners
        // are still two miners, and one being built is one on its way.
        let miners: u32 = sim
            .rules()
            .entities()
            .filter(|(kind, _)| is_harvester(sim, *kind))
            .map(|(kind, _)| sim.count_of(self.player, kind) as u32)
            .sum();
        if miners < self.difficulty.harvesters_wanted() {
            return self.cheapest(sim, is_harvester);
        }
        None
    }

    /// Somewhere to make units from, if it has none.
    fn wants_production(&self, sim: &Sim) -> Option<EntityKind> {
        for category in ["infantry", "vehicle"] {
            let makes_them = sim
                .view()
                .units()
                .filter(|(_, u)| u.owner == self.player && u.is_alive())
                .any(|(_, u)| {
                    sim.rules().entity(u.kind).traits.iter().any(|t| {
                        matches!(t, Trait::Produces { categories } if categories
                            .iter()
                            .any(|c| c == category))
                    })
                });
            if makes_them {
                continue;
            }
            let wanted = self.cheapest(sim, |sim, kind| {
                sim.rules().entity(kind).traits.iter().any(|t| {
                    matches!(t, Trait::Produces { categories } if categories
                        .iter()
                        .any(|c| c == category))
                })
            });
            if let Some(wanted) = wanted
                && !self.already_coming(sim, wanted)
            {
                return Some(wanted);
            }
        }
        None
    }

    /// Something that shoots.
    ///
    /// The cheapest armed thing it can build, endlessly. A dummy has no plan
    /// for its army beyond having one — which is exactly what it is for.
    fn wants_army(&self, sim: &Sim) -> Option<EntityKind> {
        self.cheapest(sim, |sim, kind| {
            let stats = sim.stats().get(PlayerId(0), kind);
            stats.mobile && sim.combat().weapon(kind).is_some()
        })
    }

    /// The cheapest thing it could build right now that passes a test.
    ///
    /// Ties break on the lower entity index, which comes free from iterating
    /// the rules in order — and has to break *somehow*, or two equally cheap
    /// units would be chosen by whatever the iterator happened to yield first.
    fn cheapest(&self, sim: &Sim, wanted: impl Fn(&Sim, EntityKind) -> bool) -> Option<EntityKind> {
        let mut best: Option<(EntityKind, u32)> = None;
        for (kind, _) in sim.rules().entities() {
            let stats = sim.stats().get(self.player, kind);
            if stats.cost == 0 || !wanted(sim, kind) {
                continue;
            }
            if sim.producer_for(self.player, kind).is_none() {
                continue;
            }
            if !sim.prerequisites_met(self.player, kind)
                || !sim.within_build_limit(self.player, kind)
            {
                continue;
            }
            if best.is_none_or(|(_, cost)| stats.cost < cost) {
                best = Some((kind, stats.cost));
            }
        }
        best.map(|(kind, _)| kind)
    }
}
