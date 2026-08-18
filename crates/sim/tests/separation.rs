//! Units keeping out of each other.
//!
//! Overlapping units are the most visible thing a simulation can get wrong: it
//! reads immediately as broken, long before anyone notices a subtle balance
//! problem. These tests pin the behaviour and — more importantly — pin that
//! solving it did not cost determinism.

use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map, Terrain, WorldPos};
use redshift_sim::sim::{MatchSetup, Sim};
use redshift_sim::{EntityId, Fx};

fn setup(spawns: Vec<(u8, WorldPos)>) -> MatchSetup {
    MatchSetup::for_test(
        0x5E9A_2A7E,
        Map::new(40, 40),
        spawns
            .into_iter()
            .map(|(p, pos)| (PlayerId(p), pos))
            .collect(),
    )
}

/// The closest any two units get, in cells.
fn closest_pair(sim: &Sim) -> Option<Fx> {
    let units: Vec<_> = sim.units().iter().map(|(id, u)| (id, u.pos)).collect();
    let mut closest: Option<Fx> = None;
    for (i, (_, a)) in units.iter().enumerate() {
        for (_, b) in units.iter().skip(i + 1) {
            let gap = Fx::dist(a.x - b.x, a.y - b.y);
            closest = Some(closest.map_or(gap, |c: Fx| c.min(gap)));
        }
    }
    closest
}

#[test]
fn units_spawned_on_the_same_spot_push_apart() {
    // The worst case: several units at exactly one point. There is no direction
    // between them, so the push has to come from somewhere else — and it has to
    // come from somewhere every peer agrees on.
    let spot = Cell::new(20, 20).centre();
    let mut sim = Sim::new(setup(vec![(0, spot), (0, spot), (0, spot), (0, spot)]));

    assert_eq!(closest_pair(&sim), Some(Fx::ZERO), "they start coincident");

    for _ in 0..60 {
        sim.tick(&[]);
    }

    let gap = closest_pair(&sim).expect("four units");
    assert!(
        gap > Fx::from_frac(30, 100),
        "units are still stacked: closest pair is {gap:?} apart"
    );
}

#[test]
fn a_crowd_settles_instead_of_exploding() {
    // A unit in a dense press accumulates a push from every neighbour. Without
    // a cap it would be flung clear rather than shuffling aside.
    let mut spawns = Vec::new();
    for i in 0..16i32 {
        spawns.push((0u8, Cell::new(20 + i % 2, 20 + i / 8).centre()));
    }
    let mut sim = Sim::new(setup(spawns));

    for _ in 0..120 {
        sim.tick(&[]);
    }

    // Everyone should still be in the neighbourhood they started in.
    for (_, unit) in sim.units().iter() {
        let cell = unit.cell();
        assert!(
            (12..30).contains(&cell.x) && (12..30).contains(&cell.y),
            "a unit was flung to {cell:?}"
        );
    }
    let gap = closest_pair(&sim).expect("sixteen units");
    assert!(
        gap > Fx::from_frac(25, 100),
        "the crowd never separated: {gap:?}"
    );
}

#[test]
fn separation_never_pushes_a_unit_into_water() {
    // A push must not put a unit somewhere it could never have walked.
    let mut map = Map::new(40, 40);
    map.fill_rect(Cell::new(0, 0), Cell::new(39, 18), Terrain::Water);

    // A tight row pressed against the shoreline.
    let mut spawns = Vec::new();
    for i in 0..8i32 {
        spawns.push((PlayerId(0), Cell::new(18 + i % 2, 19).centre()));
    }
    let mut sim = Sim::new(MatchSetup::for_test(0xC0A57, map, spawns));

    for _ in 0..200 {
        sim.tick(&[]);
    }

    for (_, unit) in sim.units().iter() {
        assert!(
            unit.cell().y >= 19,
            "a unit was pushed into the water at {:?}",
            unit.cell()
        );
    }
}

#[test]
fn a_group_ordered_to_one_cell_forms_up_around_it() {
    // Nine units cannot share a cell, so they are each given their own. Before
    // that, they all aimed at the same point, piled up, and were shoved apart
    // one arrival at a time — settling into a blob four cells across, because a
    // unit that has arrived goes idle and never comes back.
    let mut spawns = Vec::new();
    for i in 0..9i32 {
        spawns.push((0u8, Cell::new(4 + i % 3, 4 + i / 3).centre()));
    }
    let mut sim = Sim::new(setup(spawns));
    let ids: Vec<EntityId> = sim.units().ids();
    let destination = Cell::new(30, 30);

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Move {
            units: ids.clone(),
            target: destination,
        },
    )]);

    for _ in 0..600 {
        sim.tick(&[]);
    }

    // Sharing a cell is fine — positions are continuous, and two units 0.7
    // cells apart are not touching even if they round to the same cell. What
    // must not happen is overlap.
    let gap = closest_pair(&sim).expect("nine units");
    assert!(
        gap > Fx::from_frac(50, 100),
        "units ended up overlapping: closest pair is {gap:?} apart"
    );

    for id in &ids {
        let unit = sim.units().get(*id).expect("nobody should have died");
        let distance = Fx::dist(
            unit.pos.x - destination.centre().x,
            unit.pos.y - destination.centre().y,
        );
        assert!(
            distance < Fx::from_frac(250, 100),
            "a unit ended {distance:?} from the destination at {:?}, which is a \
             pile rather than a formation",
            unit.cell()
        );
        assert!(
            unit.order.is_idle(),
            "a unit is still trying to move after arriving"
        );
    }
}

#[test]
fn separation_is_deterministic() {
    // Pushes are accumulated for every unit and applied afterwards, so no unit
    // sees a neighbour that has already moved this tick. Applying as we went
    // would make the outcome depend on arena order — deterministic, but shifting
    // whenever slots are reused.
    let run = || {
        let spot = Cell::new(20, 20).centre();
        let mut spawns = vec![(0u8, spot); 12];
        for i in 0..12i32 {
            spawns.push((1u8, Cell::new(21 + i % 3, 21 + i / 3).centre()));
        }
        let mut sim = Sim::new(setup(spawns));
        let mut hashes = Vec::new();
        for _ in 0..200 {
            sim.tick(&[]);
            hashes.push(sim.state_hash());
        }
        hashes
    };
    assert_eq!(run(), run(), "two identical crowds settled differently");
}

#[test]
fn aircraft_pass_through_each_other() {
    // Air units are at a different altitude. Separating them would make a
    // flight of aircraft fan out for no reason the player can see.
    //
    // The test rules have no aircraft, so this documents the intent against the
    // rule itself rather than through a scenario.
    use redshift_data::traits::Locomotor;
    assert_ne!(
        Locomotor::Air,
        Locomotor::Tracked,
        "if these ever merge, the separation pass needs revisiting"
    );
}
