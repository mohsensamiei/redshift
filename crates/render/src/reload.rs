//! Restarting the match when the rules or the map change on disk.
//!
//! Note the word: *restarting*, not patching. Rules feed the stat table, the
//! combat table and the rules hash, and that hash is part of what two peers
//! agree on. Swapping values into a running simulation would desync a
//! multiplayer match on the next comparison, and would leave a single-player
//! one in a state no replay could reproduce — units mid-order under stats they
//! were not built with.
//!
//! So the whole match starts again from the same seed. For the thing this is
//! actually for — turning a number in a RON file and seeing what it does — that
//! is not a loss. You are iterating on values, not on a battle.
//!
//! Off unless asked for, and refused outright in a networked match, where one
//! peer restarting is exactly the divergence the desync detector exists to
//! catch.

use std::path::PathBuf;
use std::time::SystemTime;

use bevy::prelude::*;

/// Files being watched, and what they looked like last time.
#[derive(Resource)]
pub struct RulesWatch {
    pub roots: Vec<PathBuf>,
    /// The newest modification time seen across every watched file.
    newest: Option<SystemTime>,
    /// Frames between checks. Stat-ing a directory tree every frame would be a
    /// silly thing to do sixty times a second for a file that changes when
    /// somebody saves an editor.
    countdown: u32,
}

/// How often to look, in frames. Twice a second is faster than anyone can
/// notice and slower than anything that matters.
const CHECK_EVERY: u32 = 30;

impl RulesWatch {
    pub fn new(roots: Vec<PathBuf>) -> RulesWatch {
        let mut watch = RulesWatch {
            roots,
            newest: None,
            countdown: CHECK_EVERY,
        };
        watch.newest = watch.scan();
        watch
    }

    /// The newest modification time under any watched root.
    ///
    /// A single timestamp rather than a per-file map: the question is only
    /// "has anything changed", and the answer moves forward whenever any file
    /// is saved. Deleting a file makes the newest time go *backwards*, which
    /// also reads as a change, which is correct.
    fn scan(&self) -> Option<SystemTime> {
        fn walk(path: &std::path::Path, newest: &mut Option<SystemTime>) {
            let Ok(entries) = std::fs::read_dir(path) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, newest);
                } else if let Ok(modified) = entry.metadata().and_then(|m| m.modified())
                    && newest.is_none_or(|n| modified > n)
                {
                    *newest = Some(modified);
                }
            }
        }
        let mut newest = None;
        for root in &self.roots {
            if root.is_dir() {
                walk(root, &mut newest);
            } else if let Ok(modified) = std::fs::metadata(root).and_then(|m| m.modified()) {
                if newest.is_none_or(|n| modified > n) {
                    newest = Some(modified);
                }
            }
        }
        newest
    }

    /// Whether anything has changed since the last time this said so.
    pub fn changed(&mut self) -> bool {
        if self.countdown > 0 {
            self.countdown -= 1;
            return false;
        }
        self.countdown = CHECK_EVERY;
        let now = self.scan();
        if now != self.newest {
            self.newest = now;
            return true;
        }
        false
    }
}

/// Asks the host to rebuild the match. Raised rather than acted on here,
/// because the renderer does not know how a session is constructed — the
/// client does, and it is the one that read the files in the first place.
#[derive(Message, Default)]
pub struct RulesChanged;

pub fn watch_rules(
    watch: Option<ResMut<RulesWatch>>,
    session: Res<crate::session::Session>,
    mut changed: MessageWriter<RulesChanged>,
) {
    let Some(mut watch) = watch else { return };
    // A networked match is exactly where one peer quietly restarting is the
    // divergence everything else is built to detect.
    if session.is_networked() {
        return;
    }
    if watch.changed() {
        info!("rules or map changed on disk — restarting the match");
        changed.write(RulesChanged);
    }
}
