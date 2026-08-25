//! What each player can see.
//!
//! # Two layers, not one
//!
//! The original distinguished ground you have never seen from ground you have
//! seen and are not currently watching, and the difference matters to play. The
//! first hides the terrain itself; the second shows the terrain you remember
//! but not what is standing on it. Collapsing them into "visible or not" would
//! lose the whole idea of scouting: a map you have explored would go black
//! again the moment you left.
//!
//! # This is simulation state, not presentation
//!
//! It would be tempting to keep visibility in the renderer, since it is about
//! what a player sees. It cannot live there: units only acquire targets they
//! can see, so visibility changes who shoots whom. That makes it part of the
//! simulation, subject to every determinism rule, and part of the state hash.
//!
//! # Cost
//!
//! Recomputed from scratch each tick rather than updated as units move.
//! Incremental updates would mean clearing an old vision circle and stamping a
//! new one on every step of every unit, and a single missed clear leaves a
//! permanently visible patch that nobody can explain. Explored ground is
//! cumulative and never cleared, so only the *visible* layer is rebuilt.

use serde::{Deserialize, Serialize};

use crate::command::PlayerId;
use crate::fx::Fx;
use crate::hash::{StateHash, StateHasher};
use crate::map::Cell;

/// How much of a cell a player knows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sight {
    /// Never seen. The terrain itself is hidden.
    Unseen,
    /// Seen before, not currently watched. Terrain is remembered; what stands
    /// on it is not.
    Fogged,
    /// Currently watched.
    Visible,
}

/// Per-player knowledge of the map.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Visibility {
    width: u16,
    height: u16,
    /// Cumulative: once seen, a cell stays explored for the rest of the match.
    explored: Vec<Vec<bool>>,
    /// Rebuilt every tick from what is standing.
    visible: Vec<Vec<bool>>,
    /// Ground a detector is watching, per player.
    ///
    /// A separate layer from `visible` because they answer different questions:
    /// a player can see a patch of ground perfectly well and still not see the
    /// cloaked unit standing on it.
    detected: Vec<Vec<bool>>,
    /// Water a sonar is listening to, per player.
    ///
    /// A third layer rather than a second use of `detected`, because they are
    /// different senses: a dog that can smell a spy standing in front of it has
    /// no reason to hear a submarine. One layer answering both would make the
    /// two concealments the same concealment.
    sonar: Vec<Vec<bool>>,
    /// When false, everything reads as visible.
    ///
    /// Not a debug convenience: a replay being watched after the fact, or a
    /// spectator, should see the whole map, and switching it off is cleaner
    /// than teaching every caller to ask whether fog applies.
    enabled: bool,
}

impl Visibility {
    pub fn new(width: u16, height: u16, players: usize) -> Visibility {
        let cells = width as usize * height as usize;
        Visibility {
            width,
            height,
            explored: vec![vec![false; cells]; players],
            visible: vec![vec![false; cells]; players],
            detected: vec![vec![false; cells]; players],
            sonar: vec![vec![false; cells]; players],
            enabled: true,
        }
    }

    /// Turns fog off, revealing everything to everyone.
    pub fn reveal_all(&mut self) {
        self.enabled = false;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[inline]
    fn index(&self, cell: Cell) -> Option<usize> {
        if cell.x < 0 || cell.y < 0 || cell.x >= self.width as i32 || cell.y >= self.height as i32 {
            return None;
        }
        Some(cell.y as usize * self.width as usize + cell.x as usize)
    }

    /// Clears the visible layer. Explored ground is left alone.
    pub fn begin_tick(&mut self) {
        for row in &mut self.visible {
            row.iter_mut().for_each(|v| *v = false);
        }
        for row in &mut self.detected {
            row.iter_mut().for_each(|v| *v = false);
        }
        for row in &mut self.sonar {
            row.iter_mut().for_each(|v| *v = false);
        }
    }

    /// Takes ground back from a player: not visible, and not explored either.
    ///
    /// The only subtractive operation here, and the reason it exists at all is
    /// the Gap Generator — a structure that reveals nothing for its owner and
    /// *hides* ground from everyone else. Every other source of visibility
    /// adds, and `explored` is otherwise cumulative for the whole match.
    ///
    /// Clearing `explored` as well as `visible` is the point. Leaving the
    /// explored layer alone would hide live units and leave the terrain and the
    /// buildings on it drawn exactly where they were, which is no concealment
    /// at all.
    pub fn hide(&mut self, player: PlayerId, centre: Cell, radius: Fx) {
        let Some(explored) = self.explored.get_mut(player.0 as usize) else {
            return;
        };
        let Some(visible) = self.visible.get_mut(player.0 as usize) else {
            return;
        };
        let cells = radius.to_int().max(0);
        let radius_sq = radius.sq();
        for dy in -cells..=cells {
            for dx in -cells..=cells {
                let cell = Cell::new(centre.x + dx, centre.y + dy);
                if Fx::dist_sq(Fx::from_int(dx), Fx::from_int(dy)) > radius_sq {
                    continue;
                }
                if cell.x < 0
                    || cell.y < 0
                    || cell.x >= self.width as i32
                    || cell.y >= self.height as i32
                {
                    continue;
                }
                let i = cell.y as usize * self.width as usize + cell.x as usize;
                explored[i] = false;
                visible[i] = false;
            }
        }
    }

    /// Marks everything within `radius` of `centre` as seen by `player`.
    ///
    /// A circle, not a square: a square would let a unit see further along the
    /// diagonal than straight ahead, which is visible to a player as a
    /// diamond-shaped hole in the fog.
    pub fn reveal(&mut self, player: PlayerId, centre: Cell, radius: Fx) {
        let Some(explored) = self.explored.get_mut(player.0 as usize) else {
            return;
        };
        let Some(visible) = self.visible.get_mut(player.0 as usize) else {
            return;
        };

        let cells = radius.to_int().max(0);
        let radius_sq = radius.sq();
        for dy in -cells..=cells {
            for dx in -cells..=cells {
                let cell = Cell::new(centre.x + dx, centre.y + dy);
                if cell.x < 0
                    || cell.y < 0
                    || cell.x >= self.width as i32
                    || cell.y >= self.height as i32
                {
                    continue;
                }
                if Fx::dist_sq(Fx::from_int(dx), Fx::from_int(dy)) > radius_sq {
                    continue;
                }
                let i = cell.y as usize * self.width as usize + cell.x as usize;
                visible[i] = true;
                explored[i] = true;
            }
        }
    }

    /// Marks everything within `radius` of `centre` as watched by a detector.
    pub fn reveal_cloaked(&mut self, player: PlayerId, centre: Cell, radius: Fx) {
        let Some(detected) = self.detected.get_mut(player.0 as usize) else {
            return;
        };
        let cells = radius.to_int().max(0);
        let radius_sq = radius.sq();
        for dy in -cells..=cells {
            for dx in -cells..=cells {
                let cell = Cell::new(centre.x + dx, centre.y + dy);
                if cell.x < 0
                    || cell.y < 0
                    || cell.x >= self.width as i32
                    || cell.y >= self.height as i32
                {
                    continue;
                }
                if Fx::dist_sq(Fx::from_int(dx), Fx::from_int(dy)) > radius_sq {
                    continue;
                }
                detected[cell.y as usize * self.width as usize + cell.x as usize] = true;
            }
        }
    }

    /// Whether a player has a detector watching this cell.
    /// Marks water within `radius` as being listened to by `player`.
    pub fn listen(&mut self, player: PlayerId, centre: Cell, radius: Fx) {
        let Some(sonar) = self.sonar.get_mut(player.0 as usize) else {
            return;
        };
        let cells = radius.to_int().max(0);
        let radius_sq = radius.sq();
        for dy in -cells..=cells {
            for dx in -cells..=cells {
                let cell = Cell::new(centre.x + dx, centre.y + dy);
                if Fx::dist_sq(Fx::from_int(dx), Fx::from_int(dy)) > radius_sq {
                    continue;
                }
                if cell.x < 0
                    || cell.y < 0
                    || cell.x >= self.width as i32
                    || cell.y >= self.height as i32
                {
                    continue;
                }
                sonar[cell.y as usize * self.width as usize + cell.x as usize] = true;
            }
        }
    }

    /// Whether a sonar is listening to this cell.
    pub fn is_heard(&self, player: PlayerId, cell: Cell) -> bool {
        if !self.enabled {
            return true;
        }
        let Some(i) = self.index(cell) else {
            return false;
        };
        self.sonar.get(player.0 as usize).is_some_and(|d| d[i])
    }

    pub fn is_detected(&self, player: PlayerId, cell: Cell) -> bool {
        if !self.enabled {
            return true;
        }
        let Some(i) = self.index(cell) else {
            return false;
        };
        self.detected.get(player.0 as usize).is_some_and(|d| d[i])
    }

    /// How much a player knows about a cell.
    pub fn sight(&self, player: PlayerId, cell: Cell) -> Sight {
        if !self.enabled {
            return Sight::Visible;
        }
        let Some(i) = self.index(cell) else {
            return Sight::Unseen;
        };
        let p = player.0 as usize;
        if self.visible.get(p).is_some_and(|v| v[i]) {
            Sight::Visible
        } else if self.explored.get(p).is_some_and(|e| e[i]) {
            Sight::Fogged
        } else {
            Sight::Unseen
        }
    }

    /// Whether a player is currently watching a cell.
    #[inline]
    pub fn is_visible(&self, player: PlayerId, cell: Cell) -> bool {
        self.sight(player, cell) == Sight::Visible
    }

    /// Whether a player has ever seen a cell.
    #[inline]
    pub fn is_explored(&self, player: PlayerId, cell: Cell) -> bool {
        self.sight(player, cell) != Sight::Unseen
    }

    /// How much of the map a player has explored, as a percentage.
    pub fn explored_percent(&self, player: PlayerId) -> u32 {
        let Some(explored) = self.explored.get(player.0 as usize) else {
            return 0;
        };
        if explored.is_empty() {
            return 0;
        }
        let seen = explored.iter().filter(|e| **e).count();
        ((seen as u64 * 100) / explored.len() as u64) as u32
    }
}

impl StateHash for Visibility {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_u16(self.width);
        h.write_u16(self.height);
        h.write_bool(self.enabled);
        // Both layers: what a player has explored changes what they can target
        // later, so two peers disagreeing about it is a divergence even before
        // anything visibly differs.
        for row in &self.explored {
            for v in row {
                h.write_bool(*v);
            }
        }
        for row in &self.visible {
            for v in row {
                h.write_bool(*v);
            }
        }
        for row in &self.detected {
            for v in row {
                h.write_bool(*v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Visibility {
        Visibility::new(32, 32, 2)
    }

    #[test]
    fn a_fresh_map_is_entirely_unseen() {
        let v = grid();
        assert_eq!(v.sight(PlayerId(0), Cell::new(5, 5)), Sight::Unseen);
        assert!(!v.is_explored(PlayerId(0), Cell::new(5, 5)));
        assert_eq!(v.explored_percent(PlayerId(0)), 0);
    }

    #[test]
    fn revealing_marks_a_cell_visible_and_explored() {
        let mut v = grid();
        v.reveal(PlayerId(0), Cell::new(10, 10), Fx::from_int(3));
        assert_eq!(v.sight(PlayerId(0), Cell::new(10, 10)), Sight::Visible);
        assert!(v.is_explored(PlayerId(0), Cell::new(10, 10)));
    }

    #[test]
    fn ground_you_walk_away_from_stays_remembered() {
        // The whole point of two layers. A map that went black again the moment
        // you left would make scouting worthless.
        let mut v = grid();
        v.reveal(PlayerId(0), Cell::new(10, 10), Fx::from_int(3));

        v.begin_tick();
        assert_eq!(
            v.sight(PlayerId(0), Cell::new(10, 10)),
            Sight::Fogged,
            "explored ground should be remembered, not forgotten"
        );
        assert!(v.is_explored(PlayerId(0), Cell::new(10, 10)));
        assert!(!v.is_visible(PlayerId(0), Cell::new(10, 10)));
    }

    #[test]
    fn vision_is_a_circle_rather_than_a_square() {
        // A square lets a unit see further along the diagonal than straight
        // ahead, which shows up as a diamond-shaped hole in the fog.
        let mut v = grid();
        let centre = Cell::new(16, 16);
        v.reveal(PlayerId(0), centre, Fx::from_int(4));

        // Four cells straight out: inside.
        assert!(v.is_visible(PlayerId(0), Cell::new(20, 16)));
        // Four cells diagonally out is nearly six away: outside.
        assert!(
            !v.is_visible(PlayerId(0), Cell::new(20, 20)),
            "the corner of the bounding box should not be visible"
        );
    }

    #[test]
    fn one_player_seeing_something_does_not_reveal_it_to_another() {
        let mut v = grid();
        v.reveal(PlayerId(0), Cell::new(10, 10), Fx::from_int(3));
        assert!(v.is_visible(PlayerId(0), Cell::new(10, 10)));
        assert_eq!(v.sight(PlayerId(1), Cell::new(10, 10)), Sight::Unseen);
    }

    #[test]
    fn revealing_off_the_map_is_ignored_rather_than_panicking() {
        // A unit near the edge has most of its vision circle outside the map.
        let mut v = grid();
        v.reveal(PlayerId(0), Cell::new(0, 0), Fx::from_int(5));
        v.reveal(PlayerId(0), Cell::new(31, 31), Fx::from_int(5));
        assert!(v.is_visible(PlayerId(0), Cell::new(0, 0)));
        assert_eq!(v.sight(PlayerId(0), Cell::new(-1, -1)), Sight::Unseen);
        assert_eq!(v.sight(PlayerId(0), Cell::new(99, 99)), Sight::Unseen);
    }

    #[test]
    fn an_unknown_player_sees_nothing_rather_than_everything() {
        // Failing open here would hand a spectator slot full vision of the map.
        let v = grid();
        assert_eq!(v.sight(PlayerId(9), Cell::new(5, 5)), Sight::Unseen);
        assert_eq!(v.explored_percent(PlayerId(9)), 0);
    }

    #[test]
    fn a_zero_radius_still_reveals_where_the_unit_stands() {
        let mut v = grid();
        v.reveal(PlayerId(0), Cell::new(8, 8), Fx::ZERO);
        assert!(v.is_visible(PlayerId(0), Cell::new(8, 8)));
        assert!(!v.is_visible(PlayerId(0), Cell::new(9, 8)));
    }

    #[test]
    fn revealing_everything_overrides_both_layers() {
        // For replays and spectators, which should see the whole map.
        let mut v = grid();
        assert_eq!(v.sight(PlayerId(0), Cell::new(5, 5)), Sight::Unseen);
        v.reveal_all();
        assert_eq!(v.sight(PlayerId(0), Cell::new(5, 5)), Sight::Visible);
        assert!(v.is_visible(PlayerId(1), Cell::new(30, 30)));
    }

    #[test]
    fn explored_percentage_tracks_what_has_been_seen() {
        let mut v = grid();
        v.reveal(PlayerId(0), Cell::new(16, 16), Fx::from_int(4));
        let some = v.explored_percent(PlayerId(0));
        assert!(
            some > 0 && some < 100,
            "one vision circle covered {some}% of the map"
        );

        for y in (0..32).step_by(4) {
            for x in (0..32).step_by(4) {
                v.reveal(PlayerId(0), Cell::new(x, y), Fx::from_int(4));
            }
        }
        assert!(v.explored_percent(PlayerId(0)) > some);
    }

    #[test]
    fn both_layers_are_hashed() {
        // Explored ground changes what a player can target later, so two peers
        // disagreeing about it is a divergence before anything visibly differs.
        let hash = |v: &Visibility| {
            let mut h = StateHasher::new();
            h.write(v);
            h.finish()
        };
        let base = grid();

        let mut seen = grid();
        seen.reveal(PlayerId(0), Cell::new(4, 4), Fx::from_int(2));
        assert_ne!(hash(&seen), hash(&base));

        // Same explored ground, different visible ground.
        let mut remembered = seen.clone();
        remembered.begin_tick();
        assert_ne!(
            hash(&remembered),
            hash(&seen),
            "fogged and visible must not hash alike"
        );
    }
}
