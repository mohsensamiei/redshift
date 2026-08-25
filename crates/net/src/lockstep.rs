//! Turn scheduling: the rule that decides when a tick may run.
//!
//! # The rule
//!
//! A peer may execute tick `N` only once it holds **every** player's commands
//! for tick `N`. If one is missing, the simulation waits. It never guesses,
//! extrapolates, or runs ahead — a wrong guess is a desync, and a desync ends
//! the match.
//!
//! Commands issued at tick `N` execute at tick `N + D`, where `D` is the input
//! delay: enough ticks for a packet to reach every peer. That delay is what
//! makes waiting rare in practice.
//!
//! # Why input delay is not felt
//!
//! The *unit* acts `D` ticks late, but the *interface* responds immediately —
//! the click sound plays and the move marker appears on the frame of the click.
//! This is what the original did, and it is why 200 ms of input delay is
//! invisible in a strategy game and unplayable in a shooter.
//!
//! This module is deliberately transport-free. It decides *what may run when*,
//! and is driven by whatever moves bytes — a UDP socket, a relay, a replay
//! file, or a test harness. That is what lets the whole scheduling rule be
//! tested without opening a socket.

use std::collections::BTreeMap;

use redshift_sim::Tick;
use redshift_sim::command::{Command, PlayerId};

/// Input delay bounds, in ticks. At 20 Hz each tick is 50 ms.
pub const MIN_INPUT_DELAY: u32 = 2;
pub const MAX_INPUT_DELAY: u32 = 10;

/// Ticks between state hash comparisons — once per second at 20 Hz.
///
/// Frequent enough that a divergence is caught while its cause is still
/// findable, rare enough to cost nothing.
pub const HASH_INTERVAL: u32 = 20;

/// How many executed ticks' hashes to retain for comparison.
///
/// A peer's hash for tick `N` arrives after it has moved past `N`, so both
/// sides must keep a window of history. Sized well beyond the maximum input
/// delay so a slow peer's report still finds its counterpart.
const HASH_HISTORY: usize = 64;

/// Chooses an input delay from a measured round trip.
///
/// Rounds up: too much delay costs a little responsiveness, too little costs
/// a stall on every packet that arrives late. The asymmetry is not close.
pub fn input_delay_for_rtt(rtt_ms: u32, tick_ms: u32) -> u32 {
    // Half the round trip is the one-way time; a full round trip of headroom on
    // top absorbs ordinary jitter.
    let needed_ms = rtt_ms + rtt_ms / 2;
    let ticks = needed_ms.div_ceil(tick_ms.max(1));
    ticks.clamp(MIN_INPUT_DELAY, MAX_INPUT_DELAY)
}

/// What the scheduler wants the caller to do next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnStatus {
    /// Every player's commands are in. Run the tick with these, in this order.
    Ready(Vec<Command>),
    /// Waiting on the listed players. Do not advance.
    Waiting { tick: Tick, missing: Vec<PlayerId> },
    /// A hash mismatch was found. The match is over; do not continue.
    Desynced(DesyncReport),
}

/// Everything known about a divergence at the moment it was detected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesyncReport {
    pub tick: Tick,
    /// This peer's hash.
    pub local_hash: u64,
    /// The peer that disagreed, and what it had.
    pub remote_player: PlayerId,
    pub remote_hash: u64,
}

/// Collects commands and decides when ticks may run.
#[derive(Debug)]
pub struct TurnScheduler {
    local_player: PlayerId,
    players: Vec<PlayerId>,
    input_delay: u32,
    /// The next tick to execute.
    next_tick: Tick,
    /// Commands received, by tick then by player. `BTreeMap` rather than a hash
    /// map because this is walked in order and the order must be identical on
    /// every peer.
    pending: BTreeMap<Tick, BTreeMap<PlayerId, Vec<Command>>>,
    /// Hashes this peer produced, for comparison against reports that arrive
    /// later.
    local_hashes: BTreeMap<Tick, u64>,
    /// The tick number the next outgoing turn will carry.
    ///
    /// Deliberately independent of `next_tick`. A peer runs several ticks in
    /// one poll whenever a backlog clears, so deriving the outgoing number from
    /// execution progress would skip turn numbers — and a turn nobody ever
    /// sends is a tick nobody can ever run. The match then hangs a few ticks in,
    /// which looks like a network fault and is not one.
    ///
    /// Turns are emitted strictly one at a time, in order, with no gaps.
    next_outgoing: Tick,
    /// Local commands not yet scheduled.
    outbox: Vec<Command>,
    next_sequence: u16,
    desync: Option<DesyncReport>,
    /// Consecutive polls spent waiting.
    stalled_polls: u32,
    /// The most recent tick at which a peer's hash matched our own.
    ///
    /// The only positive evidence of agreement there is. "No desync reported"
    /// is not the same thing — it is equally consistent with no hashes having
    /// been compared at all, which is exactly what a silently broken checkpoint
    /// path looks like.
    last_verified: Option<(Tick, u64)>,
    /// Hash comparisons that agreed.
    pub comparisons_made: u32,
}

impl TurnScheduler {
    pub fn new(local_player: PlayerId, mut players: Vec<PlayerId>, input_delay: u32) -> Self {
        players.sort();
        players.dedup();
        assert!(
            players.contains(&local_player),
            "the local player must be in the match"
        );
        let input_delay = input_delay.clamp(MIN_INPUT_DELAY, MAX_INPUT_DELAY);

        // Seed the opening ticks with empty command sets for everyone.
        //
        // Commands issued at tick N execute at N + D, so nothing anyone does
        // can land before tick D. Without this the first D ticks would sit
        // waiting on commands that, by construction, can never arrive — and the
        // match would hang at the starting line rather than fail visibly.
        //
        // Every peer does this identically at construction, so it costs no
        // agreement.
        let mut pending: BTreeMap<Tick, BTreeMap<PlayerId, Vec<Command>>> = BTreeMap::new();
        for tick in 0..input_delay {
            let slot = pending.entry(tick).or_default();
            for player in &players {
                slot.insert(*player, Vec::new());
            }
        }

        TurnScheduler {
            local_player,
            players,
            input_delay,
            next_tick: 0,
            // Nothing issued now can execute before the input delay has passed,
            // so that is where outgoing turn numbers begin.
            next_outgoing: input_delay,
            pending,
            local_hashes: BTreeMap::new(),
            outbox: Vec::new(),
            next_sequence: 0,
            desync: None,
            stalled_polls: 0,
            last_verified: None,
            comparisons_made: 0,
        }
    }

    pub fn local_player(&self) -> PlayerId {
        self.local_player
    }

    pub fn input_delay(&self) -> u32 {
        self.input_delay
    }

    /// The tick that will run next.
    pub fn next_tick(&self) -> Tick {
        self.next_tick
    }

    /// The tick that locally-issued commands will execute at.
    pub fn scheduled_tick(&self) -> Tick {
        self.next_outgoing
    }

    /// How far the outgoing turn counter has run ahead of execution.
    pub fn turns_in_flight(&self) -> u32 {
        self.next_outgoing.saturating_sub(self.next_tick)
    }

    /// Consecutive polls spent waiting on a peer.
    pub fn stalled_polls(&self) -> u32 {
        self.stalled_polls
    }

    /// Queues a local command for its scheduled tick.
    pub fn issue(&mut self, kind: redshift_sim::command::CommandKind) {
        let player = self.local_player;
        self.issue_for(player, kind);
    }

    /// Issues a command on behalf of a player this peer speaks for.
    ///
    /// A computer opponent is a player like any other: it has an id, its
    /// commands are sequenced and scheduled, and they reach the simulation
    /// through the same queue a human's do. That is not tidiness — a shortcut
    /// that applied them directly would make this peer play a subtly different
    /// game from the one its replay reproduces.
    ///
    /// Sequence numbers are shared across every player this peer speaks for.
    /// They only have to be unique within a player within a tick, and one
    /// counter that always moves forward is easier to be sure of than several.
    pub fn issue_for(&mut self, player: PlayerId, kind: redshift_sim::command::CommandKind) {
        self.outbox
            .push(Command::new(player, self.next_sequence, kind));
        self.next_sequence = self.next_sequence.wrapping_add(1);
    }

    /// Takes the next turn's local commands for transmission, recording them
    /// locally at the same time.
    ///
    /// A peer must schedule its own commands through exactly the same path as
    /// everyone else's. Applying them locally by a shortcut is the classic way
    /// to end up with a host that plays a subtly different game from its
    /// clients.
    ///
    /// Returns `None` when the caller is already a full input delay ahead of
    /// what it has executed. Running further ahead cannot help — the turns
    /// would sit unused — and it would grow without bound behind a peer that
    /// has stopped responding.
    pub fn take_outgoing(&mut self) -> Option<(Tick, Vec<Command>)> {
        if self.turns_in_flight() > self.input_delay {
            return None;
        }
        let tick = self.next_outgoing;
        self.next_outgoing += 1;
        let commands = std::mem::take(&mut self.outbox);
        self.next_sequence = 0;
        self.accept(tick, self.local_player, commands.clone());
        Some((tick, commands))
    }

    /// Records a player's commands for a tick.
    ///
    /// Duplicates are ignored, so the redundant copies every packet carries
    /// cost nothing. Commands for ticks already executed are dropped.
    pub fn accept(&mut self, tick: Tick, player: PlayerId, commands: Vec<Command>) {
        if tick < self.next_tick || !self.players.contains(&player) {
            return;
        }
        self.pending
            .entry(tick)
            .or_default()
            .entry(player)
            .or_insert(commands);
    }

    /// Records a hash this peer computed after executing `tick`.
    pub fn record_local_hash(&mut self, tick: Tick, hash: u64) {
        self.local_hashes.insert(tick, hash);
        // Keep the window bounded. A match runs for hours; the history is only
        // needed for as long as a peer's report might still be in flight.
        while self.local_hashes.len() > HASH_HISTORY {
            let oldest = *self.local_hashes.keys().next().expect("non-empty");
            self.local_hashes.remove(&oldest);
        }
    }

    /// Compares a peer's hash against our own for the same tick.
    ///
    /// A tick we no longer have is not an error — the peer may be far enough
    /// behind that our record has aged out. Only an actual disagreement counts.
    pub fn check_remote_hash(&mut self, player: PlayerId, tick: Tick, remote_hash: u64) {
        if self.desync.is_some() {
            return;
        }
        let Some(&local_hash) = self.local_hashes.get(&tick) else {
            return;
        };
        if local_hash != remote_hash {
            self.desync = Some(DesyncReport {
                tick,
                local_hash,
                remote_player: player,
                remote_hash,
            });
            return;
        }
        self.comparisons_made += 1;
        if self.last_verified.is_none_or(|(seen, _)| tick > seen) {
            self.last_verified = Some((tick, local_hash));
        }
    }

    /// The most recent tick a peer independently confirmed, and the hash both
    /// sides agreed on.
    ///
    /// The only positive evidence of agreement there is. "No desync reported"
    /// is not the same thing — it is equally consistent with no hashes having
    /// been compared at all, which is exactly what a silently broken checkpoint
    /// path looks like.
    pub fn last_verified(&self) -> Option<(Tick, u64)> {
        self.last_verified
    }

    /// Whether this tick should be hash-checked.
    pub fn should_hash(tick: Tick) -> bool {
        tick.is_multiple_of(HASH_INTERVAL)
    }

    /// The divergence, if one was found.
    pub fn desync(&self) -> Option<&DesyncReport> {
        self.desync.as_ref()
    }

    /// Asks whether the next tick may run.
    ///
    /// On [`TurnStatus::Ready`] the scheduler has advanced: the returned
    /// commands belong to the tick the caller must now execute.
    pub fn poll(&mut self) -> TurnStatus {
        if let Some(report) = &self.desync {
            return TurnStatus::Desynced(report.clone());
        }

        let tick = self.next_tick;
        let received = self.pending.get(&tick);
        let missing: Vec<PlayerId> = self
            .players
            .iter()
            .copied()
            .filter(|p| received.is_none_or(|r| !r.contains_key(p)))
            .collect();

        if !missing.is_empty() {
            self.stalled_polls += 1;
            return TurnStatus::Waiting { tick, missing };
        }

        self.stalled_polls = 0;
        let mut commands: Vec<Command> = self
            .pending
            .remove(&tick)
            .map(|by_player| by_player.into_values().flatten().collect())
            .unwrap_or_default();

        // The simulation requires a total order. `BTreeMap` already gave us
        // player order; this makes the sequence ordering explicit rather than
        // relying on insertion order within a player's vector.
        commands.sort_by_key(|c| c.order_key());

        self.next_tick += 1;
        TurnStatus::Ready(commands)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redshift_sim::command::CommandKind;
    use redshift_sim::map::Cell;

    fn move_to(x: i32, y: i32) -> CommandKind {
        CommandKind::Move {
            units: Vec::new(),
            target: Cell::new(x, y),
        }
    }

    const DELAY: u32 = 2;

    fn two_player() -> TurnScheduler {
        TurnScheduler::new(PlayerId(0), vec![PlayerId(0), PlayerId(1)], DELAY)
    }

    /// Runs the pre-seeded opening ticks, leaving the scheduler at the first
    /// tick that genuinely needs commands from every player.
    fn past_opening(s: &mut TurnScheduler) {
        for _ in 0..s.input_delay() {
            assert!(
                matches!(s.poll(), TurnStatus::Ready(_)),
                "opening ticks should not stall"
            );
        }
        assert_eq!(s.next_tick(), DELAY);
    }

    #[test]
    fn a_tick_waits_for_every_player() {
        let mut s = two_player();
        past_opening(&mut s);
        assert!(matches!(s.poll(), TurnStatus::Waiting { tick: DELAY, .. }));

        s.accept(DELAY, PlayerId(0), Vec::new());
        match s.poll() {
            TurnStatus::Waiting {
                tick: DELAY,
                missing,
            } => assert_eq!(missing, vec![PlayerId(1)]),
            other => panic!("expected to still be waiting, got {other:?}"),
        }

        s.accept(DELAY, PlayerId(1), Vec::new());
        assert!(matches!(s.poll(), TurnStatus::Ready(_)));
        assert_eq!(
            s.next_tick(),
            DELAY + 1,
            "a ready poll advances the scheduler"
        );
    }

    #[test]
    fn the_simulation_never_runs_ahead_of_a_missing_peer() {
        // The property the whole model rests on. However far ahead one peer's
        // commands arrive, the tick will not run until the other's do.
        let mut s = two_player();
        past_opening(&mut s);
        for tick in DELAY..50 {
            s.accept(tick, PlayerId(0), Vec::new());
        }
        for _ in 0..10 {
            assert!(matches!(s.poll(), TurnStatus::Waiting { tick: DELAY, .. }));
        }
        assert_eq!(s.next_tick(), DELAY);
    }

    #[test]
    fn stall_counter_tracks_consecutive_waits() {
        // Drives the "waiting for player" indicator without needing a clock.
        let mut s = two_player();
        past_opening(&mut s);
        for expected in 1..=5 {
            s.poll();
            assert_eq!(s.stalled_polls(), expected);
        }
        s.accept(DELAY, PlayerId(0), Vec::new());
        s.accept(DELAY, PlayerId(1), Vec::new());
        s.poll();
        assert_eq!(s.stalled_polls(), 0, "a successful tick clears the stall");
    }

    #[test]
    fn local_commands_go_through_the_same_queue_as_everyone_elses() {
        // A host that applied its own commands by a shortcut would play a
        // subtly different game from its clients.
        let mut s = two_player();
        s.issue(move_to(5, 5));
        let (tick, sent) = s
            .take_outgoing()
            .expect("the first turn is always available");

        assert_eq!(
            tick, 2,
            "input delay of 2 means the command lands at tick 2"
        );
        assert_eq!(sent.len(), 1);

        // Ticks 0 and 1 carry nothing from us, and must still run.
        for t in 0..2 {
            s.accept(t, PlayerId(0), Vec::new());
            s.accept(t, PlayerId(1), Vec::new());
            assert!(
                matches!(s.poll(), TurnStatus::Ready(c) if c.is_empty()),
                "tick {t}"
            );
        }

        s.accept(2, PlayerId(1), Vec::new());
        match s.poll() {
            TurnStatus::Ready(commands) => {
                assert_eq!(commands.len(), 1, "our own command arrives via the queue");
                assert_eq!(commands[0].player, PlayerId(0));
            }
            other => panic!("expected ready, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_deliveries_are_ignored() {
        // Every packet repeats the last few ticks, so duplicates are the normal
        // case, not an anomaly. Counting a command twice would execute the
        // order twice.
        let mut s = two_player();
        past_opening(&mut s);
        let cmd = Command::new(PlayerId(1), 0, move_to(9, 9));
        for _ in 0..5 {
            s.accept(DELAY, PlayerId(1), vec![cmd.clone()]);
        }
        s.accept(DELAY, PlayerId(0), Vec::new());
        match s.poll() {
            TurnStatus::Ready(commands) => assert_eq!(commands.len(), 1),
            other => panic!("expected ready, got {other:?}"),
        }
    }

    #[test]
    fn late_commands_for_executed_ticks_are_dropped() {
        // A redundant copy of a tick we already ran must not resurrect it.
        let mut s = two_player();
        past_opening(&mut s);
        s.accept(DELAY, PlayerId(0), Vec::new());
        s.accept(DELAY, PlayerId(1), Vec::new());
        assert!(matches!(s.poll(), TurnStatus::Ready(_)));

        s.accept(
            DELAY,
            PlayerId(1),
            vec![Command::new(PlayerId(1), 0, move_to(1, 1))],
        );
        assert!(matches!(s.poll(), TurnStatus::Waiting { tick: 3, .. }));
    }

    #[test]
    fn commands_from_unknown_players_are_refused() {
        let mut s = two_player();
        past_opening(&mut s);
        s.accept(
            DELAY,
            PlayerId(9),
            vec![Command::new(PlayerId(9), 0, move_to(1, 1))],
        );
        match s.poll() {
            TurnStatus::Waiting { missing, .. } => {
                assert_eq!(missing, vec![PlayerId(0), PlayerId(1)]);
            }
            other => panic!("expected waiting, got {other:?}"),
        }
    }

    #[test]
    fn ready_commands_are_in_total_order() {
        // Two peers assembling the same tick must produce byte-identical
        // command lists, whatever order the packets arrived in.
        let build = |reverse: bool| {
            let mut s = TurnScheduler::new(PlayerId(0), vec![PlayerId(0), PlayerId(1)], DELAY);
            past_opening(&mut s);
            let p0 = vec![
                Command::new(PlayerId(0), 1, move_to(1, 1)),
                Command::new(PlayerId(0), 0, move_to(2, 2)),
            ];
            let p1 = vec![Command::new(PlayerId(1), 0, move_to(3, 3))];
            if reverse {
                s.accept(DELAY, PlayerId(1), p1);
                s.accept(DELAY, PlayerId(0), p0);
            } else {
                s.accept(DELAY, PlayerId(0), p0);
                s.accept(DELAY, PlayerId(1), p1);
            }
            match s.poll() {
                TurnStatus::Ready(c) => c,
                other => panic!("expected ready, got {other:?}"),
            }
        };
        let forwards = build(false);
        assert_eq!(
            forwards,
            build(true),
            "arrival order must not affect the result"
        );
        let keys: Vec<_> = forwards.iter().map(|c| c.order_key()).collect();
        assert_eq!(keys, [(0, 0), (0, 1), (1, 0)]);
    }

    #[test]
    fn matching_hashes_are_not_a_desync() {
        let mut s = two_player();
        s.record_local_hash(20, 0xABCD);
        s.check_remote_hash(PlayerId(1), 20, 0xABCD);
        assert!(s.desync().is_none());
    }

    #[test]
    fn a_mismatched_hash_halts_the_match() {
        let mut s = two_player();
        s.record_local_hash(20, 0xABCD);
        s.check_remote_hash(PlayerId(1), 20, 0x1234);

        let report = s.desync().expect("divergence should be reported").clone();
        assert_eq!(report.tick, 20);
        assert_eq!(report.local_hash, 0xABCD);
        assert_eq!(report.remote_hash, 0x1234);
        assert_eq!(report.remote_player, PlayerId(1));

        // And the scheduler must refuse to go on, even with everything present.
        s.accept(0, PlayerId(0), Vec::new());
        s.accept(0, PlayerId(1), Vec::new());
        assert!(matches!(s.poll(), TurnStatus::Desynced(_)));
    }

    #[test]
    fn a_hash_for_an_unknown_tick_is_not_a_desync() {
        // A peer far enough behind reports a tick we have already forgotten.
        // That is ignorance, not disagreement.
        let mut s = two_player();
        s.check_remote_hash(PlayerId(1), 999, 0xFFFF);
        assert!(s.desync().is_none());
    }

    #[test]
    fn hash_history_stays_bounded() {
        let mut s = two_player();
        for tick in 0..10_000 {
            s.record_local_hash(tick, tick as u64);
        }
        assert!(
            s.local_hashes.len() <= HASH_HISTORY,
            "history grew to {}",
            s.local_hashes.len()
        );
        // The most recent entries are the ones kept.
        assert!(s.local_hashes.contains_key(&9_999));
    }

    #[test]
    fn hash_checkpoints_land_once_a_second() {
        let checked: Vec<Tick> = (0..60).filter(|t| TurnScheduler::should_hash(*t)).collect();
        assert_eq!(checked, vec![0, 20, 40]);
    }

    #[test]
    fn input_delay_scales_with_latency_and_stays_in_bounds() {
        // LAN: the minimum is already more than enough.
        assert_eq!(input_delay_for_rtt(2, 50), MIN_INPUT_DELAY);
        assert_eq!(input_delay_for_rtt(20, 50), MIN_INPUT_DELAY);
        // A 60 ms round trip needs 90 ms of cover, which two 50 ms ticks
        // provide — so the minimum still holds.
        assert_eq!(input_delay_for_rtt(60, 50), MIN_INPUT_DELAY);
        // Beyond that the delay has to grow.
        assert!(input_delay_for_rtt(150, 50) > MIN_INPUT_DELAY);
        assert!(input_delay_for_rtt(250, 50) > input_delay_for_rtt(150, 50));
        // Intercontinental, and then absurd — both clamp.
        assert!(input_delay_for_rtt(300, 50) <= MAX_INPUT_DELAY);
        assert_eq!(input_delay_for_rtt(100_000, 50), MAX_INPUT_DELAY);
    }

    #[test]
    fn input_delay_rounds_up_never_down() {
        // Too much delay costs a little responsiveness; too little costs a
        // stall on every late packet. The asymmetry is not close.
        for rtt in 1..200u32 {
            let ticks = input_delay_for_rtt(rtt, 50);
            let covered_ms = ticks * 50;
            let needed_ms = rtt + rtt / 2;
            assert!(
                covered_ms >= needed_ms.min(MAX_INPUT_DELAY * 50),
                "rtt {rtt}ms got {ticks} ticks, covering only {covered_ms}ms of {needed_ms}ms"
            );
        }
    }

    #[test]
    fn outgoing_turn_numbers_never_skip() {
        // The bug this pins: deriving the outgoing turn number from execution
        // progress skips numbers whenever a backlog clears and several ticks
        // run at once. A turn nobody sends is a tick nobody can run, so the
        // match hangs a few ticks in — looking for all the world like a network
        // fault.
        let mut s = two_player();
        let mut emitted = Vec::new();
        for step in 0..200u32 {
            if let Some((tick, _)) = s.take_outgoing() {
                emitted.push(tick);
            }
            // Feed the other player irregularly, so this peer runs in bursts.
            if step.is_multiple_of(3) {
                for t in 0..=step {
                    s.accept(t, PlayerId(1), Vec::new());
                }
            }
            while let TurnStatus::Ready(_) = s.poll() {}
        }

        assert!(
            emitted.len() > 50,
            "only {} turns were emitted",
            emitted.len()
        );
        assert_eq!(
            emitted[0], DELAY,
            "the first turn lands one input delay out"
        );
        for pair in emitted.windows(2) {
            assert_eq!(
                pair[1],
                pair[0] + 1,
                "turn numbers jumped: {} -> {}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn a_stalled_peer_does_not_run_away() {
        // Without a cap, a peer whose partner has gone silent would keep
        // producing turns for ticks that can never run, growing without bound.
        let mut s = two_player();
        for _ in 0..10_000 {
            s.take_outgoing();
            while let TurnStatus::Ready(_) = s.poll() {}
        }
        assert!(
            s.turns_in_flight() <= s.input_delay() + 1,
            "ran {} turns ahead of execution",
            s.turns_in_flight()
        );
    }

    #[test]
    fn the_opening_ticks_run_without_waiting_on_anyone() {
        // The bug this pins: nothing anyone does can land before tick D, so
        // without pre-seeded empty turns the first D ticks wait forever on
        // commands that cannot exist. The match hangs at the starting line —
        // and it looks like a network fault rather than a scheduling one.
        for delay in MIN_INPUT_DELAY..=MAX_INPUT_DELAY {
            let mut s = TurnScheduler::new(PlayerId(0), vec![PlayerId(0), PlayerId(1)], delay);
            for tick in 0..delay {
                match s.poll() {
                    TurnStatus::Ready(commands) => assert!(commands.is_empty()),
                    other => panic!("delay {delay} stalled at opening tick {tick}: {other:?}"),
                }
            }
            // From tick D onwards, real commands are required again.
            assert!(
                matches!(s.poll(), TurnStatus::Waiting { .. }),
                "delay {delay}: tick {delay} should wait for real commands"
            );
        }
    }

    #[test]
    fn a_single_player_match_needs_no_waiting() {
        // Single-player runs the identical path with one peer, so the
        // multiplayer code is exercised constantly rather than bolted on later.
        let mut s = TurnScheduler::new(PlayerId(0), vec![PlayerId(0)], MIN_INPUT_DELAY);
        let mut executed = 0;
        for step in 0..100u32 {
            if let Some((scheduled, _)) = s.take_outgoing() {
                assert_eq!(
                    scheduled,
                    step + MIN_INPUT_DELAY,
                    "turn numbers must not skip"
                );
            }
            while let TurnStatus::Ready(_) = s.poll() {
                executed += 1;
            }
        }
        assert!(
            executed >= 100,
            "single player only reached tick {executed}"
        );
    }
}
