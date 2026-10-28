//! Renders `packaging/StarScript_Logo.svg` into the icon set: square PNGs and a multi-size
//! `icon.ico`, the logo centred with padding. Run from the repo root:
//!
//! `cargo run -p app --example gen_icons -- apps/starscript/packaging/icons`

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ssmap_render::raster::Art;

fn square(side: u32, fill: f32) -> Vec<u8> {
    let art = Art::from_svg(include_bytes!("../packaging/StarScript_Logo.svg")).unwrap();
    let [w, h] = art.size();
    let lh = (side as f32 * fill).round() as u32;
    let r = art.rasterise((lh as f32 * w / h).round() as u32);
    let px = r.unpremultiplied();
    let mut out = vec![0u8; (side * side * 4) as usize];
    let (x0, y0) = ((side - r.width) / 2, (side - r.height) / 2);
    let len = (r.width * 4) as usize;
    for row in 0..r.height {
        let from = (row * r.width * 4) as usize;
        let to = (((y0 + row) * side + x0) * 4) as usize;
        out[to..to + len].copy_from_slice(&px[from..from + len]);
    }
    out
}

fn fill_for(side: u32) -> f32 {
    if side <= 32 { 0.94 } else { 0.88 }
}

fn png(side: u32) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    image::write_buffer_with_format(
        &mut buf,
        &square(side, fill_for(side)),
        side,
        side,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .unwrap();
    buf.into_inner()
}

fn main() {
    let dir = std::env::args().nth(1).unwrap();
    for s in [32, 128, 256, 512] {
        std::fs::write(format!("{dir}/{s}x{s}.png"), png(s)).unwrap();
    }
    let sizes = [16u32, 32, 48, 64, 128, 256];
    let imgs: Vec<_> = sizes.iter().map(|&s| png(s)).collect();
    let mut ico = vec![0, 0, 1, 0, sizes.len() as u8, 0];
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (s, d) in sizes.iter().zip(&imgs) {
        let b = if *s >= 256 { 0 } else { *s as u8 };
        ico.extend([b, b, 0, 0, 1, 0, 32, 0]);
        ico.extend((d.len() as u32).to_le_bytes());
        ico.extend(offset.to_le_bytes());
        offset += d.len() as u32;
    }
    for d in imgs {
        ico.extend(d);
    }
    std::fs::write(format!("{dir}/icon.ico"), ico).unwrap();
}
