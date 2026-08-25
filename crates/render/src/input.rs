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
    mut session: ResMut<Session>,
    mut selection: ResMut<Selection>,
    mut drag: ResMut<DragState>,
    mut rig: ResMut<CameraRig>,
    mut sell: ResMut<crate::sidebar::SellMode>,
    mut aiming: ResMut<crate::sidebar::AimingPower>,
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

        // The panel eats its own clicks. Without this, clicking a build row
        // would also select whatever unit happened to be behind the panel —
        // and the selection would change under the player every time they
        // queued anything.
        if crate::sidebar::pointer_over_sidebar(window.width(), cursor.x) {
            drag.is_box = false;
            return;
        }

        // A superweapon takes a *place*, which nothing else in the interface
        // does — so it is aimed from the panel and fired with the next click.
        if let Some(building) = aiming.building
            && let Some(ground) = screen_to_ground(camera, camera_transform, cursor)
        {
            aiming.building = None;
            let at = Cell::new(ground.x.floor() as i32, ground.y.floor() as i32);
            if session.sim().map().contains(at) {
                // A power that wants a second place asks for another click; the
                // rest fire now. Which is which is a fact about the power, so
                // the interface asks the simulation rather than deciding.
                if session.sim().power_wants_destination(building) && aiming.from.is_none() {
                    aiming.building = Some(building);
                    aiming.from = Some(at);
                } else {
                    let from = aiming.from.take();
                    session.issue(CommandKind::FirePower {
                        building,
                        at: from.unwrap_or(at),
                        to: from.map(|_| at),
                    });
                }
            }
            drag.is_box = false;
            return;
        }

        // Selling is armed on the panel and spent on one click, so a player
        // cannot demolish their base by leaving the mode on and clicking about.
        if sell.armed
            && let Some(ground) = screen_to_ground(camera, camera_transform, cursor)
        {
            let cell = Cell::new(ground.x.floor() as i32, ground.y.floor() as i32);
            let local = session.local_player();
            let target = session.sim().view().units().find(|(_, u)| {
                u.owner == local
                    && u.is_alive()
                    && u.cell() == cell
                    && !session.sim().stats().get(u.owner, u.kind).mobile
            });
            if let Some((building, _)) = target {
                session.issue(CommandKind::Sell { building });
            }
            crate::sidebar::take_sell_click(&mut sell);
            drag.is_box = false;
            return;
        }

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
    } else if let Some(order) = friendly_click(&session, &selection.units, target) {
        session.issue(order);
    } else if attack_move {
        session.issue(CommandKind::AttackMove {
            units: selection.units.clone(),
            target,
        });
    } else {
        // A factory cannot walk anywhere, so a move order aimed at one is a
        // rally point. The original drew no distinction either: the same click
        // means "go there" to a tank and "send what you build there" to the
        // building that makes tanks.
        let producers: Vec<EntityId> = selection
            .units
            .iter()
            .copied()
            .filter(|id| {
                session
                    .sim()
                    .unit(*id)
                    .is_some_and(|u| !session.sim().stats().get(u.owner, u.kind).mobile)
            })
            .collect();
        for building in producers {
            session.issue(CommandKind::SetRally {
                building,
                at: target,
            });
        }

        let movers: Vec<EntityId> = selection
            .units
            .iter()
            .copied()
            .filter(|id| {
                session
                    .sim()
                    .unit(*id)
                    .is_some_and(|u| session.sim().stats().get(u.owner, u.kind).mobile)
            })
            .collect();
        if !movers.is_empty() {
            session.issue(CommandKind::Move {
                units: movers,
                target,
            });
        }
    }
}

/// What right-clicking one of your own things means.
///
/// The original never asked a player to pick a verb: you chose a unit, you
/// clicked a thing, and the sensible act happened. An engineer clicked on a
/// building captures or repairs it; a damaged tank clicked on a Service Depot
/// goes in to be mended; infantry clicked on a transport climb aboard; a
/// factory clicked on open ground sets its rally point.
///
/// Seven of the simulation's fourteen commands had no way to be issued at all
/// before this — capture, transports, rally points among them. They were
/// tested, correct, and unreachable, which is a strange thing to have shipped.
///
/// Returns `None` when nothing sensible applies, so the caller falls through to
/// move and attack-move.
fn friendly_click(session: &Session, selected: &[EntityId], target: Cell) -> Option<CommandKind> {
    let local = session.local_player();
    let sim = session.sim();

    // A rally point is set by clicking *ground*, so it is checked first and
    // only when nothing of ours is under the pointer.
    let clicked = sim
        .view()
        .units()
        .find(|(_, u)| u.is_alive() && u.cell() == target && !u.is_aboard())?;
    let (clicked_id, clicked_unit) = clicked;
    let clicked_stats = sim.stats().get(clicked_unit.owner, clicked_unit.kind);

    // Somebody else's, and not visible, or not ours at all: leave it to attack.
    if clicked_unit.owner != local && !clicked_unit.owner.is_neutral() {
        return None;
    }

    // Anything that can be sent inside the thing under the pointer. One list
    // rather than a chain of special cases, because from the player's side it
    // is one gesture.
    let boarders: Vec<EntityId> = selected
        .iter()
        .copied()
        .filter(|id| *id != clicked_id)
        .filter(|id| {
            sim.unit(*id)
                .is_some_and(|u| u.owner == local && u.is_alive() && !u.is_aboard())
        })
        .collect();
    if boarders.is_empty() {
        return None;
    }

    // A transport of ours with room: climb in.
    if clicked_unit.owner == local && clicked_stats.capacity > 0 {
        return Some(CommandKind::Load {
            units: boarders,
            transport: clicked_id,
        });
    }

    // A building — ours, or nobody's. Entering it is capture, repair, garrison,
    // infiltration, servicing or bridge repair, and which of those it turns out
    // to be is the simulation's business rather than the interface's.
    if !clicked_stats.mobile {
        return Some(CommandKind::EnterBuilding {
            units: boarders,
            target: clicked_id,
        });
    }

    None
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

/// Order hotkeys, with the original's letters.
///
/// `S` stop, `D` deploy or unload, `G` guard, `Escape` clear. These are the
/// keys the original used and the ones anyone who has played it will reach for.
/// They were unavailable until the camera stopped panning with WASD — which it
/// never did in the original either.
pub fn handle_hotkeys(
    keys: Res<ButtonInput<KeyCode>>,
    mut selection: ResMut<Selection>,
    mut session: ResMut<Session>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        selection.clear();
    }
    if selection.is_empty() {
        // Everything below needs something selected. `X` is kept as an alias
        // for stop out of muscle memory from before the keys were corrected.
        if keys.just_pressed(KeyCode::Space) {
            session.toggle_pause();
        }
        return;
    }
    if keys.any_just_pressed([KeyCode::KeyS, KeyCode::KeyX]) {
        session.issue(CommandKind::Stop {
            units: selection.units.clone(),
        });
    }
    if keys.just_pressed(KeyCode::KeyG) {
        session.issue(CommandKind::Guard {
            units: selection.units.clone(),
        });
    }
    // One key for three things, exactly as the original had it: an MCV unpacks,
    // a Construction Yard packs up, and a loaded transport puts its passengers
    // down. Which of those a given unit does is a fact about the unit, so the
    // interface sends both commands and lets the simulation ignore what does
    // not apply. Asking the player to know which verb they wanted would be
    // inventing a decision the game does not have.
    if keys.just_pressed(KeyCode::KeyD) {
        session.issue(CommandKind::Deploy {
            units: selection.units.clone(),
        });
        let loaded: Vec<EntityId> = selection
            .units
            .iter()
            .copied()
            .filter(|id| session.sim().unit(*id).is_some_and(|u| !u.cargo.is_empty()))
            .collect();
        for transport in loaded {
            let at = session
                .sim()
                .unit(transport)
                .map(|u| u.cell())
                .unwrap_or(Cell::new(0, 0));
            session.issue(CommandKind::Unload { transport, at });
        }
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
    units: Query<(Entity, &UnitView, &Transform), Without<SelectionRing>>,
    rings: Query<Entity, With<SelectionRing>>,
) {
    if !selection.is_changed() && !rings.is_empty() {
        // Rings still need to follow their units every frame.
        return;
    }
    for entity in &rings {
        commands.entity(entity).despawn();
    }
    for (entity, view, transform) in &units {
        if !selection.units.contains(&view.0) {
            continue;
        }
        let _ = transform;
        // A child of the unit, like the blob shadow. This used to be a separate
        // entity at world y = 0.02, chased across the map by `move_selection_rings`
        // — which was fine while the map was flat and buried the ring two levels
        // underground the moment elevation arrived. As a child the offset is
        // local and the parent's height carries it, and there is nothing left
        // to chase.
        let half_height = transform.translation.y;
        let ring = commands
            .spawn((
                Mesh3d(assets.ring_mesh.clone()),
                MeshMaterial3d(assets.selection_material.clone()),
                Transform::from_xyz(0.0, -half_height + crate::world::GROUND_CLEARANCE, 0.0),
                SelectionRing,
            ))
            .id();
        commands.entity(entity).add_child(ring);
    }
}

// Rings used to be chased across the map by a system that paired them to units
// by sorting both lists by position. It worked, and it was the reason a ring
// could sit at the wrong height: nothing in it knew what ground its unit was
// standing on. Parenting removed the need for it entirely.

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
