//! Colour conversions for the GPU. The colour type itself is the Map's ([`Color`]).

pub use ssmap_core::Color;

/// What the renderer needs from a [`Color`].
pub(crate) trait ColorExt {
    /// Linear, premultiplied RGBA, which is what the sRGB render target blends in.
    fn to_linear_premultiplied(self) -> [f32; 4];

    /// Linear RGBA with straight alpha, for shaders that apply alpha themselves.
    fn to_linear(self) -> [f32; 4];

    fn to_wgpu(self) -> wgpu::Color;

    /// The same colour with its alpha multiplied by `opacity`, from 0 to 1.
    fn faded(self, opacity: f32) -> Color;
}

impl ColorExt for Color {
    fn to_linear_premultiplied(self) -> [f32; 4] {
        let a = f32::from(self.a) / 255.0;
        [
            srgb_to_linear(self.r) * a,
            srgb_to_linear(self.g) * a,
            srgb_to_linear(self.b) * a,
            a,
        ]
    }

    fn to_linear(self) -> [f32; 4] {
        [
            srgb_to_linear(self.r),
            srgb_to_linear(self.g),
            srgb_to_linear(self.b),
            f32::from(self.a) / 255.0,
        ]
    }

    fn to_wgpu(self) -> wgpu::Color {
        let [r, g, b, a] = self.to_linear_premultiplied();
        wgpu::Color {
            r: r.into(),
            g: g.into(),
            b: b.into(),
            a: a.into(),
        }
    }

    fn faded(self, opacity: f32) -> Color {
        let a = (f32::from(self.a) * opacity.clamp(0.0, 1.0)).round() as u8;
        Color { a, ..self }
    }
}

fn srgb_to_linear(channel: u8) -> f32 {
    let c = f32::from(channel) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
