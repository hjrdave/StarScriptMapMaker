//! Tiled export (ADR 0007): no seams between tiles, hidden Layers absent and locked ones
//! present, and an export started during a rebuild equal to one made after it.
//!
//! Needs a GPU adapter; without one each test returns early, as the golden tests do.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ssmap_core::asset::{PackLibrary, PackRoots};
use ssmap_core::{
    Canvas, Color, Command, DistanceUnit, ExportSettings, Id, Map, MultiPolygon, Polygon,
    Roughness, RoughnessCache, Scale, SizeMode, Stamp,
};
use ssmap_render::{
    CanvasRenderer, ExportContext, ExportJob, ExportStatus, ExportStep, Fonts, GpuCapabilities,
    Headless, MapScene, PngMeta, RgbaImage, SpriteStore, TiledExport,
};

fn gpu() -> Option<Headless> {
    Headless::new(true).or_else(|_| Headless::new(false)).ok()
}

fn library() -> Arc<PackLibrary> {
    let roots = PackRoots {
        built_in: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../core/tests/packs"),
        user: None,
    };
    let (library, problems) = PackLibrary::load(&roots);
    assert!(problems.is_empty(), "{problems:?}");
    Arc::new(library)
}

fn execute(map: &mut Map, command: Command) {
    map.execute(&command).expect("the Command applies");
}

fn layer(map: &mut Map, name: &str) -> Id {
    let id = Id::new();
    execute(
        map,
        Command::CreateLayer {
            id,
            parent: None,
            index: None,
            name: name.into(),
        },
    );
    id
}

/// A block spanning `min_x` to `max_x` and the middle of the 400-unit-high Canvas.
fn square(min_x: f64, max_x: f64) -> MultiPolygon {
    MultiPolygon(vec![Polygon {
        exterior: vec![[min_x, 50.0], [max_x, 50.0], [max_x, 350.0], [min_x, 350.0]],
        holes: Vec::new(),
    }])
}

fn landmass(map: &mut Map, parent: Id, geometry: MultiPolygon, fill: Color) {
    execute(
        map,
        Command::CreateLandmass {
            id: Id::new(),
            parent,
            index: None,
            geometry,
            fill: Some(fill),
        },
    );
}

fn canvas() -> Canvas {
    Canvas::new(600, 400, Scale::new(1.0, DistanceUnit::Miles))
}

/// Settings for an image `width` pixels wide.
fn settings(width: u32) -> ExportSettings {
    ExportSettings {
        size: SizeMode::Width { pixels: width },
        ..Default::default()
    }
}

struct Setup {
    renderer: CanvasRenderer,
    scene: MapScene,
    roughness: RoughnessCache,
}

impl Setup {
    fn new(headless: &Headless, sprites: SpriteStore) -> Self {
        let mut renderer =
            CanvasRenderer::new(&headless.device, GpuCapabilities::query(&headless.adapter));
        renderer.set_sprites(sprites);
        Self {
            renderer,
            scene: MapScene::new(Fonts::bundled()),
            roughness: RoughnessCache::new(),
        }
    }

    fn export(
        &mut self,
        headless: &Headless,
        map: &Map,
        settings: &ExportSettings,
        max_tile_side: Option<u32>,
        status: impl FnMut(ExportStatus),
    ) -> RgbaImage {
        let mut pixels = Vec::new();
        let size = TiledExport {
            device: &headless.device,
            queue: &headless.queue,
            renderer: &mut self.renderer,
            scene: &mut self.scene,
            map,
            roughness: &mut self.roughness,
            settings,
            max_tile_side,
        }
        .run(status, |row| {
            pixels.extend_from_slice(row);
            Ok::<_, Infallible>(())
        })
        .expect("the export renders");
        assert_eq!(pixels.len(), size.width as usize * size.height as usize * 4);
        RgbaImage {
            width: size.width,
            height: size.height,
            pixels,
        }
    }
}

fn pixel(image: &RgbaImage, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * image.width + x) * 4) as usize;
    image.pixels[i..i + 4].try_into().unwrap()
}

/// A Map with rough, rippled coasts, so effects reach across tile edges.
fn coastal_map() -> Map {
    let mut map = Map::new(canvas());
    execute(
        &mut map,
        Command::SetDefaultRoughness {
            roughness: Roughness::new(8.0, 40.0, 5),
        },
    );
    let land = layer(&mut map, "Land");
    landmass(
        &mut map,
        land,
        square(60.0, 330.0),
        Color::rgb(0xc8, 0xb4, 0x7c),
    );
    landmass(
        &mut map,
        land,
        square(380.0, 560.0),
        Color::rgb(0x8c, 0xa8, 0x5c),
    );
    map
}

#[test]
fn tiles_leave_no_seams() {
    let Some(headless) = gpu() else { return };
    let map = coastal_map();
    let settings = settings(600);
    let mut setup = Setup::new(&headless, SpriteStore::synchronous(library()));
    let whole = setup.export(&headless, &map, &settings, None, |_| {});
    // A 256px limit makes 128px tiles: a 5 x 4 grid, with seams at every multiple of 128.
    let mut last = None;
    let tiled = setup.export(&headless, &map, &settings, Some(256), |status| {
        last = Some(status);
    });
    assert_eq!(
        last,
        Some(ExportStatus::Rendering {
            done: 20,
            total: 20
        })
    );
    assert_eq!((tiled.width, tiled.height), (whole.width, whole.height));
    let mut differing = Vec::new();
    for y in 0..whole.height {
        for x in 0..whole.width {
            let (a, b) = (pixel(&whole, x, y), pixel(&tiled, x, y));
            if a.iter().zip(b).any(|(a, b)| a.abs_diff(b) > 3) {
                differing.push((x, y));
            }
        }
    }
    // Tiles are rendered at a different origin than the whole image, so on some GPUs (Metal)
    // an antialiased coast pixel can round differently. A real seam would differ along a whole
    // tile edge (hundreds of pixels), so a handful of isolated pixels is tolerated. CI saw one
    // such pixel, well away from any tile boundary.
    const ROUNDING_PIXELS: usize = 8;
    assert!(
        differing.len() <= ROUNDING_PIXELS,
        "{} pixels differ, first at {:?}",
        differing.len(),
        differing.first()
    );
}

#[test]
fn hidden_layers_are_absent_and_locked_layers_present() {
    let Some(headless) = gpu() else { return };
    let mut map = Map::new(canvas());
    let hidden_fill = Color::rgb(0xff, 0x00, 0x00);
    let locked_fill = Color::rgb(0x00, 0x00, 0xff);
    let hidden = layer(&mut map, "Hidden");
    landmass(&mut map, hidden, square(50.0, 250.0), hidden_fill);
    let locked = layer(&mut map, "Locked");
    landmass(&mut map, locked, square(350.0, 550.0), locked_fill);
    execute(
        &mut map,
        Command::SetVisible {
            id: hidden,
            visible: false,
        },
    );
    execute(
        &mut map,
        Command::SetLocked {
            id: locked,
            locked: true,
        },
    );

    let mut setup = Setup::new(&headless, SpriteStore::synchronous(library()));
    let image = setup.export(&headless, &map, &settings(600), Some(256), |_| {});
    let close = |actual: [u8; 4], expected: Color| {
        [expected.r, expected.g, expected.b]
            .iter()
            .zip(actual)
            .all(|(e, a)| e.abs_diff(a) <= 3)
    };
    assert!(
        close(pixel(&image, 450, 200), locked_fill),
        "locked Layer: {:?}",
        pixel(&image, 450, 200)
    );
    let water = pixel(&image, 150, 200);
    assert!(!close(water, hidden_fill), "hidden Layer is absent");
    assert!(water[2] > water[0], "ocean where the hidden Layer was");
}

/// A Map with rough coasts and Stamps, whose derived data takes time to build.
fn busy_map() -> Map {
    let mut map = coastal_map();
    let assets = layer(&mut map, "Assets");
    for i in 0..6 {
        let stamp = Stamp::new(
            "test-pack/mountain-1".parse().expect("a valid reference"),
            [100.0 + 70.0 * f64::from(i), 200.0],
        );
        execute(
            &mut map,
            Command::CreateStamp {
                id: Id::new(),
                parent: assets,
                index: None,
                stamp: Box::new(stamp),
            },
        );
    }
    map
}

#[test]
fn an_export_started_during_a_rebuild_matches_one_after_it() {
    let Some(headless) = gpu() else { return };
    let map = busy_map();
    let settings = settings(450);

    // Everything built and ready before the export starts.
    let mut after = Setup::new(&headless, SpriteStore::synchronous(library()));
    let expected = after.export(&headless, &map, &settings, Some(256), |_| {});

    // Derived data still rebuilding on other threads when the export starts.
    let mut during = Setup::new(&headless, SpriteStore::new(library()));
    let rebuild = during.roughness.rebuild(&map);
    let content = during.scene.content(&map, &mut during.roughness);
    let tessellation = during.renderer.start_tessellation(&content);
    let threads = [
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            rebuild.run(|| {});
        }),
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            tessellation.run(|| {});
        }),
    ];
    let mut statuses = Vec::new();
    let actual = during.export(&headless, &map, &settings, Some(256), |status| {
        statuses.push(status);
    });
    for thread in threads {
        thread.join().expect("the rebuild finishes");
    }
    assert_eq!(statuses[0], ExportStatus::WaitingForDerivedData);
    assert!(matches!(
        statuses[1],
        ExportStatus::Rendering { done: 0, .. }
    ));
    assert!(
        actual == expected,
        "an export during a rebuild differs from one after it"
    );
}

fn step(job: &mut ExportJob, headless: &Headless, setup: &mut Setup, map: &Map) -> ExportStep {
    job.step(&mut ExportContext {
        device: &headless.device,
        queue: &headless.queue,
        renderer: &mut setup.renderer,
        scene: &mut setup.scene,
        map,
        roughness: &mut setup.roughness,
    })
    .expect("the step works")
}

#[test]
fn a_stepped_export_writes_the_same_pixels_as_a_streamed_one() {
    let Some(headless) = gpu() else { return };
    let map = coastal_map();
    let settings = settings(300);
    let mut setup = Setup::new(&headless, SpriteStore::synchronous(library()));
    let expected = setup.export(&headless, &map, &settings, Some(256), |_| {});

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("map.png");
    let mut job = ExportJob::new(&map, &settings, PngMeta::default(), &dest)
        .unwrap()
        .with_max_tile_side(256);
    let mut steps = 0;
    let size = loop {
        steps += 1;
        match step(&mut job, &headless, &mut setup, &map) {
            ExportStep::Working(_) => assert!(!dest.exists(), "no file until the end"),
            ExportStep::Finished(size) => break size,
        }
    };
    assert!(steps > 2, "one tile per step");
    assert_eq!((size.width, size.height), (expected.width, expected.height));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    let file = std::fs::File::open(&dest).unwrap();
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut pixels).unwrap();
    assert!(pixels == expected.pixels);
}

#[test]
fn cancelling_leaves_no_file() {
    let Some(headless) = gpu() else { return };
    let map = coastal_map();
    let mut setup = Setup::new(&headless, SpriteStore::synchronous(library()));
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("map.png");
    let mut job = ExportJob::new(&map, &settings(300), PngMeta::default(), &dest)
        .unwrap()
        .with_max_tile_side(256);
    // Past the wait and into the tile rows, with the temporary file open.
    while !matches!(
        step(&mut job, &headless, &mut setup, &map),
        ExportStep::Working(ExportStatus::Rendering { done: 2.., .. })
    ) {}
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    job.cancel();
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn a_stepped_export_waits_while_a_rebuild_runs() {
    let Some(headless) = gpu() else { return };
    let map = coastal_map();
    let mut setup = Setup::new(&headless, SpriteStore::synchronous(library()));
    let rebuild = setup.roughness.rebuild(&map);
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("a.png");
    let mut job = ExportJob::new(&map, &settings(300), PngMeta::default(), &dest).unwrap();
    for _ in 0..3 {
        assert_eq!(
            step(&mut job, &headless, &mut setup, &map),
            ExportStep::Working(ExportStatus::WaitingForDerivedData)
        );
    }
    rebuild.run(|| {});
    while !matches!(
        step(&mut job, &headless, &mut setup, &map),
        ExportStep::Finished(_)
    ) {}
    assert!(dest.exists());
}
