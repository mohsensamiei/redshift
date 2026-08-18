//! Shots in flight.
//!
//! # Why travel time matters
//!
//! Until now every shot landed on the tick it was fired. That is fine for a
//! rifle and wrong for everything else, and the difference is not cosmetic:
//!
//! - Artillery that lands instantly cannot be dodged, so outranging something
//!   becomes strictly better rather than a trade.
//! - A missile with no flight time cannot be shot down, so anti-missile
//!   defences have nothing to do.
//! - A slow shell that tracks its target and a slow shell that does not are
//!   different weapons, and neither is expressible without flight.
//!
//! # Homing or not
//!
//! Both, chosen per weapon. A missile follows its target and hits what it was
//! aimed at; a shell flies to where the target *was* and misses if it moved.
//! That distinction is most of what separates artillery from a tank gun, and
//! making it a weapon property rather than a global rule follows ADR 0006.
//!
//! A weapon with zero projectile speed still hits instantly, so a rifle needs
//! no special case and the existing behaviour is preserved exactly.

use serde::{Deserialize, Serialize};

use crate::arena::EntityId;
use crate::combat::WarheadId;
use crate::command::PlayerId;
use crate::fx::Fx;
use crate::hash::{StateHash, StateHasher};
use crate::map::WorldPos;

/// A shot between firing and landing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Projectile {
    /// Who fired it. Kept so a kill can be credited after the shooter dies —
    /// a shell already in the air is still that unit's kill.
    pub attacker: EntityId,
    pub owner: PlayerId,
    /// The target it follows, for a homing weapon.
    pub target: Option<EntityId>,
    /// Where it is going. For a homing shot this is refreshed as the target
    /// moves; for a ballistic one it is fixed at the moment of firing.
    pub aim: WorldPos,
    pub pos: WorldPos,
    /// Cells travelled per tick.
    pub speed: Fx,
    pub damage: u32,
    pub warhead: WarheadId,
    pub splash_radius: Fx,
    /// Ticks left before it is given up on.
    ///
    /// A homing shot chasing something faster than itself would otherwise fly
    /// forever, and a projectile that never lands is a slow memory leak that
    /// also drags the state hash along with it.
    pub fuse: u32,
}

/// How close a projectile must get to count as having arrived.
///
/// Generous enough that a shot cannot skip past its aim point between ticks: a
/// fast projectile covers real distance per tick, and an exact-equality test
/// would let it overshoot forever.
pub const IMPACT_TOLERANCE: Fx = Fx::from_frac(30, 100);

/// Ticks a projectile may stay in the air.
///
/// Ten seconds at 20 Hz — far longer than any sane weapon's flight, short
/// enough that a bug cannot accumulate them.
pub const MAX_FLIGHT_TICKS: u32 = 200;

impl Projectile {
    /// Advances one tick. Returns `true` when it has arrived.
    ///
    /// Movement is a fixed step along the line to the aim point, so a
    /// projectile's path does not depend on frame rate or on how long it has
    /// been flying — only on the tick count, which every peer agrees on.
    pub fn advance(&mut self) -> bool {
        self.fuse = self.fuse.saturating_sub(1);

        let dx = self.aim.x - self.pos.x;
        let dy = self.aim.y - self.pos.y;
        let remaining = Fx::dist(dx, dy);

        if remaining <= self.speed.max(IMPACT_TOLERANCE) {
            self.pos = self.aim;
            return true;
        }

        // Normalised step. `remaining` is known non-zero here, since it exceeds
        // the tolerance.
        self.pos = WorldPos {
            x: self.pos.x + dx.div(remaining).mul(self.speed),
            y: self.pos.y + dy.div(remaining).mul(self.speed),
        };
        false
    }

    /// Whether this shot has run out of time.
    pub fn is_spent(&self) -> bool {
        self.fuse == 0
    }
}

impl StateHash for Projectile {
    fn state_hash(&self, h: &mut StateHasher) {
        h.write_u32(self.attacker.index());
        h.write_u32(self.attacker.generation());
        h.write(&self.owner);
        match self.target {
            Some(id) => {
                h.write_u8(1);
                h.write_u32(id.index());
                h.write_u32(id.generation());
            }
            None => h.write_u8(0),
        }
        h.write(&self.aim);
        h.write(&self.pos);
        h.write_i32(self.speed.raw());
        h.write_u32(self.damage);
        h.write_u16(self.warhead.0);
        h.write_i32(self.splash_radius.raw());
        h.write_u32(self.fuse);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::Cell;

    fn shot(from: Cell, to: Cell, speed: Fx) -> Projectile {
        Projectile {
            attacker: EntityId::NONE,
            owner: PlayerId(0),
            target: None,
            aim: to.centre(),
            pos: from.centre(),
            speed,
            damage: 50,
            warhead: WarheadId(0),
            splash_radius: Fx::ZERO,
            fuse: MAX_FLIGHT_TICKS,
        }
    }

    #[test]
    fn a_shot_takes_time_to_arrive() {
        // The entire point. A projectile that arrived immediately would be the
        // instant-hit model with extra steps.
        let mut p = shot(Cell::new(0, 0), Cell::new(10, 0), Fx::from_frac(50, 100));
        let mut ticks = 0;
        while !p.advance() && ticks < 100 {
            ticks += 1;
        }
        assert!(
            ticks > 10,
            "ten cells at half a cell a tick took only {ticks} ticks"
        );
        assert!(
            ticks < 30,
            "took {ticks} ticks, which is slower than the speed implies"
        );
    }

    #[test]
    fn it_travels_towards_its_aim_and_not_past_it() {
        let mut p = shot(Cell::new(0, 0), Cell::new(10, 0), Fx::from_frac(50, 100));
        let start = p.pos;
        p.advance();
        assert!(p.pos.x > start.x, "it did not move");
        assert!(p.pos.x < p.aim.x, "it overshot in one tick");
        assert_eq!(p.pos.y, start.y, "it drifted off the line");
    }

    #[test]
    fn a_fast_shot_does_not_skip_past_its_target() {
        // An exact-equality arrival test would let a projectile moving further
        // per tick than the distance remaining fly on forever.
        let mut p = shot(Cell::new(0, 0), Cell::new(1, 0), Fx::from_int(50));
        assert!(
            p.advance(),
            "a very fast shot should arrive on its first tick"
        );
        assert_eq!(p.pos, p.aim, "it should land exactly on its aim point");
    }

    #[test]
    fn a_shot_already_at_its_target_arrives_immediately() {
        let mut p = shot(Cell::new(5, 5), Cell::new(5, 5), Fx::from_frac(50, 100));
        assert!(p.advance());
    }

    #[test]
    fn a_diagonal_shot_moves_on_both_axes() {
        let mut p = shot(Cell::new(0, 0), Cell::new(10, 10), Fx::from_int(1));
        let start = p.pos;
        p.advance();
        assert!(
            p.pos.x > start.x && p.pos.y > start.y,
            "a diagonal shot moved on one axis"
        );
    }

    #[test]
    fn a_shot_that_never_arrives_runs_out_of_fuse() {
        // A homing shot chasing something faster than itself would otherwise
        // fly forever, leaking memory and dragging the state hash with it.
        let mut p = shot(Cell::new(0, 0), Cell::new(500, 500), Fx::from_frac(1, 100));
        p.fuse = 5;
        for _ in 0..5 {
            assert!(!p.advance(), "it should not have arrived");
        }
        assert!(p.is_spent(), "the fuse should have run out");
    }

    #[test]
    fn flight_is_reproducible() {
        // Two peers must agree on where every shot is, every tick.
        let path = || {
            let mut p = shot(Cell::new(0, 0), Cell::new(17, 9), Fx::from_frac(37, 100));
            let mut trail = Vec::new();
            while !p.advance() {
                trail.push((p.pos.x.raw(), p.pos.y.raw()));
            }
            trail
        };
        let a = path();
        assert_eq!(a, path());
        assert!(
            a.len() > 5,
            "the shot arrived too quickly to prove anything"
        );
    }

    #[test]
    fn a_projectile_hashes_its_whole_state() {
        let hash = |p: &Projectile| {
            let mut h = StateHasher::new();
            h.write(p);
            h.finish()
        };
        let base = shot(Cell::new(0, 0), Cell::new(10, 0), Fx::ONE);
        let mut moved = base;
        moved.advance();
        assert_ne!(hash(&moved), hash(&base), "position must be in the hash");

        let mut aimed_elsewhere = base;
        aimed_elsewhere.aim = Cell::new(9, 0).centre();
        assert_ne!(hash(&aimed_elsewhere), hash(&base));
    }
}
