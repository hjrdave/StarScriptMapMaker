// Stamps: one instanced quad per Stamp, sampling the Asset's line art and Silhouette from
// atlas pages. Colours arrive linear and premultiplied.

@group(1) @binding(0) var art_texture: texture_2d<f32>;
@group(1) @binding(1) var silhouette_texture: texture_2d<f32>;
@group(1) @binding(2) var atlas_sampler: sampler;

struct Instance {
    // Where the Anchor sits, and the columns mapping art-local offsets (Canvas units from the
    // Anchor) to Canvas offsets; they carry scale, flip and rotation.
    @location(0) anchor: vec2<f32>,
    @location(1) column_x: vec2<f32>,
    @location(2) column_y: vec2<f32>,
    // The art's rectangle in art-local space.
    @location(3) rect_min: vec2<f32>,
    @location(4) rect_size: vec2<f32>,
    @location(5) art_uv: vec4<f32>,
    @location(6) silhouette_uv: vec4<f32>,
    @location(7) tint: vec4<f32>,
    @location(8) fill: vec4<f32>,
    @location(9) opacity: f32,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) art_uv: vec2<f32>,
    @location(1) silhouette_uv: vec2<f32>,
    @location(2) tint: vec4<f32>,
    @location(3) fill: vec4<f32>,
    @location(4) opacity: f32,
};

const CORNERS = array<vec2<f32>, 6>(
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    vec2(0.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
);

@vertex
fn vs_main(@builtin(vertex_index) index: u32, instance: Instance) -> VertexOut {
    let corner = CORNERS[index];
    let local = instance.rect_min + corner * instance.rect_size;
    let canvas = instance.anchor + instance.column_x * local.x + instance.column_y * local.y;
    var out: VertexOut;
    out.position = to_clip(canvas);
    out.art_uv = mix(instance.art_uv.xy, instance.art_uv.zw, corner);
    out.silhouette_uv = mix(instance.silhouette_uv.xy, instance.silhouette_uv.zw, corner);
    out.tint = instance.tint;
    out.fill = instance.fill;
    out.opacity = instance.opacity;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // The fill shows through wherever the line art leaves gaps inside the Silhouette.
    let art = textureSample(art_texture, atlas_sampler, in.art_uv) * in.tint;
    let inside = textureSample(silhouette_texture, atlas_sampler, in.silhouette_uv).a;
    let under = in.fill * inside;
    return (art + under * (1.0 - art.a)) * in.opacity;
}
