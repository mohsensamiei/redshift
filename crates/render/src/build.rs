//! Queueing and siting buildings.
//!
//! Just the siting, now that the sidebar chooses what to build. This module
//! used to carry four function keys bound to four hard-coded structure names,
//! which meant the build list lived in the renderer rather than in the rules —
//! and one of those keys was `F5`, which also saved a replay, so queueing a
//! power plant wrote a file to disk every time.
//!
//! The one thing done properly rather than provisionally is the preview: a
//! player must be able to see whether a site is legal *before* committing, and
//! the answer has to come from the simulation rather than from a second copy of
//! the rules in the renderer. Two implementations of "can I build here" would
//! disagree eventually, and the disagreement would look like the game randomly
//! refusing valid sites.

use bevy::prelude::*;
use redshift_sim::command::CommandKind;
use redshift_sim::map::Cell;

use crate::camera::{GameCamera, screen_to_ground};
use crate::flat::{FlatMaterial, coloured};
use crate::session::Session;
use crate::world::fx_to_f32;

/// Marks the translucent footprint that follows the cursor.
#[derive(Component)]
pub struct PlacementPreview;

#[derive(Resource)]
pub struct PlacementAssets {
    pub mesh: Handle<Mesh>,
    pub valid: Handle<FlatMaterial>,
    pub invalid: Handle<FlatMaterial>,
}

pub fn build_placement_assets(
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<FlatMaterial>,
) -> PlacementAssets {
    let tint =
        |r: f32, g: f32, b: f32| crate::flat::FlatMaterial::unlit(Color::srgba(r, g, b, 0.45));
    PlacementAssets {
        // A unit cube, scaled to the footprint of whatever is pending.
        mesh: meshes.add(coloured(Cuboid::new(1.0, 1.0, 1.0))),
        valid: materials.add(tint(0.35, 0.9, 0.4)),
        invalid: materials.add(tint(0.9, 0.3, 0.25)),
    }
}

/// Shows where a pending structure would go, and whether it may.
pub fn update_placement_preview(
    mut commands: Commands,
    session: Res<Session>,
    assets: Res<PlacementAssets>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform), With<GameCamera>>,
    existing: Query<Entity, With<PlacementPreview>>,
    mut previews: Query<
        (&mut Transform, &mut MeshMaterial3d<FlatMaterial>),
        With<PlacementPreview>,
    >,
) {
    let local = session.local_player();
    let pending = session.sim().ready_to_place(local);

    let Some((_, kind)) = pending else {
        // Nothing waiting: clear any preview left over.
        for entity in &existing {
            commands.entity(entity).despawn();
        }
        return;
    };

    let Some(cell) = cursor_cell(&windows, &cameras) else {
        return;
    };
    let stats = session.sim().stats().get(local, kind);
    let (w, h) = stats.footprint;
    let legal = session.sim().can_place_kind(local, kind, cell);

    if existing.is_empty() {
        commands.spawn((
            Mesh3d(assets.mesh.clone()),
            MeshMaterial3d(assets.valid.clone()),
            Transform::IDENTITY,
            PlacementPreview,
        ));
        return;
    }

    for (mut transform, mut material) in &mut previews {
        // Centred on the footprint, which starts at the cursor's cell.
        transform.translation = Vec3::new(
            cell.x as f32 + w as f32 / 2.0,
            fx_to_f32(stats.radius),
            cell.y as f32 + h as f32 / 2.0,
        );
        transform.scale = Vec3::new(
            w as f32,
            stats.radius.raw() as f32 / 65536.0 * 2.0,
            h as f32,
        );

        let wanted = if legal {
            &assets.valid
        } else {
            &assets.invalid
        };
        if material.0.id() != wanted.id() {
            material.0 = wanted.clone();
        }
    }
}

/// Left click sites the pending structure.
pub fn handle_placement_click(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform), With<GameCamera>>,
    mut session: ResMut<Session>,
) {
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    let local = session.local_player();
    let Some((producer, _)) = session.sim().ready_to_place(local) else {
        return;
    };
    let Some(cell) = cursor_cell(&windows, &cameras) else {
        return;
    };

    // Sent whether or not the renderer thinks it is legal. The simulation is
    // the authority, and a client that filtered orders would let a modified one
    // through while quietly disagreeing with its own peers.
    session.issue(CommandKind::PlaceBuilding { producer, at: cell });
}

/// Whether a placement is in progress, so selection can stand aside.
pub fn placing(session: &Session) -> bool {
    session
        .sim()
        .ready_to_place(session.local_player())
        .is_some()
}

fn cursor_cell(
    windows: &Query<&Window>,
    cameras: &Query<(&Camera, &GlobalTransform), With<GameCamera>>,
) -> Option<Cell> {
    let window = windows.single().ok()?;
    let (camera, transform) = cameras.single().ok()?;
    let cursor = window.cursor_position()?;
    let ground = screen_to_ground(camera, transform, cursor)?;
    Some(Cell::new(ground.x.floor() as i32, ground.y.floor() as i32))
}
