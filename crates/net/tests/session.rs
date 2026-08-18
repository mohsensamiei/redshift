//! `MatchSession` end to end: solo, networked, and diverged.
//!
//! The two-peer and LAN tests drive the scheduler and the socket directly. This
//! one drives the object the game actually holds, so it covers the wiring
//! between them — the part where a correct scheduler and a correct socket can
//! still be joined together wrongly.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use redshift_net::MatchSession;
use redshift_net::session::{first_divergence, replay_hashes};
use redshift_net::transport::Transport;
use redshift_sim::EntityId;
use redshift_sim::command::{CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map, Terrain};
use redshift_sim::sim::{MatchSetup, Spawn, TEST_KIND};

const SEED: u64 = 0xC0DE_1234_5678_9ABC;

fn setup() -> MatchSetup {
    let mut map = Map::new(32, 32);
    map.fill_rect(Cell::new(12, 2), Cell::new(12, 22), Terrain::Rock);
    let mut spawns = Vec::new();
    for i in 0..6i32 {
        spawns.push((PlayerId(0), Cell::new(2 + i % 3, 2 + i / 3).centre()));
        spawns.push((PlayerId(1), Cell::new(29 - i % 3, 29 - i / 3).centre()));
    }
    MatchSetup::for_test(SEED, map, spawns)
}

fn local(port: u16) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, port))
}

/// One frame's worth of real time. The session paces itself against the clock,
/// so a fixed step keeps the test deterministic in tick count.
const FRAME: f32 = 1.0 / 60.0;

/// Builds two sessions wired to each other over loopback.
///
/// Both sockets are bound before either session exists, so each can be told the
/// other's real address.
fn networked_pair_with(a_setup: MatchSetup, b_setup: MatchSetup) -> (MatchSession, MatchSession) {
    let a_transport = Transport::bind_port(0).expect("bind a");
    let b_transport = Transport::bind_port(0).expect("bind b");
    let a_addr = local(a_transport.local_addr().unwrap().port());
    let b_addr = local(b_transport.local_addr().unwrap().port());
    let players = vec![PlayerId(0), PlayerId(1)];

    (
        MatchSession::networked(
            a_setup,
            PlayerId(0),
            players.clone(),
            vec![b_addr],
            3,
            a_transport,
        ),
        MatchSession::networked(b_setup, PlayerId(1), players, vec![a_addr], 3, b_transport),
    )
}

fn networked_pair() -> (MatchSession, MatchSession) {
    networked_pair_with(setup(), setup())
}

#[test]
fn a_solo_match_runs_and_records_a_replay() {
    // Single-player is a match with one peer. It uses the same scheduler, the
    // same queue, and records the same replay — which is why the multiplayer
    // path gets exercised during single-player development rather than at the
    // end.
    let mut session = MatchSession::solo(setup(), PlayerId(0));
    assert!(!session.is_networked());
    assert_eq!(session.peer_count(), 0);

    let units: Vec<EntityId> = session
        .sim()
        .units()
        .iter()
        .filter(|(_, u)| u.owner == PlayerId(0))
        .map(|(id, _)| id)
        .collect();
    session.issue(CommandKind::Move {
        units,
        target: Cell::new(28, 28),
    });

    for _ in 0..400 {
        let outcome = session.update(FRAME);
        assert!(outcome.desync.is_none());
        assert!(
            outcome.waiting_on.is_empty(),
            "solo must never wait on anyone"
        );
    }

    assert!(
        session.tick_number() > 100,
        "only reached tick {}",
        session.tick_number()
    );
    assert_eq!(
        session.replay().length(),
        session.tick_number(),
        "the replay must have one entry per executed tick"
    );

    // And the recording must reproduce the match.
    let hashes = replay_hashes(setup(), session.replay());
    assert_eq!(hashes.len() as u32, session.tick_number());
    assert_eq!(
        *hashes.last().unwrap(),
        session.sim().state_hash(),
        "replaying the log must land on the same world"
    );
}

#[test]
fn two_sessions_stay_in_step_over_loopback() {
    let (mut a, mut b) = networked_pair();

    let units: Vec<EntityId> = a
        .sim()
        .units()
        .iter()
        .filter(|(_, u)| u.owner == PlayerId(0))
        .map(|(id, _)| id)
        .collect();
    a.issue(CommandKind::Move {
        units,
        target: Cell::new(28, 28),
    });

    let units: Vec<EntityId> = b
        .sim()
        .units()
        .iter()
        .filter(|(_, u)| u.owner == PlayerId(1))
        .map(|(id, _)| id)
        .collect();
    b.issue(CommandKind::Move {
        units,
        target: Cell::new(3, 3),
    });

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut verified = None;
    while a.tick_number() < 120 && Instant::now() < deadline {
        let a_update = a.update(FRAME);
        let b_update = b.update(FRAME);
        assert!(a_update.desync.is_none(), "peer A diverged");
        assert!(b_update.desync.is_none(), "peer B diverged");
        verified = a_update.last_verified.or(verified);

        if a.tick_number() == b.tick_number() {
            assert_eq!(
                a.sim().state_hash(),
                b.sim().state_hash(),
                "peers diverged at tick {}",
                a.tick_number()
            );
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    assert!(
        a.tick_number() >= 120,
        "only reached tick {}",
        a.tick_number()
    );

    // The check that matters: a peer independently confirmed our state. Without
    // it, "no desync" is equally consistent with no hash ever having been
    // compared — which is exactly what a silently broken checkpoint path looks
    // like.
    let (tick, _) = verified.expect("no hash was ever confirmed by the peer");
    assert!(tick > 0, "the only confirmed tick was zero");
    assert!(a.replay().length() > 100);
}

#[test]
fn a_divergence_halts_both_peers_and_writes_dumps() {
    // B starts with an extra unit, so its world is a different game from the
    // first tick — while every command it exchanges stays perfectly valid.
    // That is precisely why this class of bug is invisible without hash
    // comparison: nothing about the traffic looks wrong.
    let mut divergent = setup();
    divergent.spawns.push(Spawn {
        owner: PlayerId(1),
        kind: TEST_KIND,
        pos: Cell::new(16, 16).centre(),
    });
    let (mut a, mut b) = networked_pair_with(setup(), divergent);

    let dumps = std::env::temp_dir().join("redshift-desync-test");
    let _ = std::fs::remove_dir_all(&dumps);

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut report = None;
    while report.is_none() && Instant::now() < deadline {
        let a_update = a.update(FRAME);
        let b_update = b.update(FRAME);
        report = a_update.desync.or(b_update.desync);
        std::thread::sleep(Duration::from_millis(1));
    }

    let report = report.expect("a divergence must be detected");
    assert_ne!(report.local_hash, report.remote_hash);

    // Both sides write a dump. Comparing the two logs offline and bisecting to
    // the first differing tick is the whole diagnostic method.
    let path = a.write_desync_dump(&dumps, &report).expect("write dump");
    assert!(path.exists());
    let text = std::fs::read_to_string(&path).expect("read dump");
    assert!(
        text.contains("local_hash"),
        "the dump should carry the hashes"
    );
    assert!(
        text.contains("replay"),
        "the dump should carry the command log"
    );
    assert!(
        text.len() > 500,
        "the dump looks empty: {} bytes",
        text.len()
    );

    let _ = std::fs::remove_dir_all(&dumps);
}

#[test]
fn diverging_replays_are_bisected_to_the_first_bad_tick() {
    // The offline half of desync diagnosis. Given both peers' logs, the tool
    // must name the tick, not merely say that they differ.
    let mut good = MatchSession::solo(setup(), PlayerId(0));
    for _ in 0..200 {
        good.update(FRAME);
    }

    let mut altered = setup();
    altered.spawns.push(Spawn {
        owner: PlayerId(0),
        kind: TEST_KIND,
        pos: Cell::new(20, 20).centre(),
    });

    let a = replay_hashes(setup(), good.replay());
    let b = replay_hashes(altered, good.replay());

    let at = first_divergence(&a, &b).expect("these worlds differ");
    assert_eq!(at, 0, "an extra unit shows up on the very first tick");
    assert_eq!(
        first_divergence(&a, &a),
        None,
        "identical logs must not report a divergence"
    );
}
