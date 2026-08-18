//! Wire format.
//!
//! Everything two peers say to each other, and the rules for saying it.
//!
//! # Design constraints
//!
//! - **Small.** A lockstep match sends a packet every tick, twenty times a
//!   second. The common case — one player, one order — must fit comfortably
//!   inside a single datagram with room to spare for the redundancy below.
//! - **Self-describing enough to reject mismatches.** A peer running a
//!   different build must be refused at the handshake, not allowed to connect
//!   and desync three minutes later.
//! - **No retransmission requests.** Because commands are tiny, every packet
//!   carries the last few ticks again. A single lost datagram then costs
//!   nothing, and there is no acknowledgement state machine to get wrong.

use serde::{Deserialize, Serialize};

use redshift_sim::Tick;
use redshift_sim::command::{Command, PlayerId};

/// Bumped whenever the wire format or the simulation's behaviour changes.
///
/// Two peers must agree exactly. This covers the simulation as well as the
/// protocol: a build whose pathfinding tie-break changed speaks the same wire
/// format but plays a different game, and connecting the two would desync.
pub const PROTOCOL_VERSION: u32 = 1;

/// Magic bytes at the head of every datagram.
///
/// Cheap protection against another application's broadcast traffic being
/// parsed as a game packet — which, on a busy network, otherwise shows up as
/// baffling deserialisation failures.
pub const MAGIC: [u8; 4] = *b"RSFT";

/// UDP port used for LAN discovery announcements.
pub const DISCOVERY_PORT: u16 = 47654;

/// Default port a host listens on for gameplay traffic.
pub const DEFAULT_GAME_PORT: u16 = 47655;

/// How many past ticks each packet repeats.
///
/// Three covers a single loss plus a reordering. Commands are small enough that
/// repeating them costs less than the machinery to request a resend would.
pub const REDUNDANT_TICKS: usize = 3;

/// Largest datagram we will send or accept.
///
/// Comfortably inside the smallest MTU in practical use, so packets are never
/// fragmented. Fragmentation would turn one loss into a whole-packet loss and
/// undermine the redundancy above.
pub const MAX_PACKET_BYTES: usize = 1200;

/// One player's commands for one tick.
///
/// Sent even when empty: a silent player and a disconnected one must be
/// distinguishable, or the match would stall for the full timeout every time
/// somebody stopped clicking.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnCommands {
    pub tick: Tick,
    pub player: PlayerId,
    pub commands: Vec<Command>,
}

/// A datagram.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Packet {
    /// Broadcast by a host so clients on the same network can find it.
    Announce(Announce),

    /// Client asks to join.
    JoinRequest {
        protocol: u32,
        /// Hash of the loaded rules. A mismatch means the two builds would
        /// simulate differently, so it is refused at the handshake.
        rules_hash: u64,
        player_name: String,
    },

    /// Host accepts, assigning a slot and the match parameters.
    JoinAccepted {
        player: PlayerId,
        /// Every peer must start from the same seed.
        seed: u64,
        /// Ticks between issuing a command and executing it.
        input_delay: u32,
        players: Vec<PlayerSlot>,
    },

    /// Host refuses, with a reason the player can act on.
    JoinRejected {
        reason: RejectReason,
    },

    /// Gameplay. The bulk of all traffic.
    Turn {
        /// The newest tick in this packet.
        tick: Tick,
        /// This tick and up to [`REDUNDANT_TICKS`] before it, newest first.
        /// Receivers ignore any they already have.
        turns: Vec<TurnCommands>,
        /// State hash for a tick already executed, or `None` between checks.
        /// Carried on the gameplay packet rather than its own so it costs no
        /// extra datagram.
        hash: Option<StateCheck>,
    },

    /// Everyone is ready; begin at tick zero.
    Start,

    /// Sent on a clean exit, so peers do not wait out the timeout.
    Leave {
        player: PlayerId,
    },

    /// Divergence detected. The sender is halting and so should everyone else.
    DesyncHalt {
        tick: Tick,
        sender_hash: u64,
        expected_hash: u64,
    },

    /// Liveness probe, used to measure round-trip time before a match starts.
    Ping {
        nonce: u64,
    },
    Pong {
        nonce: u64,
    },
}

/// A state hash for one tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateCheck {
    pub tick: Tick,
    pub hash: u64,
}

/// What a host broadcasts so clients can list it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Announce {
    pub protocol: u32,
    pub match_name: String,
    pub map_name: String,
    pub players: u8,
    pub max_players: u8,
    /// Port to connect on. Carried explicitly rather than assumed, so a host on
    /// a non-default port is still reachable.
    pub game_port: u16,
    pub in_progress: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RejectReason {
    /// Different build. Carries what the host has, so the client can say so.
    ProtocolMismatch {
        host_protocol: u32,
    },
    /// Same protocol, different rules data — which would still desync.
    RulesMismatch {
        host_rules_hash: u64,
    },
    MatchFull,
    AlreadyStarted,
}

impl RejectReason {
    /// A message fit to show a player.
    pub fn describe(&self) -> String {
        match self {
            RejectReason::ProtocolMismatch { host_protocol } => format!(
                "Different game version: the host speaks protocol {host_protocol}, this build \
                 speaks {PROTOCOL_VERSION}."
            ),
            RejectReason::RulesMismatch { .. } => {
                "The host's game rules differ from yours. Both sides need the same build and the \
                 same rules files."
                    .into()
            }
            RejectReason::MatchFull => "That match is full.".into(),
            RejectReason::AlreadyStarted => "That match has already started.".into(),
        }
    }
}

/// A player in the lobby.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerSlot {
    pub id: PlayerId,
    pub name: String,
    pub ready: bool,
}

/// Serialises a packet, framed with the magic bytes.
///
/// # Errors
/// If the packet does not fit in [`MAX_PACKET_BYTES`]. That is a bug in the
/// caller — commands are meant to be small — and truncating instead would
/// produce a packet that deserialises into a different game action.
pub fn encode(packet: &Packet) -> Result<Vec<u8>, EncodeError> {
    let body = bincode::serde::encode_to_vec(packet, bincode::config::standard())
        .map_err(|_| EncodeError::Serialisation)?;
    let total = MAGIC.len() + body.len();
    if total > MAX_PACKET_BYTES {
        return Err(EncodeError::TooLarge {
            size: total,
            limit: MAX_PACKET_BYTES,
        });
    }
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Parses a datagram.
///
/// Returns `None` for anything that is not ours — stray broadcast traffic,
/// a truncated packet, a packet from a wildly different version. Silently
/// ignoring these is correct: the network is shared, and other applications'
/// packets are not errors.
pub fn decode(bytes: &[u8]) -> Option<Packet> {
    if bytes.len() < MAGIC.len() || bytes[..MAGIC.len()] != MAGIC {
        return None;
    }
    bincode::serde::decode_from_slice(&bytes[MAGIC.len()..], bincode::config::standard())
        .ok()
        .map(|(packet, _)| packet)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncodeError {
    Serialisation,
    TooLarge { size: usize, limit: usize },
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EncodeError::Serialisation => write!(f, "could not serialise packet"),
            EncodeError::TooLarge { size, limit } => {
                write!(f, "packet is {size} bytes, over the {limit} byte limit")
            }
        }
    }
}

impl std::error::Error for EncodeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use redshift_sim::EntityId;
    use redshift_sim::command::CommandKind;
    use redshift_sim::map::Cell;

    fn sample_turn(command_count: usize, units_each: usize) -> Packet {
        let commands: Vec<Command> = (0..command_count)
            .map(|i| {
                Command::new(
                    PlayerId(0),
                    i as u16,
                    CommandKind::Move {
                        units: vec![EntityId::NONE; units_each],
                        target: Cell::new(40, 40),
                    },
                )
            })
            .collect();
        Packet::Turn {
            tick: 100,
            turns: vec![TurnCommands {
                tick: 100,
                player: PlayerId(0),
                commands,
            }],
            hash: Some(StateCheck {
                tick: 80,
                hash: 0xDEAD_BEEF,
            }),
        }
    }

    #[test]
    fn roundtrip_preserves_every_variant() {
        let packets = vec![
            Packet::Start,
            Packet::Leave {
                player: PlayerId(2),
            },
            Packet::Ping { nonce: 7 },
            Packet::Pong { nonce: 7 },
            Packet::JoinRequest {
                protocol: PROTOCOL_VERSION,
                rules_hash: 42,
                player_name: "mohsen".into(),
            },
            Packet::JoinRejected {
                reason: RejectReason::MatchFull,
            },
            Packet::DesyncHalt {
                tick: 500,
                sender_hash: 1,
                expected_hash: 2,
            },
            sample_turn(1, 4),
        ];
        for packet in packets {
            let bytes = encode(&packet).expect("should encode");
            assert_eq!(
                decode(&bytes),
                Some(packet.clone()),
                "roundtrip failed for {packet:?}"
            );
        }
    }

    #[test]
    fn foreign_traffic_is_ignored_not_misparsed() {
        // The network is shared. Another application's broadcast must be
        // dropped quietly, never parsed into a game action.
        assert_eq!(decode(b""), None);
        assert_eq!(decode(b"RS"), None);
        assert_eq!(decode(b"HTTP/1.1 200 OK"), None);
        assert_eq!(decode(&[0u8; 64]), None);
    }

    #[test]
    fn truncated_packets_are_rejected() {
        let bytes = encode(&sample_turn(2, 8)).unwrap();
        for cut in 1..bytes.len() {
            // Correct magic but a chopped body must not yield a valid packet
            // that happens to deserialise into something else.
            let partial = &bytes[..cut];
            if let Some(packet) = decode(partial) {
                panic!("a {cut}-byte prefix decoded to {packet:?}");
            }
        }
    }

    #[test]
    fn a_typical_turn_is_small() {
        // The claim in docs/03-networking.md is a few hundred bytes per second
        // per player. At 20 Hz that means a typical packet in the tens of
        // bytes.
        let idle = Packet::Turn {
            tick: 1000,
            turns: vec![TurnCommands {
                tick: 1000,
                player: PlayerId(0),
                commands: Vec::new(),
            }],
            hash: None,
        };
        let size = encode(&idle).unwrap().len();
        assert!(size < 32, "an idle turn packet is {size} bytes");

        let one_order = encode(&sample_turn(1, 12)).unwrap().len();
        assert!(
            one_order < 160,
            "a twelve-unit move order is {one_order} bytes"
        );
    }

    #[test]
    fn a_large_order_still_fits_in_one_datagram() {
        // Selecting a big army and ordering it must not fragment. 200 units is
        // a plausible late-game select-all.
        let packet = sample_turn(1, 100);
        let bytes = encode(&packet).expect("100 units must fit");
        assert!(bytes.len() <= MAX_PACKET_BYTES);
    }

    #[test]
    fn oversized_packets_are_refused_rather_than_truncated() {
        // Truncation would produce a packet that deserialises into a *different*
        // order than the player gave — the worst possible failure, because it
        // desyncs silently.
        let huge = sample_turn(40, 200);
        match encode(&huge) {
            Err(EncodeError::TooLarge { size, limit }) => {
                assert!(size > limit);
                assert_eq!(limit, MAX_PACKET_BYTES);
            }
            Err(other) => panic!("wrong error: {other:?}"),
            Ok(bytes) => panic!("expected refusal, got {} bytes", bytes.len()),
        }
    }

    #[test]
    fn reject_reasons_explain_themselves() {
        for reason in [
            RejectReason::ProtocolMismatch { host_protocol: 99 },
            RejectReason::RulesMismatch { host_rules_hash: 7 },
            RejectReason::MatchFull,
            RejectReason::AlreadyStarted,
        ] {
            let text = reason.describe();
            assert!(!text.is_empty());
            assert!(text.ends_with('.'), "{text:?} should read as a sentence");
        }
    }
}
