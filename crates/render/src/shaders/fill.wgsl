// Tessellated vector fills. Colours arrive linear and premultiplied.
//
// A fill can carry grain: `grain.x` is its strength (0 for none) and `grain.y` the width of one
// speck in Canvas units. The speck is worked out from the Canvas position, so the grain stays
// put under pan and zoom and is the same in every export tile.

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

// Smooth noise from 0 to 1: the four nearest cell numbers, eased and blended, so there are no
// hard cell edges.
fn smooth_noise(p: vec2<f32>) -> f32 {
    let cell = vec2<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = cell_noise(cell);
    let b = cell_noise(cell + vec2<i32>(1, 0));
    let c = cell_noise(cell + vec2<i32>(0, 1));
    let d = cell_noise(cell + vec2<i32>(1, 1));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
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
    // Fine flecks of the given size, with softer, wider ones under them (the second sample is
    // shifted and turned a little so the two never line up). Blending smooth noise pulls it
    // towards the middle, so it is stretched back out to fill the range from -1 to 1.
    let p = in.canvas / in.grain.y;
    let fine = smooth_noise(p);
    let broad = smooth_noise(vec2<f32>(p.x * 0.31 + p.y * 0.14, p.y * 0.31 - p.x * 0.14) + 37.0);
    let speck = clamp((0.65 * fine + 0.35 * broad - 0.5) * 3.4, -1.0, 1.0);
    let factor = 1.0 + in.grain.x * visible * speck * 0.5;
    return vec4<f32>(min(in.color.rgb * factor, vec3<f32>(in.color.a)), in.color.a);
}
