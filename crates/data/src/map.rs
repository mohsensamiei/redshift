//! The map file format.
//!
//! A map is authored as a list of *edits* rather than as a grid of cells. That
//! is the whole design decision here, and it is worth stating why: a 48×48 map
//! written out cell by cell is 2,304 entries of which perhaps thirty carry any
//! information, and a human cannot edit it or read a diff of it. The edits are
//! what the person who made the map actually meant — "a ridge here, water
//! there, ore in the corners" — and they read as that.
//!
//! It costs one thing: two different edit lists can produce the same grid, so
//! a map file is not a canonical form. Nothing depends on it being one. What
//! *does* have to be canonical is the resulting grid, and that is hashed as
//! part of the simulation state like everything else.
//!
//! Deliberately in `redshift-data` rather than `redshift-sim`. A map is
//! authored content, the same as a rules file, and the simulation should no
//! more parse RON than it should open a window.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// A rectangle of cells, inclusive at both ends.
///
/// Inclusive because map authors think in "cells 15 to 17", not "15 up to but
/// not including 18". An off-by-one in a map file is a wall with a hole in it,
/// and the format should not invite one.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Rect {
    pub from: (i32, i32),
    pub to: (i32, i32),
}

/// What a cell is made of, as a map file names it.
///
/// A separate enum from the simulation's `Terrain`, and not a re-export. The
/// simulation's is an implementation detail that may gain variants for reasons
/// a map author does not care about; this is the vocabulary of the file format,
/// and changing it should be a deliberate act with a version bump behind it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Ground {
    #[default]
    Land,
    Water,
    Rock,
}

/// One thing done to the map, in order.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum Edit {
    /// Paints a rectangle of terrain.
    Terrain { area: Rect, ground: Ground },
    /// Raises a rectangle to a level. Zero is the ground floor.
    Elevation { area: Rect, level: u8 },
    /// Scatters an ore field, thickest at the centre.
    Ore {
        at: (i32, i32),
        radius: i32,
        amount: u16,
    },
}

/// Where a player starts, and what they start with.
///
/// Named by entity id rather than by index, so a map survives the roster being
/// reordered — the same reason prerequisites name ids.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct StartingPosition {
    /// Which slot this belongs to. Slots are filled in lobby order.
    pub slot: u8,
    pub at: (i32, i32),
    /// What is placed, and where relative to `at`.
    pub units: Vec<PlacedUnit>,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct PlacedUnit {
    pub id: String,
    pub offset: (i32, i32),
}

/// Things belonging to nobody: civilians, tech buildings, bridges, ore mines.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct NeutralUnit {
    pub id: String,
    pub at: (i32, i32),
}

/// A playable map.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct MapDef {
    pub name: String,
    pub width: u16,
    pub height: u16,
    /// How many players it is built for. A map is refused rather than silently
    /// half-filled if a lobby asks for more than it has starts.
    pub players: u8,
    #[serde(default)]
    pub edits: Vec<Edit>,
    pub starts: Vec<StartingPosition>,
    #[serde(default)]
    pub neutrals: Vec<NeutralUnit>,
}

/// What can go wrong reading a map.
#[derive(Debug)]
pub enum MapError {
    Io(std::io::Error),
    Malformed { path: String, message: String },
    Invalid { problems: Vec<String> },
}

impl std::fmt::Display for MapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MapError::Io(e) => write!(f, "{e}"),
            MapError::Malformed { path, message } => write!(f, "{path} is malformed: {message}"),
            MapError::Invalid { problems } => {
                writeln!(f, "the map has {} problem(s):", problems.len())?;
                for p in problems {
                    writeln!(f, "  - {p}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for MapError {}

impl MapDef {
    pub fn load(path: &Path) -> Result<MapDef, MapError> {
        let text = std::fs::read_to_string(path).map_err(MapError::Io)?;
        let map: MapDef = ron::from_str(&text).map_err(|e| MapError::Malformed {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        map.validate()?;
        Ok(map)
    }

    /// Reads a map from text.
    ///
    /// Not `FromStr`: that trait's `Err` would have to carry the same
    /// information and the call site reads worse for it. Named to say what it
    /// does rather than to satisfy a trait nothing asks for.
    pub fn parse(text: &str) -> Result<MapDef, MapError> {
        let map: MapDef = ron::from_str(text).map_err(|e| MapError::Malformed {
            path: "<memory>".into(),
            message: e.to_string(),
        })?;
        map.validate()?;
        Ok(map)
    }

    /// Everything worth refusing a map for.
    ///
    /// Checked at load rather than discovered mid-match. A start position off
    /// the edge of its own map is the sort of thing that produces a unit at
    /// (0,0) and a puzzled player twenty minutes later.
    fn validate(&self) -> Result<(), MapError> {
        let mut problems = Vec::new();

        if self.width == 0 || self.height == 0 {
            problems.push("the map has no area".to_string());
        }
        let inside = |(x, y): (i32, i32)| {
            x >= 0 && y >= 0 && x < self.width as i32 && y < self.height as i32
        };

        if self.starts.len() < self.players as usize {
            problems.push(format!(
                "it says it is for {} players and gives {} starting positions",
                self.players,
                self.starts.len()
            ));
        }
        let mut slots: Vec<u8> = self.starts.iter().map(|s| s.slot).collect();
        slots.sort_unstable();
        slots.dedup();
        if slots.len() != self.starts.len() {
            problems.push("two starting positions claim the same slot".to_string());
        }
        for start in &self.starts {
            if !inside(start.at) {
                problems.push(format!(
                    "starting position {} is at {:?}, off the map",
                    start.slot, start.at
                ));
            }
        }
        for neutral in &self.neutrals {
            if !inside(neutral.at) {
                problems.push(format!(
                    "neutral {:?} is at {:?}, off the map",
                    neutral.id, neutral.at
                ));
            }
        }
        for edit in &self.edits {
            let area = match edit {
                Edit::Terrain { area, .. } | Edit::Elevation { area, .. } => Some(*area),
                Edit::Ore { .. } => None,
            };
            // Only the corners, because an edit is clamped when it is applied —
            // an author who paints past the edge means "to the edge".
            if let Some(area) = area
                && !inside(area.from)
                && !inside(area.to)
            {
                problems.push(format!("an edit covers {area:?}, entirely off the map"));
            }
        }

        if problems.is_empty() {
            Ok(())
        } else {
            Err(MapError::Invalid { problems })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smallest map that is actually valid.
    fn minimal() -> String {
        r#"(
            name: "Test",
            width: 16,
            height: 16,
            players: 1,
            starts: [(slot: 0, at: (4, 4), units: [(id: "tank", offset: (0, 0))])],
        )"#
        .to_string()
    }

    #[test]
    fn a_minimal_map_loads() {
        let map = MapDef::parse(&minimal()).expect("should be valid");
        assert_eq!(map.name, "Test");
        assert_eq!(map.starts.len(), 1);
        // Edits and neutrals are optional: a map with neither is a flat field,
        // which is a perfectly good thing to test on.
        assert!(map.edits.is_empty());
        assert!(map.neutrals.is_empty());
    }

    #[test]
    fn a_start_off_the_map_is_refused() {
        // The failure this exists to prevent produces a unit at the origin and
        // a puzzled player twenty minutes later, rather than an error.
        let text = minimal().replace("at: (4, 4)", "at: (40, 4)");
        assert!(MapDef::parse(&text).is_err());
    }

    #[test]
    fn two_starts_in_one_slot_are_refused() {
        let text = r#"(
            name: "Test", width: 16, height: 16, players: 2,
            starts: [
                (slot: 0, at: (2, 2), units: []),
                (slot: 0, at: (9, 9), units: []),
            ],
        )"#;
        assert!(MapDef::parse(text).is_err());
    }

    #[test]
    fn a_map_that_cannot_seat_the_players_it_claims_is_refused() {
        let text = minimal().replace("players: 1", "players: 4");
        assert!(MapDef::parse(&text).is_err());
    }

    #[test]
    fn an_edit_running_off_the_edge_is_allowed() {
        // Painting past the edge means "to the edge", and clamping is the
        // sensible reading. Only an edit that touches nothing at all is wrong.
        let text = minimal().replace(
            "players: 1,",
            "players: 1,\n            edits: [Terrain(area: (from: (10, 10), to: (99, 99)), ground: Water)],",
        );
        assert!(MapDef::parse(&text).is_ok());
    }

    #[test]
    fn an_edit_entirely_off_the_map_is_refused() {
        let text = minimal().replace(
            "players: 1,",
            "players: 1,\n            edits: [Terrain(area: (from: (60, 60), to: (99, 99)), ground: Water)],",
        );
        assert!(MapDef::parse(&text).is_err());
    }

    #[test]
    fn a_map_with_no_area_is_refused() {
        let text = minimal().replace("width: 16", "width: 0");
        assert!(MapDef::parse(&text).is_err());
    }

    #[test]
    fn the_shipped_map_loads() {
        // The one that actually gets played. A format nobody has written a real
        // map in is a format that does not work yet.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../maps/crossing.ron");
        let map = MapDef::load(&path).expect("the shipped map should load");
        assert_eq!(map.players, 2);
        assert_eq!(map.starts.len(), 2);
        assert!(!map.edits.is_empty());
    }
}
