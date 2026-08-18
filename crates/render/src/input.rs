//! Selection and orders.
//!
//! Input never reaches the simulation directly. A click becomes a
//! [`CommandKind`], which is queued on the [`Session`] and applied at a
//! scheduled tick — the same path a network match uses. See
//! `docs/01-architecture.md`.

use bevy::prelude::*;
use redshift_sim::command::CommandKind;
use redshift_sim::{Cell, EntityId};

use crate::camera::{CameraRig, GameCamera, screen_to_ground};
use crate::session::Session;
use crate::world::{RenderAssets, SelectionRing, UnitView, fx_to_f32};

/// How far the pointer must travel before a click becomes a box selection, in
/// pixels. Without a threshold, an ordinary click with a shaky hand would be
/// treated as a zero-area box and select nothing.
const DRAG_THRESHOLD: f32 = 6.0;

/// Radius within which a click counts as hitting a unit, in cells.
const CLICK_RADIUS: f32 = 0.45;

/// What the player currently has selected.
#[derive(Resource, Default)]
pub struct Selection {
    /// Selected units, kept sorted so the order handed to a command is stable.
    /// A command's contents feed the state hash, so an unsorted selection would
    /// make two clients disagree over an identical player action.
    pub units: Vec<EntityId>,
}

impl Selection {
    pub fn set(&mut self, mut units: Vec<EntityId>) {
        units.sort();
        units.dedup();
        self.units = units;
    }

    pub fn clear(&mut self) {
        self.units.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }
}

/// An in-progress selection drag.
#[derive(Resource, Default)]
pub struct DragState {
    pub start: Option<Vec2>,
    pub current: Vec2,
    pub is_box: bool,
}

/// Everything `handle_selection` needs to read from the world.
///
/// Grouped into one parameter rather than eight: Bevy allows a long system
/// signature, but a list that long stops being readable and makes the ordering
/// of arguments load-bearing.
#[derive(bevy::ecs::system::SystemParam)]
pub struct SelectionInput<'w, 's> {
    buttons: Res<'w, ButtonInput<MouseButton>>,
    keys: Res<'w, ButtonInput<KeyCode>>,
    windows: Query<'w, 's, &'static Window>,
    cameras: Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<GameCamera>>,
}

/// Handles left-click selection and left-drag box selection.
pub fn handle_selection(
    input: SelectionInput,
    session: Res<Session>,
    mut selection: ResMut<Selection>,
    mut drag: ResMut<DragState>,
    mut rig: ResMut<CameraRig>,
) {
    let (buttons, keys) = (&input.buttons, &input.keys);
    let Ok(window) = input.windows.single() else {
        return;
    };
    let Ok((camera, camera_transform)) = input.cameras.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };

    if buttons.just_pressed(MouseButton::Left) {
        drag.start = Some(cursor);
        drag.current = cursor;
        drag.is_box = false;
        // Edge panning during a drag would run the camera away from the box.
        rig.edge_pan_enabled = false;
    }

    if let Some(start) = drag.start
        && buttons.pressed(MouseButton::Left)
    {
        drag.current = cursor;
        if start.distance(cursor) > DRAG_THRESHOLD {
            drag.is_box = true;
        }
    }

    if buttons.just_released(MouseButton::Left) {
        let Some(start) = drag.start.take() else {
            return;
        };
        rig.edge_pan_enabled = true;

        let additive = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
        let mut picked: Vec<EntityId> = if additive {
            selection.units.clone()
        } else {
            Vec::new()
        };

        if drag.is_box {
            let (Some(a), Some(b)) = (
                screen_to_ground(camera, camera_transform, start),
                screen_to_ground(camera, camera_transform, cursor),
            ) else {
                return;
            };
            let min = a.min(b);
            let max = a.max(b);
            for (id, unit) in session.sim().view().units() {
                if unit.owner != session.local_player() {
                    continue;
                }
                let pos = Vec2::new(fx_to_f32(unit.pos.x), fx_to_f32(unit.pos.y));
                if pos.x >= min.x && pos.x <= max.x && pos.y >= min.y && pos.y <= max.y {
                    picked.push(id);
                }
            }
        } else {
            let Some(ground) = screen_to_ground(camera, camera_transform, cursor) else {
                return;
            };
            // Nearest unit within the click radius, rather than the first one
            // found — with units overlapping, "first" would depend on iteration
            // order and feel arbitrary.
            let mut best: Option<(EntityId, f32)> = None;
            for (id, unit) in session.sim().view().units() {
                if unit.owner != session.local_player() {
                    continue;
                }
                let pos = Vec2::new(fx_to_f32(unit.pos.x), fx_to_f32(unit.pos.y));
                let distance = pos.distance(ground);
                if distance <= CLICK_RADIUS && best.is_none_or(|(_, d)| distance < d) {
                    best = Some((id, distance));
                }
            }
            match best {
                Some((id, _)) => picked.push(id),
                // Clicking empty ground clears the selection, unless the player
                // is adding to it.
                None if !additive => picked.clear(),
                None => {}
            }
        }

        selection.set(picked);
        drag.is_box = false;
    }
}

/// Right-click issues a move order for the current selection.
pub fn handle_orders(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform), With<GameCamera>>,
    selection: Res<Selection>,
    mut session: ResMut<Session>,
) {
    if !buttons.just_pressed(MouseButton::Right) || selection.is_empty() {
        return;
    }
    let Ok(window) = windows.single() else { return };
    let Ok((camera, camera_transform)) = cameras.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Some(ground) = screen_to_ground(camera, camera_transform, cursor) else {
        return;
    };

    let target = Cell::new(ground.x.floor() as i32, ground.y.floor() as i32);
    if !session.sim().map().contains(target) {
        return;
    }

    session.issue(CommandKind::Move {
        units: selection.units.clone(),
        target,
    });
}

/// `S` stops the selection; `Escape` clears it.
pub fn handle_hotkeys(
    keys: Res<ButtonInput<KeyCode>>,
    mut selection: ResMut<Selection>,
    mut session: ResMut<Session>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        selection.clear();
    }
    if keys.just_pressed(KeyCode::KeyX) && !selection.is_empty() {
        session.issue(CommandKind::Stop {
            units: selection.units.clone(),
        });
    }
    if keys.just_pressed(KeyCode::Space) {
        session.toggle_pause();
    }
    // F5 writes the match so far. A replay is the seed plus the command log —
    // a few kilobytes — so saving one mid-match costs nothing and is the single
    // most useful thing to attach to a bug report.
    if keys.just_pressed(KeyCode::F5) {
        match session.save_replay() {
            Ok(path) => info!("replay saved to {}", path.display()),
            Err(e) => error!("could not save the replay: {e}"),
        }
    }
}

/// Draws a ring under each selected unit.
///
/// Rings are spawned and despawned to match the selection rather than being
/// pooled: a selection changes only on player input, so this runs a handful of
/// times a second at most.
pub fn sync_selection_rings(
    mut commands: Commands,
    selection: Res<Selection>,
    assets: Res<RenderAssets>,
    units: Query<(&UnitView, &Transform), Without<SelectionRing>>,
    rings: Query<Entity, With<SelectionRing>>,
) {
    if !selection.is_changed() && !rings.is_empty() {
        // Rings still need to follow their units every frame.
        return;
    }
    for entity in &rings {
        commands.entity(entity).despawn();
    }
    for (view, transform) in &units {
        if !selection.units.contains(&view.0) {
            continue;
        }
        commands.spawn((
            Mesh3d(assets.ring_mesh.clone()),
            MeshMaterial3d(assets.selection_material.clone()),
            // Just above the ground, to stay clear of the terrain surface.
            Transform::from_xyz(transform.translation.x, 0.02, transform.translation.z),
            SelectionRing,
        ));
    }
}

/// Keeps rings under their units as they move.
pub fn move_selection_rings(
    selection: Res<Selection>,
    units: Query<(&UnitView, &Transform), Without<SelectionRing>>,
    mut rings: Query<&mut Transform, With<SelectionRing>>,
) {
    let mut positions: Vec<Vec3> = units
        .iter()
        .filter(|(view, _)| selection.units.contains(&view.0))
        .map(|(_, t)| t.translation)
        .collect();
    positions.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.z.total_cmp(&b.z)));

    let mut ring_transforms: Vec<Mut<Transform>> = rings.iter_mut().collect();
    ring_transforms.sort_by(|a, b| {
        a.translation
            .x
            .total_cmp(&b.translation.x)
            .then(a.translation.z.total_cmp(&b.translation.z))
    });

    for (ring, position) in ring_transforms.iter_mut().zip(positions) {
        ring.translation.x = position.x;
        ring.translation.z = position.z;
    }
}
