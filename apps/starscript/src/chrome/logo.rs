//! The StarScript logo (`packaging/StarScript_Logo.svg`): the window icon, and the mark shown
//! on Home and the Credits screen. Rasterised with resvg on first use, like the UI icons.

use eframe::egui;
use ssmap_render::raster::{Art, Raster};

const SVG: &[u8] = include_bytes!("../../packaging/StarScript_Logo.svg");

/// Side of the window icon in pixels; the OS scales it for title bars and taskbars.
const WINDOW_ICON_SIDE: u32 = 256;

/// The logo is tall, so the window icon centres it on a square canvas. The share of the
/// canvas the logo's height fills.
const WINDOW_ICON_FILL: f32 = 0.88;

/// The logo rasterised `height` pixels tall, as premultiplied RGBA8.
fn raster(height: u32) -> Option<Raster> {
    let art = Art::from_svg(SVG).ok()?;
    let [width, art_height] = art.size();
    Some(art.rasterise((height as f32 * width / art_height).round() as u32))
}

/// The window icon: the logo centred on a transparent square.
pub(crate) fn window_icon() -> Option<egui::IconData> {
    let side = WINDOW_ICON_SIDE;
    let logo = raster((side as f32 * WINDOW_ICON_FILL).round() as u32)?;
    let pixels = logo.unpremultiplied();
    let (x0, y0) = (
        side.saturating_sub(logo.width) / 2,
        side.saturating_sub(logo.height) / 2,
    );
    let mut rgba = vec![0; (side * side * 4) as usize];
    let len = (logo.width.min(side) * 4) as usize;
    for row in 0..logo.height.min(side) {
        let from = (row * logo.width * 4) as usize;
        let to = (((y0 + row) * side + x0) * 4) as usize;
        rgba[to..to + len].copy_from_slice(&pixels[from..from + len]);
    }
    Some(egui::IconData {
        rgba,
        width: side,
        height: side,
    })
}

/// Draws the logo `height` points tall, at the current pixel density.
pub(crate) fn show(ui: &mut egui::Ui, height: f32) -> egui::Response {
    let ctx = ui.ctx();
    let id = egui::Id::new("ssmap-logo");
    let ppp = ctx.pixels_per_point();
    let pixels = (height * ppp).round().max(1.0) as u32;
    let cached = ctx.data(|d| d.get_temp::<(u32, egui::TextureHandle)>(id));
    let texture = match cached {
        Some((cached_pixels, texture)) if cached_pixels == pixels => texture,
        _ => {
            let image = raster(pixels).map_or_else(
                || egui::ColorImage::filled([1, 1], egui::Color32::TRANSPARENT),
                |r| {
                    egui::ColorImage::from_rgba_premultiplied(
                        [r.width as usize, r.height as usize],
                        &r.pixels,
                    )
                },
            );
            let texture = ctx.load_texture("logo", image, egui::TextureOptions::LINEAR);
            ctx.data_mut(|d| d.insert_temp(id, (pixels, texture.clone())));
            texture
        }
    };
    let size = texture.size_vec2() / ppp;
    ui.add(egui::Image::new((texture.id(), size)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_icon_is_a_centred_square() {
        let icon = window_icon().unwrap();
        assert_eq!(
            (icon.width, icon.height),
            (WINDOW_ICON_SIDE, WINDOW_ICON_SIDE)
        );
        assert_eq!(icon.rgba.len(), (icon.width * icon.height * 4) as usize);
        let alpha = |x: u32, y: u32| icon.rgba[((y * icon.width + x) * 4 + 3) as usize];
        // Corners are padding; the middle is the mark.
        assert_eq!(alpha(0, 0), 0);
        assert_eq!(alpha(icon.width - 1, icon.height - 1), 0);
        assert!(alpha(icon.width / 2, icon.height / 2) > 0);
        // The padding left and right of the mark matches.
        let opaque = |x: &u32| (0..icon.height).any(|y| alpha(*x, y) > 0);
        let first = (0..icon.width).find(opaque).unwrap();
        let last = (0..icon.width).rev().find(opaque).unwrap();
        assert!(first.abs_diff(icon.width - 1 - last) <= 2);
    }

    #[test]
    fn logo_raster_is_as_tall_as_asked_and_narrower() {
        let r = raster(64).unwrap();
        assert_eq!(r.height, 64);
        assert!(r.width < r.height);
    }
}
