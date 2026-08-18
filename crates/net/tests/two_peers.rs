//! Two peers playing a whole match through the lockstep scheduler.
//!
//! This is the test Phase 1 exists to pass. Everything else in the crate is in
//! service of it: two independent simulations, fed only each other's commands
//! over a simulated network, must stay bit-identical for the length of a match.
//!
//! The network here is deliberately hostile — packets are delayed, duplicated,
//! reordered and dropped — because a lockstep model that only works on a
//! perfect link does not work.

use std::collections::VecDeque;

use redshift_net::lockstep::{TurnScheduler, TurnStatus};
use redshift_net::protocol::{Packet, TurnCommands, decode, encode};
use redshift_sim::EntityId;
use redshift_sim::command::{CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map, Terrain};
use redshift_sim::sim::{MatchSetup, Sim};

const MATCH_SEED: u64 = 0xFEED_FACE_0000_0001;

/// Input delay, in ticks.
///
/// Must cover the link's worst-case one-way latency, which [`Link`] sets at
/// three steps. Below that the two peers spend most of their time waiting on
/// each other: A cannot run tick T until B's commands for T arrive, and B
/// cannot produce them until it has advanced, so a shortfall in delay
/// compounds rather than averaging out.
///
/// This is exactly what `input_delay_for_rtt` computes from a measured round
/// trip at match start.
const INPUT_DELAY: u32 = 6;

fn match_setup() -> MatchSetup {
    let mut map = Map::new(40, 40);
    map.fill_rect(Cell::new(14, 2), Cell::new(14, 26), Terrain::Rock);
    map.fill_rect(Cell::new(28, 12), Cell::new(28, 38), Terrain::Rock);
    map.fill_rect(Cell::new(4, 32), Cell::new(16, 34), Terrain::Water);

    let mut spawns = Vec::new();
    for i in 0..8i32 {
        spawns.push((PlayerId(0), Cell::new(2 + i % 3, 2 + i / 3).centre()));
        spawns.push((PlayerId(1), Cell::new(37 - i % 3, 37 - i / 3).centre()));
    }
    MatchSetup {
        seed: MATCH_SEED,
        map,
        spawns,
    }
}

/// A deterministic, unpleasant network.
///
/// Reproducible on purpose: a flaky test that fails one run in fifty is worse
/// than no test, because the failure gets dismissed as "the network".
struct Link {
    /// Datagrams in flight, as `(deliver_at_step, bytes)`.
    in_flight: VecDeque<(u32, Vec<u8>)>,
    step: u32,
    seed: u64,
    pub sent: u32,
    pub dropped: u32,
    pub duplicated: u32,
}

impl Link {
    fn new(seed: u64) -> Link {
        Link {
            in_flight: VecDeque::new(),
            step: 0,
            seed,
            sent: 0,
            dropped: 0,
            duplicated: 0,
        }
    }

    /// A cheap reproducible generator. This is test scaffolding, not
    /// simulation state, so it deliberately does not use `SimRng`.
    fn next(&mut self) -> u32 {
        self.seed = self
            .seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (self.seed >> 33) as u32
    }

    fn send(&mut self, bytes: Vec<u8>) {
        self.sent += 1;

        // One packet in eight is lost. Far worse than any real LAN, and worse
        // than most internet links.
        if self.next().is_multiple_of(8) {
            self.dropped += 1;
            return;
        }

        // Latency of one to three steps, which also reorders packets.
        let delay = 1 + self.next() % 3;
        self.in_flight.push_back((self.step + delay, bytes.clone()));

        // One in six arrives twice.
        if self.next().is_multiple_of(6) {
            self.duplicated += 1;
            let extra = 1 + self.next() % 4;
            self.in_flight.push_back((self.step + delay + extra, bytes));
        }
    }

    /// Everything due by now, in arrival order.
    fn receive(&mut self) -> Vec<Vec<u8>> {
        self.step += 1;
        let mut out = Vec::new();
        let mut remaining = VecDeque::new();
        while let Some((due, bytes)) = self.in_flight.pop_front() {
            if due <= self.step {
                out.push(bytes);
            } else {
                remaining.push_back((due, bytes));
            }
        }
        self.in_flight = remaining;
        out
    }
}

/// One side of the match.
struct Peer {
    sim: Sim,
    scheduler: TurnScheduler,
    /// Recent turns, repeated in every packet so a single loss costs nothing.
    history: VecDeque<TurnCommands>,
    /// A state hash waiting to be sent.
    ///
    /// Captured at the moment the tick is executed, not recomputed when a
    /// packet is composed. A peer can run several ticks between packets, so
    /// asking "is the *current* tick a checkpoint?" at send time skips
    /// checkpoints — and a checkpoint that is never sent is never compared.
    pending_hash: Option<redshift_net::protocol::StateCheck>,
}

impl Peer {
    fn new(id: PlayerId) -> Peer {
        Peer {
            sim: Sim::new(match_setup()),
            scheduler: TurnScheduler::new(id, vec![PlayerId(0), PlayerId(1)], INPUT_DELAY),
            history: VecDeque::new(),
            pending_hash: None,
        }
    }

    /// Builds this step's outgoing packet.
    ///
    /// A packet still goes out when the scheduler is throttled: it repeats
    /// recent turns, which is exactly what a peer waiting on us needs.
    fn compose(&mut self) -> Packet {
        if let Some((tick, commands)) = self.scheduler.take_outgoing() {
            self.history.push_back(TurnCommands {
                tick,
                player: self.scheduler.local_player(),
                commands,
            });
            while self.history.len() > redshift_net::protocol::REDUNDANT_TICKS {
                self.history.pop_front();
            }
        }
        let tick = self.scheduler.scheduled_tick();

        Packet::Turn {
            tick,
            turns: self.history.iter().cloned().collect(),
            hash: self.pending_hash.take(),
        }
    }

    fn ingest(&mut self, packet: Packet) {
        let Packet::Turn { turns, hash, .. } = packet else {
            return;
        };
        for turn in turns {
            self.scheduler.accept(turn.tick, turn.player, turn.commands);
        }
        if let Some(check) = hash {
            // Whose hash it is only matters for the report.
            let other = if self.scheduler.local_player() == PlayerId(0) {
                PlayerId(1)
            } else {
                PlayerId(0)
            };
            self.scheduler
                .check_remote_hash(other, check.tick, check.hash);
        }
    }

    /// Runs every tick that is ready.
    ///
    /// Returns a divergence report if one was found, rather than panicking:
    /// one test wants a desync to be a failure, and another wants it to be the
    /// expected outcome.
    fn advance(&mut self) -> Option<redshift_net::DesyncReport> {
        loop {
            match self.scheduler.poll() {
                TurnStatus::Ready(commands) => {
                    self.sim.tick(&commands);
                    let executed = self.sim.tick_number() - 1;
                    if TurnScheduler::should_hash(executed) {
                        let hash = self.sim.state_hash();
                        self.scheduler.record_local_hash(executed, hash);
                        self.pending_hash = Some(redshift_net::protocol::StateCheck {
                            tick: executed,
                            hash,
                        });
                    }
                }
                TurnStatus::Waiting { .. } => return None,
                TurnStatus::Desynced(report) => return Some(report),
            }
        }
    }
}

/// The orders both players issue, scripted so every run is identical.
fn script(peer: &mut Peer, step: u32) {
    let local = peer.scheduler.local_player();
    let targets: &[(u32, i32, i32)] = &[(3, 36, 36), (120, 2, 36), (260, 36, 2), (400, 20, 20)];
    for &(at, x, y) in targets {
        if step != at {
            continue;
        }
        let units: Vec<EntityId> = peer
            .sim
            .units()
            .iter()
            .filter(|(_, u)| u.owner == local)
            .map(|(id, _)| id)
            .collect();
        let target = if local == PlayerId(0) {
            Cell::new(x, y)
        } else {
            Cell::new(38 - x, 38 - y)
        };
        peer.scheduler.issue(CommandKind::Move { units, target });
    }
}

#[test]
fn two_peers_stay_in_sync_over_a_hostile_link() {
    let mut a = Peer::new(PlayerId(0));
    let mut b = Peer::new(PlayerId(1));
    let mut a_to_b = Link::new(0x1111);
    let mut b_to_a = Link::new(0x2222);

    const STEPS: u32 = 600;
    for step in 0..STEPS {
        script(&mut a, step);
        script(&mut b, step);

        let from_a = encode(&a.compose()).expect("packet must fit");
        let from_b = encode(&b.compose()).expect("packet must fit");
        a_to_b.send(from_a);
        b_to_a.send(from_b);

        for bytes in a_to_b.receive() {
            if let Some(packet) = decode(&bytes) {
                b.ingest(packet);
            }
        }
        for bytes in b_to_a.receive() {
            if let Some(packet) = decode(&bytes) {
                a.ingest(packet);
            }
        }

        if let Some(report) = a.advance() {
            panic!(
                "peer A reported a desync at tick {}: {report:?}",
                report.tick
            );
        }
        if let Some(report) = b.advance() {
            panic!(
                "peer B reported a desync at tick {}: {report:?}",
                report.tick
            );
        }

        // The strongest check available: compare the two simulations directly
        // whenever they are on the same tick. A real match can only compare the
        // hashes it exchanges; here we can look at both sides at once.
        if a.sim.tick_number() == b.sim.tick_number() {
            assert_eq!(
                a.sim.state_hash(),
                b.sim.state_hash(),
                "peers diverged at tick {} (step {step})",
                a.sim.tick_number()
            );
        }
    }

    assert!(
        a_to_b.dropped > 20,
        "the link should have actually dropped packets"
    );
    assert!(
        a_to_b.duplicated > 20,
        "the link should have actually duplicated packets"
    );
    // A stalled match would trivially never desync, so progress has to be
    // asserted too. Under this much loss and jitter the peers will not keep up
    // with one tick per step, but they must stay close.
    assert!(
        a.sim.tick_number() > STEPS / 2,
        "only {} ticks ran in {STEPS} steps — the match stalled rather than synced",
        a.sim.tick_number()
    );

    // Both sides must end on the same tick, not merely have agreed along the
    // way — one peer lagging permanently would be a stall, not a sync.
    let behind = a.sim.tick_number().abs_diff(b.sim.tick_number());
    assert!(behind <= 1, "peers ended {behind} ticks apart");

    println!(
        "ran {} ticks; {} packets sent, {} dropped, {} duplicated",
        a.sim.tick_number(),
        a_to_b.sent,
        a_to_b.dropped,
        a_to_b.duplicated
    );
}

#[test]
fn a_diverging_peer_is_caught_within_a_second() {
    // Inject a divergence and confirm the hash exchange catches it promptly.
    // Detection speed is the whole point: a desync found ten minutes later is
    // nearly impossible to diagnose.
    let mut a = Peer::new(PlayerId(0));
    let mut b = Peer::new(PlayerId(1));

    // Give b an extra unit. Its simulation is now a different game, but every
    // command it exchanges is still perfectly valid.
    b.sim.spawn_unit(PlayerId(1), Cell::new(20, 20).centre());

    let mut detected_at = None;
    for step in 0..200u32 {
        let from_a = encode(&a.compose()).unwrap();
        let from_b = encode(&b.compose()).unwrap();
        b.ingest(decode(&from_a).unwrap());
        a.ingest(decode(&from_b).unwrap());

        let found = a.advance().or_else(|| b.advance());
        if found.is_some() {
            detected_at = Some(step);
            break;
        }
    }

    let step = detected_at.expect("a divergence must be detected");
    let report = a
        .scheduler
        .desync()
        .or_else(|| b.scheduler.desync())
        .unwrap();
    println!("divergence caught at step {step}, tick {}", report.tick);

    // Hashes are exchanged once a second. Catching it within a handful of
    // those is the requirement.
    assert!(step < 60, "took {step} steps to notice a divergence");
    assert_ne!(report.local_hash, report.remote_hash);
}

#[test]
fn a_replay_reproduces_the_match_exactly() {
    // A replay is the seed plus the command log — a few kilobytes for a whole
    // match. This is what makes desync debugging tractable, so it is worth
    // proving rather than assuming.
    let mut peer = Peer::new(PlayerId(0));
    let mut solo = TurnScheduler::new(PlayerId(0), vec![PlayerId(0)], INPUT_DELAY);
    std::mem::swap(&mut peer.scheduler, &mut solo);

    let mut log: Vec<(u32, Vec<redshift_sim::command::Command>)> = Vec::new();
    let mut hashes = Vec::new();

    for step in 0..300u32 {
        script(&mut peer, step);
        if let Some((tick, commands)) = peer.scheduler.take_outgoing() {
            log.push((tick, commands));
        }
        while let TurnStatus::Ready(commands) = peer.scheduler.poll() {
            peer.sim.tick(&commands);
            hashes.push(peer.sim.state_hash());
        }
    }

    // Replay: same seed, same commands, no network at all.
    let mut replay_sim = Sim::new(match_setup());
    let mut replay_scheduler = TurnScheduler::new(PlayerId(0), vec![PlayerId(0)], INPUT_DELAY);
    let mut replay_hashes = Vec::new();
    for (tick, commands) in &log {
        replay_scheduler.accept(*tick, PlayerId(0), commands.clone());
        while let TurnStatus::Ready(commands) = replay_scheduler.poll() {
            replay_sim.tick(&commands);
            replay_hashes.push(replay_sim.state_hash());
        }
    }

    assert_eq!(
        hashes.len(),
        replay_hashes.len(),
        "the replay ran a different number of ticks"
    );
    for (tick, (live, replayed)) in hashes.iter().zip(&replay_hashes).enumerate() {
        assert_eq!(live, replayed, "the replay diverged at tick {tick}");
    }
    assert!(!hashes.is_empty());

    let bytes: usize = log.iter().map(|(_, c)| c.len() * 32 + 8).sum();
    println!("{} ticks logged, roughly {bytes} bytes", log.len());
}
