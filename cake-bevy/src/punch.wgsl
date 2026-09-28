// Writes fully transparent pixels. Drawn as an opaque (unblended) mesh, it
// overwrites whatever is underneath, which is how the circle window gets its
// see-through corners. ColorMaterial cannot do this: in opaque mode it forces
// alpha to 1.
#import bevy_sprite::mesh2d_vertex_output::VertexOutput

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(0.0, 0.0, 0.0, 0.0);
}
