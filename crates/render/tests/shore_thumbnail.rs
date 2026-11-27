//! Shore preset thumbnails (issue 651): the sample coast, and its render for each preset.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::{Background, Color, Map, Node, ShoreKind, ShoreStyle};
use ssmap_render::{
    GpuCapabilities, Headless, RgbaImage, SHORE_THUMBNAIL_SCALE, SHORE_THUMBNAIL_SIZE, ShoreSample,
    ShoreThumbnailer, shore_sample_map,
};

const LAND: Color = Color::rgb(0xc8, 0xb4, 0x7c);
const WATER: Color = Color::rgb(0x20, 0x60, 0xa0);

fn sample() -> ShoreSample {
    let mut background = Background {
        water: WATER,
        ..Background::default()
    };
    background.texture.strength = 0.0;
    ShoreSample {
        land: LAND,
        background,
    }
}

fn gpu() -> Option<Headless> {
    Headless::new(true).or_else(|_| Headless::new(false)).ok()
}

/// The Objects of `map` with their own Shore style.
fn shored(map: &Map) -> Vec<ShoreStyle> {
    map.ids()
        .filter_map(|id| match map.node(id) {
            Some(Node::Object(object)) => object.kind.shore().cloned(),
            _ => None,
        })
        .collect()
}

#[test]
fn the_sample_has_the_style_on_one_object_and_the_map_s_colours() {
    for kind in [ShoreKind::Landmass, ShoreKind::Lake] {
        for (name, style) in ShoreStyle::presets(kind) {
            let map = shore_sample_map(kind, &style, &sample()).unwrap();
            assert_eq!(shored(&map), vec![style], "{kind:?} {name}");
            assert_eq!(map.canvas().background, sample().background);
            assert_eq!(
                [map.canvas().width, map.canvas().height],
                SHORE_THUMBNAIL_SIZE
            );
        }
    }
}

#[test]
fn the_sample_of_a_map_takes_its_land_fill_and_background() {
    let map = Map::new(ssmap_core::Canvas::new(
        10,
        10,
        ssmap_core::Scale::new(1.0, ssmap_core::DistanceUnit::Miles),
    ));
    let of = ShoreSample::of(&map);
    assert_eq!(of.land, map.defaults().land_fill);
    assert_eq!(of.background, map.canvas().background);
}

#[test]
fn a_style_out_of_range_is_an_error_not_a_panic() {
    let mut style = ShoreStyle::landmass_default();
    style.outline.width = -1.0;
    assert!(shore_sample_map(ShoreKind::Landmass, &style, &sample()).is_err());
}

fn render(
    thumbnailer: &mut ShoreThumbnailer,
    gpu: &Headless,
    kind: ShoreKind,
    name: &str,
) -> RgbaImage {
    let style = ShoreStyle::picked(kind, name).unwrap();
    thumbnailer
        .render(&gpu.device, &gpu.queue, kind, &style, &sample())
        .unwrap()
}

#[test]
fn each_landmass_preset_renders_its_own_thumbnail() {
    let Some(gpu) = gpu() else {
        eprintln!("skipping render test: no adapter");
        return;
    };
    let capabilities = GpuCapabilities::query(&gpu.adapter, &gpu.device);
    let mut thumbnailer = ShoreThumbnailer::new(&gpu.device, capabilities);
    let images: Vec<_> = ShoreStyle::presets(ShoreKind::Landmass)
        .into_iter()
        .map(|(name, _)| render(&mut thumbnailer, &gpu, ShoreKind::Landmass, name))
        .collect();
    for image in &images {
        assert_eq!(
            [image.width, image.height],
            SHORE_THUMBNAIL_SIZE.map(|n| n * SHORE_THUMBNAIL_SCALE)
        );
        // Land on the left edge, water on the right.
        let at = |x: u32| {
            let i = ((image.height / 2 * image.width + x) * 4) as usize;
            [image.pixels[i], image.pixels[i + 1], image.pixels[i + 2]]
        };
        assert!(at(2)[0] > at(2)[2], "land at the left: {:?}", at(2));
        let right = at(image.width - 3);
        assert!(right[2] > right[0], "water at the right: {right:?}");
    }
    for (i, a) in images.iter().enumerate() {
        for b in &images[i + 1..] {
            assert_ne!(a.pixels, b.pixels, "each preset draws differently");
        }
    }
    // The same style draws the same again.
    assert_eq!(
        render(&mut thumbnailer, &gpu, ShoreKind::Landmass, "Default"),
        images[0]
    );
}

#[test]
fn a_lake_thumbnail_is_land_around_water() {
    let Some(gpu) = gpu() else {
        eprintln!("skipping render test: no adapter");
        return;
    };
    let capabilities = GpuCapabilities::query(&gpu.adapter, &gpu.device);
    let mut thumbnailer = ShoreThumbnailer::new(&gpu.device, capabilities);
    let image = render(&mut thumbnailer, &gpu, ShoreKind::Lake, "Default");
    let at = |x: u32, y: u32| {
        let i = ((y * image.width + x) * 4) as usize;
        [image.pixels[i], image.pixels[i + 1], image.pixels[i + 2]]
    };
    let corner = at(1, 1);
    assert!(corner[0] > corner[2], "land in the corner: {corner:?}");
    let middle = at(image.width / 2, image.height / 2);
    assert!(middle[2] > middle[0], "water in the middle: {middle:?}");
}
