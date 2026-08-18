//! Cross-platform golden hashes.
//!
//! The rest of the determinism suite proves the simulation agrees with *itself*.
//! This one proves it agrees with a value recorded on a different machine —
//! which is the property multiplayer actually needs, and the one that
//! same-machine testing structurally cannot check.
//!
//! CI runs this on x86 Linux and ARM macOS. If the two disagree, something in
//! the simulation depends on the architecture: a float that slipped past the
//! lint, a differently-sized integer, a shift whose overflow behaviour differs.
//!
//! # When this test fails
//!
//! Assume the simulation is wrong before assuming the constants are stale.
//! Only update them when the change in behaviour was deliberate — and when it
//! is, say so in the commit message, because every recorded replay from before
//! the change becomes invalid.

use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map, Terrain};
use redshift_sim::sim::{MatchSetup, Sim};
use redshift_sim::{Angle, EntityId, Fx};

/// A scripted scenario, fixed forever. Changing anything here changes the
/// expected hashes.
fn scenario() -> MatchSetup {
    let mut map = Map::new(40, 40);
    map.fill_rect(Cell::new(10, 2), Cell::new(10, 28), Terrain::Rock);
    map.fill_rect(Cell::new(26, 12), Cell::new(26, 38), Terrain::Rock);
    map.fill_rect(Cell::new(4, 33), Cell::new(18, 33), Terrain::Water);

    let mut spawns = Vec::new();
    for i in 0..8i32 {
        spawns.push((PlayerId(0), Cell::new(2 + i % 3, 2 + i / 3).centre()));
        spawns.push((PlayerId(1), Cell::new(37 - i % 3, 37 - i / 3).centre()));
    }
    MatchSetup::for_test(0x5EED_1234_ABCD_0001, map, spawns)
}

fn commands(tick: u32, units: &[EntityId]) -> Vec<Command> {
    let half = units.len() / 2;
    let mut out = Vec::new();
    for &(at, x, y) in &[(2u32, 36i32, 36i32), (60, 2, 36), (130, 36, 2)] {
        if tick == at {
            out.push(Command::new(
                PlayerId(0),
                0,
                CommandKind::Move {
                    units: units[..half].to_vec(),
                    target: Cell::new(x, y),
                },
            ));
            out.push(Command::new(
                PlayerId(1),
                0,
                CommandKind::Move {
                    units: units[half..].to_vec(),
                    target: Cell::new(y, x),
                },
            ));
        }
    }
    out.sort_by_key(|c| c.order_key());
    out
}

/// Ticks at which the hash is compared. Spread across the match so a late
/// divergence is caught as well as an early one.
/// Re-recorded when passability moved out of the locomotor enum and into the
/// unit's own declared surfaces. Unit stats carry a surface mask now, so the
/// hash covers different bytes. Determinism was confirmed first across every
/// behaviour suite.
///
/// These values are load-bearing for *cross-platform* agreement, not for
/// immutability: while the state layout is still being built out, an intended
/// change moves them. Once Phase 3 settles, a change here should be treated as
/// a defect until proven otherwise.
const CHECKPOINTS: &[(u32, u64)] = &[
    (10, 0x769809b7f088dcae),
    (50, 0x8b43ca43b3e401fe),
    (100, 0xa39a5c8e03ce11ed),
    (200, 0xafe93585545188ff),
    (400, 0x25a5edb12874e3e0),
];

#[test]
fn state_hashes_match_the_recorded_values() {
    let mut sim = Sim::new(scenario());
    let ids: Vec<EntityId> = sim.units().ids();

    let mut checkpoint = 0usize;
    let last_tick = CHECKPOINTS.last().unwrap().0;

    for tick in 0..=last_tick {
        sim.tick(&commands(tick, &ids));
        if checkpoint < CHECKPOINTS.len() && sim.tick_number() == CHECKPOINTS[checkpoint].0 {
            let (at, expected) = CHECKPOINTS[checkpoint];
            let actual = sim.state_hash();
            println!("tick {at:>4}: 0x{actual:016x}");
            assert_eq!(
                actual, expected,
                "\nstate hash diverged at tick {at}\n  expected 0x{expected:016x}\n  \
                 got      0x{actual:016x}\n\nThis architecture disagrees with the recorded \
                 value. Suspect the simulation before the constant.\n"
            );
            checkpoint += 1;
        }
    }
    assert_eq!(
        checkpoint,
        CHECKPOINTS.len(),
        "not every checkpoint was reached"
    );
}

/// Prints the current hashes in a form that can be pasted into `CHECKPOINTS`.
///
/// Ignored by default: it must never be the thing that makes CI pass. Run it
/// deliberately, and only after establishing that a difference is *intended* —
/// a state layout change, a new field in the hash, a rules change.
///
/// ```sh
/// cargo test -p redshift-sim --test golden_hashes -- --ignored --nocapture
/// ```
///
/// Before pasting, confirm the simulation is still deterministic:
///
/// ```sh
/// cargo test -p redshift-sim --test determinism
/// ```
///
/// If those pass and these numbers changed, the state changed shape. If those
/// fail, the numbers are the least of the problem.
#[test]
#[ignore = "regenerates the recorded values; run deliberately"]
fn regenerate_golden_hashes() {
    let mut sim = Sim::new(scenario());
    let ids: Vec<EntityId> = sim.units().ids();
    let last_tick = CHECKPOINTS.last().unwrap().0;
    let wanted: Vec<u32> = CHECKPOINTS.iter().map(|(t, _)| *t).collect();

    println!("\nconst CHECKPOINTS: &[(u32, u64)] = &[");
    for tick in 0..=last_tick {
        sim.tick(&commands(tick, &ids));
        if wanted.contains(&sim.tick_number()) {
            println!("    ({}, 0x{:016x}),", sim.tick_number(), sim.state_hash());
        }
    }
    println!("];\n");
}

/// Pins the arithmetic primitives directly, so a failure points at the cause
/// rather than at a hash 400 ticks downstream.
#[test]
fn fixed_point_primitives_are_stable() {
    assert_eq!(Fx::from_int(2).sqrt().raw(), 92681);
    assert_eq!(Fx::from_frac(1, 3).raw(), 21845);
    assert_eq!(Fx::from_int(150).mul(Fx::from_int(200)).raw(), 1966080000);
    assert_eq!(Fx::from_int(7).div(Fx::from_int(3)).raw(), 152917);
    assert_eq!(Fx::ONE.mul_ratio(1, 3).raw(), 21845);

    assert_eq!(Angle::from_degrees(30).sin().raw(), 32739);
    assert_eq!(Angle::from_degrees(60).cos().raw(), 32826);
    assert_eq!(
        Angle::from_vector(Fx::from_int(3), Fx::from_int(4)).raw(),
        9672
    );
    assert_eq!(
        Angle::from_vector(-Fx::from_int(1), Fx::from_int(2)).raw(),
        21220
    );
}

/// Pins the generator, which drives every random decision in a match.
#[test]
fn rng_stream_is_stable() {
    use redshift_sim::SimRng;
    let mut rng = SimRng::new(0x1234_5678_9ABC_DEF0);
    let drawn: Vec<u32> = (0..8).map(|_| rng.next_u32()).collect();
    assert_eq!(
        drawn,
        vec![
            1129897928, 689246165, 17769723, 3219780061, 2503233616, 1385957503, 1440891452,
            2028510725,
        ]
    );
    assert_eq!(rng.state(), 8784588315254135370);
}
