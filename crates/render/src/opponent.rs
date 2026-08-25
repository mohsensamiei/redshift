//! Running computer opponents.
//!
//! One system, and it is deliberately thin: ask each commander what it wants,
//! and put the answer through the same ordered queue a human's commands go
//! through. A shortcut that applied them directly would make this peer play a
//! subtly different game from the one its own replay reproduces.
//!
//! Held in the renderer rather than the simulation because a computer opponent
//! is a *player*, and which players a given peer speaks for is a fact about
//! this process rather than about the world.

use bevy::prelude::*;
use redshift_ai::Commander;

use crate::session::Session;

/// The opponents this peer is playing.
#[derive(Resource, Default)]
pub struct Opponents {
    pub commanders: Vec<Commander>,
}

pub fn run_opponents(mut session: ResMut<Session>, mut opponents: ResMut<Opponents>) {
    // Nothing to decide while the world is not advancing. Thinking during a
    // pause would let an opponent bank a hundred decisions and spend them all
    // on the tick play resumes.
    if session.ticks_this_frame == 0 {
        return;
    }
    for commander in &mut opponents.commanders {
        let player = commander.player();
        // Split so the borrow of the simulation ends before the commands are
        // issued: `think` reads the world and `issue_for` writes to the queue,
        // and the two must not overlap.
        let orders = commander.think(session.sim());
        for order in orders {
            session.issue_for(player, order);
        }
    }
}
