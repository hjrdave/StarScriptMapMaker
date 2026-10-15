//! Open to first usable frame on a large Map (spec §7 and §11, ADR 0005).
//!
//! The Map is the size of the one `core`'s `large_map` benchmark builds: 100k Stamps and about
//! 1M coastline vertices. It is saved, then timed from opening the file to the first frame
//! drawn on a software adapter (lavapipe, WARP), the way the editor does it: the scene is
//! built and a frame drawn while tessellation and Stamp rasterisation carry on in the
//! background. It then waits for that background work and prints how long it took, and fails
//! if the first usable frame is over 2x the 3 s open target, like the other benchmark guards.
//! `SSMAP_BENCH_SCALE` (default 1.0) shrinks the Map for quick local runs and turns the guard
//! off below 1.0. `SSMAP_BENCH_ALLOW_HARDWARE=1` falls back to a real GPU.
//!
//! Run with `cargo bench -p ssmap-render --bench open_to_frame`.

use std::f64::consts::TAU;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ssmap_core::asset::{PackLibrary, PackRoots};
use ssmap_core::file::MapFile;
use ssmap_core::{
    AssetRef, Canvas, Command, DistanceUnit, Id, Landmass, Layer, Map, MultiPolygon, Node, Object,
    ObjectKind, Op, PlacedStamp, Polygon, RoughnessCache, Scale, ShoreKind, ShoreStyle, Stamp,
    Subtree,
};
use ssmap_render::{
    CanvasRenderer, CanvasScene, Fonts, GpuCapabilities, Headless, MapScene, SpriteStore, View,
    wgpu,
};

const STAMPS: usize = 100_000;
const LANDMASSES: usize = 20;
const VERTICES_PER_LANDMASS: usize = 50_000;

/// ADR 0005 target for open to interactive; the guard fails above twice this.
const OPEN_TARGET: Duration = Duration::from_secs(3);

fn scale() -> f64 {
    std::env::var("SSMAP_BENCH_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0)
}

fn coastline(centre: [f64; 2], vertices: usize) -> MultiPolygon {
    let ring = (0..vertices)
        .map(|i| {
            let t = i as f64 / vertices as f64 * TAU;
            let r = 1_000.0 + 40.0 * (t * 37.0).sin();
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
    // Outline only, as in the pan and zoom benchmark: ripples and shading on 1M vertices make
    // building the scene take many minutes.
    let mut outline_only = ShoreStyle::landmass_default();
    outline_only.ripples.count = 0;
    outline_only.shading.width = 0.0;
    map.execute(&Command::SetDefaultShore {
        kind: ShoreKind::Landmass,
        shore: outline_only,
    })
    .unwrap();
    let layer = insert(&mut map, None, Node::Layer(Layer::new("Bench")));
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

fn library() -> Arc<PackLibrary> {
    let roots = PackRoots {
        built_in: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../core/tests/packs"),
        user: None,
    };
    let (library, problems) = PackLibrary::load(&roots);
    assert!(problems.is_empty(), "{problems:?}");
    Arc::new(library)
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

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large.ssmap");
    MapFile::create(&path, &large_map(scale), None).unwrap();
    let library = library();

    // The renderer and its pipelines exist before a Map is opened.
    let mut renderer = CanvasRenderer::new(&gpu.device, capabilities);
    renderer.set_sprites(SpriteStore::new(library));

    let started = Instant::now();
    let (_file, map) = MapFile::open(&path).unwrap();
    let opened = started.elapsed();
    let content = MapScene::new(Fonts::bundled()).content(&map, &mut RoughnessCache::new());
    let scene_built = started.elapsed();
    // Tessellation of every Object goes on in the background while frames are drawn.
    let rebuild = renderer.start_tessellation(&content);
    let worker = std::thread::spawn(move || rebuild.run(|| {}));
    let scene = CanvasScene::new(&map.canvas).with_content(content);
    let view = View {
        origin: [1_000.0, 1_000.0],
        zoom: 0.5,
        size: [1280, 720],
    };
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    renderer.render(&gpu.device, &gpu.queue, &mut encoder, &scene, &view);
    gpu.queue.submit([encoder.finish()]);
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let first_frame = started.elapsed();

    worker.join().expect("tessellation finishes");
    renderer.wait_until_ready(&scene.content);
    let ready = started.elapsed();

    println!(
        "open to first usable frame: {first_frame:?} (target {OPEN_TARGET:?}); file open \
         {opened:?}, scene built {scene_built:?}; all background work ready {ready:?}"
    );
    if scale >= 1.0 && first_frame > OPEN_TARGET * 2 {
        eprintln!(
            "error: open to first usable frame took {first_frame:?}, over 2x the {OPEN_TARGET:?} target"
        );
        std::process::exit(1);
    }
}
