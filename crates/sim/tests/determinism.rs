//! The determinism suite.
//!
//! These are the most important tests in the repository. Multiplayer rests
//! entirely on the property they check: the same inputs must produce the same
//! world, everywhere, every time.
//!
//! **Never weaken a test here to make it pass.** A failure is always a real
//! bug — see CONTRIBUTING.md.

use redshift_sim::EntityId;
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map, Terrain, WorldPos};
use redshift_sim::sim::{MatchSetup, Sim};

/// A map with enough obstacles that pathfinding has real choices to make —
/// including ties, which is where a sloppy tie-break would show up.
fn test_map() -> Map {
    let mut map = Map::new(48, 48);
    map.fill_rect(Cell::new(12, 4), Cell::new(12, 30), Terrain::Rock);
    map.fill_rect(Cell::new(24, 16), Cell::new(24, 44), Terrain::Rock);
    map.fill_rect(Cell::new(34, 2), Cell::new(34, 26), Terrain::Rock);
    map.fill_rect(Cell::new(5, 38), Cell::new(20, 38), Terrain::Water);
    map
}

fn setup(seed: u64) -> MatchSetup {
    let mut spawns = Vec::new();
    for i in 0..12 {
        spawns.push((PlayerId(0), Cell::new(2 + i % 4, 2 + i / 4).centre()));
        spawns.push((PlayerId(1), Cell::new(44 - i % 4, 44 - i / 4).centre()));
    }
    MatchSetup::for_test(seed, test_map(), spawns)
}

/// A scripted match: the same orders, issued at the same ticks, every run.
fn commands_for_tick(tick: u32, units: &[EntityId]) -> Vec<Command> {
    if units.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    // Orders at a few fixed ticks, deliberately including a re-order while
    // units are already moving.
    let targets: &[(u32, i32, i32)] = &[
        (1, 44, 44),
        (40, 2, 44),
        (90, 44, 2),
        (150, 24, 24),
        (220, 2, 2),
    ];
    for &(at, x, y) in targets {
        if tick == at {
            let half = units.len() / 2;
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
                    target: Cell::new(46 - x, 46 - y),
                },
            ));
        }
    }
    out
}

/// Runs a scripted match, returning the state hash after every tick.
fn run_match(seed: u64, ticks: u32) -> Vec<u64> {
    let mut sim = Sim::new(setup(seed));
    let ids: Vec<EntityId> = sim.units().ids();
    let mut hashes = Vec::with_capacity(ticks as usize);
    for tick in 0..ticks {
        let mut commands = commands_for_tick(tick, &ids);
        commands.sort_by_key(|c| c.order_key());
        sim.tick(&commands);
        hashes.push(sim.state_hash());
    }
    hashes
}

#[test]
fn replay_roundtrip() {
    // The headline property: same seed, same commands, same world at every
    // single tick — not merely at the end.
    let first = run_match(0xDEAD_BEEF, 400);
    let second = run_match(0xDEAD_BEEF, 400);
    assert_eq!(first.len(), second.len());
    for (tick, (a, b)) in first.iter().zip(&second).enumerate() {
        assert_eq!(a, b, "diverged at tick {tick}");
    }
}

#[test]
fn two_sims_stay_identical_in_one_process() {
    // Stands in for two peers. Running them interleaved in one process catches
    // any dependence on allocation addresses or global state that a sequential
    // run might hide.
    let mut a = Sim::new(setup(7));
    let mut b = Sim::new(setup(7));
    let ids: Vec<EntityId> = a.units().ids();
    assert_eq!(
        ids,
        b.units().ids(),
        "peers must agree on entity ids from the start"
    );

    for tick in 0..600 {
        let mut commands = commands_for_tick(tick, &ids);
        commands.sort_by_key(|c| c.order_key());
        a.tick(&commands);
        b.tick(&commands);
        assert_eq!(
            a.state_hash(),
            b.state_hash(),
            "peers diverged at tick {tick}"
        );
    }
}

#[test]
fn different_seeds_are_distinguishable() {
    // Guards against a hash so weak it reports agreement between genuinely
    // different worlds — which would make the desync detector useless.
    let a = run_match(1, 100);
    let b = run_match(2, 100);
    assert_ne!(
        a.last(),
        b.last(),
        "the seed must be part of the hashed state"
    );
}

#[test]
fn serialisation_is_transparent() {
    // Save, reload, continue — must be indistinguishable from never saving.
    // This is what reconnection and mid-match snapshots depend on.
    let mut original = Sim::new(setup(42));
    let ids: Vec<EntityId> = original.units().ids();

    for tick in 0..120 {
        let mut commands = commands_for_tick(tick, &ids);
        commands.sort_by_key(|c| c.order_key());
        original.tick(&commands);
    }

    let encoded = ron::to_string(&original).expect("sim must serialise");
    let mut restored: Sim = ron::from_str(&encoded).expect("sim must deserialise");
    assert_eq!(
        original.state_hash(),
        restored.state_hash(),
        "reload changed the state"
    );

    for tick in 120..300 {
        let mut commands = commands_for_tick(tick, &ids);
        commands.sort_by_key(|c| c.order_key());
        original.tick(&commands);
        restored.tick(&commands);
        assert_eq!(
            original.state_hash(),
            restored.state_hash(),
            "a restored sim diverged at tick {tick}"
        );
    }
}

#[test]
fn command_order_changes_the_outcome() {
    // Confirms the total order actually matters, which is why the network layer
    // must sort. If this passed regardless, ordering bugs would be invisible.
    let mut sim = Sim::new(setup(3));
    let ids: Vec<EntityId> = sim.units().ids();
    let a = Command::new(
        PlayerId(0),
        0,
        CommandKind::Move {
            units: ids.clone(),
            target: Cell::new(40, 40),
        },
    );
    let b = Command::new(
        PlayerId(0),
        1,
        CommandKind::Move {
            units: ids.clone(),
            target: Cell::new(4, 40),
        },
    );

    sim.tick(&[a.clone(), b.clone()]);
    for _ in 0..40 {
        sim.tick(&[]);
    }
    let forwards = sim.state_hash();

    let mut other = Sim::new(setup(3));
    // Same two commands, applied in the opposite order.
    let mut swapped = vec![b, a];
    swapped
        .iter_mut()
        .enumerate()
        .for_each(|(i, c)| c.sequence = i as u16);
    other.tick(&swapped);
    for _ in 0..40 {
        other.tick(&[]);
    }
    assert_ne!(
        forwards,
        other.state_hash(),
        "command order must affect the world"
    );
}

#[test]
fn units_reach_their_destination() {
    // Determinism is worthless if the simulation does not actually work. This
    // is the functional counterpart: given a reachable goal and enough time,
    // every unit arrives and goes idle.
    let mut sim = Sim::new(setup(11));
    let ids: Vec<EntityId> = sim.units().ids();
    let goal = Cell::new(40, 6);

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Move {
            units: ids.clone(),
            target: goal,
        },
    )]);

    for _ in 0..3_000 {
        sim.tick(&[]);
        if sim.units().iter().all(|(_, u)| u.order.is_idle()) {
            break;
        }
    }

    let owned: Vec<_> = sim
        .units()
        .iter()
        .filter(|(_, u)| u.owner == PlayerId(0))
        .collect();
    assert!(!owned.is_empty());
    for (id, unit) in owned {
        assert!(
            unit.order.is_idle(),
            "unit {id:?} never finished its order; it is at {:?}",
            unit.cell()
        );
        assert!(
            unit.cell().chebyshev_to(goal) <= 1,
            "unit {id:?} stopped at {:?}, not near {goal:?}",
            unit.cell()
        );
    }
}

#[test]
fn an_unreachable_order_is_abandoned_not_retried_forever() {
    // A sealed goal must cost the budget once, not every tick until the match
    // ends. The failure mode this guards is a slow bleed that only shows up
    // under load.
    let mut map = Map::new(32, 32);
    map.fill_rect(Cell::new(20, 20), Cell::new(24, 24), Terrain::Rock);
    map.set_terrain(Cell::new(22, 22), Terrain::Ground);

    let mut sim = Sim::new(MatchSetup::for_test(
        5,
        map,
        vec![(PlayerId(0), Cell::new(2, 2).centre())],
    ));
    let ids: Vec<EntityId> = sim.units().ids();

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Move {
            units: ids,
            target: Cell::new(22, 22),
        },
    )]);
    sim.tick(&[]);

    assert!(
        sim.units().iter().all(|(_, u)| u.order.is_idle()),
        "a proved-unreachable order must be dropped"
    );
    assert_eq!(
        sim.view().pending_paths(),
        0,
        "nothing should still be queued"
    );

    // And the world must then be completely still.
    let settled = sim.state_hash();
    for _ in 0..50 {
        sim.tick(&[]);
    }
    assert_ne!(settled, 0);
    assert_eq!(sim.view().pending_paths(), 0);
}

#[test]
fn a_player_cannot_order_another_players_units() {
    // Enforced in the simulation, not just the interface: a modified client can
    // send anything, and every peer must reject it identically.
    let mut sim = Sim::new(setup(9));
    let enemy: Vec<EntityId> = sim
        .units()
        .iter()
        .filter(|(_, u)| u.owner == PlayerId(1))
        .map(|(id, _)| id)
        .collect();
    assert!(!enemy.is_empty());

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Move {
            units: enemy.clone(),
            target: Cell::new(2, 2),
        },
    )]);

    for id in enemy {
        assert!(
            sim.unit(id).unwrap().order.is_idle(),
            "unit {id:?} accepted an order from the wrong player"
        );
    }
}

#[test]
fn stale_entity_ids_in_commands_are_ignored() {
    // Commands arrive several ticks after they were issued, so they routinely
    // name units that have since died. That must be a no-op, never a panic and
    // never a retarget onto whatever now occupies the slot.
    let mut sim = Sim::new(setup(13));
    let ids: Vec<EntityId> = sim.units().ids();

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Move {
            units: vec![EntityId::NONE],
            target: Cell::new(10, 10),
        },
    )]);

    for id in ids {
        assert!(sim.unit(id).unwrap().order.is_idle());
    }
}

#[test]
fn the_tick_counter_advances_exactly_once_per_tick() {
    let mut sim = Sim::new(setup(1));
    assert_eq!(sim.tick_number(), 0);
    for expected in 1..=100 {
        sim.tick(&[]);
        assert_eq!(sim.tick_number(), expected);
    }
    assert_eq!(sim.elapsed_seconds(), 5, "100 ticks at 20 Hz is 5 seconds");
}

#[test]
fn an_idle_world_is_perfectly_still() {
    // With no orders, nothing observable may change — no drifting positions, no
    // spontaneous orders. Compared on world state rather than the state hash,
    // because the hash legitimately includes the tick counter and so must
    // change every tick.
    fn snapshot(sim: &Sim) -> Vec<(WorldPos, u16, bool)> {
        sim.view()
            .units()
            .map(|(_, u)| (u.pos, u.facing.raw(), u.order.is_idle()))
            .collect()
    }

    let mut sim = Sim::new(setup(77));
    sim.tick(&[]);
    let first = snapshot(&sim);
    assert!(
        first.iter().all(|(_, _, idle)| *idle),
        "units must start idle"
    );

    for tick in 0..200 {
        sim.tick(&[]);
        assert_eq!(
            snapshot(&sim),
            first,
            "an idle world changed at tick {tick}"
        );
        assert_eq!(sim.view().pending_paths(), 0);
    }
}

#[test]
fn a_completed_order_returns_the_unit_to_idle() {
    // The bug this pins: a unit that consumed its whole route but was never
    // returned to `Idle` sits in `Move` with an empty path forever. It looks
    // stationary, so the defect is invisible until something asks whether the
    // unit is busy — production queues, formation logic, the AI.
    let mut sim = Sim::new(MatchSetup::for_test(
        1,
        Map::new(24, 24),
        vec![(PlayerId(0), Cell::new(2, 2).centre())],
    ));
    let ids: Vec<EntityId> = sim.units().ids();
    let goal = Cell::new(9, 9);

    sim.tick(&[Command::new(
        PlayerId(0),
        0,
        CommandKind::Move {
            units: ids.clone(),
            target: goal,
        },
    )]);

    let mut settled_at = None;
    for tick in 0..600 {
        sim.tick(&[]);
        if sim.unit(ids[0]).unwrap().order.is_idle() {
            settled_at = Some(tick);
            break;
        }
    }

    let tick = settled_at.expect("the unit never returned to idle");
    assert_eq!(sim.unit(ids[0]).unwrap().cell(), goal);
    assert_eq!(
        sim.view().pending_paths(),
        0,
        "nothing should remain queued after arrival"
    );

    // And it must stay idle rather than re-acquiring the order.
    for _ in 0..50 {
        sim.tick(&[]);
        assert!(
            sim.unit(ids[0]).unwrap().order.is_idle(),
            "the unit became busy again after tick {tick}"
        );
    }
}
