//! The performance budget check, run with `--bench`.
//!
//! `docs/04-rendering.md` calls the budget "a test, not a wish". This is the
//! test. It runs the simulation headless under a deliberately heavy load and
//! reports every measurable ceiling.
//!
//! Rendering ceilings — frame time, triangle count — need a window and are
//! checked by the on-screen overlay instead. This covers the simulation side,
//! which is the part that runs on the dedicated server too and the part CI can
//! check without a GPU.

use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map, Terrain};
use redshift_sim::sim::{MatchSetup, Sim};
use redshift_sim::{EntityId, TICK_MS};

/// Units on the field. The budget in `docs/04-rendering.md` is stated at "a few
/// hundred"; 400 is the number quoted for the simulation tick.
const UNIT_COUNT: i32 = 400;

/// Ticks to run. 600 at 20 Hz is thirty seconds of play.
const TICKS: u32 = 600;

/// Milliseconds per simulation tick, from `docs/04-rendering.md`.
const SIM_TICK_BUDGET_MS: f32 = 5.0;

pub fn run() -> i32 {
    println!("Redshift performance budget check");
    println!(
        "  {UNIT_COUNT} units, {TICKS} ticks ({} seconds of play)\n",
        TICKS / 20
    );

    let mut sim = Sim::new(setup());
    let ids: Vec<EntityId> = sim.units().ids();

    let mut worst_ms = 0.0f32;
    let mut total_ms = 0.0f32;
    let mut worst_tick = 0;

    for tick in 0..TICKS {
        // Re-order everything every few seconds, so pathfinding is under
        // constant load rather than settling into an idle steady state.
        let mut commands = Vec::new();
        if tick % 100 == 1 {
            let corner = (tick / 100) % 4;
            let target = match corner {
                0 => Cell::new(60, 60),
                1 => Cell::new(3, 60),
                2 => Cell::new(60, 3),
                _ => Cell::new(3, 3),
            };
            commands.push(Command::new(
                PlayerId(0),
                0,
                CommandKind::Move {
                    units: ids.clone(),
                    target,
                },
            ));
        }

        let started = std::time::Instant::now();
        sim.tick(&commands);
        let elapsed = started.elapsed().as_secs_f32() * 1000.0;

        total_ms += elapsed;
        if elapsed > worst_ms {
            worst_ms = elapsed;
            worst_tick = tick;
        }
    }

    let mean_ms = total_ms / TICKS as f32;
    let realtime_headroom = TICK_MS as f32 / worst_ms;

    let mut failures = 0;
    println!("{:<22}{:>10}  {:>10}", "metric", "value", "ceiling");
    println!("{}", "-".repeat(46));
    failures += report("sim tick, mean", mean_ms, SIM_TICK_BUDGET_MS, "ms");
    failures += report("sim tick, worst", worst_ms, SIM_TICK_BUDGET_MS, "ms");
    println!("{:<22}{:>10}  {:>10}", "worst tick number", worst_tick, "");
    println!(
        "{:<22}{:>10.1}x {:>10}",
        "realtime headroom", realtime_headroom, ">1 needed"
    );
    println!(
        "{:<22}{:>10}  {:>10}",
        "units",
        sim.units().len(),
        UNIT_COUNT
    );

    println!();
    if failures == 0 {
        println!("PASS — within budget");
        0
    } else {
        println!("FAIL — {failures} metric(s) over budget");
        println!("See docs/04-rendering.md. The budget is not the thing to change.");
        1
    }
}

fn report(name: &str, value: f32, ceiling: f32, unit: &str) -> i32 {
    let over = value > ceiling;
    println!(
        "{:<22}{:>10.3}  {:>10}  {}",
        name,
        value,
        format!("{ceiling:.1}{unit}"),
        if over { "OVER" } else { "ok" }
    );
    over as i32
}

/// A deliberately awkward map: obstacles everywhere, so pathfinding cannot
/// shortcut. Measuring on open ground would flatter the numbers.
fn setup() -> MatchSetup {
    let mut map = Map::new(64, 64);
    for i in 0..6 {
        let x = 8 + i * 9;
        let (top, bottom) = if i % 2 == 0 { (0, 44) } else { (18, 63) };
        map.fill_rect(Cell::new(x, top), Cell::new(x, bottom), Terrain::Rock);
    }
    map.fill_rect(Cell::new(2, 30), Cell::new(12, 33), Terrain::Water);

    let mut spawns = Vec::new();
    let mut placed = 0;
    let mut y = 2;
    while placed < UNIT_COUNT {
        for x in 2..8 {
            if placed >= UNIT_COUNT {
                break;
            }
            let cell = Cell::new(x, y);
            if map.is_passable(cell, redshift_sim::Locomotor::Tracked) {
                spawns.push((PlayerId(0), cell.centre()));
                placed += 1;
            }
        }
        y += 1;
        if y >= 62 {
            y = 2;
        }
    }
    MatchSetup::for_test(0xBE0C_0DE0_0000_0001, map, spawns)
}
