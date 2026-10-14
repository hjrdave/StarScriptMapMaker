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

use ssmap_core::{Align, Canvas, Color, Command, DistanceUnit, FontFace, Id, Label, Map, Scale};
use ssmap_render::{
    CanvasRenderer, CanvasScene, Fonts, GpuCapabilities, Headless, MapScene, RgbaImage, View,
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

/// A Map with one Layer holding `labels`, rendered whole at half size.
fn render_labels(headless: &Headless, size: [u32; 2], labels: Vec<Label>) -> RgbaImage {
    let mut map = Map::new(Canvas::new(
        size[0],
        size[1],
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Labels".into(),
    })
    .unwrap();
    for label in labels {
        map.execute(&Command::CreateLabel {
            id: Id::new(),
            parent: layer,
            index: None,
            label: Box::new(label),
        })
        .unwrap();
    }
    let mut scene = MapScene::new(Fonts::bundled());
    let canvas = CanvasScene::new(&map.canvas).with_content(scene.content(&map));
    let view = View::whole_canvas(canvas.canvas_size, size[0].max(size[1]) / 2);
    render(headless, &canvas, &view)
}

fn label(text: &str, family: &str, position: [f64; 2]) -> Label {
    Label {
        text: text.into(),
        position,
        font: FontFace {
            family: family.into(),
            ..FontFace::default()
        },
        size: 64.0,
        ..Label::default()
    }
}

/// Multi-line, letter-spaced, all-caps and haloed Labels, one in each quarter.
#[test]
fn labels() {
    let Some(gpu) = gpu() else { return };
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let multi_line = Label {
        align: Align::Left,
        line_height: 1.1,
        ..label(
            "The Sundered\nIsles of\nWestmarch",
            "IM Fell Double Pica",
            [40.0, 200.0],
        )
    };
    let spaced = Label {
        letter_spacing: 0.35,
        rotation: -12.0,
        ..label("Spaced Out", "Almendra", [900.0, 200.0])
    };
    let all_caps = Label {
        all_caps: true,
        letter_spacing: 0.1,
        font: FontFace {
            family: "Cinzel".into(),
            weight: 700,
            ..FontFace::default()
        },
        ..label("Kingdom of Thule", "Cinzel", [520.0, 640.0])
    };
    let haloed = Label {
        fill: Color::rgb(0x6b, 0x16, 0x10),
        halo: ssmap_core::Halo {
            width: 5.0,
            color: Color::rgb(0xf4, 0xec, 0xd8),
            ..Default::default()
        },
        align: Align::Right,
        ..label("Haloed\nRight", "Uncial Antiqua", [1160.0, 560.0])
    };
    let image = render_labels(
        headless,
        [1200, 800],
        vec![multi_line, spaced, all_caps, haloed],
    );
    if let Gpu::Software(_) = gpu {
        assert_matches_golden("labels", &image);
    }
}

/// "Ærendál" in every display family, then Greek that only Noto Serif has, with no
/// missing-glyph boxes.
#[test]
fn label_fallback() {
    let Some(gpu) = gpu() else { return };
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let mut labels: Vec<Label> = ssmap_fonts::LABEL_FAMILIES
        .iter()
        .enumerate()
        .map(|(i, family)| label("Ærendál", family, [300.0, 70.0 + 110.0 * i as f64]))
        .collect();
    labels.push(Label {
        size: 56.0,
        ..label("Ærendál\nΩμέγα", "Uncial Antiqua", [880.0, 300.0])
    });
    let image = render_labels(headless, [1200, 600], labels);
    if let Gpu::Software(_) = gpu {
        assert_matches_golden("label_fallback", &image);
    }
}

/// A translucent Label looks the same inside its letters with or without a halo: the halo
/// sits only around the fill, and its own overlaps don't build up.
#[test]
fn halos_never_show_through_translucent_fill() {
    let Some(gpu) = gpu() else { return };
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let stem = Label {
        opacity: 0.5,
        size: 400.0,
        ..label("I", "Noto Serif", [200.0, 200.0])
    };
    let haloed = Label {
        halo: ssmap_core::Halo {
            width: 12.0,
            color: Color::rgba(0xff, 0xff, 0xff, 0x80),
            ..Default::default()
        },
        ..stem.clone()
    };
    let plain = render_labels(headless, [400, 400], vec![stem]);
    let with_halo = render_labels(headless, [400, 400], vec![haloed]);
    // The centre of the stem, at half size.
    let (inside, inside_haloed) = (pixel(&plain, 100, 100), pixel(&with_halo, 100, 100));
    for (a, b) in inside.iter().zip(inside_haloed.iter()) {
        assert!(a.abs_diff(*b) <= 2, "{inside:?} vs {inside_haloed:?}");
    }
    // Just outside the stem, the halo shows.
    let beside = |image: &RgbaImage| (60..100).map(|x| pixel(image, x, 100)[0]).max().unwrap();
    assert!(beside(&with_halo) > beside(&plain) + 40);
}
