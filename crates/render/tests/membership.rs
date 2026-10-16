//! Canvas members clip at the Canvas edge, pasteboard Objects draw in full, and export covers
//! only the Canvas, wherever its origin is.

use std::convert::Infallible;

use ssmap_core::{
    Canvas, Color, Command, DistanceUnit, ExportSettings, Id, Map, Membership, MultiPolygon,
    Polygon, RoughnessCache, Scale, ShoreKind, ShoreStyle, SizeMode,
};
use ssmap_render::{
    CanvasRenderer, CanvasScene, Fonts, GpuCapabilities, Headless, MapScene, RgbaImage,
    TiledExport, View, render_to_image,
};

const LAND: Color = Color::rgb(0xc8, 0xb4, 0x7c);
const WATER: Color = Color::rgb(0x20, 0x60, 0xa0);

fn gpu() -> Option<Headless> {
    Headless::new(true).or_else(|_| Headless::new(false)).ok()
}

fn pixel(image: &RgbaImage, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * image.width + x) * 4) as usize;
    image.pixels[i..i + 4].try_into().unwrap()
}

fn is(actual: [u8; 4], expected: Color) -> bool {
    let expected = [expected.r, expected.g, expected.b, 0xff];
    actual.iter().zip(expected).all(|(a, e)| a.abs_diff(e) <= 2)
}

/// A flat-water 200 x 200 Canvas at `origin`, holding a block spanning workspace `x` from
/// `left` to `right` and `y` from `top` to `bottom`.
fn map_with_block(
    origin: [f64; 2],
    [left, right]: [f64; 2],
    [top, bottom]: [f64; 2],
    membership: Membership,
) -> Map {
    let mut canvas = Canvas::new(200, 200, Scale::new(1.0, DistanceUnit::Miles));
    canvas.origin = origin;
    canvas.background.water = WATER;
    canvas.background.texture.strength = 0.0;
    let mut map = Map::new(canvas);
    map.execute(&Command::SetDefaultShore {
        kind: ShoreKind::Landmass,
        shore: ShoreStyle::plain(),
    })
    .expect("a valid Shore style");
    let (layer, block) = (Id::new(), Id::new());
    for command in [
        Command::CreateLayer {
            id: layer,
            parent: None,
            index: None,
            name: "Land".into(),
        },
        Command::CreateLandmass {
            id: block,
            parent: layer,
            index: None,
            geometry: MultiPolygon(vec![Polygon {
                exterior: vec![[left, top], [right, top], [right, bottom], [left, bottom]],
                holes: Vec::new(),
            }]),
            fill: Some(LAND),
        },
        Command::SetMembership {
            ids: vec![block],
            membership,
        },
    ] {
        map.execute(&command).expect("the Command applies");
    }
    map
}

fn render(headless: &Headless, map: &Map, view: &View) -> RgbaImage {
    let content = MapScene::new(Fonts::bundled()).content(map, &mut RoughnessCache::new());
    let scene = CanvasScene::new(&map.canvas).with_content(content);
    let mut renderer =
        CanvasRenderer::new(&headless.device, GpuCapabilities::query(&headless.adapter));
    render_to_image(
        &headless.device,
        &headless.queue,
        &mut renderer,
        &scene,
        view,
    )
}

/// A 400 px view of the workspace from (-100, -100): the Canvas at the origin covers pixels
/// 100 to 300 on both axes.
fn view() -> View {
    View {
        origin: [-100.0, -100.0],
        zoom: 1.0,
        size: [400, 400],
    }
}

fn pasteboard_colour(image: &RgbaImage) -> [u8; 4] {
    pixel(image, 5, 5)
}

#[test]
fn a_member_overflowing_the_edge_is_clipped() {
    let Some(headless) = gpu() else { return };
    let map = map_with_block(
        [0.0, 0.0],
        [100.0, 300.0],
        [50.0, 150.0],
        Membership::Canvas,
    );
    let image = render(&headless, &map, &view());
    let outside = pasteboard_colour(&image);
    assert!(!is(outside, LAND) && !is(outside, WATER));
    assert!(is(pixel(&image, 250, 200), LAND), "inside the Canvas");
    assert!(is(pixel(&image, 150, 200), WATER), "left of the block");
    assert_eq!(pixel(&image, 320, 200), outside, "clipped at the edge");
    assert_eq!(pixel(&image, 380, 200), outside, "clipped at the edge");
}

#[test]
fn a_pasteboard_object_is_drawn_in_full() {
    let Some(headless) = gpu() else { return };
    let map = map_with_block(
        [0.0, 0.0],
        [100.0, 300.0],
        [50.0, 150.0],
        Membership::Pasteboard,
    );
    let image = render(&headless, &map, &view());
    assert!(is(pixel(&image, 250, 200), LAND), "over the Canvas");
    assert!(is(pixel(&image, 320, 200), LAND), "past the edge");
    assert!(is(pixel(&image, 380, 200), LAND), "past the edge");
    assert!(
        !is(pixel(&image, 380, 350), LAND),
        "only inside its own outline"
    );
}

#[test]
fn a_canvas_with_an_origin_renders_where_it_sits() {
    let Some(headless) = gpu() else { return };
    // The Canvas covers workspace 100..300; the block crosses its top-left corner.
    let map = map_with_block(
        [100.0, 100.0],
        [50.0, 200.0],
        [50.0, 200.0],
        Membership::Canvas,
    );
    let view = View {
        origin: [0.0, 0.0],
        zoom: 1.0,
        size: [400, 400],
    };
    let image = render(&headless, &map, &view);
    let outside = pasteboard_colour(&image);
    assert!(!is(outside, LAND) && !is(outside, WATER));
    assert_eq!(pixel(&image, 75, 75), outside, "the block is clipped");
    assert_eq!(pixel(&image, 150, 50), outside, "above the Canvas");
    assert!(is(pixel(&image, 150, 150), LAND), "block inside the Canvas");
    assert!(is(pixel(&image, 250, 250), WATER), "Canvas water");
    assert_eq!(pixel(&image, 350, 350), outside, "right of the Canvas");
}

fn export(headless: &Headless, map: &Map, max_tile_side: Option<u32>) -> RgbaImage {
    let mut renderer =
        CanvasRenderer::new(&headless.device, GpuCapabilities::query(&headless.adapter));
    let settings = ExportSettings {
        size: SizeMode::Width { pixels: 200 },
        ..Default::default()
    };
    let mut pixels = Vec::new();
    let size = TiledExport {
        device: &headless.device,
        queue: &headless.queue,
        renderer: &mut renderer,
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

#[test]
fn export_holds_only_the_clipped_canvas() {
    let Some(headless) = gpu() else { return };
    for origin in [[0.0, 0.0], [100.0, -40.0]] {
        // The block spans 100 units each side of the Canvas's left edge, level with its top.
        let (left, top) = (origin[0] - 100.0, origin[1] + 50.0);
        let map = map_with_block(
            origin,
            [left, left + 200.0],
            [top, top + 100.0],
            Membership::Canvas,
        );
        for tiles in [None, Some(256)] {
            let image = export(&headless, &map, tiles);
            assert_eq!((image.width, image.height), (200, 200));
            assert!(
                is(pixel(&image, 50, 100), LAND),
                "member inside, {origin:?}"
            );
            assert!(is(pixel(&image, 150, 100), WATER), "past the block");
            assert!(is(pixel(&image, 100, 10), WATER), "above the block");
            assert!(is(pixel(&image, 100, 190), WATER), "below the block");
            for (x, y) in [(0, 0), (199, 0), (0, 199), (199, 199)] {
                assert!(is(pixel(&image, x, y), WATER), "corner ({x}, {y})");
            }
        }
    }
}

#[test]
fn export_leaves_out_pasteboard_objects() {
    let Some(headless) = gpu() else { return };
    let map = map_with_block(
        [0.0, 0.0],
        [50.0, 150.0],
        [50.0, 150.0],
        Membership::Pasteboard,
    );
    let image = export(&headless, &map, None);
    assert!(
        image
            .pixels
            .chunks_exact(4)
            .all(|p| is(p.try_into().unwrap(), WATER))
    );
}
