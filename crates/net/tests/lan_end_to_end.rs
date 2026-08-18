//! Discovery, handshake and a played match, over real sockets.
//!
//! The unit tests cover each layer on its own and the two-peer test covers the
//! scheduling rule against a simulated link. This one puts the whole stack
//! together on the loopback interface: a host announces, a client finds it,
//! they negotiate, and then they play — with every byte going through an actual
//! UDP socket.
//!
//! It is deliberately the slowest test in the crate. It is also the one that
//! would catch a mistake none of the others can: a layer that works perfectly
//! but has been wired to the wrong port, or a reply sent to the address a
//! packet came from rather than the one the peer listens on.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use redshift_net::discovery::{Announcer, Discovery};
use redshift_net::lobby::{HostLobby, LobbyEvent};
use redshift_net::lockstep::{TurnScheduler, TurnStatus};
use redshift_net::protocol::{PROTOCOL_VERSION, Packet, StateCheck, TurnCommands};
use redshift_net::transport::Transport;
use redshift_sim::EntityId;
use redshift_sim::command::{CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map, Terrain};
use redshift_sim::sim::{MatchSetup, Sim};

const RULES_HASH: u64 = 0x1234_5678_9ABC_DEF0;
const SEED: u64 = 0xA11A_CE00_1234_5678;

fn local(port: u16) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, port))
}

fn setup() -> MatchSetup {
    let mut map = Map::new(32, 32);
    map.fill_rect(Cell::new(12, 2), Cell::new(12, 22), Terrain::Rock);
    map.fill_rect(Cell::new(22, 10), Cell::new(22, 30), Terrain::Rock);
    let mut spawns = Vec::new();
    for i in 0..6i32 {
        spawns.push((PlayerId(0), Cell::new(2 + i % 3, 2 + i / 3).centre()));
        spawns.push((PlayerId(1), Cell::new(29 - i % 3, 29 - i / 3).centre()));
    }
    MatchSetup::for_test(SEED, map, spawns)
}

/// Pumps a closure until it reports success, or gives up.
///
/// Loopback is fast but not synchronous. Spinning briefly is what separates a
/// dependable test from one that fails occasionally and gets ignored.
fn until(deadline: Duration, mut step: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if step() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

#[test]
fn a_client_discovers_a_host_joins_and_they_play() {
    // -- The host opens a match -------------------------------------------
    let mut host_transport = Transport::bind_port(0).expect("host socket");
    let host_port = host_transport.local_addr().unwrap().port();

    let mut lobby = HostLobby::new(
        "mohsen's game".into(),
        "crossroads".into(),
        2,
        RULES_HASH,
        SEED,
        host_port,
    );

    let mut discovery = Discovery::on_port(0).expect("discovery socket");
    let discovery_port = discovery.local_addr().unwrap().port();
    let mut announcer = Announcer::with_target(local(discovery_port)).expect("announcer");

    // -- The client finds it ----------------------------------------------
    announcer.tick(&lobby.announce());
    assert!(
        until(Duration::from_secs(3), || {
            discovery.tick();
            !discovery.joinable().is_empty()
        }),
        "the client never discovered the host"
    );

    let found = discovery.joinable()[0].clone();
    assert_eq!(found.announce.match_name, "mohsen's game");
    assert_eq!(
        found.address.port(),
        host_port,
        "discovery must use the advertised game port, not the port the announcement came from"
    );

    // -- Handshake ---------------------------------------------------------
    let mut client_transport = Transport::bind_port(0).expect("client socket");
    let host_addr = local(found.address.port());

    client_transport
        .send(
            &Packet::JoinRequest {
                protocol: PROTOCOL_VERSION,
                rules_hash: RULES_HASH,
                player_name: "guest".into(),
            },
            host_addr,
        )
        .expect("join request");

    let mut events = Vec::new();
    let mut client_addr = None;
    assert!(
        until(Duration::from_secs(3), || {
            for (from, packet) in host_transport.poll(16) {
                if let Some(reply) = lobby.handle(from, &packet, &mut events) {
                    host_transport.send(&reply, from).expect("reply");
                    client_addr = Some(from);
                }
            }
            client_addr.is_some()
        }),
        "the host never received the join request"
    );

    let mut assigned = None;
    let mut input_delay = 0;
    assert!(
        until(Duration::from_secs(3), || {
            for (_, packet) in client_transport.poll(16) {
                if let Packet::JoinAccepted {
                    player,
                    seed,
                    input_delay: delay,
                    ..
                } = packet
                {
                    assert_eq!(seed, SEED, "both peers must start from the same seed");
                    assigned = Some(player);
                    input_delay = delay;
                }
            }
            assigned.is_some()
        }),
        "the client was never accepted"
    );

    let client_id = assigned.expect("assigned a slot");
    assert_eq!(client_id, PlayerId(1), "the host holds slot zero");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, LobbyEvent::PlayerJoined { .. }))
    );

    // -- Both sides ready, and the match begins ---------------------------
    lobby.set_ready(PlayerId(0), true, &mut events);
    lobby.set_ready(client_id, true, &mut events);
    assert!(lobby.can_start());
    let start = lobby.start(&mut events).expect("the match should start");
    host_transport
        .send(&start, client_addr.unwrap())
        .expect("start packet");
    assert!(
        until(Duration::from_secs(3), || {
            client_transport
                .poll(16)
                .iter()
                .any(|(_, p)| matches!(p, Packet::Start))
        }),
        "the client never saw the start"
    );

    // -- Play --------------------------------------------------------------
    let players = vec![PlayerId(0), client_id];
    let mut host_sim = Sim::new(setup());
    let mut client_sim = Sim::new(setup());
    let mut host_sched = TurnScheduler::new(PlayerId(0), players.clone(), input_delay);
    let mut client_sched = TurnScheduler::new(client_id, players, input_delay);

    let mut host_history: Vec<TurnCommands> = Vec::new();
    let mut client_history: Vec<TurnCommands> = Vec::new();
    let mut host_hash: Option<StateCheck> = None;
    let mut client_hash: Option<StateCheck> = None;

    let host_peer = client_addr.unwrap();
    let client_peer = local(host_port);

    let deadline = Instant::now() + Duration::from_secs(20);
    while host_sim.tick_number() < 200 && Instant::now() < deadline {
        // Orders, once each, from both sides.
        if host_sim.tick_number() == 5 {
            let units: Vec<EntityId> = host_sim
                .units()
                .iter()
                .filter(|(_, u)| u.owner == PlayerId(0))
                .map(|(id, _)| id)
                .collect();
            if !units.is_empty() {
                host_sched.issue(CommandKind::Move {
                    units,
                    target: Cell::new(28, 28),
                });
            }
        }
        if client_sim.tick_number() == 8 {
            let units: Vec<EntityId> = client_sim
                .units()
                .iter()
                .filter(|(_, u)| u.owner == client_id)
                .map(|(id, _)| id)
                .collect();
            if !units.is_empty() {
                client_sched.issue(CommandKind::Move {
                    units,
                    target: Cell::new(3, 3),
                });
            }
        }

        // Compose and send.
        for (sched, history, hash, transport, to, id) in [
            (
                &mut host_sched,
                &mut host_history,
                &mut host_hash,
                &mut host_transport,
                host_peer,
                PlayerId(0),
            ),
            (
                &mut client_sched,
                &mut client_history,
                &mut client_hash,
                &mut client_transport,
                client_peer,
                client_id,
            ),
        ] {
            if let Some((tick, commands)) = sched.take_outgoing() {
                history.push(TurnCommands {
                    tick,
                    player: id,
                    commands,
                });
                while history.len() > redshift_net::protocol::REDUNDANT_TICKS {
                    history.remove(0);
                }
            }
            let packet = Packet::Turn {
                tick: sched.scheduled_tick(),
                turns: history.clone(),
                hash: hash.take(),
            };
            transport.send(&packet, to).expect("turn packet");
        }

        // Receive.
        for (transport, sched) in [
            (&mut host_transport, &mut host_sched),
            (&mut client_transport, &mut client_sched),
        ] {
            for (_, packet) in transport.poll(32) {
                let Packet::Turn { turns, hash, .. } = packet else {
                    continue;
                };
                for turn in turns {
                    sched.accept(turn.tick, turn.player, turn.commands);
                }
                if let Some(check) = hash {
                    let other = if sched.local_player() == PlayerId(0) {
                        PlayerId(1)
                    } else {
                        PlayerId(0)
                    };
                    sched.check_remote_hash(other, check.tick, check.hash);
                }
            }
        }

        // Advance.
        for (sched, sim, pending) in [
            (&mut host_sched, &mut host_sim, &mut host_hash),
            (&mut client_sched, &mut client_sim, &mut client_hash),
        ] {
            loop {
                match sched.poll() {
                    TurnStatus::Ready(commands) => {
                        sim.tick(&commands);
                        let executed = sim.tick_number() - 1;
                        if TurnScheduler::should_hash(executed) {
                            let h = sim.state_hash();
                            sched.record_local_hash(executed, h);
                            *pending = Some(StateCheck {
                                tick: executed,
                                hash: h,
                            });
                        }
                    }
                    TurnStatus::Waiting { .. } => break,
                    TurnStatus::Desynced(report) => {
                        panic!(
                            "desync over a real socket at tick {}: {report:?}",
                            report.tick
                        )
                    }
                }
            }
        }

        if host_sim.tick_number() == client_sim.tick_number() {
            assert_eq!(
                host_sim.state_hash(),
                client_sim.state_hash(),
                "peers diverged at tick {}",
                host_sim.tick_number()
            );
        }
    }

    assert!(
        host_sim.tick_number() >= 200,
        "the match only reached tick {} before the deadline",
        host_sim.tick_number()
    );
    assert!(host_sched.desync().is_none());
    assert!(client_sched.desync().is_none());

    // Both simulations must agree at the end, not merely along the way.
    let behind = host_sim.tick_number().abs_diff(client_sim.tick_number());
    assert!(behind <= 2, "peers ended {behind} ticks apart");

    println!(
        "played {} ticks over real sockets; host sent {}, client sent {}",
        host_sim.tick_number(),
        host_transport.packets_sent,
        client_transport.packets_sent
    );
}

#[test]
fn a_client_on_a_different_build_is_turned_away_with_a_reason() {
    // The failure this prevents: two mismatched builds connect, play for
    // minutes, then diverge — producing a bug report about random
    // disconnections that is nearly impossible to act on.
    let mut host_transport = Transport::bind_port(0).expect("host socket");
    let host_port = host_transport.local_addr().unwrap().port();
    let mut lobby = HostLobby::new("t".into(), "m".into(), 2, RULES_HASH, SEED, host_port);
    let mut client_transport = Transport::bind_port(0).expect("client socket");

    client_transport
        .send(
            &Packet::JoinRequest {
                protocol: PROTOCOL_VERSION + 1,
                rules_hash: RULES_HASH,
                player_name: "stale build".into(),
            },
            local(host_port),
        )
        .expect("join request");

    let mut events = Vec::new();
    assert!(
        until(Duration::from_secs(3), || {
            let mut replied = false;
            for (from, packet) in host_transport.poll(16) {
                if let Some(reply) = lobby.handle(from, &packet, &mut events) {
                    host_transport.send(&reply, from).expect("reply");
                    replied = true;
                }
            }
            replied
        }),
        "the host never replied"
    );

    let mut reason = None;
    assert!(
        until(Duration::from_secs(3), || {
            for (_, packet) in client_transport.poll(16) {
                if let Packet::JoinRejected { reason: r } = packet {
                    reason = Some(r);
                }
            }
            reason.is_some()
        }),
        "the client never learned why it was refused"
    );

    let text = reason.unwrap().describe();
    assert!(
        text.contains("version"),
        "the message should name the problem: {text}"
    );
    assert_eq!(
        lobby.peers().len(),
        1,
        "the mismatched client must not have been admitted"
    );
}
