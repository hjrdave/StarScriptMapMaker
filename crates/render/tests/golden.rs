//! Golden-image tests (ADR 0008).
//!
//! On a software adapter (lavapipe on Linux, WARP on Windows) renders are compared with PNGs in
//! `tests/golden/`, allowing a per-channel tolerance and a cap on the share of pixels that
//! differ. With only a hardware adapter, as on macOS, each test is a render smoke test instead.
//!
//! - `SSMAP_GOLDEN_UPDATE=1` rewrites the golden PNGs from the current renders.
//! - `SSMAP_REQUIRE_SOFTWARE_GPU=1` fails instead of falling back when there is no software
//!   adapter. CI sets it on Linux and Windows.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use std::path::PathBuf;

use ssmap_core::asset::{ArtFormat, PackSource, read_pack};
use ssmap_core::{
    Align, Canvas, Color, Command, DistanceUnit, FontFace, Id, Label, LabelStyle, LabelStyles,
    Lake, Landmass, Map, Roughness, RoughnessCache, Scale, ShoreKind, ShoreRipples, ShoreStyle,
};
use ssmap_render::atlas::{Atlas, AtlasEntry, MIN_PAGE_SIZE};
use ssmap_render::raster::{Art, Raster};
use ssmap_render::{
    CanvasRenderer, CanvasScene, Fonts, GpuCapabilities, Headless, MapScene, RgbaImage, View,
    read_texture, render_to_image,
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
    .expect("the readback succeeds")
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

/// A small Canvas fitted with a margin, so the image shows the off-Canvas, the Canvas edge
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
    // Smoke checks that hold on any adapter: off-Canvas outside, water inside.
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
fn flat_background_is_one_colour_and_follows_the_map() {
    let Some(gpu) = gpu() else { return };
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let mut canvas = Canvas::new(800, 600, Scale::new(1.0, DistanceUnit::Miles));
    canvas.background.water = ssmap_core::Color::rgb(0xe0, 0xc8, 0x90);
    canvas.background.texture.strength = 0.0;
    let scene = CanvasScene::new(&canvas);
    let (_, view) = backdrop_scene();
    let image = render(headless, &scene, &view);
    for (x, y) in [(40, 40), (176, 132), (300, 200)] {
        assert_eq!(
            pixel(&image, x, y),
            [0xe0, 0xc8, 0x90, 0xff],
            "at ({x}, {y})"
        );
    }
}

#[test]
fn whole_canvas_preview_shows_no_off_canvas() {
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
            "corner ({x}, {y}) is water, not off-Canvas: {corner:?}"
        );
    }
}

/// The Map built by `core`'s `land_and_water` Command script: an island with a hole and an
/// islet, plus a Lake listed beneath the island in the tree.
/// Everything `map` draws, as the editor and export build it.
fn scene_of(map: &Map) -> CanvasScene {
    let content = MapScene::new(Fonts::bundled()).content(map, &mut RoughnessCache::new());
    CanvasScene::new(&map.canvas).with_content(content)
}

fn land_and_water() -> Map {
    let text = include_str!("../../core/tests/scripts/land_and_water.json");
    let script: serde_json::Value = serde_json::from_str(text).expect("the script is JSON");
    let mut map = Map::new(Canvas::new(
        1000,
        1000,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    for step in script["steps"].as_array().expect("steps") {
        let command: Command =
            serde_json::from_value(step["step"].clone()).expect("every step is a Command");
        map.execute(&command).expect("the script's Commands apply");
    }
    // These scenes predate Shore style: draw them as bare fills.
    for kind in [ShoreKind::Landmass, ShoreKind::Lake] {
        map.execute(&Command::SetDefaultShore {
            kind,
            shore: ShoreStyle::plain(),
        })
        .expect("a valid Shore style");
    }
    map
}

/// 0.4 px per unit: the 1000-unit Canvas is 400 px at (16, 16).
fn land_and_water_view() -> View {
    View {
        origin: [-40.0, -40.0],
        zoom: 0.4,
        size: [432, 432],
    }
}

#[test]
fn island_with_a_hole_and_a_lake() {
    let Some(gpu) = gpu() else { return };
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let scene = scene_of(&land_and_water());
    let view = land_and_water_view();
    let image = render(headless, &scene, &view);
    let at = |x: f64, y: f64| pixel(&image, (16.0 + x * 0.4) as u32, (16.0 + y * 0.4) as u32);
    let near = |actual: [u8; 4], expected: ssmap_core::Color| {
        let expected = [expected.r, expected.g, expected.b, expected.a];
        actual.iter().zip(expected).all(|(a, e)| a.abs_diff(e) <= 2)
    };
    assert!(near(at(450.0, 650.0), Landmass::DEFAULT_FILL), "land");
    assert!(
        near(at(600.0, 400.0), Lake::DEFAULT_FILL),
        "the Lake draws above the land"
    );
    assert!(near(at(860.0, 850.0), Landmass::DEFAULT_FILL), "the islet");
    for (x, y) in [(320.0, 560.0), (50.0, 50.0)] {
        let water = at(x, y);
        assert!(water[2] > water[0], "ocean at ({x}, {y}): {water:?}");
    }
    if let Gpu::Software(_) = gpu {
        assert_matches_golden("island_with_a_hole_and_a_lake", &image);
    }
}

/// A frame drawn mid-gesture skips ripples and inner shading; the redraw once the gesture
/// stops, on the same renderer, is the fully rendered frame.
#[test]
fn redraw_after_a_gesture_is_fully_rendered() {
    use ssmap_render::Quality;

    let Some(gpu) = gpu() else { return };
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let mut map = land_and_water();
    for (kind, shore) in [
        (ShoreKind::Landmass, ShoreStyle::landmass_default()),
        (ShoreKind::Lake, ShoreStyle::lake_default()),
    ] {
        map.execute(&Command::SetDefaultShore { kind, shore })
            .expect("a valid Shore style");
    }
    let full = scene_of(&map);
    let gesture = full.clone().with_quality(Quality::Interactive);
    let mut view = land_and_water_view();

    let capabilities = GpuCapabilities::query(&headless.adapter);
    let mut renderer = CanvasRenderer::new(&headless.device, capabilities);
    let mut draw = |scene: &CanvasScene, view: &View| {
        render_to_image(
            &headless.device,
            &headless.queue,
            &mut renderer,
            scene,
            view,
        )
        .expect("the readback succeeds")
    };
    let before = draw(&full, &view);
    // Pan and zoom, drawing at gesture quality, then come back to where the view started.
    for step in 0..4 {
        view.pan([7.0, -3.0]);
        view.zoom_at([200.0, 200.0], if step % 2 == 0 { 1.2 } else { 1.0 / 1.2 });
        draw(&gesture, &view);
    }
    let start = land_and_water_view();
    let during = draw(&gesture, &start);
    let after = draw(&full, &start);
    assert_ne!(
        during.pixels, after.pixels,
        "the gesture frame has no effects"
    );
    assert_eq!(before.pixels, after.pixels, "the redraw is the full frame");
    if let Gpu::Software(_) = gpu {
        assert_matches_golden("shore_effects", &after);
    }
}

#[test]
fn rough_coastlines() {
    let Some(gpu) = gpu() else { return };
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let mut map = land_and_water();
    map.execute(&Command::SetDefaultRoughness {
        roughness: Roughness::new(14.0, 60.0, 3),
    })
    .expect("valid Roughness");
    let scene = scene_of(&map);
    let image = render(headless, &scene, &land_and_water_view());
    let smooth = render(
        headless,
        &scene_of(&land_and_water()),
        &land_and_water_view(),
    );
    assert_ne!(image.pixels, smooth.pixels, "Roughness changes the coast");
    if let Gpu::Software(_) = gpu {
        assert_matches_golden("rough_coastlines", &image);
    }
}

/// Two islands a few ripple-widths apart, in one Layer, with ripples but no outline or shading.
/// Ripples draw beneath every fill of the Layer, so neither island's ripples may cover the
/// other island: every pixel inside either island is the land colour.
#[test]
fn ripples_of_nearby_islands_never_cover_land() {
    let Some(gpu) = gpu() else { return };
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let mut map = Map::new(Canvas::new(400, 200, Scale::new(1.0, DistanceUnit::Miles)));
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Land".into(),
    })
    .expect("a Layer");
    let square = |min: [f64; 2], max: [f64; 2]| {
        ssmap_core::MultiPolygon(vec![ssmap_core::Polygon {
            exterior: vec![min, [max[0], min[1]], max, [min[0], max[1]]],
            holes: Vec::new(),
        }])
    };
    for geometry in [
        square([40.0, 50.0], [160.0, 150.0]),
        square([180.0, 50.0], [300.0, 150.0]),
    ] {
        map.execute(&Command::CreateLandmass {
            id: Id::new(),
            parent: layer,
            index: None,
            geometry,
            fill: None,
        })
        .expect("a Landmass");
    }
    let ripples = ShoreRipples {
        count: 4,
        spacing: 10.0,
        width: 3.0,
        color: ssmap_core::Color::rgb(255, 255, 255),
        fade: 0.0,
    };
    map.execute(&Command::SetDefaultShore {
        kind: ShoreKind::Landmass,
        shore: ShoreStyle {
            ripples,
            ..ShoreStyle::plain()
        },
    })
    .expect("a valid Shore style");

    let scene = scene_of(&map);
    let view = View {
        origin: [0.0, 0.0],
        zoom: 2.0,
        size: [800, 400],
    };
    let image = render(headless, &scene, &view);
    let land = Landmass::DEFAULT_FILL;
    let mut ripple_pixels = 0;
    for y in (60..140).step_by(3) {
        for x in (50..290).step_by(3) {
            let inside = (40..=160).contains(&x) || (180..=300).contains(&x);
            let px = pixel(&image, x * 2, y * 2);
            if inside && !(150..=190).contains(&x) {
                assert!(
                    px[0].abs_diff(land.r) <= 2 && px[1].abs_diff(land.g) <= 2,
                    "land at ({x}, {y}) is {px:?}"
                );
            } else if !inside && px[0] > 200 && px[1] > 200 && px[2] > 200 {
                ripple_pixels += 1;
            }
        }
    }
    // The gap is 20 units wide: ripples from both islands show in it.
    assert!(ripple_pixels > 0, "ripples are drawn over the water");
    let plain = {
        let mut map = map.clone();
        map.execute(&Command::SetDefaultShore {
            kind: ShoreKind::Landmass,
            shore: ShoreStyle::plain(),
        })
        .expect("valid");
        render(headless, &scene_of(&map), &view)
    };
    assert_ne!(image.pixels, plain.pixels, "ripples change the picture");
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

    let page = read_texture(&headless.device, &headless.queue, atlas.page_texture(0))
        .expect("the readback succeeds");
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
    let canvas =
        CanvasScene::new(&map.canvas).with_content(scene.content(&map, &mut RoughnessCache::new()));
    let view = View::whole_canvas(canvas.canvas_size, size[0].max(size[1]) / 2);
    render(headless, &canvas, &view)
}

fn label(text: &str, family: &str, position: [f64; 2]) -> Label {
    Label {
        text: text.into(),
        position,
        font: Some(FontFace {
            family: family.into(),
            ..FontFace::default()
        }),
        size: Some(64.0),
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
        line_height: Some(1.1),
        ..label(
            "The Sundered\nIsles of\nWestmarch",
            "IM Fell Double Pica",
            [40.0, 200.0],
        )
    };
    let spaced = Label {
        letter_spacing: Some(0.35),
        rotation: -12.0,
        ..label("Spaced Out", "Almendra", [900.0, 200.0])
    };
    let all_caps = Label {
        all_caps: Some(true),
        letter_spacing: Some(0.1),
        font: Some(FontFace {
            family: "Cinzel".into(),
            weight: 700,
            ..FontFace::default()
        }),
        ..label("Kingdom of Thule", "Cinzel", [520.0, 640.0])
    };
    let haloed = Label {
        fill: Some(Color::rgb(0x6b, 0x16, 0x10)),
        halo: Some(ssmap_core::Halo {
            width: 5.0,
            color: Color::rgb(0xf4, 0xec, 0xd8),
            ..Default::default()
        }),
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
        size: Some(56.0),
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
        opacity: Some(0.5),
        size: Some(400.0),
        ..label("I", "Noto Serif", [200.0, 200.0])
    };
    let haloed = Label {
        halo: Some(ssmap_core::Halo {
            width: 12.0,
            color: Color::rgba(0xff, 0xff, 0xff, 0x80),
            ..Default::default()
        }),
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

/// Deleting a Label style in use copies its values into the Labels as overrides, so the Map
/// looks exactly the same afterwards.
#[test]
fn deleting_a_style_in_use_changes_nothing_visually() {
    let Some(gpu) = gpu() else { return };
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let mut map = Map::new(Canvas::new(1200, 600, Scale::new(1.0, DistanceUnit::Miles)))
        .with_label_styles(LabelStyles::starter());
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Labels".into(),
    })
    .unwrap();
    let styles: Vec<Id> = map.label_styles().iter().map(|s| s.id).collect();
    for (i, style) in styles.iter().enumerate() {
        map.execute(&Command::CreateLabel {
            id: Id::new(),
            parent: layer,
            index: None,
            label: Box::new(Label {
                text: "Thule".into(),
                position: [600.0, 70.0 + 110.0 * i as f64],
                style: *style,
                ..Label::default()
            }),
        })
        .unwrap();
    }
    let draw = |map: &Map| {
        let mut scene = MapScene::new(Fonts::bundled());
        let canvas = CanvasScene::new(&map.canvas)
            .with_content(scene.content(map, &mut RoughnessCache::new()));
        let view = View::whole_canvas(canvas.canvas_size, 600);
        render(headless, &canvas, &view)
    };
    let before = draw(&map);
    for style in styles
        .into_iter()
        .filter(|id| *id != LabelStyle::DEFAULT_ID)
    {
        map.execute(&Command::DeleteLabelStyle { id: style })
            .unwrap();
    }
    let after = draw(&map);
    assert_eq!(before.pixels, after.pixels);
    if let Gpu::Software(_) = gpu {
        assert_matches_golden("label_styles_deleted", &after);
    }
}

/// A store over the core crate's test pack.
fn test_sprites() -> ssmap_render::SpriteStore {
    use ssmap_core::asset::{PackLibrary, PackRoots};
    let roots = PackRoots {
        built_in: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../core/tests/packs"),
        user: None,
    };
    let (library, problems) = PackLibrary::load(&roots);
    assert!(problems.is_empty(), "{problems:?}");
    ssmap_render::SpriteStore::synchronous(std::sync::Arc::new(library))
}

fn stamp(asset: &str, position: [f64; 2]) -> ssmap_core::Stamp {
    ssmap_core::Stamp::new(asset.parse().expect("a valid reference"), position)
}

fn render_stamps(headless: &Headless, stamps: Vec<ssmap_core::Stamp>) -> RgbaImage {
    render_stamps_sorted(headless, stamps, false)
}

fn render_stamps_sorted(
    headless: &Headless,
    stamps: Vec<ssmap_core::Stamp>,
    depth_sort: bool,
) -> RgbaImage {
    let mut map = Map::new(Canvas::new(480, 320, Scale::new(1.0, DistanceUnit::Miles)));
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Assets".into(),
    })
    .unwrap();
    if depth_sort {
        map.execute(&Command::SetDepthSort {
            id: layer,
            depth_sort,
        })
        .unwrap();
    }
    for stamp in stamps {
        map.execute(&Command::CreateStamp {
            id: Id::new(),
            parent: layer,
            index: None,
            stamp: Box::new(stamp),
        })
        .unwrap();
    }
    let canvas = scene_of(&map);
    let view = View::whole_canvas(canvas.canvas_size, 960);
    let capabilities = GpuCapabilities::query(&headless.adapter);
    let mut renderer = CanvasRenderer::new(&headless.device, capabilities);
    renderer.set_sprites(test_sprites());
    render_to_image(
        &headless.device,
        &headless.queue,
        &mut renderer,
        &canvas,
        &view,
    )
    .expect("the readback succeeds")
}

#[test]
fn stamps() {
    let Some(gpu) = gpu() else { return };
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let mut tinted = stamp("test-pack/mountain-2", [380.0, 150.0]);
    tinted.tint = Some(Color::rgb(0xa0, 0x30, 0x20));
    tinted.fill = Some(Color::rgb(0xe8, 0xd8, 0xa8));
    let mut flipped = stamp("test-pack/castle", [90.0, 270.0]);
    flipped.flip = true;
    flipped.scale = 1.5;
    let mut turned = stamp("test-pack/tree", [220.0, 280.0]);
    turned.rotation = 30.0;
    turned.scale = 2.0;
    turned.opacity = 0.6;
    let stamps = vec![
        // The nearer mountain hides the lines of the one behind it.
        stamp("test-pack/mountain-1", [110.0, 120.0]),
        stamp("test-pack/mountain-1", [150.0, 150.0]),
        stamp("test-pack/mountain-2", [250.0, 150.0]),
        tinted,
        flipped,
        turned,
    ];
    let image = render_stamps(headless, stamps);
    if let Gpu::Software(_) = gpu {
        assert_matches_golden("stamps", &image);
    }
}

/// A mountain range placed in random order draws back to front under Depth sort, exactly as
/// the same range placed back to front draws without it.
#[test]
fn depth_sorted_mountain_range() {
    let Some(gpu) = gpu() else { return };
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let range = |order: &[usize]| -> Vec<ssmap_core::Stamp> {
        let places = [
            [170.0, 110.0],
            [192.0, 124.0],
            [214.0, 138.0],
            [236.0, 152.0],
            [258.0, 166.0],
            [280.0, 180.0],
        ];
        order
            .iter()
            .map(|&i| stamp("test-pack/mountain-1", places[i]))
            .collect()
    };
    let shuffled = render_stamps_sorted(headless, range(&[3, 0, 5, 2, 4, 1]), true);
    let in_order = render_stamps(headless, range(&[0, 1, 2, 3, 4, 5]));
    let unsorted = render_stamps(headless, range(&[3, 0, 5, 2, 4, 1]));
    assert!(
        shuffled.pixels == in_order.pixels,
        "sorted draws back to front"
    );
    assert!(
        shuffled.pixels != unsorted.pixels,
        "placement order matters without Depth sort"
    );
    if let Gpu::Software(_) = gpu {
        assert_matches_golden("depth_sorted_mountain_range", &shuffled);
    }
}

/// A square of land with a Lake in it, and a Layer of Stamps over both, optionally clipped to
/// the land.
fn render_clipped(headless: &Headless, clip: bool, stamps: &[ssmap_core::Stamp]) -> RgbaImage {
    use ssmap_core::{MultiPolygon, Polygon};
    let square = |min: [f64; 2], max: [f64; 2]| {
        MultiPolygon(vec![Polygon {
            exterior: vec![min, [max[0], min[1]], max, [min[0], max[1]]],
            holes: vec![],
        }])
    };
    let new_layer = |map: &mut Map, name: &str| {
        let id = Id::new();
        map.execute(&Command::CreateLayer {
            id,
            parent: None,
            index: None,
            name: name.into(),
        })
        .unwrap();
        id
    };
    let mut map = Map::new(Canvas::new(480, 320, Scale::new(1.0, DistanceUnit::Miles)));
    let land_layer = new_layer(&mut map, "Land");
    let land = Id::new();
    map.execute(&Command::CreateLandmass {
        id: land,
        parent: land_layer,
        index: None,
        geometry: square([120.0, 60.0], [360.0, 260.0]),
        fill: None,
    })
    .unwrap();
    map.execute(&Command::CreateLake {
        id: Id::new(),
        parent: land_layer,
        index: None,
        geometry: square([220.0, 110.0], [300.0, 200.0]),
        fill: None,
    })
    .unwrap();
    let art = new_layer(&mut map, "Art");
    if clip {
        map.execute(&Command::SetClip {
            id: art,
            target: Some(land),
        })
        .unwrap();
    }
    for stamp in stamps {
        map.execute(&Command::CreateStamp {
            id: Id::new(),
            parent: art,
            index: None,
            stamp: Box::new(stamp.clone()),
        })
        .unwrap();
    }
    for kind in [ShoreKind::Landmass, ShoreKind::Lake] {
        map.execute(&Command::SetDefaultShore {
            kind,
            shore: ShoreStyle::plain(),
        })
        .unwrap();
    }
    let canvas = scene_of(&map);
    let view = View::whole_canvas(canvas.canvas_size, 960);
    let mut renderer =
        CanvasRenderer::new(&headless.device, GpuCapabilities::query(&headless.adapter));
    renderer.set_sprites(test_sprites());
    render_to_image(
        &headless.device,
        &headless.queue,
        &mut renderer,
        &canvas,
        &view,
    )
    .expect("the readback succeeds")
}

/// Stamps on a clipped Layer are cut at the coast and absent inside a Lake.
#[test]
fn clipped_stamps_stop_at_the_coast_and_the_lake() {
    let Some(gpu) = gpu() else { return };
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let big = |asset: &str, at: [f64; 2]| {
        let mut s = stamp(asset, at);
        s.scale = 2.5;
        s
    };
    let stamps = [
        // On the west coast, half in the sea.
        big("test-pack/mountain-1", [120.0, 160.0]),
        // Wholly in the sea.
        big("test-pack/castle", [50.0, 100.0]),
        // On the Lake.
        big("test-pack/mountain-2", [260.0, 150.0]),
        // On land.
        big("test-pack/tree", [180.0, 230.0]),
    ];
    let clipped = render_clipped(headless, true, &stamps);
    let unclipped = render_clipped(headless, false, &stamps);
    let bare = render_clipped(headless, true, &[]);
    // The image is 2 px per unit.
    let region_equal = |a: &RgbaImage, b: &RgbaImage, min: [u32; 2], max: [u32; 2]| {
        (min[1] * 2..max[1] * 2)
            .all(|y| (min[0] * 2..max[0] * 2).all(|x| pixel(a, x, y) == pixel(b, x, y)))
    };
    assert!(
        !region_equal(&unclipped, &bare, [0, 0], [118, 320]),
        "the Stamps do draw in the sea when not clipped"
    );
    assert!(
        region_equal(&clipped, &bare, [0, 0], [118, 320]),
        "clipped Stamps are absent from the sea"
    );
    assert!(
        !region_equal(&unclipped, &bare, [222, 112], [298, 198]),
        "the Stamp over the Lake draws when not clipped"
    );
    assert!(
        region_equal(&clipped, &bare, [222, 112], [298, 198]),
        "clipped Stamps are absent inside the Lake"
    );
    assert!(
        !region_equal(&clipped, &bare, [125, 62], [215, 258]),
        "clipped Stamps still draw on land"
    );
    if let Gpu::Software(_) = gpu {
        assert_matches_golden("clipped_stamps", &clipped);
    }
}

/// Stamps of an Asset no pack has draw a placeholder box where they sit, and nothing else changes.
#[test]
fn missing_art_draws_placeholder() {
    let Some(gpu) = gpu() else { return };
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let empty = render_stamps(headless, Vec::new());
    let ghost = render_stamps(headless, vec![stamp("no-such/asset", [240.0, 160.0])]);
    assert_ne!(empty.pixels, ghost.pixels, "the placeholder box is drawn");
    let (width, height) = (empty.width, empty.height);
    let differing: Vec<(u32, u32)> = (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .filter(|&(x, y)| pixel(&empty, x, y) != pixel(&ghost, x, y))
        .collect();
    assert!(
        differing
            .iter()
            .all(|&(x, y)| x.abs_diff(width / 2) < width / 4 && y.abs_diff(height / 2) < height / 4),
        "the placeholder is drawn around the Stamp, not elsewhere"
    );
    assert_eq!(pixel(&empty, 0, 0), pixel(&ghost, 0, 0));
}

/// 10k Stamps: renders panned and zoomed frames and reports the time per frame and per pick.
/// The bound is loose (it must hold on a software adapter); the numbers are in the output of
/// `cargo test -p ssmap-render --test golden ten_thousand_stamps -- --nocapture`.
#[test]
fn ten_thousand_stamps() {
    use std::time::Instant;

    let Some(gpu) = gpu() else { return };
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let mut map = Map::new(Canvas::new(
        4000,
        2500,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Assets".into(),
    })
    .unwrap();
    let assets = ["test-pack/mountain-1", "test-pack/castle", "test-pack/tree"];
    for i in 0..10_000usize {
        let mut s = stamp(
            assets[i % assets.len()],
            [
                (i % 100) as f64 * 40.0 + 20.0,
                (i / 100) as f64 * 25.0 + 12.0,
            ],
        );
        s.rotation = (i % 7) as f64 * 5.0;
        s.flip = i % 2 == 0;
        map.execute(&Command::CreateStamp {
            id: Id::new(),
            parent: layer,
            index: None,
            stamp: Box::new(s),
        })
        .unwrap();
    }

    let started = Instant::now();
    let canvas = scene_of(&map);
    eprintln!("10k Stamps: scene built in {:?}", started.elapsed());
    let capabilities = GpuCapabilities::query(&headless.adapter);
    let mut renderer = CanvasRenderer::new(&headless.device, capabilities);
    renderer.set_sprites(test_sprites());
    let mut view = View::whole_canvas(canvas.canvas_size, 960);
    let frames = 8;
    let started = Instant::now();
    for frame in 0..frames {
        view.pan([12.0, 6.0]);
        if frame % 2 == 0 {
            view.zoom_at([480.0, 300.0], 1.1);
        }
        render_to_image(
            &headless.device,
            &headless.queue,
            &mut renderer,
            &canvas,
            &view,
        )
        .expect("the readback succeeds");
    }
    let per_frame = started.elapsed() / frames;
    eprintln!("10k Stamps: {per_frame:?} per frame (including readback)");
    assert!(per_frame.as_secs_f64() < 5.0, "{per_frame:?} per frame");

    let shape = ssmap_core::SpriteShape {
        size: [32.0, 32.0],
        anchor: [0.5, 1.0],
    };
    let started = Instant::now();
    let index = ssmap_core::StampIndex::build(&map, |_| Some(shape));
    eprintln!("10k Stamps: index built in {:?}", started.elapsed());
    let started = Instant::now();
    let picks = 1000;
    for i in 0..picks {
        index.pick([(i * 4) as f64, (i * 2) as f64], |_, _| true);
    }
    eprintln!("10k Stamps: {:?} per pick", started.elapsed() / picks);
}

/// A Map with several rough islands and a few hundred Stamps, small enough to render a dozen
/// times in CI but with enough Objects to see them arrive one by one.
fn progressive_map() -> Map {
    use ssmap_core::{MultiPolygon, Polygon};

    let mut map = Map::new(Canvas::new(1200, 800, Scale::new(1.0, DistanceUnit::Miles)));
    map.execute(&Command::SetDefaultRoughness {
        roughness: Roughness::new(6.0, 30.0, 5),
    })
    .unwrap();
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Land".into(),
    })
    .unwrap();
    for i in 0..8 {
        let x = 150.0 + (i % 4) as f64 * 290.0;
        let y = 200.0 + (i / 4) as f64 * 400.0;
        map.execute(&Command::CreateLandmass {
            id: Id::new(),
            parent: layer,
            index: None,
            geometry: MultiPolygon(vec![Polygon {
                exterior: vec![
                    [x, y],
                    [x + 220.0, y + 20.0],
                    [x + 200.0, y + 240.0],
                    [x, y + 200.0],
                ],
                holes: Vec::new(),
            }]),
            fill: None,
        })
        .unwrap();
    }
    let assets = ["test-pack/mountain-1", "test-pack/castle", "test-pack/tree"];
    for i in 0..300usize {
        map.execute(&Command::CreateStamp {
            id: Id::new(),
            parent: layer,
            index: None,
            stamp: Box::new(stamp(
                assets[i % assets.len()],
                [40.0 + (i % 30) as f64 * 38.0, 60.0 + (i / 30) as f64 * 70.0],
            )),
        })
        .unwrap();
    }
    map
}

/// How many pixels differ by more than the golden tolerance.
fn differing(a: &RgbaImage, b: &RgbaImage) -> usize {
    assert_eq!((a.width, a.height), (b.width, b.height));
    a.pixels
        .chunks(4)
        .zip(b.pixels.chunks(4))
        .filter(|(a, b)| {
            a.iter()
                .zip(b.iter())
                .any(|(a, b)| a.abs_diff(*b) > CHANNEL_TOLERANCE)
        })
        .count()
}

struct Progressive {
    /// Drawn with nothing tessellated or rasterised yet.
    first: RgbaImage,
    /// Frames drawn as each Object landed, compared with the first frame.
    changed: Vec<usize>,
    /// Drawn once all the background work was ready.
    last: RgbaImage,
    /// Drawn by a renderer that built everything itself, with no background work.
    built: RgbaImage,
}

/// Renders `map` as the editor does after a Map opens (a frame at once, more as tessellation
/// completes) and as a renderer that was never in a hurry.
fn progressive(headless: &Headless, map: &Map) -> Progressive {
    use ssmap_core::asset::{PackLibrary, PackRoots};
    use std::sync::Arc;

    let scene = scene_of(map);
    let view = View::whole_canvas(scene.canvas_size, 600);
    let capabilities = || GpuCapabilities::query(&headless.adapter);
    let draw = |renderer: &mut CanvasRenderer| {
        render_to_image(&headless.device, &headless.queue, renderer, &scene, &view)
            .expect("the readback succeeds")
    };

    let mut renderer = CanvasRenderer::new(&headless.device, capabilities());
    renderer.set_sprites(test_sprites());
    let built = draw(&mut renderer);

    let roots = PackRoots {
        built_in: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../core/tests/packs"),
        user: None,
    };
    let (library, problems) = PackLibrary::load(&roots);
    assert!(problems.is_empty(), "{problems:?}");
    let mut renderer = CanvasRenderer::new(&headless.device, capabilities());
    renderer.set_sprites(ssmap_render::SpriteStore::new(Arc::new(library)));
    let rebuild = renderer.start_tessellation(&scene.content);
    let cache = renderer.tessellation_cache();
    assert!(rebuild.len() >= 8, "every Object is still to do");
    assert!(cache.is_building());
    let first = draw(&mut renderer);

    let mut changed = Vec::new();
    rebuild.run(|| changed.push(differing(&first, &draw(&mut renderer))));
    renderer.wait_until_ready(&scene.content);
    assert!(!cache.is_building());
    let last = draw(&mut renderer);
    Progressive {
        first,
        changed,
        last,
        built,
    }
}

#[test]
fn the_first_frame_comes_before_the_rebuild_and_objects_appear_as_they_complete() {
    let Some(gpu) = gpu() else { return };
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let run = progressive(headless, &progressive_map());

    // The first frame is already a Canvas (ocean) but lacks the Objects and Stamps.
    assert!(
        differing(&run.first, &run.built) > 1000,
        "the first frame leaves out work that is not done"
    );
    // One frame per Object landing, plus the one when the rebuild ends; each shows at least
    // as much as the one before, and more than the first frame by the end.
    assert!(run.changed.len() >= 9, "{:?}", run.changed);
    assert!(
        run.changed.windows(2).all(|pair| pair[0] <= pair[1] + 50),
        "Objects only get added: {:?}",
        run.changed
    );
    assert!(run.changed[0] > 0, "the first Object to land shows");
    assert!(
        run.changed.last() > run.changed.first(),
        "more Objects appear as they complete: {:?}",
        run.changed
    );
}

#[test]
fn the_canvas_after_a_rebuild_matches_a_fully_built_map() {
    let Some(gpu) = gpu() else { return };
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let run = progressive(headless, &progressive_map());
    let share =
        differing(&run.last, &run.built) as f64 / f64::from(run.last.width * run.last.height);
    assert!(share <= MAX_DIFFERING, "{share:.4} of pixels differ");
}

#[test]
fn the_final_frame_matches_the_golden_image_of_a_fully_built_map() {
    let Some(gpu) = gpu() else { return };
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let run = progressive(headless, &progressive_map());
    if let Gpu::Software(_) = gpu {
        assert_matches_golden("progressive_rebuild", &run.last);
    }
}
