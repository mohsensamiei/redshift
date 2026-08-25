//! Turning an authored map into a match.
//!
//! In the client rather than in `redshift-sim`, deliberately. The simulation
//! takes a `MatchSetup` and knows nothing about where it came from — no file
//! reading, no RON, no paths — which is what lets it run headless in a test, a
//! replay and a dedicated server without any of them pretending to be a client.
//!
//! And in the client rather than in `redshift-data` for a smaller reason: this
//! is where the two meet, and `redshift-data` should not depend on the
//! simulation just to build one of its types.

use redshift_data::map::{Edit, Ground, MapDef, Rect};
use redshift_data::rules::Rules;
use redshift_sim::command::PlayerId;
use redshift_sim::map::{Cell, Map, Terrain};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Spawn};

/// What can go wrong turning a map file into a match.
#[derive(Debug)]
pub enum BuildError {
    /// A map names something the rules do not have. Caught here rather than at
    /// map load, because a map is only wrong *against a particular ruleset* —
    /// the same file is fine with a mod that adds the unit back.
    UnknownEntity { id: String },
    /// More players asked for than the map has room for.
    TooManyPlayers { asked: usize, available: usize },
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::UnknownEntity { id } => {
                write!(f, "the map places {id:?}, which the rules do not define")
            }
            BuildError::TooManyPlayers { asked, available } => write!(
                f,
                "the match wants {asked} players and the map has {available} starting positions"
            ),
        }
    }
}

impl std::error::Error for BuildError {}

fn corners(area: Rect) -> (Cell, Cell) {
    (
        Cell::new(area.from.0, area.from.1),
        Cell::new(area.to.0, area.to.1),
    )
}

/// Applies a map file's edits to a fresh grid.
///
/// In order, so a later edit overwrites an earlier one — which is what lets an
/// author paint a lake and then bridge it without expressing the bridge as a
/// hole in the lake.
pub fn terrain_of(def: &MapDef) -> Map {
    let mut map = Map::new(def.width, def.height);
    for edit in &def.edits {
        match edit {
            Edit::Terrain { area, ground } => {
                let (from, to) = corners(*area);
                let terrain = match ground {
                    Ground::Land => Terrain::Ground,
                    Ground::Water => Terrain::Water,
                    Ground::Rock => Terrain::Rock,
                };
                map.fill_rect(from, to, terrain);
            }
            Edit::Elevation { area, level } => {
                let (from, to) = corners(*area);
                map.raise_rect(from, to, *level);
            }
            Edit::Ore { at, radius, amount } => {
                map.add_ore_field(Cell::new(at.0, at.1), *radius, *amount);
            }
        }
    }
    map
}

/// Builds a match from a map file, a ruleset and a seed.
pub fn match_setup(
    def: &MapDef,
    rules: Rules,
    seed: u64,
    players: usize,
) -> Result<MatchSetup, BuildError> {
    if players > def.starts.len() {
        return Err(BuildError::TooManyPlayers {
            asked: players,
            available: def.starts.len(),
        });
    }
    let map = terrain_of(def);

    // Sorted by slot so the order does not depend on how the file happened to
    // list them. Two peers reading the same file must build the same match, and
    // "the order they were written in" is a fact about a text file rather than
    // about the map.
    let mut starts = def.starts.clone();
    starts.sort_by_key(|s| s.slot);

    let mut spawns = Vec::new();
    for (index, start) in starts.iter().take(players).enumerate() {
        let owner = PlayerId(index as u8);
        for placed in &start.units {
            let kind = rules
                .kind_of(&placed.id)
                .ok_or_else(|| BuildError::UnknownEntity {
                    id: placed.id.clone(),
                })?;
            let at = Cell::new(start.at.0 + placed.offset.0, start.at.1 + placed.offset.1);
            spawns.push(Spawn {
                owner,
                kind,
                pos: at.centre(),
            });
        }
    }
    for neutral in &def.neutrals {
        let kind = rules
            .kind_of(&neutral.id)
            .ok_or_else(|| BuildError::UnknownEntity {
                id: neutral.id.clone(),
            })?;
        spawns.push(Spawn {
            owner: PlayerId::NEUTRAL,
            kind,
            pos: Cell::new(neutral.at.0, neutral.at.1).centre(),
        });
    }

    Ok(MatchSetup {
        seed,
        map,
        rules,
        players: (0..players)
            .map(|i| PlayerSetup {
                id: PlayerId(i as u8),
                faction: None,
            })
            .collect(),
        spawns,
    })
}
