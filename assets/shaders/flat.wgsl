// Flat / cel shading.
//
// The look the project is after is the original's: solid colour, one obvious
// light direction, a clear edge between lit and unlit. Everything below exists
// to get that and nothing else.
//
// Written as a replacement for `StandardMaterial` rather than a configuration
// of it. Turning PBR's roughness to one and its metallic to zero produces
// something close to flat, but it still pays for the whole physically based
// pipeline — the split-sum approximation, the Fresnel term, the environment
// lookups — to compute a result that is thrown away. This does the small
// amount of arithmetic the look actually needs.
//
// It is also, deliberately, not a lighting model anybody would defend on
// physical grounds. Banding light into steps is a *drawing* decision. See
// docs/04-rendering.md.

#import bevy_pbr::{
    mesh_functions,
    view_transformations::position_world_to_clip,
}

struct FlatMaterial {
    // The base colour. Multiplied by the vertex colour, so terrain can vary
    // per cell from one material while units get their team colour from here.
    colour: vec4<f32>,
    // Direction the light comes *from*, in world space. Normalised on the CPU
    // so the shader does not do it per fragment.
    light: vec3<f32>,
    // How many steps to band the lighting into.
    //
    // Two is a hard cel look and three reads better on curved surfaces; one
    // would be no shading at all, which loses the shape of everything. Kept as
    // a uniform so the look can be tuned without a recompile.
    steps: f32,
    // How dark the unlit side goes. Not zero: a black underside reads as a
    // hole rather than as a shadowed face, and this scene has no bounce light
    // to fill it in.
    ambient: f32,
    _pad: vec3<f32>,
}

// The group index is not a constant: Bevy decides it, and in a bindless
// configuration it is not two. Writing `2` produced a pipeline whose binding
// zero was a storage buffer while the shader asked for a uniform — which
// reports as "storage class doesn't match" and looks nothing like a typo.
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: FlatMaterial;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(5) colour: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) colour: vec4<f32>,
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    var world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(vertex.position, 1.0),
    );
    out.clip_position = position_world_to_clip(world_position.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(
        vertex.normal,
        vertex.instance_index,
    );
    out.colour = vertex.colour;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let normal = normalize(in.world_normal);
    // Lambert, and then thrown away into steps. The banding is the whole point:
    // a smooth gradient across a curved surface is exactly what this look is
    // avoiding.
    let lambert = max(dot(normal, material.light), 0.0);
    let banded = floor(lambert * material.steps + 0.5) / material.steps;
    // Remapped so the darkest step is `ambient` rather than black. A face
    // pointing away from the light should read as a shadowed face, not as a
    // hole in the model.
    let shade = material.ambient + (1.0 - material.ambient) * banded;

    let base = material.colour * in.colour;
    return vec4<f32>(base.rgb * shade, base.a);
}
