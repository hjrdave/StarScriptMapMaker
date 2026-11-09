// Tessellated vector fills. Colours arrive linear and premultiplied.
//
// A fill can carry grain: `grain.x` is its strength (0 for none) and `grain.y` the width of one
// speck in Canvas units. The speck is picked from the Canvas position, so the grain stays put
// under pan and zoom and is the same in every export tile.

struct VertexIn {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) grain: vec2<f32>,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) grain: vec2<f32>,
    @location(2) canvas: vec2<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.position = to_clip(in.position);
    out.color = in.color;
    out.grain = in.grain;
    out.canvas = in.position;
    return out;
}

// A number from 0 to 1 that is the same for every sample in one grain cell.
fn cell_noise(cell: vec2<i32>) -> f32 {
    var h = bitcast<u32>(cell.x) * 374761393u + bitcast<u32>(cell.y) * 668265263u;
    h = (h ^ (h >> 13u)) * 1274126177u;
    h = h ^ (h >> 16u);
    return f32(h & 65535u) / 65535.0;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // The derivatives are taken before any branch, so every sample in a quad runs them.
    let pixel = max((abs(dpdx(in.canvas.x)) + abs(dpdy(in.canvas.y))) * 0.5, 0.000001);
    if (in.grain.x <= 0.0) {
        return in.color;
    }
    // Specks smaller than a pixel would only shimmer, so the grain fades out as it gets there.
    let visible = smoothstep(0.75, 2.0, in.grain.y / pixel);
    let speck = cell_noise(vec2<i32>(floor(in.canvas / in.grain.y))) * 2.0 - 1.0;
    let factor = 1.0 + in.grain.x * visible * speck * 0.5;
    return vec4<f32>(min(in.color.rgb * factor, vec3<f32>(in.color.a)), in.color.a);
}
