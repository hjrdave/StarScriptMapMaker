//! Golden-image tests (ADR 0008).
//!
//! On a software adapter (lavapipe on Linux, WARP on Windows) renders are compared with PNGs in
//! `tests/golden/`, allowing a per-channel tolerance and a cap on the share of pixels that
//! differ. With only a hardware adapter, as on macOS, each test is a render smoke test instead.
//!
//! - `SSMAP_GOLDEN_UPDATE=1` rewrites the golden PNGs from the current renders.
//! - `SSMAP_REQUIRE_SOFTWARE_GPU=1` fails instead of falling back when there is no software
//!   adapter. CI sets it on Linux and Windows.

use std::path::PathBuf;

use ssmap_core::{Canvas, DistanceUnit, Scale};
use ssmap_render::{
    CanvasRenderer, CanvasScene, GpuCapabilities, Headless, RgbaImage, View, render_to_image,
};

/// Largest allowed difference in any channel before a pixel counts as differing.
const CHANNEL_TOLERANCE: u8 = 3;
/// Largest allowed share of differing pixels.
const MAX_DIFFERING: f64 = 0.005;

enum Gpu {
    /// Reproducible pixels: compare with the golden image.
    Software(Headless),
    /// Real GPU only: render and sanity-check, but don't compare.
    Hardware(Headless),
}

fn gpu() -> Option<Gpu> {
    match Headless::new(true) {
        Ok(gpu) => return Some(Gpu::Software(gpu)),
        Err(err) if env_flag("SSMAP_REQUIRE_SOFTWARE_GPU") => {
            panic!("SSMAP_REQUIRE_SOFTWARE_GPU is set but {err}");
        }
        Err(_) => {}
    }
    match Headless::new(false) {
        Ok(gpu) => Some(Gpu::Hardware(gpu)),
        Err(err) => {
            eprintln!("skipping render test: {err}");
            None
        }
    }
}

/// Set and non-empty, so CI can pass an empty value to mean off.
fn env_flag(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
}

fn render(headless: &Headless, scene: &CanvasScene, view: &View) -> RgbaImage {
    let capabilities = GpuCapabilities::query(&headless.adapter);
    eprintln!(
        "adapter: {} ({:?}, {:?}), MSAA {}x",
        capabilities.info.name,
        capabilities.info.backend,
        capabilities.info.device_type,
        capabilities.msaa_samples
    );
    let mut renderer = CanvasRenderer::new(&headless.device, capabilities);
    render_to_image(
        &headless.device,
        &headless.queue,
        &mut renderer,
        scene,
        view,
    )
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.png"))
}

fn assert_matches_golden(name: &str, actual: &RgbaImage) {
    let path = golden_path(name);
    if env_flag("SSMAP_GOLDEN_UPDATE") {
        image::save_buffer(
            &path,
            &actual.pixels,
            actual.width,
            actual.height,
            image::ExtendedColorType::Rgba8,
        )
        .expect("golden image writes");
        return;
    }
    let expected = image::open(&path)
        .unwrap_or_else(|err| panic!("{}: {err}; run with SSMAP_GOLDEN_UPDATE=1", path.display()))
        .into_rgba8();
    assert_eq!(
        (expected.width(), expected.height()),
        (actual.width, actual.height),
        "{name}: size differs from the golden image"
    );
    let differing = expected
        .as_raw()
        .chunks(4)
        .zip(actual.pixels.chunks(4))
        .filter(|(e, a)| {
            e.iter()
                .zip(a.iter())
                .any(|(e, a)| e.abs_diff(*a) > CHANNEL_TOLERANCE)
        })
        .count();
    let share = differing as f64 / f64::from(actual.width * actual.height);
    if share > MAX_DIFFERING {
        let failed = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.actual.png"));
        let _ = image::save_buffer(
            &failed,
            &actual.pixels,
            actual.width,
            actual.height,
            image::ExtendedColorType::Rgba8,
        );
        panic!(
            "{name}: {:.3}% of pixels differ (limit {:.3}%); actual render saved to {}",
            share * 100.0,
            MAX_DIFFERING * 100.0,
            failed.display()
        );
    }
}

fn pixel(image: &RgbaImage, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * image.width + x) * 4) as usize;
    image.pixels[i..i + 4].try_into().unwrap()
}

/// A small Canvas fitted with a margin, so the image shows the pasteboard, the Canvas edge
/// and the ocean texture.
fn backdrop_scene() -> (CanvasScene, View) {
    let scene = CanvasScene::new(&Canvas {
        width: 800,
        height: 600,
        scale: Scale {
            distance_per_unit: 1.0,
            unit: DistanceUnit::Miles,
        },
    });
    // 0.4 px per unit puts the Canvas edges exactly on pixel boundaries: 320x240 at (16, 12).
    let view = View {
        origin: [-40.0, -30.0],
        zoom: 0.4,
        size: [352, 264],
    };
    (scene, view)
}

#[test]
fn ocean_backdrop() {
    let Some(gpu) = gpu() else { return };
    let (scene, view) = backdrop_scene();
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let image = render(headless, &scene, &view);
    // Smoke checks that hold on any adapter: pasteboard outside, water inside.
    assert_eq!(pixel(&image, 4, 4), [0x1c, 0x1c, 0x1f, 0xff]);
    let inside = pixel(&image, 176, 132);
    assert!(
        inside[2] > inside[0],
        "the Canvas is water-coloured: {inside:?}"
    );
    if let Gpu::Software(_) = gpu {
        assert_matches_golden("ocean_backdrop", &image);
    }
}
