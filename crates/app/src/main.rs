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
        app.add_systems(Update, (demo_order, demo_build));
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
/// Drives the build loop in demo mode: queue a power plant, then site it.
///
/// Exists to prove the whole chain end to end on screen — queue, pay, wait,
/// place, and the power reading changing as a result. The headless tests cover
/// each link; this is the one that shows them joined up.
fn demo_build(mut session: ResMut<Session>, mut frames: Local<u32>, mut queued: Local<bool>) {
    *frames += 1;
    let local = session.local_player();

    if !*queued && *frames > 120 {
        let Some(kind) = session.sim().rules().kind_of("power_plant") else {
            return;
        };
        if let Some(building) = session.sim().producer_for(local, kind) {
            session.issue(redshift_sim::command::CommandKind::Produce { building, kind });
            *queued = true;
        }
        return;
    }

    // Once it is ready, put it somewhere legal near the yard.
    if let Some((producer, kind)) = session.sim().ready_to_place(local) {
        let footprint = session.sim().stats().get(local, kind).footprint;
        let origin = session
            .sim()
            .units()
            .get(producer)
            .map(|u| u.cell())
            .unwrap_or(redshift_sim::map::Cell::new(0, 0));

        let spot = (2..8)
            .flat_map(|r| (-r..=r).map(move |d| (r, d)))
            .find_map(|(r, d)| {
                let cell = redshift_sim::map::Cell::new(origin.x + r, origin.y + d);
                session
                    .sim()
                    .can_build_at(local, cell, footprint)
                    .then_some(cell)
            });
        if let Some(at) = spot {
            session.issue(redshift_sim::command::CommandKind::PlaceBuilding { producer, at });
        }
    }
}

fn demo_order(mut session: ResMut<Session>, mut frames: Local<u32>, mut issued: Local<bool>) {
    *frames += 1;
    if *issued || *frames < 60 {
        return;
    }
    *issued = true;

    let local = session.local_player();
    // Combat units only. Ordering the harvesters to attack would drag them off
    // their run, which is exactly what the demo is meant to show working.
    let units: Vec<_> = session
        .sim()
        .view()
        .units()
        .filter(|(_, u)| u.owner == local && u.harvest.is_none())
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
    let refinery = kind("refinery");
    let construction_yard = kind("construction_yard");
    let mcv = kind("mcv");

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
        // A refinery and two harvesters each, so the economy runs from the
        // first tick. Placement is mirrored so neither side starts closer to
        // its ore than the other.
        // A construction yard, so the player has somewhere to build from and the
        // placement rules have something to anchor a build area to.
        spawns.push(Spawn {
            owner,
            kind: construction_yard,
            pos: Cell::new(base_x - dx * 9, base_y - dy * 6).centre(),
        });
        spawns.push(Spawn {
            owner,
            kind: refinery,
            pos: Cell::new(base_x - dx * 6, base_y - dy * 6).centre(),
        });
        for i in 0..2i32 {
            spawns.push(Spawn {
                owner,
                kind: harvester,
                pos: Cell::new(base_x - dx * 4, base_y - dy * (5 + i)).centre(),
            });
        }
        // A spare MCV, so deploying is something the demo can actually be used
        // to try. Parked clear of the base: it needs three by three of empty
        // ground to unpack into.
        spawns.push(Spawn {
            owner,
            kind: mcv,
            pos: Cell::new(base_x - dx * 9, base_y - dy * 11).centre(),
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

    // The two dividing walls are high ground rather than rock, now that the map
    // can tell the difference. Same barrier, but a plateau a player can fight
    // for instead of a wall they can only walk around — three cells wide, so
    // there is somewhere to stand on top.
    map.raise_rect(Cell::new(15, 0), Cell::new(17, 30), 2);
    map.raise_rect(Cell::new(31, 18), Cell::new(33, 47), 2);
    // A ramp into each, so the high ground is worth contesting rather than
    // merely being in the way. One level of step is walkable.
    map.raise_rect(Cell::new(15, 12), Cell::new(17, 13), 1);
    map.raise_rect(Cell::new(31, 30), Cell::new(33, 31), 1);

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
    // Ore fields: one near each start, and a contested pair in the middle.
    // Placed rather than scattered randomly so the map reads the same every
    // match and the two players start with the same opportunity.
    map.add_ore_field(Cell::new(8, 8), 4, 400);
    map.add_ore_field(Cell::new(40, 40), 4, 400);
    map.add_ore_field(Cell::new(24, 12), 3, 300);
    map.add_ore_field(Cell::new(24, 36), 3, 300);

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
