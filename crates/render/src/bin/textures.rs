//! Generates the seamless textures of the built-in `textures` pack from noise (ADR 0015).
//!
//! ```text
//! cargo run -p ssmap-render --bin textures -- [output-folder]
//! ```
//!
//! Writes one PNG per texture (default folder `assets/packs/textures/textures`, so run it from
//! the repository root). Nothing is downloaded or copied: every pixel comes from the seeded noise
//! below, so the output is the same on every run and every machine, and the PNGs carry no
//! licence of anyone else's. The noise wraps around the image, so a texture tiles with no seam.
//! Each texture is centred on mid-grey, which the fill shader treats as "no change".
//!
//! To change a texture, edit its function here and rerun the tool; to add one, add it to
//! `TEXTURES` and to `assets/packs/textures/pack.toml`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Pixels on a side. The renderer keeps textures at this size (see `fill_texture.rs`).
const SIZE: u32 = 256;

/// A texture's name (its file is `<name>.png`) and the colour of each pixel, as a function of
/// its place in the image from 0 to 1 on each side.
type Texture = (&'static str, fn(f64, f64) -> [f64; 3]);

const TEXTURES: [Texture; 4] = [
    ("parchment", parchment),
    ("grass", grass),
    ("sand", sand),
    ("stone", stone),
];

/// A number from 0 to 1 for a lattice point, the same for the same `seed` and point.
fn lattice(seed: u32, x: u32, y: u32) -> f64 {
    let mut h = x.wrapping_mul(374_761_393)
        ^ y.wrapping_mul(668_265_263)
        ^ seed.wrapping_mul(2_246_822_519);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    f64::from(h & 0xffff) / 65535.0
}

/// Smooth value noise that repeats every `cells_x` by `cells_y` cells, for `u` and `v` in 0 to 1
/// (which wrap), so the image it fills tiles.
fn noise(seed: u32, cells_x: u32, cells_y: u32, u: f64, v: f64) -> f64 {
    let (px, py) = (u * f64::from(cells_x), v * f64::from(cells_y));
    let (fx, fy) = (px.fract(), py.fract());
    let (ix, iy) = (px.floor() as u32, py.floor() as u32);
    let ease = |t: f64| t * t * (3.0 - 2.0 * t);
    let (tx, ty) = (ease(fx), ease(fy));
    let at = |dx: u32, dy: u32| lattice(seed, (ix + dx) % cells_x, (iy + dy) % cells_y);
    let top = at(0, 0) + (at(1, 0) - at(0, 0)) * tx;
    let bottom = at(0, 1) + (at(1, 1) - at(0, 1)) * tx;
    top + (bottom - top) * ty
}

/// Layers of noise, each twice as fine and `persistence` times as strong as the last, from 0 to 1.
fn fractal(seed: u32, cells: u32, octaves: u32, persistence: f64, u: f64, v: f64) -> f64 {
    let (mut sum, mut weight, mut amplitude) = (0.0, 0.0, 1.0);
    for octave in 0..octaves {
        let scale = cells << octave;
        sum += amplitude * noise(seed + octave, scale, scale, u, v);
        weight += amplitude;
        amplitude *= persistence;
    }
    sum / weight
}

/// Spreads a noise value from 0 to 1 about mid-grey by `contrast`.
fn about_grey(value: f64, contrast: f64) -> f64 {
    (0.5 + (value - 0.5) * contrast).clamp(0.0, 1.0)
}

/// Tints a grey; a tint near 1 on each channel keeps the texture near neutral.
fn tinted(grey: f64, tint: [f64; 3]) -> [f64; 3] {
    tint.map(|channel| (grey * channel).clamp(0.0, 1.0))
}

/// Soft blotches under fine, long paper fibres.
fn parchment(u: f64, v: f64) -> [f64; 3] {
    let blotches = fractal(11, 3, 5, 0.55, u, v);
    let fibres = 0.6 * noise(12, 96, 6, u, v) + 0.4 * noise(13, 5, 90, u, v);
    let grain = noise(14, 128, 128, u, v);
    let grey = about_grey(0.6 * blotches + 0.25 * fibres + 0.15 * grain, 1.5);
    tinted(grey, [1.08, 1.0, 0.86])
}

/// Mown-looking blades: short streaks, taller than wide, over broad patches.
fn grass(u: f64, v: f64) -> [f64; 3] {
    let patches = fractal(21, 4, 3, 0.5, u, v);
    let blades = 0.5 * noise(22, 120, 24, u, v) + 0.5 * noise(23, 64, 12, u, v);
    let grey = about_grey(0.45 * patches + 0.55 * blades, 1.25);
    tinted(grey, [0.8, 1.06, 0.72])
}

/// Fine speckle laid in low ripples.
fn sand(u: f64, v: f64) -> [f64; 3] {
    let ripples = fractal(31, 3, 3, 0.5, u, v);
    let speckle = 0.5 * noise(32, 256, 256, u, v) + 0.5 * noise(33, 128, 128, u, v);
    let grey = about_grey(0.4 * ripples + 0.6 * speckle, 1.5);
    tinted(grey, [1.1, 1.0, 0.8])
}

/// Flagstones: lines where the nearest two of a set of points are about as close as each other.
fn stone(u: f64, v: f64) -> [f64; 3] {
    const CELLS: u32 = 7;
    let (px, py) = (u * f64::from(CELLS), v * f64::from(CELLS));
    let (cx, cy) = (px.floor() as i64, py.floor() as i64);
    let (mut nearest, mut second) = (f64::MAX, f64::MAX);
    let mut shade = 0.5;
    for dy in -1..=1 {
        for dx in -1..=1 {
            // The cell numbers wrap, so the points on one edge match those on the other.
            let wrap = i64::from(CELLS);
            let (wx, wy) = (
                (cx + dx).rem_euclid(wrap) as u32,
                (cy + dy).rem_euclid(wrap) as u32,
            );
            let point = (
                (cx + dx) as f64 + 0.15 + 0.7 * lattice(41, wx, wy),
                (cy + dy) as f64 + 0.15 + 0.7 * lattice(42, wx, wy),
            );
            let distance = ((px - point.0).powi(2) + (py - point.1).powi(2)).sqrt();
            if distance < nearest {
                second = nearest;
                nearest = distance;
                shade = lattice(43, wx, wy);
            } else if distance < second {
                second = distance;
            }
        }
    }
    let crack = ((second - nearest) / 0.12).clamp(0.0, 1.0);
    let pitting = fractal(44, 32, 3, 0.5, u, v);
    let grey =
        about_grey(0.5 + (shade - 0.5) * 0.25 + (pitting - 0.5) * 0.4, 1.2) * (0.55 + 0.45 * crack);
    // The cracks are darker, so the mean sits a little under grey; lift it back.
    tinted((grey * 1.18).min(1.0), [0.96, 0.98, 1.04])
}

/// The texture as `SIZE` by `SIZE` RGB bytes.
fn render(colour: fn(f64, f64) -> [f64; 3]) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((SIZE * SIZE * 3) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let rgb = colour(
                f64::from(x) / f64::from(SIZE),
                f64::from(y) / f64::from(SIZE),
            );
            pixels.extend(rgb.map(|channel| (channel * 255.0).round() as u8));
        }
    }
    pixels
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let out = args.next().map_or_else(
        || PathBuf::from("assets/packs/textures/textures"),
        PathBuf::from,
    );
    if args.next().is_some() {
        eprintln!("usage: textures [output-folder]");
        return ExitCode::FAILURE;
    }
    match write_all(&out) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn write_all(out: &Path) -> Result<(), String> {
    std::fs::create_dir_all(out).map_err(|err| format!("{}: {err}", out.display()))?;
    for (name, colour) in TEXTURES {
        let path = out.join(format!("{name}.png"));
        image::save_buffer(
            &path,
            &render(colour),
            SIZE,
            SIZE,
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|err| format!("{}: {err}", path.display()))?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The largest channel difference between the pixels at `a` and `b`.
    fn step(pixels: &[u8], a: (u32, u32), b: (u32, u32)) -> u8 {
        let at = |(x, y): (u32, u32)| ((y * SIZE + x) * 3) as usize;
        (0..3)
            .map(|c| pixels[at(a) + c].abs_diff(pixels[at(b) + c]))
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn every_texture_tiles_and_is_near_neutral() {
        for (name, colour) in TEXTURES {
            let pixels = render(colour);
            // The step across the wrap is no bigger than the steps inside the image.
            let inside = (0..SIZE)
                .flat_map(|i| {
                    [
                        step(&pixels, (100, i), (101, i)),
                        step(&pixels, (i, 100), (i, 101)),
                    ]
                })
                .max()
                .unwrap_or(0);
            let across = (0..SIZE)
                .flat_map(|i| {
                    [
                        step(&pixels, (SIZE - 1, i), (0, i)),
                        step(&pixels, (i, SIZE - 1), (i, 0)),
                    ]
                })
                .max()
                .unwrap_or(0);
            assert!(
                across <= inside.max(40),
                "{name}: the wrap has a seam ({across}; inside {inside})"
            );
            let mean = pixels.iter().map(|&b| f64::from(b)).sum::<f64>() / pixels.len() as f64;
            assert!(
                (90.0..170.0).contains(&mean),
                "{name}: mean {mean} is far from mid-grey"
            );
        }
    }

    #[test]
    fn the_output_does_not_change_between_runs() {
        for (_, colour) in TEXTURES {
            assert_eq!(render(colour), render(colour));
        }
    }
}
