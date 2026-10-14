/// An sRGB colour with straight (not premultiplied) alpha, as users pick it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Linear, premultiplied RGBA, which is what the sRGB render target blends in.
    pub fn to_linear_premultiplied(self) -> [f32; 4] {
        let a = f32::from(self.a) / 255.0;
        [
            srgb_to_linear(self.r) * a,
            srgb_to_linear(self.g) * a,
            srgb_to_linear(self.b) * a,
            a,
        ]
    }

    pub(crate) fn to_wgpu(self) -> wgpu::Color {
        let [r, g, b, a] = self.to_linear_premultiplied();
        wgpu::Color {
            r: r.into(),
            g: g.into(),
            b: b.into(),
            a: a.into(),
        }
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

impl From<ssmap_core::Color> for Color {
    fn from(color: ssmap_core::Color) -> Self {
        let ssmap_core::Color { r, g, b, a } = color;
        Self { r, g, b, a }
    }
}
