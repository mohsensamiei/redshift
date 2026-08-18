//! Moving datagrams.
//!
//! A thin, non-blocking wrapper over a UDP socket. Deliberately thin: the
//! interesting rules live in [`crate::lockstep`], and keeping them apart is
//! what lets the scheduling logic be tested without a network.
//!
//! # Why UDP, and why no retransmission
//!
//! Lockstep sends a small packet every tick. TCP's in-order delivery is exactly
//! wrong for that: one lost segment stalls everything behind it, which in a
//! lockstep match means the whole game halts until the retransmission arrives.
//!
//! Instead every packet repeats the last few ticks' commands. A single loss
//! then costs nothing, because the next packet already carries the missing
//! data. Commands are small enough that this is cheaper than an acknowledgement
//! and retransmission scheme — and there is no state machine to get wrong.
//!
//! # Wall-clock time is allowed here
//!
//! Unlike the simulation, this layer may read the clock: timeouts, discovery
//! intervals and round-trip measurement all need it. Nothing measured here may
//! ever reach the simulation — see `docs/02-simulation.md`.

use std::io;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};

use crate::protocol::{self, MAX_PACKET_BYTES, Packet};

/// A non-blocking UDP endpoint that speaks [`Packet`].
pub struct Transport {
    socket: UdpSocket,
    /// Scratch buffer, reused so the receive path does not allocate per packet.
    buffer: Vec<u8>,
    pub packets_sent: u64,
    pub packets_received: u64,
    /// Datagrams that arrived but were not ours — other applications' traffic
    /// on a shared network. Counted rather than logged, because on a busy LAN
    /// this is normal and constant.
    pub foreign_datagrams: u64,
}

impl Transport {
    /// Binds to a local address. Port 0 asks the operating system to pick one.
    pub fn bind(addr: SocketAddr) -> io::Result<Transport> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(Transport {
            socket,
            buffer: vec![0u8; MAX_PACKET_BYTES],
            packets_sent: 0,
            packets_received: 0,
            foreign_datagrams: 0,
        })
    }

    /// Binds to any interface on `port`.
    pub fn bind_port(port: u16) -> io::Result<Transport> {
        Transport::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)))
    }

    /// The address actually bound, including an operating-system-assigned port.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// Permits sending to the broadcast address. Required for LAN discovery.
    pub fn enable_broadcast(&self) -> io::Result<()> {
        self.socket.set_broadcast(true)
    }

    /// Sends one packet.
    ///
    /// # Errors
    /// Encoding failures are a bug in the caller — see
    /// [`protocol::encode`]. Send failures are usually transient (a full
    /// buffer, an unreachable host) and are worth retrying next tick rather
    /// than treating as fatal.
    pub fn send(&mut self, packet: &Packet, to: SocketAddr) -> Result<(), SendError> {
        let bytes = protocol::encode(packet).map_err(SendError::Encode)?;
        match self.socket.send_to(&bytes, to) {
            Ok(_) => {
                self.packets_sent += 1;
                Ok(())
            }
            // A full send buffer is backpressure, not a failure. The next tick
            // repeats these commands anyway, so dropping this one is harmless.
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(e) => Err(SendError::Io(e)),
        }
    }

    /// Sends the same packet to several peers.
    ///
    /// Failures are collected rather than returned early: one unreachable peer
    /// must not stop the packet reaching the others.
    pub fn broadcast_to(
        &mut self,
        packet: &Packet,
        peers: &[SocketAddr],
    ) -> Vec<(SocketAddr, SendError)> {
        let mut failures = Vec::new();
        for &peer in peers {
            if let Err(e) = self.send(packet, peer) {
                failures.push((peer, e));
            }
        }
        failures
    }

    /// Drains everything waiting on the socket.
    ///
    /// Returns only packets that parsed as ours. Anything else is counted in
    /// [`Transport::foreign_datagrams`] and discarded — a shared network
    /// carries plenty of traffic that is not ours, and that is not an error.
    ///
    /// `limit` caps how many datagrams one call will process, so a flood cannot
    /// stall the frame that calls it.
    pub fn poll(&mut self, limit: usize) -> Vec<(SocketAddr, Packet)> {
        let mut out = Vec::new();
        for _ in 0..limit {
            match self.socket.recv_from(&mut self.buffer) {
                Ok((len, from)) => {
                    self.packets_received += 1;
                    match protocol::decode(&self.buffer[..len]) {
                        Some(packet) => out.push((from, packet)),
                        None => self.foreign_datagrams += 1,
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                // A refused datagram on some platforms surfaces as an error on
                // the *next* receive. Skipping it rather than aborting keeps a
                // departed peer from stopping us reading from the others.
                Err(_) => continue,
            }
        }
        out
    }
}

#[derive(Debug)]
pub enum SendError {
    Encode(protocol::EncodeError),
    Io(io::Error),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::Encode(e) => write!(f, "could not encode packet: {e}"),
            SendError::Io(e) => write!(f, "could not send packet: {e}"),
        }
    }
}

impl std::error::Error for SendError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{PROTOCOL_VERSION, RejectReason};
    use redshift_sim::command::PlayerId;

    fn loopback(port: u16) -> SocketAddr {
        SocketAddr::from((Ipv4Addr::LOCALHOST, port))
    }

    /// Reads until something arrives or the attempts run out.
    ///
    /// Loopback delivery is near-instant but not synchronous, so a single poll
    /// races the kernel. Retrying briefly is the difference between a reliable
    /// test and one that fails now and then for no reason anybody can find.
    fn poll_until(transport: &mut Transport, attempts: u32) -> Vec<(SocketAddr, Packet)> {
        for _ in 0..attempts {
            let got = transport.poll(16);
            if !got.is_empty() {
                return got;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        Vec::new()
    }

    #[test]
    fn a_packet_survives_a_real_socket() {
        let mut sender = Transport::bind_port(0).expect("bind sender");
        let mut receiver = Transport::bind_port(0).expect("bind receiver");
        let to = loopback(receiver.local_addr().unwrap().port());

        let packet = Packet::JoinRequest {
            protocol: PROTOCOL_VERSION,
            rules_hash: 0xABCD,
            player_name: "mohsen".into(),
        };
        sender.send(&packet, to).expect("send");

        let received = poll_until(&mut receiver, 50);
        assert_eq!(received.len(), 1, "packet did not arrive");
        assert_eq!(received[0].1, packet);
        assert_eq!(sender.packets_sent, 1);
    }

    #[test]
    fn polling_an_idle_socket_returns_immediately() {
        // The socket is non-blocking, so the game loop can poll every frame
        // without stalling. If this ever blocked, the whole client would.
        let mut transport = Transport::bind_port(0).expect("bind");
        let started = std::time::Instant::now();
        for _ in 0..1000 {
            assert!(transport.poll(16).is_empty());
        }
        assert!(started.elapsed().as_millis() < 500, "polling blocked");
    }

    #[test]
    fn foreign_traffic_is_counted_and_discarded() {
        // Another application's broadcast must not become a game packet.
        let mut receiver = Transport::bind_port(0).expect("bind receiver");
        let port = receiver.local_addr().unwrap().port();

        let raw = UdpSocket::bind(loopback(0)).expect("bind raw");
        raw.send_to(b"GET / HTTP/1.1\r\n\r\n", loopback(port))
            .expect("send junk");
        raw.send_to(&[0xFF; 40], loopback(port)).expect("send junk");

        for _ in 0..50 {
            receiver.poll(16);
            if receiver.foreign_datagrams >= 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(receiver.foreign_datagrams, 2);
        assert!(
            receiver.poll(16).is_empty(),
            "junk must not surface as packets"
        );
    }

    #[test]
    fn the_poll_limit_is_respected() {
        // A flood must not stall the frame that polls.
        let mut sender = Transport::bind_port(0).expect("bind sender");
        let mut receiver = Transport::bind_port(0).expect("bind receiver");
        let to = loopback(receiver.local_addr().unwrap().port());

        for i in 0..40 {
            sender.send(&Packet::Ping { nonce: i }, to).expect("send");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));

        let first = receiver.poll(5);
        assert!(
            first.len() <= 5,
            "poll returned {} packets for a limit of 5",
            first.len()
        );
    }

    #[test]
    fn an_oversized_packet_is_refused_before_it_reaches_the_socket() {
        use crate::protocol::TurnCommands;
        use redshift_sim::EntityId;
        use redshift_sim::command::{Command, CommandKind};
        use redshift_sim::map::Cell;

        let mut sender = Transport::bind_port(0).expect("bind");
        let commands: Vec<Command> = (0..40)
            .map(|i| {
                Command::new(
                    PlayerId(0),
                    i,
                    CommandKind::Move {
                        units: vec![EntityId::NONE; 200],
                        target: Cell::new(1, 1),
                    },
                )
            })
            .collect();
        let packet = Packet::Turn {
            tick: 1,
            turns: vec![TurnCommands {
                tick: 1,
                player: PlayerId(0),
                commands,
            }],
            hash: None,
        };

        match sender.send(&packet, loopback(9)) {
            Err(SendError::Encode(_)) => {}
            other => panic!("expected an encode refusal, got {other:?}"),
        }
        assert_eq!(sender.packets_sent, 0);
    }

    #[test]
    fn sending_to_several_peers_survives_one_bad_address() {
        // One unreachable peer must not stop the packet reaching the others.
        let mut sender = Transport::bind_port(0).expect("bind sender");
        let mut a = Transport::bind_port(0).expect("bind a");
        let mut b = Transport::bind_port(0).expect("bind b");

        let peers = vec![
            loopback(a.local_addr().unwrap().port()),
            loopback(b.local_addr().unwrap().port()),
        ];
        let packet = Packet::JoinRejected {
            reason: RejectReason::MatchFull,
        };
        let failures = sender.broadcast_to(&packet, &peers);
        assert!(
            failures.is_empty(),
            "loopback sends should succeed: {failures:?}"
        );

        assert_eq!(poll_until(&mut a, 50).len(), 1);
        assert_eq!(poll_until(&mut b, 50).len(), 1);
    }

    #[test]
    fn two_transports_can_talk_both_ways() {
        let mut a = Transport::bind_port(0).expect("bind a");
        let mut b = Transport::bind_port(0).expect("bind b");
        let a_addr = loopback(a.local_addr().unwrap().port());
        let b_addr = loopback(b.local_addr().unwrap().port());

        a.send(&Packet::Ping { nonce: 42 }, b_addr).expect("ping");
        let got = poll_until(&mut b, 50);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].1, Packet::Ping { nonce: 42 });

        b.send(&Packet::Pong { nonce: 42 }, a_addr).expect("pong");
        let got = poll_until(&mut a, 50);
        assert_eq!(got[0].1, Packet::Pong { nonce: 42 });
    }
}
