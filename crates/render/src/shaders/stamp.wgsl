// Instanced Stamp quads. Each draws its Silhouette in the fill colour beneath its line art,
// so overlapping Stamps hide each other's lines (ADR 0003). Colours arrive linear;
// the atlas pages hold premultiplied art.

struct Instance {
    @location(0) position: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) anchor: vec2<f32>,
    // Rotation in radians (clockwise on screen), then +1 or -1 to flip left to right.
    @location(3) turn: vec2<f32>,
    @location(4) art_uv: vec4<f32>,
    @location(5) sil_uv: vec4<f32>,
    // Linear tint in rgb; the opacity in a.
    @location(6) tint: vec4<f32>,
    // Linear, premultiplied.
    @location(7) fill: vec4<f32>,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) art_uv: vec2<f32>,
    @location(1) sil_uv: vec2<f32>,
    @location(2) tint: vec4<f32>,
    @location(3) fill: vec4<f32>,
};

@group(1) @binding(0) var art: texture_2d<f32>;
@group(1) @binding(1) var art_sampler: sampler;
@group(2) @binding(0) var silhouette: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) index: u32, in: Instance) -> VertexOut {
    var corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
    );
    let corner = corners[index];
    var local = (corner - in.anchor) * in.size;
    local.x = local.x * in.turn.y;
    let c = cos(in.turn.x);
    let s = sin(in.turn.x);
    let rotated = vec2(local.x * c - local.y * s, local.x * s + local.y * c);
    var out: VertexOut;
    out.position = to_clip(in.position + rotated);
    out.art_uv = in.art_uv.xy + corner * (in.art_uv.zw - in.art_uv.xy);
    out.sil_uv = in.sil_uv.xy + corner * (in.sil_uv.zw - in.sil_uv.xy);
    out.tint = in.tint;
    out.fill = in.fill;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let ink = textureSample(art, art_sampler, in.art_uv);
    let inside = textureSample(silhouette, art_sampler, in.sil_uv).a;
    let line = vec4(ink.rgb * in.tint.rgb, ink.a);
    let colour = line + in.fill * inside * (1.0 - line.a);
    return colour * in.tint.a;
}
