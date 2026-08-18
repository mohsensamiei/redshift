//! The game client.
//!
//! Thin by design: parse arguments, build a match, hand it to the renderer.
//! Everything of substance lives in `redshift-sim` and `redshift-render`.

use bevy::prelude::*;
use redshift_net::MatchSession;
use redshift_render::RedshiftRenderPlugin;
use redshift_render::session::Session;
use redshift_sim::Rules;
use redshift_sim::command::PlayerId;
use redshift_sim::map::{Cell, Map, Terrain};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Spawn};

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
    let rules = load_rules();
    let map = skirmish_map();

    // Kinds are looked up by name once, here, rather than deep in the spawn
    // loop: a typo should stop the match starting with a clear message, not
    // produce an army of whatever kind happened to be at index zero.
    let kind = |id: &str| {
        rules
            .kind_of(id)
            .unwrap_or_else(|| panic!("rules/ has no entity named {id:?}"))
    };
    let tank = kind("grizzly_tank");
    let infantry = kind("gi");
    let harvester = kind("harvester");

    let mut spawns = Vec::new();
    // Close enough that a demo run reaches contact, far enough that the walk
    // there still exercises pathfinding.
    for (owner, base_x, base_y, dx, dy) in
        [(PlayerId(0), 18, 20, 1, 1), (PlayerId(1), 28, 26, -1, -1)]
    {
        for i in 0..6i32 {
            spawns.push(Spawn {
                owner,
                kind: tank,
                pos: Cell::new(base_x + dx * (i % 3), base_y + dy * (i / 3)).centre(),
            });
        }
        for i in 0..4i32 {
            spawns.push(Spawn {
                owner,
                kind: infantry,
                pos: Cell::new(base_x + dx * (i % 2 + 3), base_y + dy * (i / 2)).centre(),
            });
        }
        spawns.push(Spawn {
            owner,
            kind: harvester,
            pos: Cell::new(base_x + dx * 5, base_y + dy * 2).centre(),
        });
    }

    MatchSetup {
        seed,
        map,
        rules,
        players: vec![
            PlayerSetup {
                id: PlayerId(0),
                faction: Some("america".into()),
            },
            PlayerSetup {
                id: PlayerId(1),
                faction: Some("korea".into()),
            },
        ],
        spawns,
    }
}

/// The placeholder skirmish map, until maps become data.
///
/// Laid out so pathfinding has something to do: two walls with staggered
/// openings, and water between the starting positions, so a straight line is
/// never the answer.
fn skirmish_map() -> Map {
    let mut map = Map::new(48, 48);

    map.fill_rect(Cell::new(16, 0), Cell::new(16, 30), Terrain::Rock);
    map.fill_rect(Cell::new(32, 18), Cell::new(32, 47), Terrain::Rock);

    // Water is impassable to everything on the ground, and a clear visual break.
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
    map
}

/// Loads the shipped rules.
///
/// Searched relative to the working directory, then to the crate, so the game
/// runs both from a checkout and from `cargo run` in a subdirectory. A missing
/// or invalid rules tree is fatal and says why — starting a match with default
/// values would look like a physics bug rather than a missing file.
fn load_rules() -> Rules {
    let candidates = [
        std::path::PathBuf::from("rules"),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rules"),
    ];
    for path in &candidates {
        if !path.is_dir() {
            continue;
        }
        match Rules::load_from(path) {
            Ok(rules) => {
                println!(
                    "loaded {} entities, {} weapons, {} factions from {} (hash {:016x})",
                    rules.entity_count(),
                    rules.weapon_count(),
                    rules.faction_count(),
                    path.display(),
                    rules.hash()
                );
                return rules;
            }
            Err(e) => {
                eprintln!("rules in {} are invalid:\n  {e}", path.display());
                std::process::exit(1);
            }
        }
    }
    eprintln!(
        "could not find a rules/ directory. Looked in: {}",
        candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    std::process::exit(1);
}
