// PROTOTYPE - ocean backdrop drawn by our own wgpu pipeline inside egui's render pass.

struct Uniforms {
    // canvas rect size in physical px, camera offset (map units), zoom (px per map unit), time
    size: vec2<f32>,
    offset: vec2<f32>,
    zoom: f32,
    time: f32,
    map_size: vec2<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    // fullscreen triangle; egui sets the viewport to the callback rect
    var p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    var out: VsOut;
    out.pos = vec4(p[i], 0.0, 1.0);
    out.uv = vec2(p[i].x * 0.5 + 0.5, 0.5 - p[i].y * 0.5);
    return out;
}

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let s = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), s.x),
               mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), s.x), s.y);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // screen px -> map units
    let map = u.offset + in.uv * u.size / u.zoom;
    let inside = all(map >= vec2(0.0)) && all(map <= u.map_size);
    if (!inside) {
        return vec4(0.11, 0.11, 0.12, 1.0); // pasteboard outside the bounded canvas
    }
    let n = noise(map * 0.01 + vec2(u.time * 0.05, 0.0)) * 0.6 + noise(map * 0.04) * 0.4;
    let deep = vec3(0.10, 0.25, 0.38);
    let shallow = vec3(0.24, 0.45, 0.55);
    var c = mix(deep, shallow, n);
    // faint grid lines every 100 map units, like a chart
    let g = abs(fract(map / 100.0 + 0.5) - 0.5) * 100.0 * u.zoom;
    if (min(g.x, g.y) < 0.6) { c = c + vec3(0.04); }
    return vec4(c, 1.0);
}
