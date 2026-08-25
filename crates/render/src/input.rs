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
            // Highest priority within the click radius, nearest breaking the
            // tie. Distance alone was the first attempt and it is wrong in the
            // case that matters: click a crowd of infantry standing on a tank
            // and you get whichever body happens to be a pixel closer, when
            // what you meant was obviously the tank.
            //
            // `Selectable`'s priority existed for exactly this and nothing read
            // it — the third instance of the same defect in this codebase, and
            // the only one a player would have felt every single match.
            let mut candidates: Vec<Candidate> = Vec::new();
            let mut ids: Vec<EntityId> = Vec::new();
            for (id, unit) in session.sim().view().units() {
                if unit.owner != session.local_player() {
                    continue;
                }
                let pos = Vec2::new(fx_to_f32(unit.pos.x), fx_to_f32(unit.pos.y));
                let distance = pos.distance(ground);
                if distance > CLICK_RADIUS {
                    continue;
                }
                ids.push(id);
                candidates.push(Candidate {
                    priority: session
                        .sim()
                        .stats()
                        .get(unit.owner, unit.kind)
                        .selection_priority,
                    distance,
                });
            }
            let best = pick_one(&candidates).map(|i| ids[i]);
            match best {
                Some(id) => picked.push(id),
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

/// What decides whether the player meant this one, for something under the
/// pointer.
///
/// Deliberately not carrying the unit's id. The rule is about *ordering*, and
/// keeping it that way is what lets it be tested without conjuring entity ids
/// the arena has no public way to make.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Candidate {
    pub priority: u8,
    pub distance: f32,
}

/// Which of the things under the pointer the player meant.
///
/// Highest priority wins; nearest breaks the tie. Distance alone was the first
/// attempt and it is wrong in the case that matters: click a crowd of infantry
/// standing around a tank and you get whichever body happens to be a pixel
/// closer, when what you obviously meant was the tank.
///
/// `Selectable`'s priority existed for exactly this and nothing read it — the
/// same defect as the harvester's gather rate, and the only one of the three a
/// player would have felt in every single match.
///
/// A free function over plain data rather than a loop inside the Bevy system,
/// so the rule can be tested at all. The renderer has no other tests, and a
/// rule about what the player meant is worth more than most of what does.
pub fn pick_one(candidates: &[Candidate]) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .fold(
            None,
            |best: Option<(usize, &Candidate)>, (i, c)| match best {
                Some((_, b))
                    if b.priority > c.priority
                        || (b.priority == c.priority && b.distance <= c.distance) =>
                {
                    best
                }
                _ => Some((i, c)),
            },
        )
        .map(|(i, _)| i)
}

/// Right-click issues a move order for the current selection.
pub fn handle_orders(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform), With<GameCamera>>,
    selection: Res<Selection>,
    mut session: ResMut<Session>,
) {
    if !buttons.just_pressed(MouseButton::Right) || selection.is_empty() {
        return;
    }
    let attack_move = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
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

    // Right-clicking a visible enemy attacks it; holding Ctrl advances rather
    // than repositioning. Both are what the original did, and both are what a
    // player who has played one of these before will try first.
    let local = session.local_player();
    let clicked_enemy = session.sim().view().units().find(|(_, unit)| {
        unit.owner != local
            && unit.is_alive()
            && unit.cell() == target
            && session.sim().can_see(local, unit)
    });

    if let Some((victim, _)) = clicked_enemy {
        session.issue(CommandKind::Attack {
            units: selection.units.clone(),
            target: victim,
        });
    } else if attack_move {
        session.issue(CommandKind::AttackMove {
            units: selection.units.clone(),
            target,
        });
    } else {
        session.issue(CommandKind::Move {
            units: selection.units.clone(),
            target,
        });
    }
}

/// Named unit selections, recalled with the number keys.
///
/// Nine groups, as the original had. Held in the renderer rather than the
/// simulation because they are a property of one player's interface: which
/// units someone has filed under "3" changes nothing about the world, so
/// putting it in simulation state would mean sending it over the network and
/// hashing it for no reason at all.
#[derive(Resource, Default)]
pub struct ControlGroups {
    groups: [Vec<EntityId>; 9],
}

impl ControlGroups {
    pub fn assign(&mut self, index: usize, units: &[EntityId]) {
        if let Some(slot) = self.groups.get_mut(index) {
            slot.clear();
            slot.extend_from_slice(units);
        }
    }

    pub fn recall(&self, index: usize) -> &[EntityId] {
        self.groups.get(index).map_or(&[], |g| g.as_slice())
    }

    /// Drops units that no longer exist.
    ///
    /// Without this a group slowly fills with the dead, and recalling it
    /// selects fewer and fewer units with nothing to explain why.
    pub fn forget_missing(&mut self, alive: &dyn Fn(EntityId) -> bool) {
        for group in &mut self.groups {
            group.retain(|id| alive(*id));
        }
    }
}

/// Number keys recall a group; `Ctrl` with a number assigns one.
pub fn handle_control_groups(
    keys: Res<ButtonInput<KeyCode>>,
    session: Res<Session>,
    mut groups: ResMut<ControlGroups>,
    mut selection: ResMut<Selection>,
) {
    const DIGITS: [KeyCode; 9] = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];

    groups.forget_missing(&|id| session.sim().units().get(id).is_some());

    let assigning = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    for (index, key) in DIGITS.iter().enumerate() {
        if !keys.just_pressed(*key) {
            continue;
        }
        if assigning {
            groups.assign(index, &selection.units);
        } else {
            let recalled = groups.recall(index).to_vec();
            if !recalled.is_empty() {
                selection.set(recalled);
            }
        }
        return;
    }
}

/// `X` stops the selection, `G` deploys it, `Escape` clears it.
///
/// `G` rather than `D`, which the camera already uses to pan right.
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
    // One key for both directions. The simulation decides which way each unit
    // goes from what it currently is, so a mixed selection of packed and
    // unpacked things does the sensible thing with all of them.
    if keys.just_pressed(KeyCode::KeyG) && !selection.is_empty() {
        session.issue(CommandKind::Deploy {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn at(priority: u8, distance: f32) -> Candidate {
        Candidate { priority, distance }
    }

    #[test]
    fn nothing_under_the_pointer_picks_nothing() {
        assert_eq!(pick_one(&[]), None);
    }

    #[test]
    fn the_only_thing_under_the_pointer_wins() {
        assert_eq!(pick_one(&[at(0, 5.0)]), Some(0));
    }

    #[test]
    fn the_nearest_wins_when_priorities_match() {
        let far = at(3, 9.0);
        let near = at(3, 1.0);
        assert_eq!(pick_one(&[far, near]), Some(1));
        assert_eq!(pick_one(&[near, far]), Some(0));
    }

    #[test]
    fn priority_beats_distance() {
        // The case the whole rule exists for: infantry crowding a tank. The
        // nearest body is a soldier and the player meant the tank.
        let soldier = at(1, 0.2);
        let tank = at(4, 0.9);
        assert_eq!(pick_one(&[soldier, tank]), Some(1));
        assert_eq!(pick_one(&[tank, soldier]), Some(0));
    }

    #[test]
    fn the_answer_does_not_depend_on_the_order_they_were_found_in() {
        // Iteration order is an arena detail and shifts as slots are reused. A
        // pick that depended on it would feel arbitrary in a way no bug report
        // could describe.
        // Identical candidates: whichever is looked at first is kept, so the
        // answer is the *same* candidate either way round rather than the same
        // index.
        let a = at(2, 3.0);
        assert_eq!(pick_one(&[a, a]), Some(0));
    }
}
