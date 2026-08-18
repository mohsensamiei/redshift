//! Bringing a networked match up from the command line.
//!
//! `--host` and `--join` exist so a match can be started without a user
//! interface, which the lobby screens will replace. They are not throwaway
//! scaffolding: driving two real client processes from a script is the only way
//! to exercise the whole stack — window, renderer, socket and all — and it is
//! how the two-process test is run.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use redshift_net::MatchSession;
use redshift_net::discovery::{Announcer, Discovery};
use redshift_net::lobby::HostLobby;
use redshift_net::lockstep::MIN_INPUT_DELAY;
use redshift_net::protocol::{DEFAULT_GAME_PORT, PROTOCOL_VERSION, Packet};
use redshift_net::transport::Transport;
use redshift_sim::command::PlayerId;
use redshift_sim::sim::MatchSetup;

/// Stands in for the rules hash until `redshift-data` exists.
///
/// Both peers must agree on it, and the handshake refuses them if they do not.
/// Once rules are data, this becomes the hash of the loaded files.
const RULES_HASH: u64 = 0x5EED_0001_0000_0001;

/// How long to wait for the other side before giving up.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);

pub struct NetOptions {
    pub port: u16,
    pub name: String,
    /// Discovery port. Overridable so two clients can run on one machine
    /// without fighting over the well-known port.
    pub discovery_port: u16,
}

impl Default for NetOptions {
    fn default() -> Self {
        NetOptions {
            port: DEFAULT_GAME_PORT,
            name: "player".into(),
            discovery_port: redshift_net::protocol::DISCOVERY_PORT,
        }
    }
}

/// Hosts a match, waits for one client, and returns the session once it starts.
///
/// Blocking. Acceptable because nothing is drawn yet — the window opens after
/// this returns, so there is no frame to stall.
pub fn host(
    setup_for: impl Fn(u64) -> MatchSetup,
    options: &NetOptions,
) -> std::io::Result<MatchSession> {
    let mut transport = Transport::bind_port(options.port)?;
    let port = transport.local_addr()?.port();

    // Derived from the clock rather than a fixed constant, so consecutive
    // matches on the same machine do not replay identically.
    let seed = seed_from_clock();

    let mut lobby = HostLobby::new(
        format!("{}'s game", options.name),
        "crossroads".into(),
        2,
        RULES_HASH,
        seed,
        port,
    );
    lobby.set_host_name(options.name.clone());

    let mut announcer = Announcer::with_target(SocketAddr::from((
        std::net::Ipv4Addr::BROADCAST,
        options.discovery_port,
    )))?;
    // Loopback as well as broadcast, so two clients on one machine find each
    // other even where the operating system does not loop broadcast back.
    let mut loopback = Announcer::with_target(SocketAddr::from((
        std::net::Ipv4Addr::LOCALHOST,
        options.discovery_port,
    )))?;

    println!("hosting on port {port}; waiting for a player...");

    let mut events = Vec::new();
    let mut client_addr: Option<SocketAddr> = None;
    let deadline = Instant::now() + CONNECT_TIMEOUT;

    while Instant::now() < deadline {
        let announce = lobby.announce();
        announcer.tick(&announce);
        loopback.tick(&announce);

        for (from, packet) in transport.poll(32) {
            if let Some(reply) = lobby.handle(from, &packet, &mut events) {
                let _ = transport.send(&reply, from);
                if matches!(reply, Packet::JoinAccepted { .. }) {
                    client_addr = Some(from);
                }
            }
        }

        for event in events.drain(..) {
            println!("lobby: {event:?}");
        }

        // Two players present, so begin. A lobby screen will make this the
        // player's decision.
        if client_addr.is_some() && lobby.peers().len() >= 2 {
            lobby.set_ready(PlayerId(0), true, &mut events);
            lobby.set_ready(PlayerId(1), true, &mut events);
            if let Some(start) = lobby.start(&mut events) {
                let peers = lobby.peer_addresses();
                // Repeat the start packet: it is the one message with no
                // natural retransmission, and losing it would strand the
                // client in the lobby forever.
                for _ in 0..5 {
                    let _ = transport.broadcast_to(&start, &peers);
                    std::thread::sleep(Duration::from_millis(20));
                }
                let input_delay = lobby.negotiated_input_delay().max(MIN_INPUT_DELAY);
                println!("starting: seed {seed:#x}, input delay {input_delay}");
                return Ok(MatchSession::networked(
                    setup_for(seed),
                    PlayerId(0),
                    vec![PlayerId(0), PlayerId(1)],
                    peers,
                    input_delay,
                    transport,
                ));
            }
        }

        std::thread::sleep(Duration::from_millis(10));
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "no player joined",
    ))
}

/// Finds a match, joins it, and returns the session once the host starts it.
pub fn join(
    setup_for: impl Fn(u64) -> MatchSetup,
    address: Option<SocketAddr>,
    options: &NetOptions,
) -> std::io::Result<MatchSession> {
    let mut transport = Transport::bind_port(0)?;

    let host_addr = match address {
        Some(addr) => addr,
        None => discover_host(options)?,
    };
    println!("joining {host_addr}...");

    let request = Packet::JoinRequest {
        protocol: PROTOCOL_VERSION,
        rules_hash: RULES_HASH,
        player_name: options.name.clone(),
    };

    let deadline = Instant::now() + CONNECT_TIMEOUT;
    let mut accepted: Option<(PlayerId, u64, u32)> = None;
    let mut last_request = Instant::now() - Duration::from_secs(1);

    while Instant::now() < deadline {
        // Repeat the request until answered: the reply may be lost, and the
        // host treats a repeat from the same address as the same player.
        if accepted.is_none() && last_request.elapsed() > Duration::from_millis(250) {
            let _ = transport.send(&request, host_addr);
            last_request = Instant::now();
        }

        for (_, packet) in transport.poll(32) {
            match packet {
                Packet::JoinAccepted {
                    player,
                    seed,
                    input_delay,
                    ..
                // The host repeats its acceptance whenever we repeat the
                // request, so only the first one is news.
                } if accepted.is_none() => {
                    println!("accepted as player {}", player.0);
                    accepted = Some((player, seed, input_delay));
                }
                Packet::JoinRejected { reason } => {
                    return Err(std::io::Error::other(reason.describe()));
                }
                Packet::Start => {
                    if let Some((player, seed, input_delay)) = accepted {
                        let input_delay = input_delay.max(MIN_INPUT_DELAY);
                        println!("starting: seed {seed:#x}, input delay {input_delay}");
                        return Ok(MatchSession::networked(
                            setup_for(seed),
                            player,
                            vec![PlayerId(0), PlayerId(1)],
                            vec![host_addr],
                            input_delay,
                            transport,
                        ));
                    }
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "no match found or never started",
    ))
}

fn discover_host(options: &NetOptions) -> std::io::Result<SocketAddr> {
    let mut discovery = Discovery::on_port(options.discovery_port)?;
    println!("looking for a match on the local network...");

    let deadline = Instant::now() + CONNECT_TIMEOUT;
    while Instant::now() < deadline {
        discovery.tick();
        if let Some(found) = discovery.joinable().first() {
            println!(
                "found \"{}\" at {}",
                found.announce.match_name, found.address
            );
            return Ok(found.address);
        }
        // Report matches we can see but cannot join, rather than leaving the
        // player wondering why their friend's game is invisible.
        for entry in discovery.matches() {
            if let Some(reason) = entry.unjoinable_reason() {
                println!("  ignoring \"{}\": {reason}", entry.announce.match_name);
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "no match found",
    ))
}

/// A seed with no relationship to game logic.
///
/// The clock is forbidden inside the simulation, but choosing the seed happens
/// before any of it runs and is exactly the kind of thing a clock is for.
fn seed_from_clock() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x5EED)
        | 1
}
