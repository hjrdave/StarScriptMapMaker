//! A renderer that has drawn a run of edits draws the same pixels as a fresh one (ADR 0019):
//! keeping meshes on the GPU between edits never changes the stacking order or leaves an
//! old mesh behind, and a tiled export after the edits matches the Canvas.
//!
//! Needs a GPU adapter; without one each test returns early. Both images come from the same
//! adapter, so they are compared exactly, also on hardware.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use std::convert::Infallible;

use ssmap_core::{
    Canvas, Color, Command, DistanceUnit, ExportSettings, Id, Map, MultiPolygon, Polygon,
    RoughnessCache, Scale, SizeMode,
};
use ssmap_render::{
    CanvasRenderer, CanvasScene, Fonts, GpuCapabilities, Headless, MapScene, RgbaImage,
    TiledExport, View, render_to_image,
};

const WIDTH: u32 = 300;
const HEIGHT: u32 = 200;

fn gpu() -> Option<Headless> {
    Headless::new(true).or_else(|_| Headless::new(false)).ok()
}

fn execute(map: &mut Map, command: Command) {
    map.execute(&command).expect("the Command applies");
}

fn block([left, top]: [f64; 2], [right, bottom]: [f64; 2]) -> MultiPolygon {
    MultiPolygon(vec![Polygon {
        exterior: vec![[left, top], [right, top], [right, bottom], [left, bottom]],
        holes: Vec::new(),
    }])
}

/// Overlapping Landmasses in two Layers, so a wrong stacking order shows.
fn map() -> (Map, Vec<Id>, Id) {
    let mut map = Map::new(Canvas::new(
        WIDTH,
        HEIGHT,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    let (lower, upper) = (Id::new(), Id::new());
    for (id, name) in [(lower, "Lower"), (upper, "Upper")] {
        execute(
            &mut map,
            Command::CreateLayer {
                id,
                parent: None,
                index: None,
                name: name.into(),
            },
        );
    }
    let colours = [
        Color::rgb(0xc8, 0xb4, 0x7c),
        Color::rgb(0x8c, 0xa8, 0x5c),
        Color::rgb(0xa0, 0x50, 0x40),
        Color::rgb(0x60, 0x60, 0x90),
    ];
    let mut ids = Vec::new();
    for (i, fill) in colours.into_iter().enumerate() {
        let id = Id::new();
        let x = 30.0 + 50.0 * i as f64;
        execute(
            &mut map,
            Command::CreateLandmass {
                id,
                parent: Some(if i < 2 { lower } else { upper }),
                index: None,
                geometry: block([x, 40.0], [x + 90.0, 160.0]),
                fill: Some(fill),
            },
        );
        ids.push(id);
    }
    (map, ids, upper)
}

fn renderer(headless: &Headless) -> CanvasRenderer {
    CanvasRenderer::new(
        &headless.device,
        GpuCapabilities::query(&headless.adapter, &headless.device),
    )
}

/// The whole Canvas at one pixel per unit.
fn view() -> View {
    View {
        origin: [0.0, 0.0],
        zoom: 1.0,
        size: [WIDTH, HEIGHT],
    }
}

fn render(
    headless: &Headless,
    renderer: &mut CanvasRenderer,
    scene: &mut MapScene,
    map: &Map,
) -> RgbaImage {
    let content = scene.content(map, &mut RoughnessCache::new());
    let canvas = CanvasScene::new(map.canvas()).with_content(content);
    render_to_image(
        &headless.device,
        &headless.queue,
        renderer,
        &canvas,
        &view(),
    )
    .expect("the readback succeeds")
}

/// Renders `map` with a renderer and scene that have drawn nothing before.
fn fresh(headless: &Headless, map: &Map) -> RgbaImage {
    render(
        headless,
        &mut renderer(headless),
        &mut MapScene::new(Fonts::bundled()),
        map,
    )
}

fn differing(a: &RgbaImage, b: &RgbaImage) -> usize {
    assert_eq!((a.width, a.height), (b.width, b.height));
    a.pixels
        .chunks_exact(4)
        .zip(b.pixels.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count()
}

/// How many pixels differ by more than a few levels in some channel.
fn far_apart(a: &RgbaImage, b: &RgbaImage) -> usize {
    a.pixels
        .chunks_exact(4)
        .zip(b.pixels.chunks_exact(4))
        .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 16))
        .count()
}

fn export(
    headless: &Headless,
    renderer: &mut CanvasRenderer,
    scene: &mut MapScene,
    map: &Map,
) -> RgbaImage {
    let settings = ExportSettings {
        size: SizeMode::Width { pixels: WIDTH },
        ..Default::default()
    };
    let mut pixels = Vec::new();
    let size = TiledExport {
        device: &headless.device,
        queue: &headless.queue,
        renderer,
        scene,
        map,
        roughness: &mut RoughnessCache::new(),
        settings: &settings,
        max_tile_side: Some(128),
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

/// Edits that change one Object, the stacking order, a colour, an outline's size (past the
/// spare room), and the set of Objects.
fn edits(ids: &[Id], upper: Id) -> Vec<(&'static str, Command)> {
    vec![
        (
            "move one Landmass",
            Command::Translate {
                ids: vec![ids[1]],
                offset: [25.0, 10.0],
            },
        ),
        (
            "raise the bottom Landmass to the top",
            Command::Move {
                id: ids[0],
                parent: Some(upper),
                index: 2,
            },
        ),
        (
            "recolour one",
            Command::SetFill {
                ids: vec![ids[2]],
                fill: Color::rgb(0x30, 0x90, 0x30),
            },
        ),
        (
            "grow one with a rough outline",
            Command::AddRegion {
                id: ids[3],
                region: MultiPolygon(vec![Polygon {
                    exterior: (0..400)
                        .map(|k| {
                            let angle = k as f64 / 400.0 * std::f64::consts::TAU;
                            let radius = 60.0 + 8.0 * (k % 5) as f64;
                            [220.0 + radius * angle.cos(), 100.0 + radius * angle.sin()]
                        })
                        .collect(),
                    holes: Vec::new(),
                }]),
            },
        ),
        ("delete one", Command::Delete { ids: vec![ids[1]] }),
        (
            "create one under the rest",
            Command::CreateLandmass {
                id: Id::new(),
                parent: Some(upper),
                index: Some(0),
                geometry: block([10.0, 10.0], [140.0, 120.0]),
                fill: Some(Color::rgb(0xe0, 0xe0, 0xe0)),
            },
        ),
    ]
}

#[test]
fn a_renderer_that_drew_each_edit_matches_a_fresh_one() {
    let Some(headless) = gpu() else { return };
    let (mut map, ids, upper) = map();
    let mut kept = renderer(&headless);
    let mut scene = MapScene::new(Fonts::bundled());
    let first = render(&headless, &mut kept, &mut scene, &map);
    assert_eq!(differing(&first, &fresh(&headless, &map)), 0);
    for (name, command) in edits(&ids, upper) {
        execute(&mut map, command);
        let after = render(&headless, &mut kept, &mut scene, &map);
        assert_eq!(
            differing(&after, &fresh(&headless, &map)),
            0,
            "after \"{name}\""
        );
    }
}

#[test]
fn undoing_back_to_an_earlier_map_draws_it_again() {
    let Some(headless) = gpu() else { return };
    let (mut map, ids, upper) = map();
    let original = map.clone();
    let mut kept = renderer(&headless);
    let mut scene = MapScene::new(Fonts::bundled());
    let before = render(&headless, &mut kept, &mut scene, &map);
    for (_, command) in edits(&ids, upper) {
        execute(&mut map, command);
        render(&headless, &mut kept, &mut scene, &map);
    }
    let again = render(&headless, &mut kept, &mut scene, &original);
    assert_eq!(differing(&again, &before), 0);
}

#[test]
fn a_tiled_export_after_edits_matches_the_canvas() {
    let Some(headless) = gpu() else { return };
    let (mut map, ids, upper) = map();
    let mut kept = renderer(&headless);
    let mut scene = MapScene::new(Fonts::bundled());
    render(&headless, &mut kept, &mut scene, &map);
    for (_, command) in edits(&ids, upper) {
        execute(&mut map, command);
        render(&headless, &mut kept, &mut scene, &map);
    }
    let canvas = render(&headless, &mut kept, &mut scene, &map);
    let after_edits = export(&headless, &mut kept, &mut scene, &map);
    let from_scratch = export(
        &headless,
        &mut renderer(&headless),
        &mut MapScene::new(Fonts::bundled()),
        &map,
    );
    assert_eq!(differing(&after_edits, &from_scratch), 0);
    // An export samples a few edge pixels slightly differently from the Canvas, edits or not
    // (master does the same), so the two are compared with a little room.
    assert_eq!(far_apart(&after_edits, &canvas), 0);
    assert!(differing(&after_edits, &canvas) <= (WIDTH * HEIGHT / 1000) as usize);
}
