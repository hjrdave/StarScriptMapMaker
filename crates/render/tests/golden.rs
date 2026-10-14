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

use ssmap_core::asset::{ArtFormat, PackLibrary, PackRoots, PackSource, read_pack};
use ssmap_core::{
    Align, Canvas, Color, Command, DistanceUnit, FontFace, Id, Label, Lake, Landmass, Map,
    Roughness, RoughnessCache, Scale, Stamp,
};
use ssmap_render::atlas::{Atlas, AtlasEntry, MIN_PAGE_SIZE};
use ssmap_render::raster::{Art, Raster};
use ssmap_render::{
    CanvasRenderer, CanvasScene, Fonts, GpuCapabilities, Headless, MapScene, RgbaImage,
    StampSource, View, read_texture, render_to_image,
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

/// The Map built by `core`'s `land_and_water` Command script: an island with a hole and an
/// islet, plus a Lake listed beneath the island in the tree.
/// Everything `map` draws, as the editor and export build it.
fn scene_of(map: &Map) -> CanvasScene {
    let content = MapScene::new(Fonts::bundled()).content(map, &mut RoughnessCache::new(), None);
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
    let canvas = CanvasScene::new(&map.canvas).with_content(scene.content(
        &map,
        &mut RoughnessCache::new(),
        None,
    ));
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

/// The test pack's Assets as Stamps on a 800x400 Canvas, rendered at half size.
fn render_stamps(headless: &Headless, stamps: Vec<Stamp>) -> RgbaImage {
    let roots = PackRoots {
        built_in: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../core/tests/packs"),
        user: None,
    };
    let (library, problems) = PackLibrary::load(&roots);
    assert!(problems.is_empty(), "{problems:?}");
    let mut map = Map::new(Canvas::new(800, 400, Scale::new(1.0, DistanceUnit::Miles)));
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Terrain".into(),
    })
    .unwrap();
    for stamp in stamps {
        map.execute(&Command::CreateStamp {
            id: Id::new(),
            parent: layer,
            index: None,
            stamp,
        })
        .unwrap();
    }
    let capabilities = GpuCapabilities::query(&headless.adapter);
    let mut renderer = CanvasRenderer::new(&headless.device, capabilities);
    let content = MapScene::new(Fonts::bundled()).content(
        &map,
        &mut RoughnessCache::new(),
        Some(&mut StampSource {
            library: &library,
            atlas: renderer.stamp_atlas(),
            device: &headless.device,
            queue: &headless.queue,
        }),
    );
    let scene = CanvasScene::new(&map.canvas).with_content(content);
    render_to_image(
        &headless.device,
        &headless.queue,
        &mut renderer,
        &scene,
        &View::whole_canvas(scene.canvas_size, 400),
    )
}

fn stamp(asset: &str, position: [f64; 2]) -> Stamp {
    Stamp {
        position,
        ..Stamp::new(format!("test-pack/{asset}").parse().unwrap())
    }
}

/// Tint, flip, fill, opacity, rotation and scale, each on its own Stamp.
#[test]
fn stamps() {
    let Some(gpu) = gpu() else { return };
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let image = render_stamps(
        headless,
        vec![
            stamp("mountain-1", [100.0, 120.0]),
            Stamp {
                tint: Color::rgb(0xb0, 0x30, 0x30),
                ..stamp("mountain-1", [260.0, 120.0])
            },
            Stamp {
                fill: Some(Color::rgb(0x4a, 0x7a, 0x3a)),
                ..stamp("mountain-1", [420.0, 120.0])
            },
            Stamp {
                opacity: 0.5,
                ..stamp("mountain-1", [580.0, 120.0])
            },
            Stamp {
                scale: 2.0,
                ..stamp("mountain-2", [720.0, 200.0])
            },
            stamp("castle", [100.0, 300.0]),
            Stamp {
                flip: true,
                ..stamp("castle", [260.0, 300.0])
            },
            Stamp {
                rotation: 30.0,
                ..stamp("tree", [420.0, 300.0])
            },
            Stamp {
                scale: 3.0,
                ..stamp("well", [580.0, 300.0])
            },
            Stamp {
                scale: 3.0,
                tint: Color::rgb(0xff, 0x90, 0x40),
                ..stamp("well", [720.0, 300.0])
            },
        ],
    );
    // Tinting the well's blue water shifts its colours from blue to orange.
    let total = |x: u32, channel: usize| -> u32 {
        (140..160)
            .flat_map(|y| (x - 15..x + 15).map(move |x| (x, y)))
            .map(|(x, y)| u32::from(pixel(&image, x, y)[channel]))
            .sum()
    };
    assert!(total(290, 2) > total(360, 2) + 500, "less blue once tinted");
    if let Gpu::Software(_) = gpu {
        assert_matches_golden("stamps", &image);
    }
}

/// Mountains placed back to front: each Silhouette hides the line art behind it.
#[test]
fn overlapping_stamps_occlude_each_other() {
    let Some(gpu) = gpu() else { return };
    let (Gpu::Software(headless) | Gpu::Hardware(headless)) = &gpu;
    let image = render_stamps(
        headless,
        vec![
            Stamp {
                scale: 3.0,
                ..stamp("mountain-1", [300.0, 250.0])
            },
            Stamp {
                scale: 3.0,
                ..stamp("mountain-2", [340.0, 300.0])
            },
        ],
    );
    if let Gpu::Software(_) = gpu {
        assert_matches_golden("stamps_occlusion", &image);
    }
}
