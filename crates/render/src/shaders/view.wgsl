// Shared by every Canvas pipeline: group 0 maps Canvas units to clip space.
struct View {
    scale: vec2<f32>,
    translate: vec2<f32>,
};

@group(0) @binding(0) var<uniform> view: View;

fn to_clip(canvas: vec2<f32>) -> vec4<f32> {
    return vec4(canvas * view.scale + view.translate, 0.0, 1.0);
}
