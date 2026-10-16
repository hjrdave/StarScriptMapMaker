// The ocean texture: soft light and dark mottling over the Canvas background fill.
// Noise is evaluated in Canvas units, so it is the same on screen and in export, and it uses
// integer hashing so software and hardware adapters agree closely.

struct Ocean {
    canvas_size: vec2<f32>,
    strength: f32,
    scale: f32,
    origin: vec2<f32>,
    light: vec4<f32>,
    dark: vec4<f32>,
};

@group(1) @binding(0) var<uniform> ocean: Ocean;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) canvas: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VertexOut {
    // Two triangles covering the Canvas rectangle.
    var corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    );
    let canvas = ocean.origin + corners[i] * ocean.canvas_size;
    var out: VertexOut;
    out.position = to_clip(canvas);
    out.canvas = canvas;
    return out;
}

fn hash(cell: vec2<i32>) -> f32 {
    var h = bitcast<u32>(cell.x) * 374761393u + bitcast<u32>(cell.y) * 668265263u;
    h = (h ^ (h >> 13u)) * 1274126177u;
    h = h ^ (h >> 16u);
    return f32(h & 0xffffffu) / 16777216.0;
}

fn value_noise(p: vec2<f32>) -> f32 {
    let cell = vec2<i32>(floor(p));
    let f = fract(p);
    let s = f * f * (3.0 - 2.0 * f);
    let a = hash(cell);
    let b = hash(cell + vec2(1, 0));
    let c = hash(cell + vec2(0, 1));
    let d = hash(cell + vec2(1, 1));
    return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let p = in.canvas / ocean.scale;
    let n = value_noise(p / 180.0) * 0.6
        + value_noise(p / 60.0) * 0.3
        + value_noise(p / 20.0) * 0.1;
    // Above the midpoint lightens, below darkens; premultiplied output. Blending is linear,
    // where white reads far stronger than black, so lightening is scaled down to balance.
    let amount = clamp((n - 0.5) * 2.0, -1.0, 1.0) * ocean.strength;
    if (amount >= 0.0) {
        return ocean.light * (amount * 0.35);
    }
    return ocean.dark * -amount;
}
