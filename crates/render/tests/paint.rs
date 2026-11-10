//! Colour painting (ADR 0017): paint strokes are clipped to land, soft at the edge, erased by
//! erasers, and the same in the Canvas, an export and the tiles of an export (ADR 0007).
//!
//! Needs a GPU adapter; without one each test returns early, as the golden tests do. These
//! tests compare renders with each other, not with golden images, so they hold on any adapter.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use std::convert::Infallible;

use ssmap_core::{
    Canvas, Color, Command, DistanceUnit, ExportSettings, Id, Map, MultiPolygon, PaintStroke,
    Polygon, Roughness, RoughnessCache, Scale, SizeMode,
};
use ssmap_render::{
    CanvasRenderer, CanvasScene, Fonts, GpuCapabilities, Headless, MapScene, RgbaImage,
    TiledExport, View, render_to_image,
};

const PAINT: Color = Color::rgb(0xd0, 0x20, 0x20);

fn gpu() -> Option<Headless> {
    Headless::new(true).or_else(|_| Headless::new(false)).ok()
}

fn execute(map: &mut Map, command: Command) {
    map.execute(&command).expect("the Command applies");
}

fn square(min: f64, max: f64) -> MultiPolygon {
    MultiPolygon(vec![Polygon {
        exterior: vec![[min, min], [max, min], [max, max], [min, max]],
        holes: Vec::new(),
    }])
}

/// A 1000-unit Canvas with a Landmass from 100 to 900 and a Lake from 400 to 600 on it, with
/// smooth coasts and no Shore effects, or a rough coast if `rough`.
fn map_of(rough: bool) -> Map {
    let mut map = Map::new(Canvas::new(
        1000,
        1000,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    for kind in [ssmap_core::ShoreKind::Landmass, ssmap_core::ShoreKind::Lake] {
        execute(
            &mut map,
            Command::SetDefaultShore {
                kind,
                shore: ssmap_core::ShoreStyle::plain(),
            },
        );
    }
    if rough {
        execute(
            &mut map,
            Command::SetDefaultRoughness {
                roughness: Roughness::new(25.0, 80.0, 7),
            },
        );
    }
    let layer = Id::new();
    execute(
        &mut map,
        Command::CreateLayer {
            id: layer,
            parent: None,
            index: None,
            name: "Land".into(),
        },
    );
    for (lake, geometry, fill) in [
        (false, square(100.0, 900.0), Color::rgb(0xa0, 0x90, 0x60)),
        (true, square(400.0, 600.0), Color::rgb(0x40, 0x70, 0xb0)),
    ] {
        let (id, parent, index) = (Id::new(), layer, None);
        let fill = Some(fill);
        execute(
            &mut map,
            if lake {
                Command::CreateLake {
                    id,
                    parent,
                    index,
                    geometry,
                    fill,
                }
            } else {
                Command::CreateLandmass {
                    id,
                    parent,
                    index,
                    geometry,
                    fill,
                }
            },
        );
    }
    map
}

/// A stroke along `points`, which Commands simplify and add.
fn stroke(points: &[[f64; 2]], size: f64, softness: f32, opacity: f32, erase: bool) -> Command {
    Command::AddPaintStroke {
        stroke: PaintStroke {
            id: Id::new(),
            points: points.to_vec(),
            color: PAINT,
            size,
            softness,
            opacity,
            erase,
        },
    }
}

fn painted(map: &mut Map, command: Command) {
    execute(map, command);
}

fn scene_of(map: &Map) -> CanvasScene {
    let content = MapScene::new(Fonts::bundled()).content(map, &mut RoughnessCache::new());
    CanvasScene::new(map.canvas()).with_content(content)
}

fn renderer(headless: &Headless) -> CanvasRenderer {
    CanvasRenderer::new(&headless.device, GpuCapabilities::query(&headless.adapter))
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

fn differs(a: [u8; 4], b: [u8; 4], tolerance: u8) -> bool {
    a.iter().zip(b).any(|(a, b)| a.abs_diff(b) > tolerance)
}

/// How far `p` is from `from`, summed over the channels.
fn distance(p: [u8; 4], from: [u8; 4]) -> u32 {
    p.iter()
        .zip(from)
        .map(|(a, b)| u32::from(a.abs_diff(b)))
        .sum()
}

fn export(headless: &Headless, map: &Map, width: u32, max_tile_side: Option<u32>) -> RgbaImage {
    let settings = ExportSettings {
        size: SizeMode::Width { pixels: width },
        ..Default::default()
    };
    let mut pixels = Vec::new();
    let size = TiledExport {
        device: &headless.device,
        queue: &headless.queue,
        renderer: &mut renderer(headless),
        scene: &mut MapScene::new(Fonts::bundled()),
        map,
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
}

const SIZE: [u32; 2] = [1000, 1000];

#[test]
fn paint_covers_land_and_stops_at_the_coast_and_the_lake() {
    let Some(headless) = gpu() else { return };
    let mut map = map_of(false);
    let bare = render(&headless, &map, [0.0, 0.0], SIZE);
    // Across the whole width at the Lake's height, and beyond the coast at both ends.
    painted(
        &mut map,
        stroke(&[[20.0, 500.0], [980.0, 500.0]], 60.0, 0.0, 1.0, false),
    );
    let image = render(&headless, &map, [0.0, 0.0], SIZE);
    // On land, the paint replaces the fill.
    assert!(differs(pixel(&image, 200, 500), pixel(&bare, 200, 500), 20));
    assert!(distance(pixel(&image, 200, 500), [0xd0, 0x20, 0x20, 0xff]) < 12);
    // On the water around the land, and in the Lake, nothing changes.
    for (x, y) in [(50, 500), (950, 500), (500, 500), (450, 520), (580, 480)] {
        assert_eq!(pixel(&image, x, y), pixel(&bare, x, y), "at {x},{y}");
    }
    // Nothing outside the stroke changes.
    assert_eq!(pixel(&image, 200, 300), pixel(&bare, 200, 300));
}

#[test]
fn paint_follows_the_rough_coast_of_the_fill() {
    let Some(headless) = gpu() else { return };
    let mut map = map_of(true);
    let bare = render(&headless, &map, [0.0, 0.0], SIZE);
    // A brush wider than the Canvas covers everything.
    painted(&mut map, stroke(&[[500.0, 500.0]], 1500.0, 0.0, 1.0, false));
    let image = render(&headless, &map, [0.0, 0.0], SIZE);
    // The ocean has a texture, so water is what a Map with no land draws.
    let ocean = render(
        &headless,
        &Map::new(Canvas::new(
            1000,
            1000,
            Scale::new(1.0, DistanceUnit::Miles),
        )),
        [0.0, 0.0],
        SIZE,
    );
    let is_water = |x: u32, y: u32| !differs(pixel(&bare, x, y), pixel(&ocean, x, y), 2);
    let (mut inside, mut spilled, mut missed) = (0, 0, 0);
    for y in 2..998 {
        for x in 2..998 {
            // Skip the antialiased edge: pixels with water or land of the other kind next to them.
            let around: Vec<bool> = [(-1, 0), (1, 0), (0, -1), (0, 1), (0, 0)]
                .iter()
                .map(|&(dx, dy)| is_water((x as i32 + dx) as u32, (y as i32 + dy) as u32))
                .collect();
            let changed = differs(pixel(&image, x, y), pixel(&bare, x, y), 2);
            if around.iter().all(|&w| w) && changed {
                spilled += 1;
            } else if around.iter().all(|&w| !w) {
                inside += 1;
                // The Lake is water of another colour, which paint does not cover either.
                if !changed && distance(pixel(&bare, x, y), [0x40, 0x70, 0xb0, 0xff]) > 30 {
                    missed += 1;
                }
            }
        }
    }
    assert!(inside > 100_000, "land was found: {inside}");
    assert_eq!(spilled, 0, "paint spilled onto the water");
    assert_eq!(missed, 0, "paint left land bare");
}

#[test]
fn a_soft_edge_fades_out_and_a_hard_edge_does_not() {
    let Some(headless) = gpu() else { return };
    let line = [[200.0, 250.0], [800.0, 250.0]];
    let render_with = |softness: f32| {
        let mut map = map_of(false);
        painted(&mut map, stroke(&line, 100.0, softness, 1.0, false));
        render(&headless, &map, [0.0, 0.0], SIZE)
    };
    let bare = render(&headless, &map_of(false), [0.0, 0.0], SIZE);
    let (hard, soft) = (render_with(0.0), render_with(1.0));
    let from_paint = |image: &RgbaImage, offset: u32| {
        distance(pixel(image, 500, 250 + offset), [0xd0, 0x20, 0x20, 0xff])
    };
    // The hard brush is solid to its edge, 50 units from the middle.
    assert!(from_paint(&hard, 45) < 12);
    assert_eq!(pixel(&hard, 500, 305), pixel(&bare, 500, 305));
    // The soft brush loses strength steadily from the middle outwards, and ends at its size.
    let profile: Vec<u32> = [0, 12, 24, 36, 48].map(|o| from_paint(&soft, o)).to_vec();
    // Fully soft is nearly solid at the middle: the first band is the strongest.
    assert!(profile[0] < 60, "strong in the middle: {profile:?}");
    assert!(
        profile.windows(2).all(|pair| pair[0] < pair[1]),
        "weaker further out: {profile:?}"
    );
    assert!(profile[4] > from_paint(&hard, 45) + 100, "{profile:?}");
    assert_eq!(pixel(&soft, 500, 305), pixel(&bare, 500, 305));
}

#[test]
fn opacity_lets_the_fill_show_through() {
    let Some(headless) = gpu() else { return };
    let line = [[200.0, 250.0], [800.0, 250.0]];
    let bare = render(&headless, &map_of(false), [0.0, 0.0], SIZE);
    let with = |opacity: f32| {
        let mut map = map_of(false);
        painted(&mut map, stroke(&line, 60.0, 0.0, opacity, false));
        render(&headless, &map, [0.0, 0.0], SIZE)
    };
    let (full, half) = (with(1.0), with(0.5));
    // The target blends in linear light, so the middle is found there.
    let linear = |byte: u8| (f64::from(byte) / 255.0).powf(2.2);
    for channel in 0..3 {
        let mid = (linear(pixel(&bare, 500, 250)[channel])
            + linear(pixel(&full, 500, 250)[channel]))
            / 2.0;
        let got = linear(pixel(&half, 500, 250)[channel]);
        assert!(
            (got - mid).abs() < 0.02,
            "channel {channel}: {got} against {mid}"
        );
    }
    // A stroke is one stroke: where its path crosses itself, it is no stronger.
    let mut map = map_of(false);
    painted(
        &mut map,
        stroke(
            &[
                [300.0, 700.0],
                [700.0, 700.0],
                [700.0, 740.0],
                [300.0, 740.0],
            ],
            60.0,
            0.0,
            0.5,
            false,
        ),
    );
    let image = render(&headless, &map, [0.0, 0.0], SIZE);
    assert!(!differs(
        pixel(&image, 500, 720),
        pixel(&image, 500, 700),
        3
    ));
}

#[test]
fn an_eraser_removes_earlier_paint_and_not_later_paint() {
    let Some(headless) = gpu() else { return };
    let bare = render(&headless, &map_of(false), [0.0, 0.0], SIZE);
    let mut map = map_of(false);
    painted(
        &mut map,
        stroke(&[[200.0, 250.0], [800.0, 250.0]], 80.0, 0.0, 1.0, false),
    );
    painted(
        &mut map,
        stroke(&[[400.0, 150.0], [400.0, 350.0]], 60.0, 0.0, 1.0, true),
    );
    // Paint added after the eraser goes over the cleared strip.
    painted(
        &mut map,
        stroke(&[[200.0, 330.0], [800.0, 330.0]], 40.0, 0.0, 1.0, false),
    );
    let image = render(&headless, &map, [0.0, 0.0], SIZE);
    assert!(distance(pixel(&image, 300, 250), [0xd0, 0x20, 0x20, 0xff]) < 12);
    assert!(distance(pixel(&image, 500, 250), [0xd0, 0x20, 0x20, 0xff]) < 12);
    // Under the eraser the fill is back, and so it is just past the paint's own edge.
    assert_eq!(pixel(&image, 400, 250), pixel(&bare, 400, 250));
    assert_eq!(pixel(&image, 380, 270), pixel(&bare, 380, 270));
    // The later stroke covers the eraser's path.
    assert!(distance(pixel(&image, 400, 330), [0xd0, 0x20, 0x20, 0xff]) < 12);
    // An eraser stroke over bare land, or water, changes nothing.
    assert_eq!(pixel(&image, 400, 160), pixel(&bare, 400, 160));
}

#[test]
fn the_canvas_an_export_and_its_tiles_look_the_same() {
    let Some(headless) = gpu() else { return };
    let mut map = map_of(true);
    // A looping soft stroke across tile edges, and an eraser through it.
    painted(
        &mut map,
        stroke(
            &[
                [150.0, 150.0],
                [850.0, 200.0],
                [200.0, 700.0],
                [880.0, 880.0],
            ],
            90.0,
            0.6,
            0.8,
            false,
        ),
    );
    painted(
        &mut map,
        stroke(&[[500.0, 100.0], [480.0, 900.0]], 50.0, 0.0, 1.0, true),
    );
    painted(
        &mut map,
        stroke(&[[100.0, 520.0], [900.0, 480.0]], 40.0, 0.2, 1.0, false),
    );
    let canvas = render(&headless, &map, [0.0, 0.0], SIZE);
    let whole = export(&headless, &map, 1000, None);
    // 256 pixel tiles give seams at multiples of 128 and a tile grid that does not divide it.
    let tiled = export(&headless, &map, 1000, Some(256));
    for (name, other) in [("export", &whole), ("tiled export", &tiled)] {
        assert_eq!((other.width, other.height), (1000, 1000));
        let off = (0..1000)
            .flat_map(|y| (0..1000).map(move |x| (x, y)))
            .filter(|&(x, y)| differs(pixel(&canvas, x, y), pixel(other, x, y), 3))
            .count();
        assert!(off < 50, "{name} differs from the Canvas at {off} pixels");
    }
}
