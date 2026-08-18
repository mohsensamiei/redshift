//! Drives the simulation from the render loop.
//!
//! This is the seam between two clocks. The simulation advances in fixed 20 Hz
//! steps; the window redraws at 60 Hz. A frame never runs a partial tick — if a
//! frame takes too long we run more ticks, never a fraction of one.
//!
//! # Single-player is a one-peer match
//!
//! Player input becomes a [`Command`] and is queued, exactly as it would be in
//! a network match. There is no path by which input reaches the simulation
//! directly. The only thing multiplayer will change here is where the commands
//! come from — which means the lockstep path is exercised on every frame of
//! development rather than integrated at the end.

use bevy::prelude::*;
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::sim::{MatchSetup, Sim};
use redshift_sim::{EntityId, TICK_MS, Tick, WorldPos};

use std::collections::BTreeMap;

/// How far behind real time the simulation may fall before it stops trying to
/// catch up.
///
/// Without a cap, a long stall — a breakpoint, a laptop lid closing — would be
/// followed by hundreds of ticks in one frame, which looks like the game
/// fast-forwarding and can cascade into a longer stall. Dropping the backlog is
/// the right call in single-player. In a network match the peers would instead
/// wait for each other, so this ceiling will move into the net layer.
const MAX_CATCHUP_TICKS: u32 = 5;

/// The running match.
#[derive(Resource)]
pub struct Session {
    sim: Sim,
    /// Real time carried over between frames, in milliseconds.
    accumulator: f32,
    /// Commands queued for the next tick.
    pending: Vec<Command>,
    /// Per-player command counter, reset each tick, giving every command a
    /// unique place in the total order.
    next_sequence: u16,
    /// The player this client controls.
    pub local_player: PlayerId,
    /// Unit positions as of the previous tick, for interpolation.
    previous: BTreeMap<EntityId, WorldPos>,
    /// Wall-clock cost of the last tick, in milliseconds. Diagnostics only —
    /// nothing in the simulation may read this.
    pub last_tick_ms: f32,
    pub ticks_this_frame: u32,
    /// How far through the current tick we are, in `[0, 1)`.
    pub interpolation: f32,
    pub paused: bool,
}

impl Session {
    pub fn new(setup: MatchSetup, local_player: PlayerId) -> Session {
        let sim = Sim::new(setup);
        let previous = sim.units().iter().map(|(id, u)| (id, u.pos)).collect();
        Session {
            sim,
            accumulator: 0.0,
            pending: Vec::new(),
            next_sequence: 0,
            local_player,
            previous,
            last_tick_ms: 0.0,
            ticks_this_frame: 0,
            interpolation: 0.0,
            paused: false,
        }
    }

    /// Read-only access to the world. The renderer gets nothing else.
    pub fn sim(&self) -> &Sim {
        &self.sim
    }

    pub fn tick_number(&self) -> Tick {
        self.sim.tick_number()
    }

    /// Where a unit was at the end of the previous tick.
    ///
    /// Falls back to its current position for a unit that has just appeared, so
    /// a new unit does not visibly slide in from wherever the last occupant of
    /// its slot happened to be.
    pub fn previous_pos(&self, id: EntityId, current: WorldPos) -> WorldPos {
        self.previous.get(&id).copied().unwrap_or(current)
    }

    /// Queues a command from the local player.
    pub fn issue(&mut self, kind: CommandKind) {
        self.pending
            .push(Command::new(self.local_player, self.next_sequence, kind));
        self.next_sequence = self.next_sequence.wrapping_add(1);
    }

    /// Advances the simulation to keep pace with real time.
    pub fn advance(&mut self, delta_seconds: f32) {
        self.ticks_this_frame = 0;
        if self.paused {
            self.interpolation = 0.0;
            return;
        }

        self.accumulator += delta_seconds * 1000.0;
        let tick_ms = TICK_MS as f32;

        let mut ticks = 0;
        while self.accumulator >= tick_ms && ticks < MAX_CATCHUP_TICKS {
            self.step();
            self.accumulator -= tick_ms;
            ticks += 1;
        }

        if ticks == MAX_CATCHUP_TICKS {
            // Discard the backlog rather than fast-forwarding through it.
            self.accumulator = 0.0;
        }
        self.ticks_this_frame = ticks;
        self.interpolation = (self.accumulator / tick_ms).clamp(0.0, 1.0);
    }

    fn step(&mut self) {
        // Snapshot before the tick, so interpolation has both endpoints.
        self.previous.clear();
        self.previous
            .extend(self.sim.units().iter().map(|(id, u)| (id, u.pos)));

        let mut commands = std::mem::take(&mut self.pending);
        // The simulation requires a total order. In a network match the net
        // layer sorts; here there is one player, but sorting anyway keeps the
        // two paths identical.
        commands.sort_by_key(|c| c.order_key());

        let started = std::time::Instant::now();
        self.sim.tick(&commands);
        self.last_tick_ms = started.elapsed().as_secs_f32() * 1000.0;

        self.next_sequence = 0;
    }
}

/// Advances the session once per frame.
pub fn advance_session(mut session: ResMut<Session>, time: Res<Time>) {
    let delta = time.delta_secs();
    session.advance(delta);
}
