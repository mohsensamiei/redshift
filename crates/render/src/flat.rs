//! The flat/cel material.
//!
//! Replaces `StandardMaterial` rather than configuring it. Setting PBR's
//! roughness to one and its metallic to zero gets close to the look, and still
//! pays for the whole physically based pipeline to compute a result that is
//! then thrown away. This does the arithmetic the look actually needs, which is
//! a dot product and a `floor`.
//!
//! The point is not only speed — though on an integrated GPU under a fan-noise
//! budget that matters. It is that "flat" stops being PBR turned down and
//! becomes a decision: light comes from one direction, it lands in a fixed
//! number of steps, and the unlit side is dark rather than black.

use bevy::asset::Asset;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::mesh::{MeshVertexBufferLayoutRef, VertexAttributeValues};
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

/// Where the light comes from.
///
/// One direction for the whole scene, matching the single directional light the
/// art direction calls for. Held here as well as on the light itself because
/// the shader needs it as a uniform and a flat material has no business reading
/// the scene's light list — that lookup is most of what makes PBR expensive.
pub const LIGHT_FROM: Vec3 = Vec3::new(-0.35, 0.86, -0.37);

/// How many steps the lighting is banded into.
///
/// Three. Two is a hard cel look that suits characters and makes a large flat
/// terrain read as two enormous slabs; four is close enough to smooth that the
/// decision stops being visible. Three keeps a top, a side and a shadow, which
/// is exactly what an isometric scene of boxes needs.
pub const SHADE_STEPS: f32 = 3.0;

/// How dark the unlit side goes.
///
/// Not zero. A black underside reads as a hole rather than as a shadowed face,
/// and this scene has no bounce light to fill one in.
pub const AMBIENT: f32 = 0.42;

#[derive(Asset, AsBindGroup, Debug, Clone, TypePath)]
pub struct FlatMaterial {
    #[uniform(0)]
    pub colour: LinearRgba,
    #[uniform(0)]
    pub light: Vec3,
    #[uniform(0)]
    pub steps: f32,
    #[uniform(0)]
    pub ambient: f32,
    #[uniform(0)]
    pub _pad: Vec3,
}

impl FlatMaterial {
    /// A material of one colour, lit the way everything else is.
    pub fn new(colour: Color) -> FlatMaterial {
        FlatMaterial {
            colour: colour.into(),
            light: LIGHT_FROM.normalize(),
            steps: SHADE_STEPS,
            ambient: AMBIENT,
            _pad: Vec3::ZERO,
        }
    }

    /// A material with no shading at all.
    ///
    /// Health bars, selection rings, the placement preview: things that are
    /// really interface drawn in the world, and that should not pick up a lit
    /// and an unlit face. Expressed as ambient light of one rather than as a
    /// second material type, so there is still one shader and one pipeline —
    /// `shade` works out to exactly one and the arithmetic falls away.
    pub fn unlit(colour: Color) -> FlatMaterial {
        FlatMaterial {
            ambient: 1.0,
            ..FlatMaterial::new(colour)
        }
    }

    /// White, so a mesh's own vertex colours come through unchanged.
    ///
    /// The terrain is one mesh carrying a colour per cell; multiplying by white
    /// leaves those alone. Units are the other way round — one grey mesh per
    /// kind, tinted per team by the material.
    pub fn vertex_coloured() -> FlatMaterial {
        FlatMaterial::new(Color::WHITE)
    }
}

impl Material for FlatMaterial {
    /// Blended when the colour asks for it.
    ///
    /// Read from the material rather than fixed, because the same type draws
    /// both the opaque world and the translucent overlays on top of it — and
    /// an opaque thing drawn through the blend pipeline is sorted per frame
    /// for no reason.
    fn alpha_mode(&self) -> AlphaMode {
        if self.colour.alpha < 1.0 {
            AlphaMode::Blend
        } else {
            AlphaMode::Opaque
        }
    }

    fn vertex_shader() -> ShaderRef {
        "shaders/flat.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/flat.wgsl".into()
    }

    /// Every mesh drawn with this must carry a vertex colour, because the
    /// shader multiplies by one. A mesh without the attribute would fail to
    /// bind and render as the fallback magenta — which looks like a material
    /// bug and is a long way from the actual problem, so the layout says so.
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(5),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        Ok(())
    }
}

/// A primitive turned into a mesh with a white vertex colour.
///
/// The convenience form of [`ensure_vertex_colours`], because every primitive
/// in this renderer needs it and forgetting once means a magenta object and a
/// puzzled half hour.
pub fn coloured(shape: impl Into<Mesh>) -> Mesh {
    let mut mesh: Mesh = shape.into();
    ensure_vertex_colours(&mut mesh);
    mesh
}

/// Gives a mesh a flat white vertex colour if it has none.
///
/// Primitive meshes from `Cuboid` and friends carry no colour attribute, and
/// the shader multiplies by one. Adding white here is less bad than making the
/// colour optional in the shader: an optional attribute means two pipeline
/// variants for a value that is always one.
pub fn ensure_vertex_colours(mesh: &mut Mesh) {
    if mesh.attribute(Mesh::ATTRIBUTE_COLOR).is_some() {
        return;
    }
    let count = mesh.count_vertices();
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        VertexAttributeValues::Float32x4(vec![[1.0, 1.0, 1.0, 1.0]; count]),
    );
}
