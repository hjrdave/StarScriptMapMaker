//! Pan and zoom frame times on a large Map (spec §7 and §11).
//!
//! The Map is the one `core`'s `large_map` benchmark builds: 100k Stamps and about 1M
//! coastline vertices. It renders on a software adapter (lavapipe, WARP), so the numbers are
//! reproducible and show the cost of a frame, not what a real GPU would do. It prints where
//! the time goes (scene build, tessellation and instance upload on the first frame, encoding,
//! GPU execution, readback), then fails if the mean pan or zoom frame is over [`FRAME_LIMIT`].
//! `SSMAP_BENCH_SCALE` (default 1.0) shrinks the Map for quick local runs and turns the guard
//! off below 1.0. `SSMAP_BENCH_ALLOW_HARDWARE=1` falls back to a real GPU.
//!
//! Run with `cargo bench -p ssmap-render --bench pan_zoom`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use std::f64::consts::TAU;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ssmap_core::asset::{PackLibrary, PackRoots};
use ssmap_core::{
    AssetRef, Canvas, Command, DistanceUnit, Id, Landmass, Layer, Map, MultiPolygon, Node, Object,
    ObjectKind, Op, PlacedStamp, Polygon, RoughnessCache, Scale, ShoreKind, ShoreStyle, Stamp,
    Subtree,
};
use ssmap_render::{
    CanvasRenderer, CanvasScene, Fonts, GpuCapabilities, Headless, MapScene, Quality, SpriteStore,
    View, read_texture, wgpu,
};

const STAMPS: usize = 100_000;
const LANDMASSES: usize = 20;
/// Small islands with ripples and shading.
const ISLANDS: usize = 20;
const VERTICES_PER_LANDMASS: usize = 50_000;
const FRAMES: u32 = 12;

/// The mean pan or zoom frame on a software adapter may not exceed this. It is loose on
/// purpose: it catches a return to drawing the whole Map every frame, not noise.
const FRAME_LIMIT: Duration = Duration::from_millis(1500);

fn scale() -> f64 {
    std::env::var("SSMAP_BENCH_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0)
}

fn coastline(centre: [f64; 2], vertices: usize) -> MultiPolygon {
    coastline_of(centre, 1_000.0, vertices)
}

fn coastline_of(centre: [f64; 2], radius: f64, vertices: usize) -> MultiPolygon {
    let ring = (0..vertices)
        .map(|i| {
            let t = i as f64 / vertices as f64 * TAU;
            let r = radius + radius * 0.04 * (t * 37.0).sin();
            [centre[0] + r * t.cos(), centre[1] + r * t.sin()]
        })
        .collect();
    MultiPolygon(vec![Polygon {
        exterior: ring,
        holes: Vec::new(),
    }])
}

fn insert(map: &mut Map, parent: Option<Id>, node: Node) -> Id {
    let id = Id::new();
    let index = map.children(parent).len();
    map.apply(Op::Insert {
        parent,
        index,
        subtree: Subtree {
            id,
            node,
            children: Vec::new(),
        },
    })
    .unwrap();
    id
}

fn large_map(scale: f64) -> Map {
    let mut map = Map::new(Canvas::new(
        16_384,
        16_384,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    // The big coastlines have an outline only: ripples and shading offset every vertex into
    // several more rings, which makes building the scene take many minutes at this size. The
    // small islands below carry the full Shore style so the effects are in view.
    let mut outline_only = ShoreStyle::landmass_default();
    outline_only.ripples.count = 0;
    outline_only.shading.width = 0.0;
    map.execute(&Command::SetDefaultShore {
        kind: ShoreKind::Landmass,
        shore: outline_only,
    })
    .unwrap();
    let layer = insert(&mut map, None, Node::Layer(Layer::new("Bench")));
    let mut islands = Vec::new();
    for i in 0..ISLANDS {
        let object = Object::new(ObjectKind::Landmass(Landmass {
            geometry: coastline_of(
                [
                    1_200.0 + (i % 5) as f64 * 700.0,
                    1_200.0 + (i / 5) as f64 * 700.0,
                ],
                150.0,
                1_000,
            ),
            ..Default::default()
        }));
        islands.push(insert(&mut map, Some(layer), Node::Object(object)));
    }
    map.execute(&Command::SetShore {
        ids: islands,
        shore: Some(ShoreStyle::landmass_default()),
    })
    .unwrap();
    for i in 0..LANDMASSES {
        let geometry = coastline(
            [
                1_500.0 + (i % 5) as f64 * 3_000.0,
                1_500.0 + (i / 5) as f64 * 3_000.0,
            ],
            (VERTICES_PER_LANDMASS as f64 * scale) as usize,
        );
        let object = Object::new(ObjectKind::Landmass(Landmass {
            geometry,
            ..Default::default()
        }));
        insert(&mut map, Some(layer), Node::Object(object));
    }
    let assets = ["test-pack/mountain-1", "test-pack/castle", "test-pack/tree"];
    let stamps = (0..(STAMPS as f64 * scale) as usize)
        .map(|i| PlacedStamp {
            id: Id::new(),
            stamp: Stamp::new(
                assets[i % assets.len()].parse::<AssetRef>().unwrap(),
                [
                    (i % 400) as f64 * 20.0 + 500.0,
                    (i / 400) as f64 * 20.0 + 500.0,
                ],
            ),
        })
        .collect();
    map.execute(&Command::PlaceStamps {
        parent: layer,
        stamps,
    })
    .unwrap();
    map
}

fn sprites() -> SpriteStore {
    let roots = PackRoots {
        built_in: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../core/tests/packs"),
        user: None,
    };
    let (library, problems) = PackLibrary::load(&roots);
    assert!(problems.is_empty(), "{problems:?}");
    SpriteStore::synchronous(Arc::new(library))
}

/// Renders one frame and waits for the GPU, returning the encode and GPU times.
fn frame(
    gpu: &Headless,
    renderer: &mut CanvasRenderer,
    scene: &CanvasScene,
    view: &View,
) -> (Duration, Duration) {
    let started = Instant::now();
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    renderer.render(&gpu.device, &gpu.queue, &mut encoder, scene, view);
    let encoded = started.elapsed();
    gpu.queue.submit([encoder.finish()]);
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    (encoded, started.elapsed() - encoded)
}

/// Times a frame at each of `views`, prints the means and returns the mean frame time.
fn run(
    name: &str,
    gpu: &Headless,
    renderer: &mut CanvasRenderer,
    scene: &CanvasScene,
    views: &[View],
) -> Duration {
    let (mut encode, mut execute) = (Duration::ZERO, Duration::ZERO);
    for view in views {
        let (e, x) = frame(gpu, renderer, scene, view);
        encode += e;
        execute += x;
    }
    let n = views.len() as u32;
    let (encode, execute) = (encode / n, execute / n);
    println!(
        "{name}: {:?} per frame (encode {encode:?}, GPU {execute:?})",
        encode + execute
    );
    encode + execute
}

fn main() {
    let scale = scale();
    let gpu = match Headless::new(true) {
        Ok(gpu) => gpu,
        Err(err) if std::env::var_os("SSMAP_BENCH_ALLOW_HARDWARE").is_some() => {
            println!("no software adapter ({err}); using a hardware adapter");
            Headless::new(false).expect("a GPU adapter")
        }
        Err(err) => {
            println!("skipping: {err}");
            return;
        }
    };
    let capabilities = GpuCapabilities::query(&gpu.adapter);
    println!(
        "adapter: {} ({:?}), MSAA {}x, scale {scale}",
        capabilities.info.name, capabilities.info.device_type, capabilities.msaa_samples
    );

    let map = large_map(scale);
    let started = Instant::now();
    let content = MapScene::new(Fonts::bundled()).content(&map, &mut RoughnessCache::new());
    println!("scene build: {:?}", started.elapsed());
    let scene = CanvasScene::new(map.canvas()).with_content(content);

    let mut renderer = CanvasRenderer::new(&gpu.device, capabilities);
    renderer.set_sprites(sprites());
    let start = View {
        origin: [1_000.0, 1_000.0],
        zoom: 0.5,
        size: [1280, 720],
    };
    let started = Instant::now();
    frame(&gpu, &mut renderer, &scene, &start);
    println!(
        "first frame (tessellation, instance upload, draw): {:?}",
        started.elapsed()
    );

    let mut pans = Vec::new();
    let mut view = start;
    for _ in 0..FRAMES {
        view.pan([-24.0, -12.0]);
        pans.push(view);
    }
    let mut zooms = Vec::new();
    let mut view = start;
    for i in 0..FRAMES {
        view.zoom_at([640.0, 360.0], if i < FRAMES / 2 { 0.9 } else { 1.1 });
        zooms.push(view);
    }
    // The whole Canvas in view is the worst case: nothing can be skipped.
    let overview = View {
        origin: [0.0, 0.0],
        zoom: 0.044,
        size: [1280, 720],
    };
    run("overview", &gpu, &mut renderer, &scene, &[overview; 3]);
    let pan = run("pan", &gpu, &mut renderer, &scene, &pans);
    let zoom = run("zoom", &gpu, &mut renderer, &scene, &zooms);
    let gesture = scene.clone().with_quality(Quality::Interactive);
    run("pan mid-gesture", &gpu, &mut renderer, &gesture, &pans);

    let started = Instant::now();
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let texture = renderer
        .render(&gpu.device, &gpu.queue, &mut encoder, &scene, &start)
        .clone();
    gpu.queue.submit([encoder.finish()]);
    read_texture(&gpu.device, &gpu.queue, &texture).expect("the readback succeeds");
    println!("frame with readback: {:?}", started.elapsed());

    if scale >= 1.0 {
        let mut failed = false;
        for (name, took) in [("pan", pan), ("zoom", zoom)] {
            if took > FRAME_LIMIT {
                eprintln!("error: mean {name} frame {took:?} is over the {FRAME_LIMIT:?} limit");
                failed = true;
            }
        }
        if failed {
            std::process::exit(1);
        }
    }
}
