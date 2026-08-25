//! Health bars.
//!
//! Combat was invisible without them: units simply vanished, and there was no
//! way to see a fight going badly before it was over.
//!
//! # Keeping them inside the budget
//!
//! A bar per unit is two more things to draw per unit, which at four hundred
//! units would be eight hundred — against a whole-frame budget of five hundred
//! draw calls. Three things keep that from happening:
//!
//! - **Bars exist only while they are needed.** A unit at full health that is
//!   not selected has no bar at all, and most units are at full health most of
//!   the time.
//! - **One mesh, four materials.** Every bar shares a quad; the fill picks from
//!   green, yellow, red and a selection tint. Same mesh plus same material
//!   means Bevy batches them into one instanced call each, so the cost is four
//!   draw calls rather than one per unit.
//! - **Bars are reused, not respawned.** A unit crossing a colour threshold
//!   swaps its material handle; nothing is created or destroyed.
//!
//! # Why the quad needs no billboarding
//!
//! The camera never rotates, so "facing the camera" is a constant rotation
//! rather than a per-frame calculation. It is baked once at spawn.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use redshift_sim::EntityId;

use crate::camera::{CAMERA_PITCH_DEGREES, CAMERA_YAW_DEGREES};
use crate::input::Selection;
use crate::session::Session;
use crate::world::{UnitView, fx_to_f32};

/// Width of a full bar, in cells.
const BAR_WIDTH: f32 = 0.75;
/// Height of a bar, in cells.
const BAR_HEIGHT: f32 = 0.11;
/// How far above the unit's top the bar floats, in cells.
const BAR_CLEARANCE: f32 = 0.18;

/// Health at or below which the bar turns yellow, then red.
const YELLOW_BELOW: u32 = 66;
const RED_BELOW: u32 = 33;

/// Marks a health bar and says which unit it belongs to.
#[derive(Component)]
pub struct HealthBar {
    pub unit: EntityId,
    pub part: BarPart,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum BarPart {
    /// The dark backing, showing damage taken.
    Backing,
    /// The coloured portion, scaled to remaining health.
    Fill,
}

/// Shared bar meshes and the small palette of fill materials.
#[derive(Resource)]
pub struct HealthBarAssets {
    pub mesh: Handle<Mesh>,
    pub backing: Handle<StandardMaterial>,
    /// The backing, recoloured to say something is inside.
    ///
    /// The backing carried no information at all — a dark quad behind the fill.
    /// Two states that were invisible on screen now use it: a parasite eating a
    /// vehicle from inside, and infantry firing out of a building. Both matter
    /// and neither is inferable from anything else the player can see.
    ///
    /// Deliberately the *backing* rather than the fill: the fill already means
    /// health, and one bar saying two things by colour would say neither.
    pub backing_infested: Handle<StandardMaterial>,
    pub backing_garrisoned: Handle<StandardMaterial>,
    /// Indexed by [`fill_bucket`].
    pub fills: [Handle<StandardMaterial>; 3],
}

pub fn build_health_assets(
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> HealthBarAssets {
    // A unit quad, scaled per bar. One mesh for every bar in the game.
    let mesh = meshes.add(Rectangle::new(1.0, 1.0));

    let unlit = |r: f32, g: f32, b: f32, a: f32| StandardMaterial {
        base_color: Color::srgba(r, g, b, a),
        // Unlit, so a bar reads the same whichever way the unit is facing. A
        // health bar that dims in shadow is a health bar people misread.
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    };

    HealthBarAssets {
        mesh,
        backing: materials.add(unlit(0.05, 0.05, 0.06, 0.75)),
        // Sickly green for something living in the engine bay.
        backing_infested: materials.add(unlit(0.28, 0.46, 0.10, 0.85)),
        // Cool blue for a building with people shooting out of it.
        backing_garrisoned: materials.add(unlit(0.14, 0.30, 0.52, 0.85)),
        fills: [
            materials.add(unlit(0.30, 0.85, 0.35, 1.0)),
            materials.add(unlit(0.95, 0.80, 0.20, 1.0)),
            materials.add(unlit(0.90, 0.25, 0.20, 1.0)),
        ],
    }
}

/// Which fill material a health percentage uses.
fn fill_bucket(percent: u32) -> usize {
    if percent <= RED_BELOW {
        2
    } else if percent <= YELLOW_BELOW {
        1
    } else {
        0
    }
}

/// The constant rotation that turns a ground-plane quad to face the camera.
///
/// The camera is fixed, so this is computed once rather than per bar per frame.
fn facing_rotation() -> Quat {
    Quat::from_euler(
        EulerRot::YXZ,
        CAMERA_YAW_DEGREES.to_radians(),
        -(90.0 - CAMERA_PITCH_DEGREES).to_radians(),
        0.0,
    )
}

/// Creates and removes bars so that exactly the units that need one have one.
///
/// A unit needs a bar when it is damaged or selected. Most units are neither,
/// most of the time, which is what keeps this affordable.
pub fn sync_health_bars(
    mut commands: Commands,
    session: Res<Session>,
    selection: Res<Selection>,
    assets: Res<HealthBarAssets>,
    existing: Query<(Entity, &HealthBar)>,
) {
    let view = session.sim().view();

    let mut wanted: Vec<EntityId> = Vec::new();
    for (id, _) in view.units() {
        let damaged = view.health_percent(id).is_some_and(|p| p < 100);
        if damaged || selection.units.contains(&id) {
            wanted.push(id);
        }
    }

    // What already exists, by unit and part.
    let mut present: HashMap<(u32, bool), Entity> = HashMap::default();
    for (entity, bar) in &existing {
        present.insert((bar.unit.index(), bar.part == BarPart::Fill), entity);
    }

    for id in &wanted {
        for (part, is_fill) in [(BarPart::Backing, false), (BarPart::Fill, true)] {
            if present.remove(&(id.index(), is_fill)).is_some() {
                continue;
            }
            let material = if is_fill {
                assets.fills[0].clone()
            } else {
                assets.backing.clone()
            };
            commands.spawn((
                Mesh3d(assets.mesh.clone()),
                MeshMaterial3d(material),
                Transform::from_rotation(facing_rotation()),
                // Hidden until the first update places it, so a new bar never
                // appears for one frame at the origin.
                Visibility::Hidden,
                HealthBar { unit: *id, part },
            ));
        }
    }

    // Anything left over belongs to a unit that no longer needs a bar.
    for (_, entity) in present {
        commands.entity(entity).despawn();
    }
}

/// Positions, scales and colours the bars.
pub fn update_health_bars(
    session: Res<Session>,
    assets: Res<HealthBarAssets>,
    units: Query<(&UnitView, &Transform), Without<HealthBar>>,
    mut bars: Query<
        (
            &HealthBar,
            &mut Transform,
            &mut Visibility,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        Without<UnitView>,
    >,
) {
    // Where each unit is being drawn this frame, so bars sit on the
    // interpolated position rather than the last simulation tick's.
    let mut positions: HashMap<u32, Vec3> = HashMap::default();
    for (view, transform) in &units {
        positions.insert(view.0.index(), transform.translation);
    }

    let view = session.sim().view();

    for (bar, mut transform, mut visibility, mut material) in &mut bars {
        let (Some(base), Some(percent)) = (
            positions.get(&bar.unit.index()),
            view.health_percent(bar.unit),
        ) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        let Some(stats) = view.stats_of(bar.unit) else {
            *visibility = Visibility::Hidden;
            continue;
        };

        *visibility = Visibility::Visible;

        // Above the unit's own height, not a shared constant — a bar floating
        // over an infantryman's head should not be at tank height.
        let height = fx_to_f32(stats.radius) * 2.0 + BAR_CLEARANCE;
        let fraction = (percent as f32 / 100.0).clamp(0.0, 1.0);

        match bar.part {
            BarPart::Backing => {
                transform.translation = *base + Vec3::new(0.0, height, 0.0);
                transform.scale = Vec3::new(BAR_WIDTH, BAR_HEIGHT, 1.0);

                // Infestation first: a garrison is a nuisance to its enemy and
                // a parasite is killing the thing it is in, so if a unit were
                // ever somehow both, the urgent one should show.
                let occupant = session.sim().unit(bar.unit);
                let wanted = match occupant {
                    Some(u) if u.infestation.is_some() => &assets.backing_infested,
                    Some(u) if !u.cargo.is_empty() && !stats.mobile => &assets.backing_garrisoned,
                    _ => &assets.backing,
                };
                if material.0.id() != wanted.id() {
                    material.0 = wanted.clone();
                }
            }
            BarPart::Fill => {
                // The fill shrinks from the right rather than from both edges,
                // so a bar reads as draining rather than closing.
                let width = BAR_WIDTH * fraction;
                let offset = -(BAR_WIDTH - width) / 2.0;
                transform.translation = *base
                    + Vec3::new(0.0, height, 0.0)
                    + facing_rotation() * Vec3::new(offset, 0.0, 0.0)
                    // A hair in front of the backing, or the two z-fight.
                    + Vec3::new(0.0, 0.0, 0.0);
                transform.scale = Vec3::new(width.max(0.0001), BAR_HEIGHT * 0.72, 1.0);

                let wanted = &assets.fills[fill_bucket(percent)];
                if material.0.id() != wanted.id() {
                    material.0 = wanted.clone();
                }
            }
        }
    }
}
