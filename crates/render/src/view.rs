/// Which part of the Canvas lands on a render target, and how big.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct View {
    /// The Canvas point at the target's top-left corner, in Canvas units.
    pub origin: [f64; 2],
    /// Target pixels per Canvas unit.
    pub zoom: f64,
    /// Target size in physical pixels.
    pub size: [u32; 2],
}

impl View {
    /// Fits a `canvas` of the given size inside `size`, centred, leaving `margin` pixels clear.
    pub fn fit(canvas: [f64; 2], size: [u32; 2], margin: f64) -> Self {
        let available = [
            (f64::from(size[0]) - 2.0 * margin).max(1.0),
            (f64::from(size[1]) - 2.0 * margin).max(1.0),
        ];
        let zoom = (available[0] / canvas[0]).min(available[1] / canvas[1]);
        let origin = [
            (canvas[0] * zoom - f64::from(size[0])) / (2.0 * zoom),
            (canvas[1] * zoom - f64::from(size[1])) / (2.0 * zoom),
        ];
        Self { origin, zoom, size }
    }

    /// The whole Canvas and nothing else, scaled so its longer side is `longest_side` pixels,
    /// as for the preview stored in a Map file.
    pub fn whole_canvas(canvas: [f64; 2], longest_side: u32) -> Self {
        let scale = f64::from(longest_side) / canvas[0].max(canvas[1]);
        let side = |length: f64| ((length * scale).round() as u32).max(1);
        Self {
            origin: [0.0, 0.0],
            zoom: scale,
            size: [side(canvas[0]), side(canvas[1])],
        }
    }

    /// The Canvas point under a target pixel position.
    pub fn to_canvas(&self, pixel: [f64; 2]) -> [f64; 2] {
        [
            self.origin[0] + pixel[0] / self.zoom,
            self.origin[1] + pixel[1] / self.zoom,
        ]
    }

    /// The Canvas area in view as `[min_x, min_y, max_x, max_y]`, padded by a couple of pixels
    /// so antialiased edges at the border are never culled.
    pub(crate) fn visible(&self) -> [f32; 4] {
        let pad = 2.0 / self.zoom;
        let max = self.to_canvas([f64::from(self.size[0]), f64::from(self.size[1])]);
        [
            (self.origin[0] - pad) as f32,
            (self.origin[1] - pad) as f32,
            (max[0] + pad) as f32,
            (max[1] + pad) as f32,
        ]
    }

    /// Zooms by `factor`, keeping the Canvas point under `pixel` fixed.
    pub fn zoom_at(&mut self, pixel: [f64; 2], factor: f64) {
        let anchor = self.to_canvas(pixel);
        self.zoom *= factor;
        self.origin = [
            anchor[0] - pixel[0] / self.zoom,
            anchor[1] - pixel[1] / self.zoom,
        ];
    }

    /// Moves the view by a drag of `delta` target pixels.
    pub fn pan(&mut self, delta: [f64; 2]) {
        self.origin[0] -= delta[0] / self.zoom;
        self.origin[1] -= delta[1] / self.zoom;
    }

    /// The Canvas-to-clip-space transform as `clip = canvas * scale + translate`.
    pub(crate) fn uniform(&self) -> ViewUniform {
        let [w, h] = [f64::from(self.size[0]), f64::from(self.size[1])];
        let sx = 2.0 * self.zoom / w;
        let sy = -2.0 * self.zoom / h;
        ViewUniform {
            scale: [sx as f32, sy as f32],
            translate: [
                (-1.0 - sx * self.origin[0]) as f32,
                (1.0 - sy * self.origin[1]) as f32,
            ],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ViewUniform {
    scale: [f32; 2],
    translate: [f32; 2],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zooming_keeps_the_point_under_the_cursor() {
        let mut view = View::fit([1000.0, 500.0], [800, 600], 20.0);
        let before = view.to_canvas([123.0, 456.0]);
        view.zoom_at([123.0, 456.0], 2.5);
        let after = view.to_canvas([123.0, 456.0]);
        assert!((before[0] - after[0]).abs() < 1e-9 && (before[1] - after[1]).abs() < 1e-9);
    }

    #[test]
    fn fit_centres_the_canvas() {
        let view = View::fit([1000.0, 500.0], [800, 600], 0.0);
        assert_eq!(view.zoom, 0.8);
        assert_eq!(view.to_canvas([400.0, 300.0]), [500.0, 250.0]);
    }

    #[test]
    fn whole_canvas_fits_the_longer_side() {
        let view = View::whole_canvas([4096.0, 3072.0], 512);
        assert_eq!(view.size, [512, 384]);
        assert_eq!(view.to_canvas([512.0, 384.0]), [4096.0, 3072.0]);
        assert_eq!(View::whole_canvas([100.0, 4000.0], 512).size, [13, 512]);
    }
}
