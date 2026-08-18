//! Drawing the world: terrain, units, and the interpolation that hides the
//! 20 Hz simulation behind 60 Hz motion.
//!
//! Everything here reads [`Session`] and writes only to Bevy components. The
//! simulation is never touched — see `docs/01-architecture.md`.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use redshift_sim::EntityId;
use redshift_sim::map::{Map, Terrain};

use crate::session::Session;

/// Height of a placeholder unit box, in cells.
const UNIT_HEIGHT: f32 = 0.55;
/// Footprint of a placeholder unit, in cells.
const UNIT_WIDTH: f32 = 0.6;

/// Vertical offset for the ground, so unit bases do not z-fight with it.
const GROUND_Y: f32 = 0.0;

/// Links a drawn entity back to the simulation entity it represents.
#[derive(Component)]
pub struct UnitView(pub EntityId);

/// Marks the selection ring under a unit.
#[derive(Component)]
pub struct SelectionRing;

/// Shared meshes and materials.
///
/// Held as handles and reused across every unit, which is what lets Bevy batch
/// same-type units into a single instanced draw call. Cloning a handle is
/// cheap; creating a second identical material is not — it would silently
/// double the draw calls and break the budget in
/// `docs/04-rendering.md`.
#[derive(Resource)]
pub struct RenderAssets {
    /// One mesh per entity kind, indexed by [`EntityKind`].
    ///
    /// A mesh per *kind* rather than per *entity*: units of the same kind and
    /// team share a mesh and a material, which is what lets Bevy batch them
    /// into a single instanced draw call. Sizing each one from the rules also
    /// means a new unit becomes visibly distinct with no renderer change —
    /// the same property the data layer is built for.
    pub unit_meshes: Vec<Handle<Mesh>>,
    /// Fallback for a kind with no mesh, which should not happen but must not
    /// crash if it does.
    pub unit_mesh: Handle<Mesh>,
    /// Half-height per kind. A unit is positioned by its centre, so a shorter
    /// box needs a lower centre or it hovers above the ground.
    pub unit_half_heights: Vec<f32>,
    pub ring_mesh: Handle<Mesh>,
    pub team_materials: Vec<Handle<StandardMaterial>>,
    pub selection_material: Handle<StandardMaterial>,
}

/// Team colours, in the original's register: saturated, high contrast, readable
/// against both ground and water at a glance.
const TEAM_COLOURS: [(u8, u8, u8); 4] = [
    (220, 60, 50),  // red
    (60, 110, 220), // blue
    (70, 180, 90),  // green
    (230, 180, 60), // yellow
];

/// The placeholder proportions for one entity kind.
///
/// Read from the rules rather than hard-coded per unit: a structure's footprint
/// is already data, and a unit's category already says whether it walks or
/// drives. Guessing here from the same source keeps the placeholder honest —
/// when a real model replaces it in Phase 4, it drops into the same volume.
fn placeholder_size(def: &redshift_sim::EntityDef, radius: f32) -> Vec3 {
    // Footprint comes from the simulation's collision radius, not from a second
    // table kept beside it. Two hand-maintained lists drift, and when they do
    // the result is units that visibly overlap while the simulation considers
    // them well clear — which reads as the collision being broken.
    let footprint = radius * 2.0;

    // Height is presentation only: nothing in the simulation has a vertical
    // extent, so it is chosen for silhouette rather than derived.
    let height = match def.category.as_str() {
        // Tall and narrow, so a squad reads as people rather than as crates.
        "infantry" => 0.55,
        // Low and wide: the classic tank silhouette at this zoom.
        "vehicle" => 0.38,
        "aircraft" => 0.22,
        "ship" => 0.35,
        "structure" => 1.0,
        _ => UNIT_HEIGHT,
    };

    // Vehicles and ships read better slightly longer than they are wide, which
    // also gives the fixed camera something to show a heading against.
    let elongation = match def.category.as_str() {
        "vehicle" => 1.2,
        "ship" => 1.5,
        _ => 1.0,
    };

    Vec3::new(footprint, height, footprint * elongation)
}

pub fn build_assets(
    rules: &redshift_sim::Rules,
    stats: &redshift_sim::StatTable,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> RenderAssets {
    let unit_mesh = meshes.add(Cuboid::new(UNIT_WIDTH, UNIT_HEIGHT, UNIT_WIDTH));

    // Built in kind order, so the vector can be indexed directly by kind.
    let mut unit_meshes = Vec::with_capacity(rules.entity_count());
    let mut unit_half_heights = Vec::with_capacity(rules.entity_count());
    for (kind, def) in rules.entities() {
        debug_assert_eq!(
            kind.0 as usize,
            unit_meshes.len(),
            "kinds must be dense and in order"
        );
        // The simulation is the authority on how much room a unit takes up.
        let radius = fx_to_f32(stats.get(redshift_sim::PlayerId(0), kind).radius);
        let size = placeholder_size(def, radius);
        unit_meshes.push(meshes.add(Cuboid::new(size.x, size.y, size.z)));
        unit_half_heights.push(size.y / 2.0);
    }
    // A flat quad standing in for the blob shadow and selection decal that
    // Phase 4 will replace with a proper texture.
    let ring_mesh = meshes.add(
        Plane3d::default()
            .mesh()
            .size(UNIT_WIDTH * 1.6, UNIT_WIDTH * 1.6),
    );

    let team_materials = TEAM_COLOURS
        .iter()
        .map(|(r, g, b)| materials.add(flat_material(Color::srgb_u8(*r, *g, *b))))
        .collect();

    let selection_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.3, 1.0, 0.4, 0.55),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    });

    RenderAssets {
        unit_meshes,
        unit_half_heights,
        unit_mesh,
        ring_mesh,
        team_materials,
        selection_material,
    }
}

/// A material with the flattest response the standard shader will give.
///
/// Roughness at maximum and metalness at zero reduces the PBR model to
/// something close to plain Lambert shading: no specular highlight, no
/// environment response. That is both the cheapest configuration and the one
/// that matches the original's flat look.
///
/// A purpose-built cel shader belongs in Phase 4; this keeps Phase 0 free of a
/// custom render pipeline.
fn flat_material(colour: Color) -> StandardMaterial {
    StandardMaterial {
        base_color: colour,
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        ..default()
    }
}

/// Builds the terrain as a single mesh.
///
/// One mesh for the whole map rather than one per cell: a 64×64 map would
/// otherwise be four thousand draw calls on its own, against a budget of five
/// hundred for the entire frame.
///
/// Raised cells get their side faces as well as their top. Emitting only the
/// top leaves a gap at every step that the background shows through, which
/// reads as a black crack along every wall.
pub fn build_terrain_mesh(
    map: &Map,
    visibility: &redshift_sim::Visibility,
    viewer: redshift_sim::PlayerId,
) -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::mesh::{Indices, PrimitiveTopology};

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut colours: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    for y in 0..map.height() {
        for x in 0..map.width() {
            let cell = redshift_sim::Cell::new(x, y);
            let terrain = map.terrain(cell);

            // Slight per-cell variation so a large expanse of ground does not
            // read as a flat sheet of colour. Derived from the coordinates so
            // it is stable, not random.
            let jitter = (((x * 7 + y * 13) % 5) as f32) * 0.015;
            // Ore tints the ground rather than replacing it. A field is
            // something lying *on* the map, and a harvester working it should
            // visibly thin the patch out over a run rather than flipping cells
            // to bare earth one at a time.
            let ore = map.ore(cell) as f32 / redshift_sim::map::MAX_ORE_PER_CELL as f32;
            // Fog is baked into the terrain colours rather than drawn as an
            // overlay. An overlay would be a second full-map mesh — the single
            // most expensive thing this scene could add — and the terrain mesh
            // has to be rebuilt when the fog moves anyway.
            let sight = visibility.sight(viewer, cell);
            let colour = match terrain {
                Terrain::Ground if ore > 0.0 => {
                    let mix = ore.clamp(0.0, 1.0);
                    let blend = |ground: f32, gold: f32| ground + (gold - ground) * mix;
                    vertex_colour(
                        blend(0.32 + jitter, 0.78),
                        blend(0.40 + jitter, 0.60),
                        blend(0.22 + jitter, 0.14),
                    )
                }
                Terrain::Ground => vertex_colour(0.32 + jitter, 0.40 + jitter, 0.22 + jitter),
                Terrain::Water => vertex_colour(0.11, 0.28, 0.46),
                Terrain::Rock => vertex_colour(0.34, 0.32, 0.30),
            };

            // Impassable rock stands proud of the ground so obstacles read as
            // obstacles rather than as a change of colour.
            //
            // Flattened where the player has never looked. Blackening the
            // colour alone still left the ridges catching the light, which drew
            // a map of every cliff on ground nobody had scouted.
            let height = match (terrain, sight) {
                (_, redshift_sim::Sight::Unseen) => 0.0,
                (Terrain::Rock, _) => 0.5,
                _ => 0.0,
            };

            let (fx, fy) = (x as f32, y as f32);

            let base = positions.len() as u32;
            positions.extend_from_slice(&[
                [fx, height, fy],
                [fx + 1.0, height, fy],
                [fx + 1.0, height, fy + 1.0],
                [fx, height, fy + 1.0],
            ]);
            // Unexplored ground is black and fogged ground is dimmed. Doing it
            // to the vertex colour keeps it free: no extra geometry, no extra
            // pass, no extra draw call.
            let colour = match sight {
                redshift_sim::Sight::Unseen => [0.02, 0.02, 0.03, 1.0],
                redshift_sim::Sight::Fogged => {
                    [colour[0] * 0.38, colour[1] * 0.38, colour[2] * 0.42, 1.0]
                }
                redshift_sim::Sight::Visible => colour,
            };

            normals.extend_from_slice(&[[0.0, 1.0, 0.0]; 4]);
            uvs.extend_from_slice(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
            colours.extend_from_slice(&[colour; 4]);
            indices.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);

            if height <= 0.0 {
                continue;
            }

            // Sides, for every edge where the neighbour sits lower. Shaded a
            // little darker than the top so the step reads as a step rather
            // than as a flat outline.
            // Darkened in linear space, which is where multiplication means
            // what it looks like it means.
            let side_colour = [colour[0] * 0.55, colour[1] * 0.55, colour[2] * 0.55, 1.0];
            /// One side face to emit: the neighbour offset that must be lower,
            /// the outward normal, and the two ground-level corners of the edge.
            type SideFace = (i32, i32, [f32; 3], [[f32; 3]; 2]);
            let neighbours: [SideFace; 4] = [
                (
                    0,
                    -1,
                    [0.0, 0.0, -1.0],
                    [[fx, 0.0, fy], [fx + 1.0, 0.0, fy]],
                ),
                (
                    1,
                    0,
                    [1.0, 0.0, 0.0],
                    [[fx + 1.0, 0.0, fy], [fx + 1.0, 0.0, fy + 1.0]],
                ),
                (
                    0,
                    1,
                    [0.0, 0.0, 1.0],
                    [[fx + 1.0, 0.0, fy + 1.0], [fx, 0.0, fy + 1.0]],
                ),
                (
                    -1,
                    0,
                    [-1.0, 0.0, 0.0],
                    [[fx, 0.0, fy + 1.0], [fx, 0.0, fy]],
                ),
            ];
            for (dx, dy, normal, [b0, b1]) in neighbours {
                let neighbour = redshift_sim::Cell::new(x + dx, y + dy);
                // Off-map counts as lower, so the map edge is walled rather
                // than open.
                if map.terrain(neighbour) == Terrain::Rock && map.contains(neighbour) {
                    continue;
                }
                let t0 = [b0[0], height, b0[2]];
                let t1 = [b1[0], height, b1[2]];
                let base = positions.len() as u32;
                positions.extend_from_slice(&[b0, b1, t1, t0]);
                normals.extend_from_slice(&[normal; 4]);
                uvs.extend_from_slice(&[[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]);
                colours.extend_from_slice(&[side_colour; 4]);
                indices.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
            }
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    // Required by the standard material's shader. Without it the pipeline fails
    // to specialise and every triangle is painted with the fallback magenta —
    // which looks like a colour bug rather than a missing attribute.
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Marks the terrain, so the fog pass can find it again.
#[derive(Component)]
pub struct TerrainMesh;

/// Tracks which simulation tick the terrain was last built for.
#[derive(Resource, Default)]
pub struct TerrainBuiltAt(pub Option<u32>);

/// Rebuilds the terrain when the fog has moved.
///
/// Once per simulation tick, not once per frame: the fog only changes when the
/// simulation advances, and at 60 Hz against a 20 Hz simulation two rebuilds in
/// three would produce an identical mesh.
///
/// Rebuilding the whole map is affordable at the sizes here — a 48×48 map is
/// about two thousand quads — but it does scale with map area, so a much larger
/// map will want the terrain split into chunks that can be rebuilt
/// individually. Noted rather than solved, since solving it now would be
/// guessing at a size nobody has asked for.
pub fn rebuild_terrain(
    session: Res<Session>,
    mut built_at: ResMut<TerrainBuiltAt>,
    mut meshes: ResMut<Assets<Mesh>>,
    terrain: Query<&Mesh3d, With<TerrainMesh>>,
) {
    let tick = session.sim().tick_number();
    if built_at.0 == Some(tick) {
        return;
    }
    built_at.0 = Some(tick);

    let Ok(handle) = terrain.single() else {
        return;
    };
    let fresh = build_terrain_mesh(
        session.sim().map(),
        session.sim().visibility(),
        session.local_player(),
    );
    if let Some(mut slot) = meshes.get_mut(&handle.0) {
        *slot = fresh;
    }
}

/// Creates and destroys drawn units to match the simulation.
///
/// The simulation is the authority: this only ever mirrors it.
pub fn sync_units(
    mut commands: Commands,
    session: Res<Session>,
    assets: Res<RenderAssets>,
    existing: Query<(Entity, &UnitView)>,
) {
    let mut drawn: HashMap<EntityId, Entity> = HashMap::default();
    for (entity, view) in &existing {
        drawn.insert(view.0, entity);
    }

    let viewer = session.local_player();
    for (id, unit) in session.sim().view().units() {
        // Anything in fog is simply not drawn. Drawing it dimmed would show the
        // player exactly what they are not supposed to know.
        if !session.sim().can_see(viewer, unit) {
            continue;
        }
        if drawn.remove(&id).is_some() {
            continue;
        }
        let half_height = assets
            .unit_half_heights
            .get(unit.kind.0 as usize)
            .copied()
            .unwrap_or(UNIT_HEIGHT / 2.0);
        let material = assets
            .team_materials
            .get(unit.owner.0 as usize % assets.team_materials.len())
            .expect("at least one team material")
            .clone();
        commands.spawn((
            Mesh3d(
                assets
                    .unit_meshes
                    .get(unit.kind.0 as usize)
                    .unwrap_or(&assets.unit_mesh)
                    .clone(),
            ),
            MeshMaterial3d(material),
            Transform::from_xyz(0.0, half_height, 0.0),
            UnitView(id),
        ));
    }

    // Anything still in the map no longer exists in the simulation.
    for (_, entity) in drawn {
        commands.entity(entity).despawn();
    }
}

/// Moves drawn units to their interpolated positions.
///
/// The simulation ticks at 20 Hz. Drawn straight, that would look like a 20 fps
/// game. Interpolating between the last two simulation states gives smooth
/// motion while leaving the simulation coarse and cheap.
///
/// This is presentation only — it uses floating point freely and never feeds
/// back into the simulation.
pub fn interpolate_units(
    session: Res<Session>,
    assets: Res<RenderAssets>,
    mut views: Query<(&UnitView, &mut Transform)>,
) {
    let t = session.interpolation();
    for (view, mut transform) in &mut views {
        let Some(unit) = session.sim().view().unit(view.0) else {
            continue;
        };

        let current = unit.pos;
        let previous = session.previous_pos(view.0, current);

        let px = fx_to_f32(previous.x);
        let py = fx_to_f32(previous.y);
        let cx = fx_to_f32(current.x);
        let cy = fx_to_f32(current.y);

        transform.translation.x = px + (cx - px) * t;
        transform.translation.z = py + (cy - py) * t;
        // Sit each kind on the ground by its own half-height, not a shared
        // constant — otherwise infantry sink and structures float.
        transform.translation.y = GROUND_Y
            + assets
                .unit_half_heights
                .get(unit.kind.0 as usize)
                .copied()
                .unwrap_or(UNIT_HEIGHT / 2.0);

        // Facing is not interpolated. Turn rates are already slow enough that
        // the step between ticks is imperceptible, and interpolating an angle
        // correctly across the wrap point costs more than it is worth here.
        let facing = unit.facing.raw() as f32 / 65536.0 * std::f32::consts::TAU;
        // Simulation angles measure anticlockwise from +x in the (x, y) plane,
        // which maps to clockwise about the world's vertical axis.
        transform.rotation = Quat::from_rotation_y(-facing);
    }
}

/// Converts an sRGB channel to linear.
///
/// Vertex colours are consumed as **linear** values, but colours are authored
/// in sRGB — the space every colour picker and palette reference uses. Writing
/// sRGB numbers straight into the attribute makes everything render far
/// brighter than intended, which is easy to mistake for a lighting problem.
#[inline]
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// An sRGB triple as a linear RGBA vertex colour.
#[inline]
fn vertex_colour(r: f32, g: f32, b: f32) -> [f32; 4] {
    [srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b), 1.0]
}

/// Converts a simulation scalar to a rendering float.
///
/// One-way by design. There is no route back: a float must never re-enter the
/// simulation, and `Fx` deliberately offers no conversion from one.
#[inline]
pub fn fx_to_f32(value: redshift_sim::Fx) -> f32 {
    value.raw() as f32 / 65536.0
}

/// The lighting setup, in full.
///
/// One directional light, no shadow maps. Shadow mapping is the single most
/// expensive thing a scene like this could switch on, and the original had no
/// dynamic shadows either — units sat on a simple dark blob. Phase 4 adds that
/// blob as a decal.
pub fn spawn_lighting(commands: &mut Commands) {
    // Calibrated for an untonemapped pipeline. The usual illuminance figures
    // assume a tone curve compressing high dynamic range into display range;
    // with that curve off, those values clip every surface to white.
    commands.spawn((
        DirectionalLight {
            illuminance: 2_600.0,
            shadow_maps_enabled: false,
            contact_shadows_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(
            EulerRot::YXZ,
            -std::f32::consts::FRAC_PI_4,
            -std::f32::consts::FRAC_PI_3,
            0.0,
        )),
    ));

    // `AmbientLight` is a per-camera component in this Bevy version; the
    // scene-wide default lives in `GlobalAmbientLight`. Using the global one
    // keeps the camera setup free of lighting concerns.
    // Enough fill that faces turned away from the sun stay readable, but well
    // short of flattening the shading that gives the low-poly shapes their
    // form.
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.62, 0.70, 0.92),
        brightness: 220.0,
        ..default()
    });
}
