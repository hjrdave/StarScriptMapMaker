// Tessellated vector fills. Colours arrive linear and premultiplied.
//
// A fill can carry grain (fine speckle) and a mottle (large soft patches of a second colour).
// Both are worked out from the Canvas position, so they stay put under pan and zoom and are
// the same in every export tile. The settings arrive packed into two 4-byte attributes (see
// `pack_style` in `vector/chunk.rs`, which this must agree with):
//   look = (grain amount, grain size, mottle contrast, mottle scale), each 0..1;
//   tint = (mottle colour in sRGB, a byte that moves the patches about).
// The size and the scale are on log scales: min * pow(max / min, value).

struct VertexIn {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) look: vec4<f32>,
    @location(3) tint: vec4<f32>,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) @interpolate(flat) look: vec4<f32>,
    @location(2) @interpolate(flat) tint: vec4<f32>,
    @location(3) canvas: vec2<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.position = to_clip(in.position);
    out.color = in.color;
    out.look = in.look;
    out.tint = in.tint;
    out.canvas = in.position;
    return out;
}

// A number from 0 to 1 that is the same for every sample in one cell.
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

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // The derivatives are taken before any branch, so every sample in a quad runs them.
    let pixel = max((abs(dpdx(in.canvas.x)) + abs(dpdy(in.canvas.y))) * 0.5, 0.000001);
    let grain_amount = in.look.x;
    let contrast = in.look.z;
    if (grain_amount <= 0.0 && contrast <= 0.0) {
        return in.color;
    }
    var color = in.color;

    if (contrast > 0.0) {
        // Wide soft patches: two sizes of smooth noise, shifted by the seed byte, pushed
        // through a smoothstep so there are clear patches with soft edges, not a haze.
        let scale = 10.0 * pow(500.0, in.look.w);
        let shift = in.tint.w * 255.0 * 7.31;
        let q = in.canvas / scale + vec2<f32>(shift, shift * 0.618);
        let n = 0.7 * smooth_noise(q) + 0.3 * smooth_noise(q * 2.3 + vec2<f32>(11.0, 5.0));
        let blot = smoothstep(0.3, 0.7, n) * contrast;
        // The patch colour takes the fill's alpha, as the fill colour does.
        let other = srgb_to_linear(in.tint.rgb) * color.a;
        color = vec4<f32>(mix(color.rgb, other, blot), color.a);
    }

    if (grain_amount > 0.0) {
        let size = 0.25 * pow(64.0, in.look.y);
        // Specks smaller than a pixel would only shimmer, so the grain fades out as it gets there.
        let visible = smoothstep(0.75, 2.0, size / pixel);
        // Fine flecks of the given size, with softer, wider ones under them (the second sample
        // is shifted and turned a little so the two never line up). Blending smooth noise pulls
        // it towards the middle, so it is stretched back out to fill the range from -1 to 1.
        let p = in.canvas / size;
        let fine = smooth_noise(p);
        let broad = smooth_noise(vec2<f32>(p.x * 0.31 + p.y * 0.14, p.y * 0.31 - p.x * 0.14) + 37.0);
        let speck = clamp((0.65 * fine + 0.35 * broad - 0.5) * 3.4, -1.0, 1.0);
        let factor = 1.0 + grain_amount * visible * speck * 0.5;
        color = vec4<f32>(min(color.rgb * factor, vec3<f32>(color.a)), color.a);
    }
    return color;
}
