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

use ssmap_core::asset::{ArtFormat, PackSource, read_pack};
use ssmap_core::{Canvas, DistanceUnit, Scale};
use ssmap_render::atlas::{Atlas, AtlasEntry, MIN_PAGE_SIZE};
use ssmap_render::raster::{Art, Raster};
use ssmap_render::{
    CanvasRenderer, CanvasScene, GpuCapabilities, Headless, RgbaImage, View, read_texture,
    render_to_image,
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
    let scene = CanvasScene::new(&Canvas::new(800, 600, Scale::new(1.0, DistanceUnit::Miles)));
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

#[test]
fn whole_canvas_preview_shows_no_pasteboard() {
    let Some(gpu) = gpu() else { return };
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let scene = CanvasScene::new(&Canvas::new(
        4096,
        3072,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    let view = View::whole_canvas(scene.canvas_size, 512);
    let image = render(headless, &scene, &view);
    assert_eq!([image.width, image.height], [512, 384]);
    for (x, y) in [(0, 0), (511, 0), (0, 383), (511, 383)] {
        let corner = pixel(&image, x, y);
        assert!(
            corner[2] > corner[0],
            "corner ({x}, {y}) is water, not pasteboard: {corner:?}"
        );
    }
}

/// Fills a 2048 page with the test pack's art and Silhouettes at many sizes, until the atlas
/// spills onto a second page, and compares the first page with its golden image. The page
/// holds premultiplied pixels, so the golden PNG shows them as stored, not as drawn.
#[test]
fn atlas_page() {
    let Some(gpu) = gpu() else { return };
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let pack_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../core/tests/packs/test-pack");
    let pack = read_pack(&pack_dir, PackSource::BuiltIn).expect("the test pack loads");
    let mut arts = Vec::new();
    for asset in &pack.assets {
        arts.push(Art::load(&pack.path(&asset.file)).unwrap());
        arts.push(Art::load(&pack.path(asset.silhouette.as_ref().unwrap())).unwrap());
    }

    let mut atlas = Atlas::new(MIN_PAGE_SIZE);
    let mut placed: Vec<(AtlasEntry, Raster)> = Vec::new();
    // Rounds of large to small SVG art, until a round spills onto a second page. Small entries late
    // in that round backfill the first page.
    let widths = [384, 256, 192, 128, 96, 64, 48, 32, 24, 16];
    for round in 0.. {
        for width in widths {
            for art in &arts {
                // PNGs, such as the Silhouettes, have one native size: place each once.
                if art.format() == ArtFormat::Png && (round > 0 || width != widths[0]) {
                    continue;
                }
                // Vary sizes a little between rounds so shelves don't line up exactly.
                let raster = art.rasterise_fit(width + round * 3);
                let entry = atlas
                    .insert(&headless.device, &headless.queue, &raster)
                    .unwrap();
                placed.push((entry, raster));
            }
        }
        if atlas.page_count() > 1 {
            break;
        }
    }
    placed.retain(|(entry, _)| entry.page == 0);
    assert!(atlas.page_count() >= 2);
    eprintln!("{} pages", atlas.page_count());

    let page = read_texture(&headless.device, &headless.queue, atlas.page_texture(0));
    assert_eq!([page.width, page.height], [MIN_PAGE_SIZE, MIN_PAGE_SIZE]);
    // Uploads are byte copies, so every entry reads back exactly, on any adapter.
    let mut used = 0u64;
    for (entry, raster) in &placed {
        used += u64::from(entry.rect.width * entry.rect.height);
        for y in 0..raster.height {
            let start = (((entry.rect.y + y) * page.width + entry.rect.x) * 4) as usize;
            let row = (y * raster.width * 4) as usize;
            assert_eq!(
                &page.pixels[start..start + (raster.width * 4) as usize],
                &raster.pixels[row..row + (raster.width * 4) as usize],
                "row {y} of {entry:?}"
            );
        }
    }
    let fill = used as f64 / f64::from(MIN_PAGE_SIZE * MIN_PAGE_SIZE);
    eprintln!(
        "{} entries fill {:.1}% of the page",
        placed.len(),
        fill * 100.0
    );
    assert!(fill > 0.7, "the page is only {:.1}% full", fill * 100.0);
    if let Gpu::Software(_) = gpu {
        assert_matches_golden("atlas_page", &page);
    }
}
