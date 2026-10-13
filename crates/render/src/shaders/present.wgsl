// Copies the resolved Canvas target into the host's viewport, one texel per pixel when the
// viewport and the target are the same size.

@group(0) @binding(0) var source: texture_2d<f32>;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VertexOut {
    var p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    var out: VertexOut;
    out.position = vec4(p[i], 0.0, 1.0);
    out.uv = vec2(p[i].x * 0.5 + 0.5, 0.5 - p[i].y * 0.5);
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(source));
    let texel = vec2<i32>(clamp(floor(in.uv * size), vec2(0.0), size - 1.0));
    return textureLoad(source, texel, 0);
}
