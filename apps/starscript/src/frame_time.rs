//! Frame-time measurement of pan and zoom on a large Map (`--frame-time`, issue #94).
//!
//! Loads a generated Map like the `large_map` benchmark's (100k Stamps, about 1M coastline
//! vertices), renders a fixed scripted pan and zoom path offscreen once, prints frame-time
//! statistics with the adapter, backend, OS and driver, and exits. It is run by hand per OS
//! on a real GPU; CI only checks it works on a software adapter.

use std::f64::consts::TAU;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ssmap_core::asset::{PackLibrary, PackRoots};
use ssmap_core::{
    AssetRef, Canvas, Command, DistanceUnit, Id, Landmass, Layer, Map, MultiPolygon, Node, Object,
    ObjectKind, Op, PlacedStamp, Polygon, RoughnessCache, Scale, Stamp, Subtree,
};
use ssmap_render::{
    CanvasRenderer, CanvasScene, Fonts, GpuCapabilities, Headless, MapScene, SpriteStore, View,
};

/// The flag that selects this mode.
pub const FLAG: &str = "--frame-time";

const STAMPS: usize = 100_000;
const LANDMASSES: usize = 20;
const VERTICES_PER_LANDMASS: usize = 50_000;
/// The render target, as for a typical editor window.
const TARGET: [u32; 2] = [1280, 800];
/// Frames per phase of the path: zoom in, pan, zoom out.
const PHASE_FRAMES: usize = 60;

/// Exit code when there is no GPU adapter at all.
pub const NO_ADAPTER: i32 = 2;

/// Whether the command line asks for the measurement.
pub fn requested(args: &[OsString]) -> bool {
    args.iter().any(|arg| arg == FLAG)
}

struct Options {
    /// Force the platform's software adapter.
    software: bool,
    /// Shrinks the Map, for tests and quick runs.
    scale: f64,
}

fn parse(args: &[OsString]) -> Result<Options, String> {
    let mut options = Options {
        software: false,
        scale: 1.0,
    };
    let mut args = args.iter().map(|arg| arg.to_string_lossy());
    while let Some(arg) = args.next() {
        match &*arg {
            FLAG => {}
            "--software-adapter" => options.software = true,
            "--scale" => {
                options.scale = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .filter(|scale: &f64| *scale > 0.0 && *scale <= 1.0)
                    .ok_or("--scale needs a number above 0 and at most 1")?;
            }
            other => return Err(format!("unexpected argument {other}")),
        }
    }
    Ok(options)
}

/// Runs the measurement, prints the report to stdout and returns the process exit code.
pub fn run(args: &[OsString]) -> i32 {
    let options = match parse(args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("error: {message}");
            return 1;
        }
    };
    let gpu = match Headless::new(options.software) {
        Ok(gpu) => gpu,
        Err(err) => {
            eprintln!("error: {err}");
            return NO_ADAPTER;
        }
    };
    let capabilities = GpuCapabilities::query(&gpu.adapter);
    let adapter = describe_adapter(&capabilities.info);
    let (library, problems) = PackLibrary::load(&PackRoots::installed());
    for problem in &problems {
        eprintln!("Asset pack problem: {problem}");
    }
    let asset = library
        .packs()
        .iter()
        .find_map(|pack| {
            let asset = pack.assets.first()?;
            AssetRef::new(pack.id.clone(), asset.id.clone()).ok()
        })
        .unwrap_or_else(|| AssetRef::new("bench", "peak").expect("a valid reference"));
    let map = large_map(options.scale, asset);

    let mut renderer = CanvasRenderer::new(&gpu.device, capabilities);
    renderer.set_sprites(SpriteStore::synchronous(Arc::new(library)));
    let mut scene = MapScene::new(Fonts::bundled());
    let content = scene.content(&map, &mut RoughnessCache::new());
    let canvas = CanvasScene::new(&map.canvas).with_content(content);

    let mut frames = Vec::new();
    for (index, view) in path(canvas.canvas_size).into_iter().enumerate() {
        let started = Instant::now();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        renderer.render(&gpu.device, &gpu.queue, &mut encoder, &canvas, &view);
        gpu.queue.submit([encoder.finish()]);
        gpu.device
            .poll(ssmap_render::wgpu::PollType::wait_indefinitely())
            .expect("the GPU finishes its work");
        // The first frame uploads the Map's geometry and Stamps, which is not pan or zoom.
        if index > 0 {
            frames.push(started.elapsed());
        }
    }
    print!("{}", report(&adapter, &Stats::new(&frames)));
    0
}

/// The scripted path: zoom into the Canvas centre, pan around, zoom back out. The same every
/// run.
fn path(canvas: [f64; 2]) -> Vec<View> {
    let mut view = View::fit(canvas, TARGET, 20.0);
    let centre = [f64::from(TARGET[0]) / 2.0, f64::from(TARGET[1]) / 2.0];
    let zoom_step = 16.0_f64.powf(1.0 / PHASE_FRAMES as f64);
    let mut views = vec![view];
    for _ in 0..PHASE_FRAMES {
        view.zoom_at(centre, zoom_step);
        views.push(view);
    }
    for i in 0..PHASE_FRAMES {
        let t = i as f64 / PHASE_FRAMES as f64 * TAU;
        view.pan([30.0 * t.cos(), 20.0 * t.sin() + 10.0]);
        views.push(view);
    }
    for _ in 0..PHASE_FRAMES {
        view.zoom_at(centre, 1.0 / zoom_step);
        views.push(view);
    }
    views
}

/// Frame-time statistics over the measured frames.
#[derive(Clone, Debug, PartialEq)]
pub struct Stats {
    pub frames: usize,
    pub mean: Duration,
    pub p95: Duration,
    pub worst: Duration,
}

impl Stats {
    pub fn new(frames: &[Duration]) -> Self {
        let mut sorted = frames.to_vec();
        sorted.sort();
        let count = sorted.len();
        let at = |rank: usize| sorted.get(rank).copied().unwrap_or_default();
        Self {
            frames: count,
            mean: sorted
                .iter()
                .sum::<Duration>()
                .checked_div(count as u32)
                .unwrap_or_default(),
            // Nearest rank: the smallest frame at least 95% of frames are no slower than.
            p95: at((count * 95).div_ceil(100).saturating_sub(1)),
            worst: at(count.saturating_sub(1)),
        }
    }
}

fn describe_adapter(info: &ssmap_render::wgpu::AdapterInfo) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "adapter: {}", info.name);
    let _ = writeln!(text, "device type: {:?}", info.device_type);
    let _ = writeln!(text, "backend: {:?}", info.backend);
    let _ = writeln!(
        text,
        "os: {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let _ = writeln!(text, "driver: {} {}", info.driver, info.driver_info);
    text
}

fn report(adapter: &str, stats: &Stats) -> String {
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    format!(
        "{adapter}frames: {}\nmean: {:.2} ms\np95: {:.2} ms\nworst: {:.2} ms\n",
        stats.frames,
        ms(stats.mean),
        ms(stats.p95),
        ms(stats.worst)
    )
}

/// A Map like the `large_map` benchmark's, scaled by `scale`.
fn large_map(scale: f64, asset: AssetRef) -> Map {
    let mut map = Map::new(Canvas::new(
        16_384,
        16_384,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    let layer = insert(&mut map, None, Node::Layer(Layer::new("Bench")));
    for i in 0..LANDMASSES {
        let geometry = coastline(
            [(i % 5) as f64 * 3_000.0, (i / 5) as f64 * 3_000.0],
            ((VERTICES_PER_LANDMASS as f64 * scale) as usize).max(3),
        );
        let object = Object::new(ObjectKind::Landmass(Landmass {
            geometry,
            ..Default::default()
        }));
        insert(&mut map, Some(layer), Node::Object(object));
    }
    let stamps = (0..(STAMPS as f64 * scale) as usize)
        .map(|i| PlacedStamp {
            id: Id::new(),
            stamp: Stamp::new(
                asset.clone(),
                [(i % 400) as f64 * 20.0, (i / 400) as f64 * 20.0],
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_use_nearest_rank() {
        let frames: Vec<Duration> = (1..=100).map(Duration::from_millis).collect();
        let stats = Stats::new(&frames);
        assert_eq!(stats.frames, 100);
        assert_eq!(stats.p95, Duration::from_millis(95));
        assert_eq!(stats.worst, Duration::from_millis(100));
        assert_eq!(stats.mean, Duration::from_micros(50_500));
        assert_eq!(Stats::new(&[]).worst, Duration::ZERO);
    }

    #[test]
    fn the_path_is_the_same_every_run() {
        assert_eq!(path([16_384.0; 2]), path([16_384.0; 2]));
        assert_eq!(path([16_384.0; 2]).len(), 3 * PHASE_FRAMES + 1);
    }
}
