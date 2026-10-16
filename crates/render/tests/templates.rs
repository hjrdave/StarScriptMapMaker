//! Every bundled Map template renders, and the screen and the export draw the same pixels.
//!
//! Needs a GPU adapter; without one each test returns early, as the golden tests do.

use std::convert::Infallible;

use ssmap_core::{
    Canvas, DistanceUnit, ExportSettings, MapTemplate, RoughnessCache, Scale, SizeMode,
};
use ssmap_render::{
    CanvasRenderer, CanvasScene, Fonts, GpuCapabilities, Headless, MapScene, RgbaImage,
    TiledExport, View, render_to_image,
};

const SIDES: [u32; 2] = [600, 400];

fn pixel(image: &RgbaImage, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * image.width + x) * 4) as usize;
    image.pixels[i..i + 4].try_into().unwrap()
}

#[test]
fn every_template_renders_and_screen_and_export_match() {
    let Some(headless) = Headless::new(true).or_else(|_| Headless::new(false)).ok() else {
        return;
    };
    let (templates, problems) = MapTemplate::load_all();
    assert!(problems.is_empty(), "{problems:?}");
    assert!(templates.len() >= 6);

    let mut waters = Vec::new();
    for template in &templates {
        let canvas = Canvas::new(SIDES[0], SIDES[1], Scale::new(1.0, DistanceUnit::Miles));
        let map = template.new_map(canvas);
        let mut renderer =
            CanvasRenderer::new(&headless.device, GpuCapabilities::query(&headless.adapter));

        let content = MapScene::new(Fonts::bundled()).content(&map, &mut RoughnessCache::new());
        let scene = CanvasScene::new(&map.canvas).with_content(content);
        let view = View::whole_canvas(scene.canvas_size, SIDES[0]);
        let screen = render_to_image(
            &headless.device,
            &headless.queue,
            &mut renderer,
            &scene,
            &view,
        );
        assert_eq!([screen.width, screen.height], SIDES);

        let settings = ExportSettings {
            size: SizeMode::Width { pixels: SIDES[0] },
            ..Default::default()
        };
        let mut pixels = Vec::new();
        let size = TiledExport {
            device: &headless.device,
            queue: &headless.queue,
            renderer: &mut renderer,
            scene: &mut MapScene::new(Fonts::bundled()),
            map: &map,
            roughness: &mut RoughnessCache::new(),
            settings: &settings,
            max_tile_side: None,
        }
        .run(
            |_| {},
            |row| {
                pixels.extend_from_slice(row);
                Ok::<_, Infallible>(())
            },
        )
        .expect("the export renders");
        let export = RgbaImage {
            width: size.width,
            height: size.height,
            pixels,
        };
        assert_eq!(
            (export.width, export.height),
            (screen.width, screen.height),
            "{}",
            template.name()
        );

        let mut differing = 0;
        for y in 0..screen.height {
            for x in 0..screen.width {
                let (a, b) = (pixel(&screen, x, y), pixel(&export, x, y));
                if a.iter().zip(b).any(|(a, b)| a.abs_diff(b) > 3) {
                    differing += 1;
                }
            }
        }
        assert_eq!(
            differing,
            0,
            "{}: screen and export differ",
            template.name()
        );

        // The Canvas is the template's water, not the pasteboard or the default ocean.
        let centre = pixel(&screen, SIDES[0] / 2, SIDES[1] / 2);
        assert_eq!(centre[3], 0xff);
        waters.push((template.name().to_owned(), centre));
    }
    for (i, (name, water)) in waters.iter().enumerate() {
        for (other, other_water) in &waters[i + 1..] {
            assert_ne!(water, other_water, "{name} and {other} look the same");
        }
    }
}
