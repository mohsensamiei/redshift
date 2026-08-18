//! The lobby: getting from "two machines on a network" to "a match everyone
//! can simulate identically".
//!
//! # What the handshake is actually for
//!
//! Not authentication — a LAN game has nobody to authenticate against. It
//! exists to **refuse mismatches before they become desyncs**.
//!
//! Two peers running different builds, or different rules data, will simulate
//! differently. Left to connect, they would play for a few minutes and then
//! diverge, producing a bug report about "random disconnects" that is nearly
//! impossible to act on. Checking at the door turns that into a clear message
//! before anybody has invested a minute in the match.
//!
//! So every join is checked on three things:
//!
//! 1. **Protocol version** — the wire format and simulation behaviour.
//! 2. **Rules hash** — the unit and building data both sides loaded.
//! 3. **Room** — whether there is a free slot and the match has not started.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use redshift_sim::command::PlayerId;

use crate::protocol::{Announce, PROTOCOL_VERSION, Packet, PlayerSlot, RejectReason};

/// How long a peer may go unheard from before it is considered gone.
///
/// Peers send a packet every tick, so silence this long means a great many
/// consecutive losses or a real disconnection.
pub const PEER_TIMEOUT: Duration = Duration::from_secs(10);

/// A connected peer.
#[derive(Clone, Debug)]
pub struct Peer {
    pub id: PlayerId,
    pub name: String,
    pub address: SocketAddr,
    pub ready: bool,
    pub last_heard: Instant,
    /// Most recent round-trip measurement, if one has completed.
    pub rtt_ms: Option<u32>,
}

impl Peer {
    pub fn is_timed_out(&self, now: Instant) -> bool {
        now.duration_since(self.last_heard) > PEER_TIMEOUT
    }
}

/// What a lobby event asks the caller to do.
#[derive(Clone, Debug, PartialEq)]
pub enum LobbyEvent {
    PlayerJoined {
        id: PlayerId,
        name: String,
    },
    PlayerLeft {
        id: PlayerId,
        reason: LeaveReason,
    },
    PlayerReady {
        id: PlayerId,
        ready: bool,
    },
    /// Everyone is ready. Start the match with these parameters.
    Starting {
        seed: u64,
        input_delay: u32,
        players: Vec<PlayerId>,
    },
    /// A join was refused. Worth surfacing on the host's screen too, so the
    /// host understands why their friend cannot get in.
    JoinRefused {
        address: SocketAddr,
        reason: RejectReason,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaveReason {
    Quit,
    TimedOut,
}

/// The host's view of a forming match.
pub struct HostLobby {
    peers: Vec<Peer>,
    match_name: String,
    map_name: String,
    max_players: u8,
    rules_hash: u64,
    seed: u64,
    game_port: u16,
    started: bool,
    next_slot: u8,
}

impl HostLobby {
    pub fn new(
        match_name: String,
        map_name: String,
        max_players: u8,
        rules_hash: u64,
        seed: u64,
        game_port: u16,
    ) -> HostLobby {
        assert!(max_players >= 1, "a match needs at least one slot");
        let mut lobby = HostLobby {
            peers: Vec::new(),
            match_name,
            map_name,
            max_players,
            rules_hash,
            seed,
            game_port,
            started: false,
            next_slot: 0,
        };
        // The host occupies slot zero. Giving it a real peer entry rather than
        // special-casing it keeps every later loop uniform.
        lobby.peers.push(Peer {
            id: PlayerId(0),
            name: "host".into(),
            address: SocketAddr::from(([0, 0, 0, 0], 0)),
            ready: false,
            last_heard: Instant::now(),
            rtt_ms: Some(0),
        });
        lobby.next_slot = 1;
        lobby
    }

    pub fn set_host_name(&mut self, name: String) {
        if let Some(host) = self.peers.first_mut() {
            host.name = name;
        }
    }

    pub fn peers(&self) -> &[Peer] {
        &self.peers
    }

    pub fn started(&self) -> bool {
        self.started
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// What to broadcast so clients can find this match.
    pub fn announce(&self) -> Announce {
        Announce {
            protocol: PROTOCOL_VERSION,
            match_name: self.match_name.clone(),
            map_name: self.map_name.clone(),
            players: self.peers.len() as u8,
            max_players: self.max_players,
            game_port: self.game_port,
            in_progress: self.started,
        }
    }

    /// Handles a packet from a client.
    ///
    /// Returns the reply to send back, if any, and pushes anything the caller
    /// should react to onto `events`.
    pub fn handle(
        &mut self,
        from: SocketAddr,
        packet: &Packet,
        events: &mut Vec<LobbyEvent>,
    ) -> Option<Packet> {
        // Any packet is evidence the peer is alive.
        let now = Instant::now();
        if let Some(peer) = self.peers.iter_mut().find(|p| p.address == from) {
            peer.last_heard = now;
        }

        match packet {
            Packet::JoinRequest {
                protocol,
                rules_hash,
                player_name,
            } => Some(self.handle_join(from, *protocol, *rules_hash, player_name, events)),
            Packet::Leave { player } => {
                self.remove(*player, LeaveReason::Quit, events);
                None
            }
            Packet::Ping { nonce } => Some(Packet::Pong { nonce: *nonce }),
            _ => None,
        }
    }

    fn handle_join(
        &mut self,
        from: SocketAddr,
        protocol: u32,
        rules_hash: u64,
        name: &str,
        events: &mut Vec<LobbyEvent>,
    ) -> Packet {
        // Rejoining from the same address is idempotent — the acceptance
        // packet may simply have been lost, and the client will ask again.
        if let Some(existing) = self.peers.iter().find(|p| p.address == from) {
            return self.acceptance_for(existing.id);
        }

        let reject = |reason: RejectReason, events: &mut Vec<LobbyEvent>| {
            events.push(LobbyEvent::JoinRefused {
                address: from,
                reason,
            });
            Packet::JoinRejected { reason }
        };

        // Version before capacity: a player on the wrong build should be told
        // that, not told the match is full.
        if protocol != PROTOCOL_VERSION {
            return reject(
                RejectReason::ProtocolMismatch {
                    host_protocol: PROTOCOL_VERSION,
                },
                events,
            );
        }
        if rules_hash != self.rules_hash {
            return reject(
                RejectReason::RulesMismatch {
                    host_rules_hash: self.rules_hash,
                },
                events,
            );
        }
        if self.started {
            return reject(RejectReason::AlreadyStarted, events);
        }
        if self.peers.len() >= self.max_players as usize {
            return reject(RejectReason::MatchFull, events);
        }

        let id = PlayerId(self.next_slot);
        self.next_slot += 1;
        self.peers.push(Peer {
            id,
            name: name.to_string(),
            address: from,
            ready: false,
            last_heard: Instant::now(),
            rtt_ms: None,
        });
        events.push(LobbyEvent::PlayerJoined {
            id,
            name: name.to_string(),
        });
        self.acceptance_for(id)
    }

    fn acceptance_for(&self, id: PlayerId) -> Packet {
        Packet::JoinAccepted {
            player: id,
            seed: self.seed,
            input_delay: self.negotiated_input_delay(),
            players: self
                .peers
                .iter()
                .map(|p| PlayerSlot {
                    id: p.id,
                    name: p.name.clone(),
                    ready: p.ready,
                })
                .collect(),
        }
    }

    /// An input delay that suits the slowest peer.
    ///
    /// One delay applies to the whole match, so it must accommodate the worst
    /// link. Sizing it for the average would leave the slowest player stalling
    /// everyone constantly.
    pub fn negotiated_input_delay(&self) -> u32 {
        let worst = self
            .peers
            .iter()
            .filter_map(|p| p.rtt_ms)
            .max()
            .unwrap_or(0);
        crate::lockstep::input_delay_for_rtt(worst, redshift_sim::TICK_MS)
    }

    pub fn set_ready(&mut self, id: PlayerId, ready: bool, events: &mut Vec<LobbyEvent>) {
        if let Some(peer) = self.peers.iter_mut().find(|p| p.id == id)
            && peer.ready != ready
        {
            peer.ready = ready;
            events.push(LobbyEvent::PlayerReady { id, ready });
        }
    }

    pub fn record_rtt(&mut self, id: PlayerId, rtt_ms: u32) {
        if let Some(peer) = self.peers.iter_mut().find(|p| p.id == id) {
            peer.rtt_ms = Some(rtt_ms);
        }
    }

    fn remove(&mut self, id: PlayerId, reason: LeaveReason, events: &mut Vec<LobbyEvent>) {
        // The host cannot leave its own lobby; that ends the match instead.
        if id == PlayerId(0) {
            return;
        }
        if let Some(index) = self.peers.iter().position(|p| p.id == id) {
            self.peers.remove(index);
            events.push(LobbyEvent::PlayerLeft { id, reason });
        }
    }

    /// Drops peers that have gone silent. Call periodically.
    pub fn expire_silent_peers(&mut self, events: &mut Vec<LobbyEvent>) {
        let now = Instant::now();
        let gone: Vec<PlayerId> = self
            .peers
            .iter()
            .filter(|p| p.id != PlayerId(0) && p.is_timed_out(now))
            .map(|p| p.id)
            .collect();
        for id in gone {
            self.remove(id, LeaveReason::TimedOut, events);
        }
    }

    /// Whether the match can begin.
    pub fn can_start(&self) -> bool {
        !self.started && self.peers.len() >= 2 && self.peers.iter().all(|p| p.ready)
    }

    /// Begins the match, if it can.
    pub fn start(&mut self, events: &mut Vec<LobbyEvent>) -> Option<Packet> {
        if !self.can_start() {
            return None;
        }
        self.started = true;
        events.push(LobbyEvent::Starting {
            seed: self.seed,
            input_delay: self.negotiated_input_delay(),
            players: self.peers.iter().map(|p| p.id).collect(),
        });
        Some(Packet::Start)
    }

    /// Addresses to send gameplay traffic to — everyone but the host.
    pub fn peer_addresses(&self) -> Vec<SocketAddr> {
        self.peers
            .iter()
            .filter(|p| p.id != PlayerId(0))
            .map(|p| p.address)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULES_HASH: u64 = 0x5A1E_D0C5;

    fn host() -> HostLobby {
        host_with_slots(2)
    }

    fn host_with_slots(max_players: u8) -> HostLobby {
        HostLobby::new(
            "test".into(),
            "map".into(),
            max_players,
            RULES_HASH,
            99,
            47655,
        )
    }

    fn client_addr(n: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], 40000 + n))
    }

    fn join(hash: u64) -> Packet {
        Packet::JoinRequest {
            protocol: PROTOCOL_VERSION,
            rules_hash: hash,
            player_name: "guest".into(),
        }
    }

    #[test]
    fn a_matching_client_is_accepted_and_given_a_slot() {
        let mut lobby = host();
        let mut events = Vec::new();
        let reply = lobby.handle(client_addr(1), &join(RULES_HASH), &mut events);

        match reply {
            Some(Packet::JoinAccepted {
                player,
                seed,
                players,
                ..
            }) => {
                assert_eq!(player, PlayerId(1), "the host holds slot zero");
                assert_eq!(seed, 99, "every peer must start from the same seed");
                assert_eq!(players.len(), 2);
            }
            other => panic!("expected acceptance, got {other:?}"),
        }
        assert!(matches!(
            events[0],
            LobbyEvent::PlayerJoined {
                id: PlayerId(1),
                ..
            }
        ));
    }

    #[test]
    fn a_different_build_is_refused_at_the_door() {
        // The entire point of the handshake. Left to connect, these two would
        // play for minutes and then diverge.
        let mut lobby = host();
        let mut events = Vec::new();
        let reply = lobby.handle(
            client_addr(1),
            &Packet::JoinRequest {
                protocol: PROTOCOL_VERSION + 1,
                rules_hash: RULES_HASH,
                player_name: "guest".into(),
            },
            &mut events,
        );
        match reply {
            Some(Packet::JoinRejected {
                reason: RejectReason::ProtocolMismatch { host_protocol },
            }) => assert_eq!(host_protocol, PROTOCOL_VERSION),
            other => panic!("expected a protocol rejection, got {other:?}"),
        }
        assert_eq!(
            lobby.peers().len(),
            1,
            "the client must not have been admitted"
        );
    }

    #[test]
    fn different_rules_data_is_refused_even_on_the_same_build() {
        // Same binary, edited unit stats. Just as fatal, and much easier to end
        // up with by accident.
        let mut lobby = host();
        let mut events = Vec::new();
        let reply = lobby.handle(client_addr(1), &join(RULES_HASH ^ 1), &mut events);
        assert!(matches!(
            reply,
            Some(Packet::JoinRejected {
                reason: RejectReason::RulesMismatch { .. }
            })
        ));
        assert!(matches!(events[0], LobbyEvent::JoinRefused { .. }));
    }

    #[test]
    fn version_is_checked_before_capacity() {
        // A player on the wrong build should be told so, not told the match is
        // full — otherwise they wait for a slot that would never work.
        let mut lobby = HostLobby::new("t".into(), "m".into(), 1, RULES_HASH, 1, 47655);
        let mut events = Vec::new();
        let reply = lobby.handle(
            client_addr(1),
            &Packet::JoinRequest {
                protocol: PROTOCOL_VERSION + 5,
                rules_hash: RULES_HASH,
                player_name: "guest".into(),
            },
            &mut events,
        );
        assert!(matches!(
            reply,
            Some(Packet::JoinRejected {
                reason: RejectReason::ProtocolMismatch { .. }
            })
        ));
    }

    #[test]
    fn a_full_match_is_refused() {
        let mut lobby = HostLobby::new("t".into(), "m".into(), 2, RULES_HASH, 1, 47655);
        let mut events = Vec::new();
        lobby.handle(client_addr(1), &join(RULES_HASH), &mut events);
        let reply = lobby.handle(client_addr(2), &join(RULES_HASH), &mut events);
        assert!(matches!(
            reply,
            Some(Packet::JoinRejected {
                reason: RejectReason::MatchFull
            })
        ));
    }

    #[test]
    fn rejoining_from_the_same_address_is_idempotent() {
        // The acceptance packet may have been lost, so the client asks again.
        // Handing out a second slot would silently create a phantom player.
        let mut lobby = host();
        let mut events = Vec::new();
        let first = lobby.handle(client_addr(1), &join(RULES_HASH), &mut events);
        let second = lobby.handle(client_addr(1), &join(RULES_HASH), &mut events);

        let slot_of = |p: &Option<Packet>| match p {
            Some(Packet::JoinAccepted { player, .. }) => *player,
            other => panic!("expected acceptance, got {other:?}"),
        };
        assert_eq!(slot_of(&first), slot_of(&second));
        assert_eq!(
            lobby.peers().len(),
            2,
            "a retry must not create a second player"
        );
    }

    #[test]
    fn a_match_starts_only_when_everyone_is_ready() {
        let mut lobby = host();
        let mut events = Vec::new();
        lobby.handle(client_addr(1), &join(RULES_HASH), &mut events);

        assert!(!lobby.can_start(), "nobody is ready yet");
        lobby.set_ready(PlayerId(0), true, &mut events);
        assert!(!lobby.can_start(), "the guest is not ready");
        lobby.set_ready(PlayerId(1), true, &mut events);
        assert!(lobby.can_start());

        assert!(matches!(lobby.start(&mut events), Some(Packet::Start)));
        assert!(lobby.started());
        assert!(
            events
                .iter()
                .any(|e| matches!(e, LobbyEvent::Starting { seed: 99, .. }))
        );
    }

    #[test]
    fn a_solo_lobby_cannot_start_a_network_match() {
        let mut lobby = host();
        let mut events = Vec::new();
        lobby.set_ready(PlayerId(0), true, &mut events);
        assert!(!lobby.can_start(), "one player is not a network match");
        assert!(lobby.start(&mut events).is_none());
    }

    #[test]
    fn a_started_match_refuses_latecomers() {
        let mut lobby = host();
        let mut events = Vec::new();
        lobby.handle(client_addr(1), &join(RULES_HASH), &mut events);
        lobby.set_ready(PlayerId(0), true, &mut events);
        lobby.set_ready(PlayerId(1), true, &mut events);
        lobby.start(&mut events);

        let reply = lobby.handle(client_addr(2), &join(RULES_HASH), &mut events);
        assert!(matches!(
            reply,
            Some(Packet::JoinRejected {
                reason: RejectReason::AlreadyStarted
            })
        ));
        assert!(lobby.announce().in_progress, "the listing must say so too");
    }

    #[test]
    fn input_delay_is_sized_for_the_slowest_peer() {
        // One delay covers the whole match, so it has to suit the worst link.
        // Sizing for the average would leave the slowest player stalling
        // everyone.
        let mut lobby = host_with_slots(4);
        let mut events = Vec::new();
        lobby.handle(client_addr(1), &join(RULES_HASH), &mut events);
        lobby.handle(client_addr(2), &join(RULES_HASH), &mut events);

        lobby.record_rtt(PlayerId(1), 10);
        let fast = lobby.negotiated_input_delay();
        lobby.record_rtt(PlayerId(2), 220);
        let mixed = lobby.negotiated_input_delay();

        assert!(
            mixed > fast,
            "a slow peer must widen the delay: {fast} -> {mixed}"
        );
    }

    #[test]
    fn a_leaving_player_is_removed() {
        let mut lobby = host();
        let mut events = Vec::new();
        lobby.handle(client_addr(1), &join(RULES_HASH), &mut events);
        assert_eq!(lobby.peers().len(), 2);

        events.clear();
        lobby.handle(
            client_addr(1),
            &Packet::Leave {
                player: PlayerId(1),
            },
            &mut events,
        );
        assert_eq!(lobby.peers().len(), 1);
        assert!(matches!(
            events[0],
            LobbyEvent::PlayerLeft {
                id: PlayerId(1),
                reason: LeaveReason::Quit
            }
        ));
    }

    #[test]
    fn the_host_cannot_be_removed_from_its_own_lobby() {
        let mut lobby = host();
        let mut events = Vec::new();
        lobby.handle(
            client_addr(1),
            &Packet::Leave {
                player: PlayerId(0),
            },
            &mut events,
        );
        assert_eq!(lobby.peers()[0].id, PlayerId(0));
    }

    #[test]
    fn a_ping_is_answered() {
        let mut lobby = host();
        let mut events = Vec::new();
        let reply = lobby.handle(client_addr(1), &Packet::Ping { nonce: 77 }, &mut events);
        assert_eq!(reply, Some(Packet::Pong { nonce: 77 }));
    }

    #[test]
    fn the_announcement_reflects_the_lobby() {
        let mut lobby = host();
        let mut events = Vec::new();
        let empty = lobby.announce();
        assert_eq!(empty.players, 1);
        assert!(!empty.in_progress);

        lobby.handle(client_addr(1), &join(RULES_HASH), &mut events);
        assert_eq!(lobby.announce().players, 2);
        assert_eq!(lobby.announce().game_port, 47655);
    }

    #[test]
    fn gameplay_traffic_skips_the_host_itself() {
        let mut lobby = host_with_slots(4);
        let mut events = Vec::new();
        lobby.handle(client_addr(1), &join(RULES_HASH), &mut events);
        lobby.handle(client_addr(2), &join(RULES_HASH), &mut events);

        let addresses = lobby.peer_addresses();
        assert_eq!(addresses.len(), 2);
        assert!(!addresses.contains(&SocketAddr::from(([0, 0, 0, 0], 0))));
    }
}
