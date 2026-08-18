//! The game client.
//!
//! Thin by design: parse arguments, build a match, hand it to the renderer.
//! Everything of substance lives in `redshift-sim` and `redshift-render`.

use bevy::prelude::*;
use redshift_net::MatchSession;
use redshift_render::RedshiftRenderPlugin;
use redshift_render::session::Session;
use redshift_sim::command::PlayerId;
use redshift_sim::map::{Cell, Map, Terrain};
use redshift_sim::sim::MatchSetup;

mod bench;
mod netplay;

fn main() {
    if std::env::args().any(|a| a == "--bench") {
        // The performance budget check. Runs headless, prints a report, and
        // exits non-zero on a breach — see docs/04-rendering.md.
        std::process::exit(bench::run());
    }

    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };

    let options = netplay::NetOptions {
        port: value("--port")
            .and_then(|p| p.parse().ok())
            .unwrap_or(redshift_net::protocol::DEFAULT_GAME_PORT),
        name: value("--name").unwrap_or_else(|| "player".into()),
        discovery_port: value("--discovery-port")
            .and_then(|p| p.parse().ok())
            .unwrap_or(redshift_net::protocol::DISCOVERY_PORT),
    };

    // The seed comes from the host, so the setup has to be built once it is
    // known — every peer must construct an identical starting world.
    let build_setup = skirmish_setup;

    let session = if flag("--host") {
        match netplay::host(build_setup, &options) {
            Ok(session) => session,
            Err(e) => {
                eprintln!("could not host: {e}");
                std::process::exit(1);
            }
        }
    } else if flag("--join") {
        let address = value("--join").and_then(|a| a.parse().ok());
        match netplay::join(build_setup, address, &options) {
            Ok(session) => session,
            Err(e) => {
                eprintln!("could not join: {e}");
                std::process::exit(1);
            }
        }
    } else {
        MatchSession::solo(skirmish_setup(0xC0FF_EE00_1234_5678), PlayerId(0))
    };

    let mut app = App::new();
    app.insert_resource(Session::new(session))
        .add_plugins(RedshiftRenderPlugin);

    // `--demo` issues a move order shortly after start. It exists to exercise
    // the whole chain without a human at the keyboard — command queued, applied
    // at a scheduled tick, path found, units moved, renderer following — which
    // is the part unit tests cannot cover because it spans the engine boundary.
    if flag("--demo") {
        app.add_systems(Update, demo_order);
    }

    // `--screenshot <path>` renders a few seconds of play, writes a frame, and
    // exits. Useful for checking a build actually draws without sitting at it.
    // `--window-pos x,y` and `--window-size w,h` let two clients sit side by
    // side on one screen.
    let parse_pair = |raw: String| -> Option<(i32, i32)> {
        let (a, b) = raw.split_once(',')?;
        Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
    };
    let position = value("--window-pos").and_then(parse_pair);
    let size = value("--window-size").and_then(parse_pair);
    if position.is_some() || size.is_some() {
        app.insert_resource(redshift_render::WindowPlacement {
            position: position.map(|(x, y)| IVec2::new(x, y)),
            size: size.map(|(w, h)| UVec2::new(w as u32, h as u32)),
        });
    }

    if let Some(path) = value("--screenshot") {
        app.insert_resource(redshift_render::AutoScreenshot {
            path,
            after_frames: value("--screenshot-after")
                .and_then(|f| f.parse().ok())
                .unwrap_or(180),
            exit_after: true,
        });
    }

    app.run();
}

/// Orders every local unit across the map, once, a second in.
fn demo_order(mut session: ResMut<Session>, mut frames: Local<u32>, mut issued: Local<bool>) {
    *frames += 1;
    if *issued || *frames < 60 {
        return;
    }
    *issued = true;

    let local = session.local_player();
    let units: Vec<_> = session
        .sim()
        .view()
        .units()
        .filter(|(_, u)| u.owner == local)
        .map(|(id, _)| id)
        .collect();

    // Across the wall, so the route has to find the gap.
    session.issue(redshift_sim::command::CommandKind::Move {
        units,
        target: redshift_sim::map::Cell::new(26, 22),
    });
}

/// A placeholder skirmish, until maps become data in Phase 3.
///
/// Deliberately laid out so the pathfinding has something to do: two walls with
/// offset gaps, and a stretch of water, between the two starting positions.
///
/// Takes the seed rather than choosing one: in a network match the host picks
/// it, and every peer must build a bit-identical starting world.
fn skirmish_setup(seed: u64) -> MatchSetup {
    let mut map = Map::new(48, 48);

    // Walls with staggered openings, so a straight line is never the answer.
    map.fill_rect(Cell::new(16, 0), Cell::new(16, 30), Terrain::Rock);
    map.fill_rect(Cell::new(32, 18), Cell::new(32, 47), Terrain::Rock);

    // Water: impassable to everything on the ground, and a clear visual break.
    map.fill_rect(Cell::new(4, 38), Cell::new(14, 41), Terrain::Water);
    map.fill_rect(Cell::new(36, 4), Cell::new(44, 8), Terrain::Water);

    // Scattered rock, for texture and to give the A* tie-break something to do.
    for (x, y) in [
        (22, 8),
        (23, 8),
        (22, 9),
        (40, 30),
        (41, 30),
        (41, 31),
        (8, 20),
        (9, 20),
    ] {
        map.set_terrain(Cell::new(x, y), Terrain::Rock);
    }

    let mut spawns = Vec::new();
    for i in 0..12i32 {
        spawns.push((PlayerId(0), Cell::new(3 + i % 4, 3 + i / 4).centre()));
    }
    for i in 0..12i32 {
        spawns.push((PlayerId(1), Cell::new(41 + i % 4, 41 + i / 4).centre()));
    }

    MatchSetup { seed, map, spawns }
}
