//! Recording and replaying matches.
//!
//! A replay is the match seed plus the command log. That is all — a full match
//! is a few kilobytes, because lockstep already requires that those two things
//! determine everything else.
//!
//! # Why this is built in Phase 1 rather than later
//!
//! It is not a feature for players. It is the primary debugging tool for the
//! rest of the project: when two peers diverge, the command log reproduces the
//! divergence offline, on one machine, as many times as needed. Without it,
//! a desync is a story about something that happened once.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use redshift_sim::Tick;
use redshift_sim::command::Command;

use crate::protocol::PROTOCOL_VERSION;

/// Bumped when the replay file layout changes.
pub const REPLAY_FORMAT_VERSION: u32 = 1;

/// A recorded match.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Replay {
    pub format_version: u32,
    /// The protocol — and therefore simulation behaviour — this was recorded
    /// under. A replay from a different build will not reproduce, so it is
    /// refused rather than silently producing a different match.
    pub protocol_version: u32,
    pub seed: u64,
    /// Identifies the map and starting units. Checked on load for the same
    /// reason as the protocol version.
    pub setup_hash: u64,
    pub player_count: u8,
    /// Commands as executed, one entry per tick that ran. Ticks with no
    /// commands are recorded too, so the log's length is the match length.
    pub turns: Vec<Vec<Command>>,
}

impl Replay {
    pub fn new(seed: u64, setup_hash: u64, player_count: u8) -> Replay {
        Replay {
            format_version: REPLAY_FORMAT_VERSION,
            protocol_version: PROTOCOL_VERSION,
            seed,
            setup_hash,
            player_count,
            turns: Vec::new(),
        }
    }

    /// Records the commands executed for one tick.
    pub fn record(&mut self, commands: &[Command]) {
        self.turns.push(commands.to_vec());
    }

    /// Ticks recorded.
    pub fn length(&self) -> Tick {
        self.turns.len() as Tick
    }

    pub fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }

    /// Commands for a tick, or `None` past the end of the recording.
    pub fn commands_at(&self, tick: Tick) -> Option<&[Command]> {
        self.turns.get(tick as usize).map(|v| v.as_slice())
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, text)
    }

    pub fn load(path: &Path) -> Result<Replay, ReplayError> {
        let text = std::fs::read_to_string(path).map_err(ReplayError::Io)?;
        let replay: Replay = ron::from_str(&text).map_err(|e| ReplayError::Parse(e.to_string()))?;

        if replay.format_version != REPLAY_FORMAT_VERSION {
            return Err(ReplayError::FormatMismatch {
                found: replay.format_version,
                expected: REPLAY_FORMAT_VERSION,
            });
        }
        // A replay from a different build will not reproduce. Refusing is far
        // better than playing back something that merely looks plausible.
        if replay.protocol_version != PROTOCOL_VERSION {
            return Err(ReplayError::ProtocolMismatch {
                found: replay.protocol_version,
                expected: PROTOCOL_VERSION,
            });
        }
        Ok(replay)
    }
}

#[derive(Debug)]
pub enum ReplayError {
    Io(io::Error),
    Parse(String),
    FormatMismatch { found: u32, expected: u32 },
    ProtocolMismatch { found: u32, expected: u32 },
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayError::Io(e) => write!(f, "could not read the replay: {e}"),
            ReplayError::Parse(e) => write!(f, "the replay file is malformed: {e}"),
            ReplayError::FormatMismatch { found, expected } => write!(
                f,
                "replay format {found} cannot be read by this build, which expects {expected}"
            ),
            ReplayError::ProtocolMismatch { found, expected } => write!(
                f,
                "this replay was recorded by protocol {found}; this build is {expected}, and \
                 would simulate it differently"
            ),
        }
    }
}

impl std::error::Error for ReplayError {}

#[cfg(test)]
mod tests {
    use super::*;
    use redshift_sim::command::{CommandKind, PlayerId};
    use redshift_sim::map::Cell;

    fn sample() -> Replay {
        let mut replay = Replay::new(0xABCD, 0x1234, 2);
        for tick in 0..50u16 {
            if tick % 10 == 0 {
                replay.record(&[Command::new(
                    PlayerId((tick % 2) as u8),
                    0,
                    CommandKind::Move {
                        units: Vec::new(),
                        target: Cell::new(tick as i32, 5),
                    },
                )]);
            } else {
                replay.record(&[]);
            }
        }
        replay
    }

    #[test]
    fn every_tick_is_recorded_including_empty_ones() {
        // The log's length is the match length, so idle ticks must be present.
        // Recording only ticks with commands would lose the timing entirely.
        let replay = sample();
        assert_eq!(replay.length(), 50);
        assert!(replay.commands_at(1).unwrap().is_empty());
        assert_eq!(replay.commands_at(10).unwrap().len(), 1);
        assert!(
            replay.commands_at(50).is_none(),
            "past the end is None, not empty"
        );
    }

    #[test]
    fn a_replay_roundtrips_through_a_file() {
        let dir = std::env::temp_dir().join("redshift-replay-test");
        let path = dir.join("match.replay.ron");
        let _ = std::fs::remove_file(&path);

        let replay = sample();
        replay.save(&path).expect("save");
        let loaded = Replay::load(&path).expect("load");

        assert_eq!(loaded.seed, replay.seed);
        assert_eq!(loaded.setup_hash, replay.setup_hash);
        assert_eq!(loaded.length(), replay.length());
        for tick in 0..replay.length() {
            assert_eq!(
                loaded.commands_at(tick),
                replay.commands_at(tick),
                "tick {tick}"
            );
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_whole_match_is_small() {
        // The claim in docs/03-networking.md: a full match is a few kilobytes.
        // Worth checking, since it is the reason replays can be attached to
        // every bug report.
        let mut replay = Replay::new(1, 2, 2);
        // Twenty minutes at 20 Hz, with an order every second.
        for tick in 0..24_000u32 {
            if tick.is_multiple_of(20) {
                replay.record(&[Command::new(
                    PlayerId(0),
                    0,
                    CommandKind::Move {
                        units: Vec::new(),
                        target: Cell::new(10, 10),
                    },
                )]);
            } else {
                replay.record(&[]);
            }
        }
        let encoded =
            bincode::serde::encode_to_vec(&replay, bincode::config::standard()).expect("encode");
        assert!(
            encoded.len() < 200_000,
            "a twenty-minute match encoded to {} bytes",
            encoded.len()
        );
    }

    #[test]
    fn a_replay_from_another_build_is_refused() {
        // It would not reproduce. Refusing is far better than playing back
        // something that merely looks plausible.
        let dir = std::env::temp_dir().join("redshift-replay-test");
        let path = dir.join("stale.replay.ron");
        let _ = std::fs::remove_file(&path);

        let mut replay = sample();
        replay.protocol_version = PROTOCOL_VERSION + 7;
        replay.save(&path).expect("save");

        match Replay::load(&path) {
            Err(ReplayError::ProtocolMismatch { found, expected }) => {
                assert_eq!(found, PROTOCOL_VERSION + 7);
                assert_eq!(expected, PROTOCOL_VERSION);
            }
            other => panic!("expected a protocol refusal, got {other:?}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_malformed_file_is_reported_not_ignored() {
        let dir = std::env::temp_dir().join("redshift-replay-test");
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join("broken.replay.ron");
        std::fs::write(&path, "this is not a replay").expect("write");

        match Replay::load(&path) {
            Err(ReplayError::Parse(_)) => {}
            other => panic!("expected a parse error, got {other:?}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn errors_read_as_sentences() {
        let cases: Vec<ReplayError> = vec![
            ReplayError::FormatMismatch {
                found: 2,
                expected: 1,
            },
            ReplayError::ProtocolMismatch {
                found: 9,
                expected: 1,
            },
            ReplayError::Parse("bad token".into()),
        ];
        for error in cases {
            let text = error.to_string();
            assert!(text.len() > 20, "unhelpfully terse: {text}");
        }
    }
}
