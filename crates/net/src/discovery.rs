//! Finding matches on the local network.
//!
//! Zero configuration is the requirement: two machines on the same Wi-Fi should
//! simply see each other, with no addresses typed and no ports forwarded.
//!
//! # Why plain UDP broadcast
//!
//! A host broadcasts a small announcement a few times a second; clients listen
//! and keep a list. That is the whole mechanism.
//!
//! mDNS/Bonjour would be the "proper" answer and is a great deal more machinery
//! — service registration, conflict resolution, and platform-specific
//! behaviour to debug on three operating systems. Broadcast is a dozen lines,
//! behaves identically everywhere, and a LAN game is exactly the case broadcast
//! was designed for.
//!
//! # Entries expire
//!
//! A host that quits stops announcing; it does not get to send a farewell,
//! because the packet carrying it might be the one that gets lost. So entries
//! age out. This is why the module reads the clock — permitted here, forbidden
//! in the simulation.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use crate::protocol::{Announce, DISCOVERY_PORT, PROTOCOL_VERSION, Packet};
use crate::transport::Transport;

/// How often a host announces itself.
///
/// Twice a second: fast enough that a match appears in the list almost
/// immediately, slow enough to be invisible on any network.
pub const ANNOUNCE_INTERVAL: Duration = Duration::from_millis(500);

/// How long an entry survives without being heard from again.
///
/// Four missed announcements. Long enough to ride out ordinary loss, short
/// enough that a host which has quit disappears while the player is still
/// looking at the list.
pub const ENTRY_TIMEOUT: Duration = Duration::from_millis(2_000);

/// A match seen on the network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredMatch {
    pub address: SocketAddr,
    pub announce: Announce,
    /// When the most recent announcement arrived.
    pub last_seen: Instant,
}

impl DiscoveredMatch {
    pub fn is_joinable(&self) -> bool {
        self.announce.protocol == PROTOCOL_VERSION
            && !self.announce.in_progress
            && self.announce.players < self.announce.max_players
    }

    /// Why this match cannot be joined, if it cannot.
    pub fn unjoinable_reason(&self) -> Option<&'static str> {
        if self.announce.protocol != PROTOCOL_VERSION {
            Some("different game version")
        } else if self.announce.in_progress {
            Some("already started")
        } else if self.announce.players >= self.announce.max_players {
            Some("full")
        } else {
            None
        }
    }
}

/// Broadcasts a match so clients can find it.
pub struct Announcer {
    transport: Transport,
    target: SocketAddr,
    last_sent: Option<Instant>,
    pub announcements_sent: u64,
}

impl Announcer {
    /// Creates an announcer broadcasting to the local network.
    pub fn new() -> std::io::Result<Announcer> {
        Announcer::with_target(SocketAddr::from((Ipv4Addr::BROADCAST, DISCOVERY_PORT)))
    }

    /// Creates an announcer aimed at a specific address.
    ///
    /// Used by tests to target loopback, and available for a player who needs
    /// to reach a host that broadcast cannot cross — a subnet boundary, or a
    /// network that filters broadcast traffic.
    pub fn with_target(target: SocketAddr) -> std::io::Result<Announcer> {
        let transport = Transport::bind_port(0)?;
        // Harmless when the target is a unicast address, and required when it
        // is not.
        let _ = transport.enable_broadcast();
        Ok(Announcer {
            transport,
            target,
            last_sent: None,
            announcements_sent: 0,
        })
    }

    /// Sends an announcement if enough time has passed.
    ///
    /// Call every frame; the interval is enforced here rather than by the
    /// caller, so the rate cannot drift with the frame rate.
    pub fn tick(&mut self, announce: &Announce) -> bool {
        let now = Instant::now();
        if self
            .last_sent
            .is_some_and(|last| now.duration_since(last) < ANNOUNCE_INTERVAL)
        {
            return false;
        }
        self.last_sent = Some(now);
        if self
            .transport
            .send(&Packet::Announce(announce.clone()), self.target)
            .is_ok()
        {
            self.announcements_sent += 1;
            return true;
        }
        false
    }
}

/// Listens for announcements and keeps a live list.
pub struct Discovery {
    transport: Transport,
    /// Keyed by address, so a host re-announcing updates its entry rather than
    /// appearing twice. `BTreeMap` keeps the listing order stable — a list that
    /// reshuffles under the pointer is maddening to click.
    seen: BTreeMap<SocketAddr, DiscoveredMatch>,
}

impl Discovery {
    /// Binds to the discovery port and starts listening.
    pub fn new() -> std::io::Result<Discovery> {
        Discovery::on_port(DISCOVERY_PORT)
    }

    pub fn on_port(port: u16) -> std::io::Result<Discovery> {
        Ok(Discovery {
            transport: Transport::bind_port(port)?,
            seen: BTreeMap::new(),
        })
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.transport.local_addr()
    }

    /// Reads waiting announcements and drops stale entries.
    ///
    /// Call every frame. Returns how many new matches appeared, so the caller
    /// can play a sound or draw attention to the list.
    pub fn tick(&mut self) -> usize {
        let now = Instant::now();
        let mut discovered = 0;

        for (from, packet) in self.transport.poll(32) {
            let Packet::Announce(announce) = packet else {
                continue;
            };

            // Reply to the address the announcement came from, but on the port
            // the host says it is listening on — the announcement was sent from
            // an ephemeral port, not the game port.
            let address = SocketAddr::new(from.ip(), announce.game_port);

            let entry = DiscoveredMatch {
                address,
                announce,
                last_seen: now,
            };
            if self.seen.insert(address, entry).is_none() {
                discovered += 1;
            }
        }

        self.seen
            .retain(|_, m| now.duration_since(m.last_seen) < ENTRY_TIMEOUT);
        discovered
    }

    /// Matches currently visible, in a stable order.
    pub fn matches(&self) -> Vec<&DiscoveredMatch> {
        self.seen.values().collect()
    }

    /// Only the matches that can actually be joined.
    pub fn joinable(&self) -> Vec<&DiscoveredMatch> {
        self.seen.values().filter(|m| m.is_joinable()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }

    /// Removes every entry. Used when leaving the browser.
    pub fn clear(&mut self) {
        self.seen.clear();
    }
}

/// A best-effort guess at this machine's address on the local network.
///
/// Opens a UDP socket towards a public address and asks the operating system
/// which local interface it would use. No packet is sent — UDP `connect` only
/// sets a default destination — so this works with no network at all, and does
/// not depend on the address being reachable.
pub fn local_ip() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(203, 0, 113, 1), 9)).ok()?;
    socket.local_addr().ok().map(|a| a.ip())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_announce(players: u8) -> Announce {
        Announce {
            protocol: PROTOCOL_VERSION,
            match_name: "mohsen's game".into(),
            map_name: "crossroads".into(),
            players,
            max_players: 2,
            game_port: 47655,
            in_progress: false,
        }
    }

    /// Pumps discovery until it sees something, or the attempts run out.
    fn discover_until(discovery: &mut Discovery, attempts: u32) -> usize {
        for _ in 0..attempts {
            discovery.tick();
            if !discovery.is_empty() {
                return discovery.matches().len();
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        0
    }

    #[test]
    fn a_host_is_found_over_a_real_socket() {
        let mut discovery = Discovery::on_port(0).expect("bind discovery");
        let port = discovery.local_addr().unwrap().port();
        let mut announcer = Announcer::with_target(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
            .expect("announcer");

        assert!(
            announcer.tick(&sample_announce(1)),
            "the first tick should send"
        );
        assert_eq!(
            discover_until(&mut discovery, 100),
            1,
            "the host was not discovered"
        );

        let found = discovery.matches()[0].clone();
        assert_eq!(found.announce.match_name, "mohsen's game");
        assert_eq!(
            found.address.port(),
            47655,
            "the advertised game port must be used"
        );
        assert!(found.is_joinable());
    }

    #[test]
    fn announcements_are_rate_limited() {
        // Enforced inside `tick` rather than by the caller, so the rate cannot
        // drift with the frame rate.
        let mut announcer =
            Announcer::with_target(SocketAddr::from((Ipv4Addr::LOCALHOST, 9))).expect("announcer");
        let announce = sample_announce(1);

        assert!(announcer.tick(&announce));
        for _ in 0..1000 {
            assert!(!announcer.tick(&announce), "should not send again so soon");
        }
        assert_eq!(announcer.announcements_sent, 1);

        std::thread::sleep(ANNOUNCE_INTERVAL + Duration::from_millis(50));
        assert!(
            announcer.tick(&announce),
            "should send again after the interval"
        );
        assert_eq!(announcer.announcements_sent, 2);
    }

    #[test]
    fn a_host_that_stops_announcing_disappears() {
        // A host that quits gets no chance to say goodbye — the packet carrying
        // it might be the one that is lost. Entries must age out on their own.
        let mut discovery = Discovery::on_port(0).expect("bind discovery");
        let port = discovery.local_addr().unwrap().port();
        let mut announcer = Announcer::with_target(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
            .expect("announcer");

        announcer.tick(&sample_announce(1));
        assert_eq!(discover_until(&mut discovery, 100), 1);

        // Stop announcing and let the entry expire.
        std::thread::sleep(ENTRY_TIMEOUT + Duration::from_millis(100));
        discovery.tick();
        assert!(discovery.is_empty(), "a silent host should have expired");
    }

    #[test]
    fn re_announcing_updates_rather_than_duplicates() {
        let mut discovery = Discovery::on_port(0).expect("bind discovery");
        let port = discovery.local_addr().unwrap().port();
        let mut announcer = Announcer::with_target(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
            .expect("announcer");

        announcer.tick(&sample_announce(1));
        assert_eq!(discover_until(&mut discovery, 100), 1);

        std::thread::sleep(ANNOUNCE_INTERVAL + Duration::from_millis(50));
        announcer.tick(&sample_announce(2));
        for _ in 0..100 {
            discovery.tick();
            if discovery.matches()[0].announce.players == 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        assert_eq!(
            discovery.matches().len(),
            1,
            "the host must not appear twice"
        );
        assert_eq!(
            discovery.matches()[0].announce.players,
            2,
            "the entry should have updated"
        );
    }

    #[test]
    fn unjoinable_matches_are_listed_but_explained() {
        // Showing a full or mismatched match with a reason is far better than
        // hiding it — otherwise the player's friend's game is simply missing,
        // with nothing to explain why.
        let now = Instant::now();
        let cases = [
            (
                Announce {
                    in_progress: true,
                    ..sample_announce(1)
                },
                "already started",
            ),
            (
                Announce {
                    players: 2,
                    ..sample_announce(2)
                },
                "full",
            ),
            (
                Announce {
                    protocol: PROTOCOL_VERSION + 1,
                    ..sample_announce(1)
                },
                "different game version",
            ),
        ];
        for (announce, expected) in cases {
            let entry = DiscoveredMatch {
                address: SocketAddr::from((Ipv4Addr::LOCALHOST, 47655)),
                announce,
                last_seen: now,
            };
            assert!(!entry.is_joinable());
            assert_eq!(entry.unjoinable_reason(), Some(expected));
        }

        let open = DiscoveredMatch {
            address: SocketAddr::from((Ipv4Addr::LOCALHOST, 47655)),
            announce: sample_announce(1),
            last_seen: now,
        };
        assert!(open.is_joinable());
        assert_eq!(open.unjoinable_reason(), None);
    }

    #[test]
    fn foreign_traffic_does_not_appear_as_a_match() {
        let mut discovery = Discovery::on_port(0).expect("bind discovery");
        let port = discovery.local_addr().unwrap().port();

        let raw = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("raw socket");
        for _ in 0..5 {
            raw.send_to(b"some other protocol", (Ipv4Addr::LOCALHOST, port))
                .ok();
        }
        for _ in 0..30 {
            discovery.tick();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(discovery.is_empty(), "junk must not become a match listing");
    }

    #[test]
    fn local_ip_is_available_without_sending_anything() {
        // Used to show the player their own address. Must work even with no
        // route to the internet, since a LAN game may have none.
        // A sandbox with no interfaces at all is a legitimate outcome, so the
        // absence of an address is not a failure — a nonsense address would be.
        if let Some(ip) = local_ip() {
            assert!(!ip.is_unspecified(), "got an unspecified address");
        }
    }
}
