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
use redshift_sim::sim::MatchSetup;

mod bench;
mod mapfile;
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
    // `--ai <difficulty>` gives player two a computer opponent. Only `dummy`
    // exists so far, and it is the default: an opponent that builds, defends
    // and never attacks is the one worth having first, because it forces every
    // piece of the machinery except choosing a target.
    if !flag("--host") && !flag("--join") {
        let difficulty = match value("--ai").as_deref() {
            None | Some("dummy") => Some(redshift_ai::Difficulty::Dummy),
            Some("none") => None,
            Some(other) => {
                eprintln!("unknown difficulty {other:?}; only \"dummy\" exists so far");
                Some(redshift_ai::Difficulty::Dummy)
            }
        };
        if let Some(difficulty) = difficulty {
            app.insert_resource(redshift_render::opponent::Opponents {
                commanders: vec![redshift_ai::Commander::new(PlayerId(1), difficulty)],
            });
        }
    }

    // `--watch` restarts the match whenever the rules or maps change on disk.
    // For turning a number and seeing what it does, which is most of what the
    // remaining unverified figures need.
    if flag("--watch") {
        app.insert_resource(redshift_render::reload::RulesWatch::new(vec![
            rules_root(),
            map_root(),
        ]))
        .add_systems(Update, restart_on_change);
    }

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
    let path = map_root().join("crossing.ron");
    let def = redshift_data::map::MapDef::load(&path)
        .unwrap_or_else(|e| panic!("could not load {}: {e}", path.display()));
    mapfile::match_setup(&def, rules, seed, 2)
        .unwrap_or_else(|e| panic!("could not start on {}: {e}", def.name))
}

/// Rebuilds the match from disk when a watched file changes.
///
/// The seed is kept, so the same map and the same opening play out again with
/// whatever numbers were just edited. That is the point: the comparison is only
/// worth anything if everything except the edited value is the same.
fn restart_on_change(
    mut changed: MessageReader<redshift_render::reload::RulesChanged>,
    mut session: ResMut<redshift_render::session::Session>,
) {
    if changed.read().next().is_none() {
        return;
    }
    // Any further messages this frame are the same save. Draining them keeps a
    // burst of writes — which is what an editor saving a file looks like — from
    // restarting the match once per file.
    changed.clear();

    // A failed reload leaves the match running on the rules it already has.
    // Refusing to start is the wrong answer while somebody is editing: a RON
    // file is briefly invalid every time it is saved mid-keystroke.
    match std::panic::catch_unwind(|| skirmish_setup(RELOAD_SEED)) {
        Ok(setup) => {
            *session =
                redshift_render::session::Session::new(MatchSession::solo(setup, PlayerId(0)));
        }
        Err(_) => eprintln!("the edited files did not load; keeping the current match"),
    }
}

/// The seed a reloaded match uses.
///
/// Fixed rather than fresh, so two runs differ only by what was edited.
const RELOAD_SEED: u64 = 0xC0FF_EE00_1234_5678;

/// Where the shipped maps live.
///
/// Beside the rules and found the same way: relative to the crate rather than
/// to the working directory, so `cargo run` from anywhere finds them.
fn map_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../maps")
}

/// Loads the shipped rules.
///
/// Searched relative to the working directory, then to the crate, so the game
/// runs both from a checkout and from `cargo run` in a subdirectory. A missing
/// or invalid rules tree is fatal and says why — starting a match with default
/// values would look like a physics bug rather than a missing file.
/// Where the shipped rules live.
fn rules_root() -> std::path::PathBuf {
    let local = std::path::PathBuf::from("rules");
    if local.is_dir() {
        return local;
    }
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rules")
}

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
