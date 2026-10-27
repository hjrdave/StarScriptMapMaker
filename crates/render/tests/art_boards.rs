//! Each board draws its own background and clips its own members, and export covers only the
//! Canvas. Software adapters render these exactly enough to compare flat colours.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

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
const ART_WATER: Color = Color::rgb(0x70, 0x30, 0x30);

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

/// A Canvas of flat blue water 200 wide at the origin and an Art board of flat red water 200
/// wide with its corner at `art_origin`, plus a block spanning workspace `x` from `left` to
/// `right` and `y` from 50 to 150 that belongs to the Canvas or to the Art board, as `on`
/// says. Returns the Map.
fn map(art_origin: [f64; 2], [left, right]: [f64; 2], on_art_board: Option<bool>) -> Map {
    let mut canvas = Canvas::new(200, 200, Scale::new(1.0, DistanceUnit::Miles));
    canvas.background.water = WATER;
    canvas.background.texture.strength = 0.0;
    let mut map = Map::new(canvas);
    let art = Id::new();
    map.execute(&Command::CreateBoard {
        id: art,
        name: None,
        origin: art_origin,
        width: 200,
        height: 200,
        index: None,
    })
    .expect("a valid board");
    // Commands give a new board the Canvas's background; give this one its own.
    let mut view = map.view();
    for board in &mut view.boards {
        if board.id == art {
            board.canvas.background.water = ART_WATER;
        }
    }
    let mut map = Map::from_view(view).expect("a valid view");
    map.execute(&Command::SetDefaultShore {
        kind: ShoreKind::Landmass,
        shore: ShoreStyle::plain(),
    })
    .expect("a valid Shore style");
    let (layer, block) = (Id::new(), Id::new());
    let mut commands = vec![
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
                exterior: vec![[left, 50.0], [right, 50.0], [right, 150.0], [left, 150.0]],
                holes: Vec::new(),
            }]),
            fill: Some(LAND),
        },
    ];
    if let Some(on_art_board) = on_art_board {
        commands.push(Command::SetMembership {
            ids: vec![block],
            membership: if on_art_board {
                Membership::Board(art)
            } else {
                Membership::Canvas
            },
        });
    }
    for command in commands {
        map.execute(&command).expect("the Command applies");
    }
    map
}

fn render(headless: &Headless, map: &Map, view: &View) -> RgbaImage {
    let content = MapScene::new(Fonts::bundled()).content(map, &mut RoughnessCache::new());
    let scene = CanvasScene::for_map(map).with_content(content);
    let mut renderer =
        CanvasRenderer::new(&headless.device, GpuCapabilities::query(&headless.adapter));
    render_to_image(
        &headless.device,
        &headless.queue,
        &mut renderer,
        &scene,
        view,
    )
    .expect("the readback succeeds")
}

/// A 600 x 300 px view from (-50, -50): the Canvas covers pixels 50 to 250 across, and an
/// Art board with its corner at (300, 0) covers 350 to 550.
fn view() -> View {
    View {
        origin: [-50.0, -50.0],
        zoom: 1.0,
        size: [600, 300],
    }
}

#[test]
fn each_board_draws_its_own_background() {
    let Some(headless) = gpu() else { return };
    let image = render(&headless, &map([300.0, 0.0], [0.0, 0.0], None), &view());
    let outside = pixel(&image, 5, 5);
    assert!(!is(outside, WATER) && !is(outside, ART_WATER));
    assert!(is(pixel(&image, 150, 150), WATER), "the Canvas");
    assert!(is(pixel(&image, 450, 150), ART_WATER), "the Art board");
    assert_eq!(pixel(&image, 300, 150), outside, "between the boards");
}

#[test]
fn an_art_board_clips_its_members_at_its_own_edge() {
    let Some(headless) = gpu() else { return };
    // The block runs from between the boards to the middle of the Art board.
    let map = map([300.0, 0.0], [250.0, 450.0], Some(true));
    let image = render(&headless, &map, &view());
    let outside = pixel(&image, 5, 5);
    assert_eq!(
        pixel(&image, 330, 150),
        outside,
        "clipped between the boards"
    );
    assert!(is(pixel(&image, 380, 150), LAND), "inside the Art board");
    assert!(is(pixel(&image, 450, 150), LAND), "inside the Art board");
    assert!(is(pixel(&image, 520, 150), ART_WATER), "past the block");
}

#[test]
fn a_canvas_member_is_not_shown_on_an_art_board() {
    let Some(headless) = gpu() else { return };
    // The block reaches from the Canvas into the Art board but belongs to the Canvas.
    let map = map([300.0, 0.0], [100.0, 450.0], Some(false));
    let image = render(&headless, &map, &view());
    let outside = pixel(&image, 5, 5);
    assert!(is(pixel(&image, 200, 150), LAND), "inside the Canvas");
    assert_eq!(
        pixel(&image, 300, 150),
        outside,
        "clipped at the Canvas edge"
    );
    assert!(
        is(pixel(&image, 450, 150), ART_WATER),
        "not on the Art board"
    );
}

#[test]
fn an_object_off_every_board_is_drawn_in_full() {
    let Some(headless) = gpu() else { return };
    let mut map = map([300.0, 0.0], [250.0, 450.0], None);
    let block = map.view().layers[0].children[0].id;
    map.execute(&Command::SetMembership {
        ids: vec![block],
        membership: Membership::OffCanvas,
    })
    .unwrap();
    let image = render(&headless, &map, &view());
    assert!(is(pixel(&image, 330, 150), LAND), "between the boards");
    assert!(is(pixel(&image, 450, 150), LAND), "over the Art board");
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
fn export_covers_only_the_canvas_and_leaves_art_board_members_out() {
    let Some(headless) = gpu() else { return };
    // The Art board overlaps the Canvas, and its block sits where both are.
    let map = map([100.0, 0.0], [120.0, 180.0], Some(true));
    for tiles in [None, Some(128)] {
        let image = export(&headless, &map, tiles);
        assert_eq!((image.width, image.height), (200, 200));
        assert!(
            image
                .pixels
                .chunks_exact(4)
                .all(|p| is(p.try_into().unwrap(), WATER)),
            "only the Canvas's water, tiles {tiles:?}"
        );
    }
}

#[test]
fn export_still_holds_the_canvas_members() {
    let Some(headless) = gpu() else { return };
    let map = map([300.0, 0.0], [50.0, 150.0], Some(false));
    let image = export(&headless, &map, None);
    assert!(is(pixel(&image, 100, 100), LAND));
    assert!(is(pixel(&image, 10, 10), WATER));
}
