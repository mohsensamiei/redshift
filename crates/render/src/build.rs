//! Queueing and siting buildings.
//!
//! Deliberately spare. A real sidebar with build tabs and icons is Phase 4
//! work; this is enough to exercise the placement rules with a mouse, which is
//! the part that cannot be tested headlessly.
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
use crate::session::Session;
use crate::world::fx_to_f32;

/// Structures the number keys queue, in order.
///
/// Named rather than indexed, so the binding survives entities being added to
/// the rules in a different order.
/// Structures the function keys queue.
///
/// Not the number keys: those are control groups, as they were in the original
/// and as anyone who has played one of these will assume. These bindings are a
/// stopgap until there is a real sidebar to click.
const HOTKEYS: [(KeyCode, &str); 4] = [
    (KeyCode::F5, "power_plant"),
    (KeyCode::F6, "barracks"),
    (KeyCode::F7, "refinery"),
    (KeyCode::F8, "war_factory"),
];

/// Marks the translucent footprint that follows the cursor.
#[derive(Component)]
pub struct PlacementPreview;

#[derive(Resource)]
pub struct PlacementAssets {
    pub mesh: Handle<Mesh>,
    pub valid: Handle<StandardMaterial>,
    pub invalid: Handle<StandardMaterial>,
}

pub fn build_placement_assets(
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> PlacementAssets {
    let tint = |r: f32, g: f32, b: f32| StandardMaterial {
        base_color: Color::srgba(r, g, b, 0.45),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    };
    PlacementAssets {
        // A unit cube, scaled to the footprint of whatever is pending.
        mesh: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        valid: materials.add(tint(0.35, 0.9, 0.4)),
        invalid: materials.add(tint(0.9, 0.3, 0.25)),
    }
}

/// Number keys queue a structure at the player's construction yard.
pub fn handle_build_hotkeys(keys: Res<ButtonInput<KeyCode>>, mut session: ResMut<Session>) {
    let pressed = HOTKEYS.iter().find(|(key, _)| keys.just_pressed(*key));
    let Some((_, id)) = pressed else { return };

    let local = session.local_player();
    let Some(kind) = session.sim().rules().kind_of(id) else {
        warn!("no entity named {id:?} in the rules");
        return;
    };

    // Asked of the simulation rather than guessed at. The renderer deciding
    // which building makes what would be a second copy of a rule that lives in
    // the data — and the first version of this looked for a building that
    // already had a queue, which no building has until something is queued, so
    // the hotkeys silently did nothing.
    let Some(building) = session.sim().producer_for(local, kind) else {
        return;
    };

    session.issue(CommandKind::Produce { building, kind });
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
        (&mut Transform, &mut MeshMaterial3d<StandardMaterial>),
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
    let legal = session.sim().can_build_at(local, cell, (w, h));

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
