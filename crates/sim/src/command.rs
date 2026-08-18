//! Player commands — the only way anything enters the simulation.
//!
//! Input never mutates the world directly. A click becomes a [`Command`], which
//! travels through the network layer's ordered queue and is applied at a
//! scheduled tick. This holds even in single-player, where the "network" is a
//! single peer with zero delay — so the multiplayer path is exercised
//! constantly rather than integrated at the end.
//!
//! Commands are small on purpose. Ordering a hundred units to move is one
//! command carrying a hundred ids, not a hundred messages, which is why
//! lockstep bandwidth barely moves as army sizes grow.

use serde::{Deserialize, Serialize};

use redshift_data::rules::EntityKind;

use crate::arena::EntityId;
use crate::hash::{StateHash, StateHasher};
use crate::map::Cell;

/// Which player issued a command.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct PlayerId(pub u8);

impl StateHash for PlayerId {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_u8(self.0);
    }
}

/// What a player asked for.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum CommandKind {
    /// Move the listed units to a destination.
    Move { units: Vec<EntityId>, target: Cell },
    /// Cancel current orders and hold position.
    Stop { units: Vec<EntityId> },
    /// Queue something at a production building.
    Produce {
        building: EntityId,
        kind: EntityKind,
    },
    /// Remove a queued item by its position in the queue.
    CancelProduction { building: EntityId, index: u8 },
    /// Site a structure that has finished building.
    PlaceBuilding {
        /// The building that produced it — usually the construction yard.
        producer: EntityId,
        /// Where its footprint should start.
        at: Cell,
    },
}

/// A command, tagged with its issuer and its place in the total order.
///
/// The ordering fields are not decoration. Every peer must apply commands in
/// exactly the same sequence, so ties between players in the same tick are
/// broken by `player`, and ties within one player by `sequence`.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Command {
    pub player: PlayerId,
    /// Per-player counter, unique within a tick.
    pub sequence: u16,
    pub kind: CommandKind,
}

impl Command {
    pub fn new(player: PlayerId, sequence: u16, kind: CommandKind) -> Command {
        Command {
            player,
            sequence,
            kind,
        }
    }

    /// The total order commands are applied in.
    ///
    /// Used to sort a tick's commands before they are applied. Deriving `Ord`
    /// on the struct would sort by `kind` as a tiebreak, which would make the
    /// order depend on how the enum happens to be declared.
    pub fn order_key(&self) -> (u8, u16) {
        (self.player.0, self.sequence)
    }
}

impl StateHash for Command {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write(&self.player);
        h.write_u16(self.sequence);
        match &self.kind {
            CommandKind::Move { units, target } => {
                h.write_u8(0);
                h.write_u32(units.len() as u32);
                for u in units {
                    h.write_u32(u.index());
                    h.write_u32(u.generation());
                }
                h.write(target);
            }
            CommandKind::Stop { units } => {
                h.write_u8(1);
                h.write_u32(units.len() as u32);
                for u in units {
                    h.write_u32(u.index());
                    h.write_u32(u.generation());
                }
            }
            CommandKind::Produce { building, kind } => {
                h.write_u8(2);
                h.write_u32(building.index());
                h.write_u32(building.generation());
                h.write_u16(kind.0);
            }
            CommandKind::PlaceBuilding { producer, at } => {
                h.write_u8(4);
                h.write_u32(producer.index());
                h.write_u32(producer.generation());
                h.write(at);
            }
            CommandKind::CancelProduction { building, index } => {
                h.write_u8(3);
                h.write_u32(building.index());
                h.write_u32(building.generation());
                h.write_u8(*index);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_key_sorts_by_player_then_sequence() {
        let mut cmds = [
            Command::new(PlayerId(1), 2, CommandKind::Stop { units: Vec::new() }),
            Command::new(PlayerId(0), 5, CommandKind::Stop { units: Vec::new() }),
            Command::new(PlayerId(1), 0, CommandKind::Stop { units: Vec::new() }),
            Command::new(PlayerId(0), 1, CommandKind::Stop { units: Vec::new() }),
        ];
        cmds.sort_by_key(|c| c.order_key());
        let keys: Vec<_> = cmds.iter().map(|c| c.order_key()).collect();
        assert_eq!(keys, [(0, 1), (0, 5), (1, 0), (1, 2)]);
    }

    #[test]
    fn hashing_distinguishes_kinds_and_payloads() {
        fn hash(c: &Command) -> u64 {
            let mut h = StateHasher::new();
            h.write(c);
            h.finish()
        }
        let stop = Command::new(PlayerId(0), 0, CommandKind::Stop { units: Vec::new() });
        let mv = Command::new(
            PlayerId(0),
            0,
            CommandKind::Move {
                units: vec![],
                target: Cell::new(0, 0),
            },
        );
        assert_ne!(hash(&stop), hash(&mv));

        let mv2 = Command::new(
            PlayerId(0),
            0,
            CommandKind::Move {
                units: vec![],
                target: Cell::new(0, 1),
            },
        );
        assert_ne!(
            hash(&mv),
            hash(&mv2),
            "a different target must hash differently"
        );
    }

    #[test]
    fn serialisation_roundtrips() {
        let c = Command::new(
            PlayerId(3),
            9,
            CommandKind::Move {
                units: vec![],
                target: Cell::new(-4, 7),
            },
        );
        let encoded = ron::to_string(&c).unwrap();
        assert_eq!(ron::from_str::<Command>(&encoded).unwrap(), c);
    }
}
