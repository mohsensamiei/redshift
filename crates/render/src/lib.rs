//! # redshift-render
//!
//! The presentation layer: a Bevy plugin that draws the simulation.
//!
//! ## The one rule
//!
//! Data flows one way. This crate reads [`Session`] and draws what it finds. It
//! **never** mutates simulation state. Player input becomes a command, which
//! enters the simulation through the session's queue at a scheduled tick — the
//! same path a network match uses.
//!
//! Floating point is used freely here. That is fine: nothing in this crate
//! feeds back into the simulation, and `Fx` deliberately offers no conversion
//! from a float to make the boundary hard to cross by accident.
//!
//! See `docs/01-architecture.md` and `docs/04-rendering.md`.

use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution};

pub mod build;
pub mod camera;
pub mod health;
pub mod input;
pub mod overlay;
pub mod session;
pub mod world;

pub use session::Session;

/// Where and how large the window should open.
///
/// Insert before running to override the default. Exists so two clients can be
/// placed side by side on one screen, which is how a networked match is watched
/// without two machines.
#[derive(Resource, Clone, Copy)]
pub struct WindowPlacement {
    pub position: Option<IVec2>,
    pub size: Option<UVec2>,
}

/// Captures a screenshot after a set number of frames, then exits.
///
/// Insert this resource before running to take an automated screenshot. Used
/// for verifying a build renders correctly without needing a human at the
/// keyboard — and, on macOS, without needing Screen Recording permission.
#[derive(Resource)]
pub struct AutoScreenshot {
    pub path: String,
    /// Frames to wait first. A few are needed for assets to load and the first
    /// simulation ticks to run, or the shot catches an empty world.
    pub after_frames: u32,
    pub exit_after: bool,
}

fn auto_screenshot(
    mut commands: Commands,
    config: Option<Res<AutoScreenshot>>,
    mut frames: Local<u32>,
    mut done: Local<bool>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(config) = config else { return };
    *frames += 1;

    if !*done && *frames >= config.after_frames {
        *done = true;
        let path = config.path.clone();
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(path.clone()));
        eprintln!("screenshot requested: {path}");
    }

    // A few frames after the request, so the capture has been written.
    if *done && config.exit_after && *frames >= config.after_frames + 10 {
        exit.write(AppExit::Success);
    }
}

/// Everything needed to draw and drive a match.
pub struct RedshiftRenderPlugin;

impl Plugin for RedshiftRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Redshift".into(),
                        resolution: WindowResolution::new(1280, 720),
                        // Vsync, always. An uncapped renderer would draw
                        // hundreds of frames a second in a scene this light,
                        // saturating the GPU and heating the machine for no
                        // perceptible benefit. This is the single most
                        // important line in the file for the power budget —
                        // see docs/04-rendering.md.
                        present_mode: PresentMode::AutoVsync,
                        ..default()
                    }),
                    ..default()
                })
                // No shadow maps anywhere in the pipeline.
                .set(bevy::pbr::PbrPlugin {
                    add_default_deferred_lighting_plugin: false,
                    ..default()
                }),
        )
        .add_plugins(FrameTimeDiagnosticsPlugin::default())
        .init_resource::<input::Selection>()
        .init_resource::<input::DragState>()
        .init_resource::<overlay::OverlayState>()
        .init_resource::<world::TerrainBuiltAt>()
        .insert_resource(ClearColor(Color::srgb(0.05, 0.06, 0.08)))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                // Ordered deliberately: input is gathered, then the simulation
                // advances, then the world is drawn from the result. Drawing
                // before the tick would show a frame of stale state.
                (
                    input::handle_selection,
                    input::handle_orders,
                    input::handle_hotkeys,
                ),
                session::advance_session,
                (
                    world::rebuild_terrain,
                    world::sync_units,
                    world::interpolate_units,
                )
                    .chain(),
                (input::sync_selection_rings, input::move_selection_rings).chain(),
                (health::sync_health_bars, health::update_health_bars).chain(),
                (
                    build::handle_build_hotkeys,
                    build::update_placement_preview,
                    build::handle_placement_click,
                )
                    .chain(),
                camera::update_camera,
                (
                    overlay::toggle_overlay,
                    overlay::count_triangles,
                    overlay::update_overlay,
                ),
                apply_window_placement,
                auto_screenshot,
            )
                .chain(),
        );
    }
}

/// Builds the scene once the session exists.
/// Applies a requested window position and size once the window exists.
fn apply_window_placement(
    placement: Option<Res<WindowPlacement>>,
    mut windows: Query<&mut Window>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Some(placement) = placement else {
        *done = true;
        return;
    };
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    if let Some(position) = placement.position {
        window.position = WindowPosition::At(position);
    }
    if let Some(size) = placement.size {
        window.resolution = WindowResolution::new(size.x, size.y);
    }
    *done = true;
}

fn setup(
    mut commands: Commands,
    session: Res<Session>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let map = session.sim().map();
    let mut rig = camera::CameraRig::new(map.width() as f32, map.height() as f32);

    // Open on the player's own units rather than the middle of the map. With a
    // 24-cell viewport and starting positions in the corners, centring on the
    // map means the player begins looking at empty ground.
    if let Some(focus) = starting_focus(&session) {
        rig.focus = focus;
    }

    let terrain_mesh = meshes.add(world::build_terrain_mesh(
        map,
        session.sim().visibility(),
        session.local_player(),
    ));
    let terrain_material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        ..default()
    });
    commands.spawn((
        Mesh3d(terrain_mesh),
        MeshMaterial3d(terrain_material),
        Transform::IDENTITY,
        world::TerrainMesh,
    ));

    let assets = world::build_assets(
        session.sim().rules(),
        session.sim().stats(),
        &mut meshes,
        &mut materials,
    );
    commands.insert_resource(assets);

    let health_assets = health::build_health_assets(&mut meshes, &mut materials);
    commands.insert_resource(health_assets);

    let placement_assets = build::build_placement_assets(&mut meshes, &mut materials);
    commands.insert_resource(placement_assets);

    world::spawn_lighting(&mut commands);
    camera::spawn_camera(&mut commands, &rig);
    overlay::spawn_overlay(&mut commands);

    commands.insert_resource(rig);
}

/// The centroid of the local player's starting units.
fn starting_focus(session: &Session) -> Option<Vec2> {
    let mut sum = Vec2::ZERO;
    let mut count = 0;
    for (_, unit) in session.sim().view().units() {
        if unit.owner != session.local_player() {
            continue;
        }
        sum += Vec2::new(world::fx_to_f32(unit.pos.x), world::fx_to_f32(unit.pos.y));
        count += 1;
    }
    (count > 0).then(|| sum / count as f32)
}
