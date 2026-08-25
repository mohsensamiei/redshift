//! The engine's handle on a running match.
//!
//! Thin by design. [`redshift_net::MatchSession`] owns the simulation, the
//! scheduler and the socket; this adds only what the renderer needs and the
//! simulation must never see: the previous tick's positions, for interpolation.
//!
//! Single-player and multiplayer are the same object here. That is deliberate —
//! see `redshift_net::session`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use bevy::prelude::*;
use redshift_net::MatchSession;
use redshift_net::lockstep::DesyncReport;
use redshift_sim::command::{CommandKind, PlayerId};
use redshift_sim::{EntityId, Tick, WorldPos};

/// Where desync dumps and replays are written.
fn diagnostics_dir() -> PathBuf {
    PathBuf::from("dumps")
}

/// The running match, as the renderer sees it.
#[derive(Resource)]
pub struct Session {
    inner: MatchSession,
    /// Unit positions as of the previous tick.
    previous: BTreeMap<EntityId, WorldPos>,
    /// Players whose commands we are waiting on, if any.
    pub waiting_on: Vec<PlayerId>,
    pub show_stall_notice: bool,
    /// The most recent tick a peer independently confirmed, with the hash both
    /// sides agreed on.
    pub last_verified: Option<(Tick, u64)>,
    pub comparisons_made: u32,
    pub ticks_this_frame: u32,
    /// Set once, when a divergence halts the match.
    pub halted: Option<DesyncReport>,
    /// Where the dump was written, for showing the player.
    pub dump_path: Option<PathBuf>,
}

impl Session {
    pub fn new(inner: MatchSession) -> Session {
        let previous = inner
            .sim()
            .units()
            .iter()
            .map(|(id, u)| (id, u.pos))
            .collect();
        Session {
            inner,
            previous,
            waiting_on: Vec::new(),
            show_stall_notice: false,
            last_verified: None,
            comparisons_made: 0,
            ticks_this_frame: 0,
            halted: None,
            dump_path: None,
        }
    }

    pub fn sim(&self) -> &redshift_sim::sim::Sim {
        self.inner.sim()
    }

    pub fn local_player(&self) -> PlayerId {
        self.inner.local_player()
    }

    pub fn tick_number(&self) -> Tick {
        self.inner.tick_number()
    }

    pub fn interpolation(&self) -> f32 {
        self.inner.interpolation
    }

    pub fn last_tick_ms(&self) -> f32 {
        self.inner.last_tick_ms
    }

    pub fn is_networked(&self) -> bool {
        self.inner.is_networked()
    }

    pub fn peer_count(&self) -> usize {
        self.inner.peer_count()
    }

    pub fn input_delay(&self) -> u32 {
        self.inner.input_delay()
    }

    pub fn paused(&self) -> bool {
        self.inner.paused
    }

    pub fn toggle_pause(&mut self) {
        // Pausing a network match would stall every other player, so it is a
        // single-player convenience only.
        if !self.inner.is_networked() {
            self.inner.paused = !self.inner.paused;
        }
    }

    /// Where a unit was at the end of the previous tick.
    ///
    /// Falls back to its current position for a unit that has just appeared, so
    /// a new unit does not visibly slide in from wherever the previous occupant
    /// of its slot happened to be.
    pub fn previous_pos(&self, id: EntityId, current: WorldPos) -> WorldPos {
        self.previous.get(&id).copied().unwrap_or(current)
    }

    /// Queues a command from the local player.
    pub fn issue(&mut self, kind: CommandKind) {
        if self.halted.is_none() {
            self.inner.issue(kind);
        }
    }

    /// Issues a command for a player this peer speaks for — a computer
    /// opponent.
    pub fn issue_for(&mut self, player: PlayerId, kind: CommandKind) {
        if self.halted.is_none() {
            self.inner.issue_for(player, kind);
        }
    }

    /// Saves the replay of the match so far.
    pub fn save_replay(&self) -> std::io::Result<PathBuf> {
        let path = diagnostics_dir().join(format!("match-tick{}.replay.ron", self.tick_number()));
        self.inner.save_replay(&path)?;
        Ok(path)
    }
}

/// Advances the match once per frame.
pub fn advance_session(mut session: ResMut<Session>, time: Res<Time>) {
    let delta = time.delta_secs();

    // Snapshot before advancing, so interpolation has both endpoints. Taken
    // even on a frame that runs no ticks, which is most of them — the snapshot
    // must be the state the last tick ended in.
    let will_tick = session.inner.interpolation >= 0.999 || delta > 0.0;
    if will_tick {
        let snapshot: BTreeMap<EntityId, WorldPos> = session
            .inner
            .sim()
            .units()
            .iter()
            .map(|(id, u)| (id, u.pos))
            .collect();

        let outcome = session.inner.update(delta);

        if outcome.ticks_run > 0 {
            session.previous = snapshot;
        }
        session.ticks_this_frame = outcome.ticks_run;
        session.show_stall_notice = outcome.should_show_stall_notice();
        session.waiting_on = outcome.waiting_on.clone();
        session.last_verified = outcome.last_verified;
        session.comparisons_made = outcome.comparisons_made;

        if let Some(report) = outcome.desync
            && session.halted.is_none()
        {
            // Tell the peer, write the dump, and stop. A divergence cannot be
            // recovered from — the two worlds have already parted — so the only
            // useful thing left is to preserve the evidence.
            error!(
                "desync at tick {}: local {:#x}, peer {:#x}",
                report.tick, report.local_hash, report.remote_hash
            );
            session.inner.announce_desync(&report);
            match session.inner.write_desync_dump(&diagnostics_dir(), &report) {
                Ok(path) => {
                    info!("desync dump written to {}", path.display());
                    session.dump_path = Some(path);
                }
                Err(e) => error!("could not write the desync dump: {e}"),
            }
            session.halted = Some(report);
        }
    }
}
