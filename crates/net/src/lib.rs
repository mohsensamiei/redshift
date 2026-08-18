//! # redshift-net
//!
//! Getting player commands between peers, in an order every peer agrees on.
//!
//! ## What this crate is responsible for
//!
//! - **Turn scheduling** ([`lockstep`]) — deciding when a tick may run, and
//!   refusing to run it early.
//! - **The wire format** ([`protocol`]) — what peers say to each other.
//! - **Desync detection** — comparing state hashes and halting on a mismatch.
//!
//! ## What it is deliberately not responsible for
//!
//! Game rules. This crate moves opaque commands and compares opaque hashes; it
//! has no idea what a unit is. That is what lets the relay server be a packet
//! switch rather than an authority — it cannot desync a match, because it holds
//! no game state.
//!
//! ## Layering
//!
//! [`lockstep`] is transport-free on purpose. It decides *what may run when*
//! and is driven by whatever moves bytes: a UDP socket, a relay connection, a
//! replay file, or a test harness. So the scheduling rule — the part that is
//! subtle and the part that desyncs matches when it is wrong — is fully
//! testable without opening a socket.
//!
//! See `docs/03-networking.md`.

pub mod lockstep;
pub mod protocol;

pub use lockstep::{DesyncReport, TurnScheduler, TurnStatus, input_delay_for_rtt};
pub use protocol::{PROTOCOL_VERSION, Packet, decode, encode};
