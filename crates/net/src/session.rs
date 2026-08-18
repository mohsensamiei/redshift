//! A running match: simulation, scheduler and socket, driven as one thing.
//!
//! This is what the client actually holds. It hides the distinction the rest of
//! the game should not care about: **single-player is a match with one peer.**
//! The same scheduler, the same command queue, the same replay recording. Only
//! the transport differs, and only by being absent.
//!
//! That is not tidiness for its own sake. It means the multiplayer path is
//! exercised on every frame of single-player development, rather than being
//! integrated at the end and discovered to be wrong.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::sim::{MatchSetup, Sim};
use redshift_sim::{TICK_MS, Tick};

use crate::lockstep::{DesyncReport, TurnScheduler, TurnStatus};
use crate::protocol::{Packet, REDUNDANT_TICKS, StateCheck, TurnCommands};
use crate::replay::Replay;
use crate::transport::Transport;

/// Ticks the session will run in a single update.
///
/// Without a ceiling, a long stall — a breakpoint, a laptop lid closing — is
/// followed by hundreds of ticks in one frame, which looks like the game
/// fast-forwarding and can cascade into a longer stall. Catching up gradually
/// is better than catching up all at once.
const MAX_TICKS_PER_UPDATE: u32 = 8;

/// How long the match must be stalled before the player is told.
///
/// Measured in real time, not in polls. Polling happens once per *frame*, so a
/// poll count means a different duration on every machine — on a 120 Hz display
/// a ten-poll threshold fires after 83 ms, which makes the notice flicker on
/// every scrap of ordinary jitter.
///
/// Half a second is long enough to be a real pause and short enough to explain
/// one the player has already noticed.
pub const STALL_NOTICE_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

/// Where the commands come from.
enum Link {
    /// One peer, no socket. Everything else is identical.
    Solo,
    Networked {
        transport: Transport,
        peers: Vec<SocketAddr>,
        /// Recent turns, repeated in every packet so a single loss costs
        /// nothing.
        history: VecDeque<TurnCommands>,
        /// A hash captured when its tick executed, waiting to be sent.
        ///
        /// Captured at execution rather than recomputed at send time: a peer
        /// can run several ticks between packets, so asking "is the current
        /// tick a checkpoint?" when composing would skip checkpoints — and a
        /// checkpoint never sent is never compared.
        pending_hash: Option<StateCheck>,
    },
}

/// What happened during an update.
#[derive(Clone, Debug, Default)]
pub struct SessionUpdate {
    pub ticks_run: u32,
    /// Players whose commands the session is waiting on.
    pub waiting_on: Vec<PlayerId>,
    /// How long the match has been waiting.
    pub stalled_for: std::time::Duration,
    /// The most recent tick a peer confirmed, and the hash both sides agreed on.
    pub last_verified: Option<(Tick, u64)>,
    /// Hash comparisons that agreed, over the match.
    pub comparisons_made: u32,
    pub desync: Option<DesyncReport>,
}

impl SessionUpdate {
    /// Whether the player should be told the match is stalled.
    pub fn should_show_stall_notice(&self) -> bool {
        !self.waiting_on.is_empty() && self.stalled_for >= STALL_NOTICE_DELAY
    }
}

/// A match in progress.
pub struct MatchSession {
    sim: Sim,
    scheduler: TurnScheduler,
    link: Link,
    replay: Replay,
    /// Real time carried between updates, in milliseconds.
    accumulator_ms: f32,
    /// How far through the current tick we are, in `[0, 1)`. Presentation only.
    pub interpolation: f32,
    /// Wall-clock cost of the last tick. Diagnostics only — nothing in the
    /// simulation may read this.
    pub last_tick_ms: f32,
    pub paused: bool,
    halted: Option<DesyncReport>,
    /// When the current stall began, if the session is stalled.
    stalled_since: Option<std::time::Instant>,
}

impl MatchSession {
    /// A single-player match: one peer, no socket.
    pub fn solo(setup: MatchSetup, local_player: PlayerId) -> MatchSession {
        let seed = setup.seed;
        let sim = Sim::new(setup);
        MatchSession {
            scheduler: TurnScheduler::new(
                local_player,
                vec![local_player],
                crate::lockstep::MIN_INPUT_DELAY,
            ),
            replay: Replay::new(seed, sim.state_hash(), 1),
            sim,
            link: Link::Solo,
            accumulator_ms: 0.0,
            interpolation: 0.0,
            last_tick_ms: 0.0,
            paused: false,
            halted: None,
            stalled_since: None,
        }
    }

    /// A networked match.
    pub fn networked(
        setup: MatchSetup,
        local_player: PlayerId,
        players: Vec<PlayerId>,
        peers: Vec<SocketAddr>,
        input_delay: u32,
        transport: Transport,
    ) -> MatchSession {
        let seed = setup.seed;
        let player_count = players.len() as u8;
        let sim = Sim::new(setup);
        MatchSession {
            scheduler: TurnScheduler::new(local_player, players, input_delay),
            replay: Replay::new(seed, sim.state_hash(), player_count),
            sim,
            link: Link::Networked {
                transport,
                peers,
                history: VecDeque::new(),
                pending_hash: None,
            },
            accumulator_ms: 0.0,
            interpolation: 0.0,
            last_tick_ms: 0.0,
            paused: false,
            halted: None,
            stalled_since: None,
        }
    }

    pub fn sim(&self) -> &Sim {
        &self.sim
    }

    pub fn local_player(&self) -> PlayerId {
        self.scheduler.local_player()
    }

    pub fn tick_number(&self) -> Tick {
        self.sim.tick_number()
    }

    pub fn input_delay(&self) -> u32 {
        self.scheduler.input_delay()
    }

    pub fn is_networked(&self) -> bool {
        matches!(self.link, Link::Networked { .. })
    }

    pub fn peer_count(&self) -> usize {
        match &self.link {
            Link::Solo => 0,
            Link::Networked { peers, .. } => peers.len(),
        }
    }

    /// The divergence that halted the match, if one did.
    pub fn halted(&self) -> Option<&DesyncReport> {
        self.halted.as_ref()
    }

    pub fn replay(&self) -> &Replay {
        &self.replay
    }

    /// Queues a command from the local player.
    ///
    /// It enters the simulation through the scheduler like anyone else's, even
    /// in single-player.
    pub fn issue(&mut self, kind: CommandKind) {
        self.scheduler.issue(kind);
    }

    /// Pumps the network and advances the simulation to keep pace with real
    /// time.
    pub fn update(&mut self, delta_seconds: f32) -> SessionUpdate {
        let mut outcome = SessionUpdate::default();
        if self.halted.is_some() {
            outcome.desync = self.halted.clone();
            return outcome;
        }

        self.receive();

        if self.paused {
            self.interpolation = 0.0;
            return outcome;
        }

        self.accumulator_ms += delta_seconds * 1000.0;
        let tick_ms = TICK_MS as f32;

        while self.accumulator_ms >= tick_ms && outcome.ticks_run < MAX_TICKS_PER_UPDATE {
            // A turn goes out before each tick is attempted, so peers always
            // have what they need to advance.
            self.send_turn();

            match self.scheduler.poll() {
                TurnStatus::Ready(commands) => {
                    let started = std::time::Instant::now();
                    self.sim.tick(&commands);
                    self.last_tick_ms = started.elapsed().as_secs_f32() * 1000.0;

                    self.replay.record(&commands);

                    let executed = self.sim.tick_number() - 1;
                    if TurnScheduler::should_hash(executed) {
                        let hash = self.sim.state_hash();
                        self.scheduler.record_local_hash(executed, hash);
                        if let Link::Networked { pending_hash, .. } = &mut self.link {
                            *pending_hash = Some(StateCheck {
                                tick: executed,
                                hash,
                            });
                        }
                    }

                    self.accumulator_ms -= tick_ms;
                    outcome.ticks_run += 1;
                    self.stalled_since = None;
                }
                TurnStatus::Waiting { missing, .. } => {
                    // Do not consume the accumulator. The tick still owes us
                    // once the commands arrive, and eating the time here would
                    // silently slow the match down for everyone.
                    outcome.waiting_on = missing;
                    let since = self
                        .stalled_since
                        .get_or_insert_with(std::time::Instant::now);
                    outcome.stalled_for = since.elapsed();
                    break;
                }
                TurnStatus::Desynced(report) => {
                    self.halted = Some(report.clone());
                    outcome.desync = Some(report);
                    break;
                }
            }
        }

        // A long stall would otherwise leave a backlog that fast-forwards the
        // moment it clears.
        let ceiling = tick_ms * MAX_TICKS_PER_UPDATE as f32;
        if self.accumulator_ms > ceiling {
            self.accumulator_ms = ceiling;
        }
        self.interpolation = (self.accumulator_ms / tick_ms).clamp(0.0, 1.0);
        outcome.last_verified = self.scheduler.last_verified();
        outcome.comparisons_made = self.scheduler.comparisons_made;
        outcome
    }

    fn send_turn(&mut self) {
        let outgoing = self.scheduler.take_outgoing();
        let scheduled = self.scheduler.scheduled_tick();
        let local = self.scheduler.local_player();

        let Link::Networked {
            transport,
            peers,
            history,
            pending_hash,
        } = &mut self.link
        else {
            return;
        };

        if let Some((tick, commands)) = outgoing {
            history.push_back(TurnCommands {
                tick,
                player: local,
                commands,
            });
            while history.len() > REDUNDANT_TICKS {
                history.pop_front();
            }
        }

        // A packet still goes out when the scheduler is throttled: it repeats
        // recent turns, which is exactly what a peer waiting on us needs.
        if history.is_empty() {
            return;
        }
        let packet = Packet::Turn {
            tick: scheduled,
            turns: history.iter().cloned().collect(),
            hash: pending_hash.take(),
        };
        let peers: Vec<SocketAddr> = peers.clone();
        transport.broadcast_to(&packet, &peers);
    }

    fn receive(&mut self) {
        let Link::Networked { transport, .. } = &mut self.link else {
            return;
        };
        let incoming = transport.poll(64);
        for (_, packet) in incoming {
            match packet {
                Packet::Turn { turns, hash, .. } => {
                    for turn in turns {
                        self.scheduler.accept(turn.tick, turn.player, turn.commands);
                    }
                    if let Some(check) = hash {
                        // Two-player matches only for now; the sender is
                        // whichever player we are not.
                        let sender = if self.scheduler.local_player() == PlayerId(0) {
                            PlayerId(1)
                        } else {
                            PlayerId(0)
                        };
                        self.scheduler
                            .check_remote_hash(sender, check.tick, check.hash);
                    }
                }
                Packet::DesyncHalt {
                    tick,
                    sender_hash,
                    expected_hash,
                } => {
                    // The peer found the divergence first. Halting on its word
                    // rather than waiting to notice independently stops both
                    // sides at the same tick, which makes the two dumps
                    // comparable.
                    self.halted = Some(DesyncReport {
                        tick,
                        local_hash: expected_hash,
                        remote_player: PlayerId(0),
                        remote_hash: sender_hash,
                    });
                }
                _ => {}
            }
        }
    }

    /// Tells peers the match is over because they have diverged.
    pub fn announce_desync(&mut self, report: &DesyncReport) {
        let packet = Packet::DesyncHalt {
            tick: report.tick,
            sender_hash: report.local_hash,
            expected_hash: report.remote_hash,
        };
        if let Link::Networked {
            transport, peers, ..
        } = &mut self.link
        {
            let peers = peers.clone();
            transport.broadcast_to(&packet, &peers);
        }
    }

    /// Writes everything needed to diagnose a divergence offline.
    ///
    /// Both peers write one. Replaying the two logs side by side and bisecting
    /// to the first differing tick is the whole diagnostic method — which is
    /// why the command log matters more here than the state snapshot.
    pub fn write_desync_dump(
        &self,
        directory: &Path,
        report: &DesyncReport,
    ) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(directory)?;
        let name = format!(
            "desync-tick{}-player{}.ron",
            report.tick,
            self.scheduler.local_player().0
        );
        let path = directory.join(name);

        let dump = DesyncDump {
            tick: report.tick,
            local_player: self.scheduler.local_player(),
            local_hash: report.local_hash,
            remote_player: report.remote_player,
            remote_hash: report.remote_hash,
            input_delay: self.scheduler.input_delay(),
            replay: self.replay.clone(),
            state: self.sim.clone(),
        };
        let text = ron::ser::to_string_pretty(&dump, ron::ser::PrettyConfig::default())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&path, text)?;
        Ok(path)
    }

    /// Saves the replay.
    pub fn save_replay(&self, path: &Path) -> std::io::Result<()> {
        self.replay.save(path)
    }
}

/// Everything one peer knew at the moment it detected a divergence.
#[derive(serde::Serialize, serde::Deserialize)]
struct DesyncDump {
    tick: Tick,
    local_player: PlayerId,
    local_hash: u64,
    remote_player: PlayerId,
    remote_hash: u64,
    input_delay: u32,
    replay: Replay,
    state: Sim,
}

/// Replays a recording, returning the state hash after every tick.
///
/// The offline half of desync diagnosis: run both peers' logs through this and
/// find the first tick where the hashes differ.
pub fn replay_hashes(setup: MatchSetup, replay: &Replay) -> Vec<u64> {
    let mut sim = Sim::new(setup);
    let mut hashes = Vec::with_capacity(replay.turns.len());
    for commands in &replay.turns {
        sim.tick(commands);
        hashes.push(sim.state_hash());
    }
    hashes
}

/// The first tick at which two recordings diverge, if they do.
pub fn first_divergence(a: &[u64], b: &[u64]) -> Option<Tick> {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .map(|i| i as Tick)
        .or_else(|| (a.len() != b.len()).then(|| a.len().min(b.len()) as Tick))
}

/// Convenience for building an outgoing command with no arguments.
pub fn stop_command(units: Vec<redshift_sim::EntityId>) -> CommandKind {
    CommandKind::Stop { units }
}

/// Re-exported so callers do not need the `redshift_sim` command module.
pub use redshift_sim::command::CommandKind as Kind;

/// The type a caller passes to [`MatchSession::issue`].
pub type IssuedCommand = Command;
