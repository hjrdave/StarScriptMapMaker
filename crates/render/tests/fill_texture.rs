//! Texture fill styles (ADR 0015): a texture changes the fill, follows its Canvas position, tiles
//! with no seam, and is the same in every export tile (ADR 0007).
//!
//! Needs a GPU adapter; without one each test returns early, as the golden tests do. These
//! tests compare renders with each other, not with golden images, so they hold on any adapter.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;

use ssmap_core::asset::{AssetRef, PackLibrary, PackRoots};
use ssmap_core::{
    Canvas, Color, Command, DistanceUnit, ExportSettings, FillStyle, FillTexture, Id, Map,
    MultiPolygon, Polygon, RoughnessCache, Scale, SizeMode,
};
use ssmap_render::{
    CanvasRenderer, CanvasScene, Fonts, GpuCapabilities, Headless, MapScene, RgbaImage,
    SpriteStore, TiledExport, View, render_to_image,
};

fn gpu() -> Option<Headless> {
    Headless::new(true).or_else(|_| Headless::new(false)).ok()
}

/// The packs that ship with the app, which include the `textures` pack.
fn sprites() -> SpriteStore {
    let roots = PackRoots {
        built_in: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/packs"),
        user: None,
    };
    let (library, problems) = PackLibrary::load(&roots);
    assert!(problems.is_empty(), "{problems:?}");
    SpriteStore::synchronous(Arc::new(library))
}

fn texture(asset: &str, amount: f64, scale: f64) -> FillStyle {
    FillStyle {
        texture: Some(FillTexture {
            amount,
            scale,
            ..FillTexture::new(asset.parse::<AssetRef>().unwrap())
        }),
        ..FillStyle::FLAT
    }
}

/// A 1000-unit Canvas with one Landmass over most of it, filled in a mid colour, with a smooth
/// coast and no Shore effects.
fn map_with(style: Option<FillStyle>) -> Map {
    let mut map = Map::new(Canvas::new(
        1000,
        1000,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    let run = |map: &mut Map, command: Command| map.execute(&command).unwrap();
    for kind in [ssmap_core::ShoreKind::Landmass, ssmap_core::ShoreKind::Lake] {
        run(
            &mut map,
            Command::SetDefaultShore {
                kind,
                shore: ssmap_core::ShoreStyle::plain(),
            },
        );
    }
    let layer = Id::new();
    run(
        &mut map,
        Command::CreateLayer {
            id: layer,
            parent: None,
            index: None,
            name: "Land".into(),
        },
    );
    let land = Id::new();
    run(
        &mut map,
        Command::CreateLandmass {
            id: land,
            parent: layer,
            index: None,
            geometry: MultiPolygon(vec![Polygon {
                exterior: vec![
                    [100.0, 100.0],
                    [900.0, 100.0],
                    [900.0, 900.0],
                    [100.0, 900.0],
                ],
                holes: Vec::new(),
            }]),
            fill: Some(Color::rgb(0xa0, 0x90, 0x60)),
        },
    );
    if style.is_some() {
        run(
            &mut map,
            Command::SetFillStyle {
                ids: vec![land],
                style,
            },
        );
    }
    map
}

fn renderer(headless: &Headless) -> CanvasRenderer {
    let mut renderer =
        CanvasRenderer::new(&headless.device, GpuCapabilities::query(&headless.adapter));
    renderer.set_sprites(sprites());
    renderer
}

fn scene_of(map: &Map) -> CanvasScene {
    let content = MapScene::new(Fonts::bundled()).content(map, &mut RoughnessCache::new());
    CanvasScene::new(map.canvas()).with_content(content)
}

/// The Map at 1 pixel per Canvas unit, the view's top-left at `origin`.
fn render(headless: &Headless, map: &Map, origin: [f64; 2], size: [u32; 2]) -> RgbaImage {
    let view = View {
        origin,
        zoom: 1.0,
        size,
    };
    render_to_image(
        &headless.device,
        &headless.queue,
        &mut renderer(headless),
        &scene_of(map),
        &view,
    )
    .expect("the readback succeeds")
}

fn pixel(image: &RgbaImage, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * image.width + x) * 4) as usize;
    image.pixels[i..i + 4].try_into().unwrap()
}

/// How many pixels of `a` and `b` differ by more than `tolerance` in a channel, among those
/// whose Canvas position (the image's `origin` plus its pixel) is inside `area`.
fn differing(a: &RgbaImage, b: &RgbaImage, tolerance: u8, area: std::ops::Range<u32>) -> usize {
    (area.clone())
        .flat_map(|y| area.clone().map(move |x| (x, y)))
        .filter(|&(x, y)| {
            pixel(a, x, y)
                .iter()
                .zip(pixel(b, x, y))
                .any(|(a, b)| a.abs_diff(b) > tolerance)
        })
        .count()
}

#[test]
fn a_texture_changes_the_fill_by_its_amount() {
    let Some(headless) = gpu() else { return };
    let size = [1000, 1000];
    let flat = render(&headless, &map_with(None), [0.0, 0.0], size);
    let strong = render(
        &headless,
        &map_with(Some(texture("textures/parchment", 1.0, 128.0))),
        [0.0, 0.0],
        size,
    );
    let faint = render(
        &headless,
        &map_with(Some(texture("textures/parchment", 0.25, 128.0))),
        [0.0, 0.0],
        size,
    );
    let none = render(
        &headless,
        &map_with(Some(texture("textures/parchment", 0.0, 128.0))),
        [0.0, 0.0],
        size,
    );
    let inside = 150..850;
    let area = 700 * 700;
    assert!(
        differing(&flat, &strong, 6, inside.clone()) > area / 2,
        "a full-strength texture changes most of the fill"
    );
    assert_eq!(
        differing(&flat, &none, 0, inside.clone()),
        0,
        "an amount of 0 is the flat fill"
    );
    // The same texture at a quarter of the amount moves the fill about a quarter as far.
    let spread = |image: &RgbaImage| {
        let values: Vec<f64> = inside
            .clone()
            .flat_map(|y| inside.clone().map(move |x| (x, y)))
            .map(|(x, y)| f64::from(pixel(image, x, y)[0]))
            .collect();
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64).sqrt()
    };
    let (strong_spread, faint_spread) = (spread(&strong), spread(&faint));
    assert!(strong_spread > 4.0, "a visible pattern: {strong_spread}");
    assert!(
        faint_spread < strong_spread * 0.5 && faint_spread > strong_spread * 0.1,
        "the amount scales it: {faint_spread} against {strong_spread}"
    );
    // The texture goes under the Landmass's edge: the water outside it is untouched.
    assert_eq!(pixel(&flat, 50, 50), pixel(&strong, 50, 50));
}

#[test]
fn a_texture_that_is_not_installed_leaves_the_fill_as_it_was() {
    let Some(headless) = gpu() else { return };
    let size = [1000, 1000];
    let flat = render(&headless, &map_with(None), [0.0, 0.0], size);
    let missing = render(
        &headless,
        &map_with(Some(texture("no-such-pack/nothing", 1.0, 128.0))),
        [0.0, 0.0],
        size,
    );
    assert!(differing(&flat, &missing, 2, 150..850) < 20);
}

#[test]
fn a_texture_tiles_seamlessly_at_its_scale() {
    let Some(headless) = gpu() else { return };
    // One tile is 200 units, drawn at 1 px per unit: 200 pixels. Shifting the view by a tile
    // gives the same picture, so the texture repeats exactly, and the repeat has no seam.
    let map = map_with(Some(texture("textures/stone", 1.0, 200.0)));
    let size = [1000, 1000];
    let a = render(&headless, &map, [0.0, 0.0], size);
    let b = render(&headless, &map, [200.0, 0.0], size);
    // a(x + 200) is b(x): compare the 600 columns that the Landmass covers in both.
    let mut off = 0;
    for y in 150..850 {
        for x in 150..650 {
            if pixel(&a, x + 200, y)
                .iter()
                .zip(pixel(&b, x, y))
                .any(|(a, b)| a.abs_diff(b) > 2)
            {
                off += 1;
            }
        }
    }
    assert_eq!(off, 0, "the texture is the same one tile on");
    // Across the seam between two tiles (x = 200 + 100 * 2 is a tile edge: 400), neighbouring
    // pixels are about as alike as any others.
    let step = |x: u32| -> f64 {
        (150..850)
            .map(|y| {
                let (p, q) = (pixel(&a, x, y), pixel(&a, x + 1, y));
                f64::from(p[0].abs_diff(q[0]))
            })
            .sum::<f64>()
            / 700.0
    };
    let (edge, inside) = (step(399), step(350));
    assert!(
        edge < inside * 2.0 + 2.0,
        "tile edge step {edge} against {inside} inside a tile"
    );
}

#[test]
fn a_texture_stays_put_when_the_view_pans() {
    let Some(headless) = gpu() else { return };
    let map = map_with(Some(texture("textures/grass", 0.8, 96.0)));
    let a = render(&headless, &map, [0.0, 0.0], [1000, 1000]);
    // An odd shift, not a multiple of the tile.
    let b = render(&headless, &map, [37.0, 11.0], [900, 900]);
    let mut off = 0;
    for y in 150..800 {
        for x in 150..800 {
            if pixel(&a, x + 37, y + 11)
                .iter()
                .zip(pixel(&b, x, y))
                .any(|(a, b)| a.abs_diff(b) > 2)
            {
                off += 1;
            }
        }
    }
    assert_eq!(off, 0, "panning moves the texture with the Canvas");
}

#[test]
fn tiles_of_an_export_agree_with_the_whole_image() {
    let Some(headless) = gpu() else { return };
    // Scaled to 0.6 px per unit, so a tile reads a smaller mip level than the image's own size.
    let map = map_with(Some(texture("textures/sand", 1.0, 150.0)));
    let settings = ExportSettings {
        size: SizeMode::Width { pixels: 600 },
        ..Default::default()
    };
    let export = |max_tile_side| {
        let mut renderer = renderer(&headless);
        let mut pixels = Vec::new();
        let size = TiledExport {
            device: &headless.device,
            queue: &headless.queue,
            renderer: &mut renderer,
            scene: &mut MapScene::new(Fonts::bundled()),
            map: &map,
            roughness: &mut RoughnessCache::new(),
            settings: &settings,
            max_tile_side,
        }
        .run(
            |_| {},
            |row| {
                pixels.extend_from_slice(row);
                Ok::<_, Infallible>(())
            },
        )
        .expect("the export renders");
        RgbaImage {
            width: size.width,
            height: size.height,
            pixels,
        }
    };
    let whole = export(None);
    // A 256px limit makes 128px tiles, with seams at every multiple of 128.
    let tiled = export(Some(256));
    assert_eq!((tiled.width, tiled.height), (whole.width, whole.height));
    let off = differing(&whole, &tiled, 3, 0..whole.width.min(whole.height));
    assert!(
        off <= 8,
        "{off} pixels differ between the tiles and the whole"
    );
}
