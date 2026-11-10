//! What many paint strokes cost a frame (ADR 0017).
//!
//! A Map with one big Landmass and `SSMAP_BENCH_STROKES` (default 5 000) soft paint strokes,
//! some of them erasers, is drawn on a software adapter (lavapipe, WARP) with no strokes and
//! then with them. It prints the first frame, which builds the scene and tessellates every
//! stroke, and the mean steady frame, which draws them again from the kept geometry, both
//! with the strokes and without. Nothing fails: the numbers are for comparing changes.
//! `SSMAP_BENCH_ERASE_EVERY` (default 50, 0 for none) makes every that-many-th stroke an
//! eraser, each of which costs the strokes before it another pass over the land.
//! `SSMAP_BENCH_ALLOW_HARDWARE=1` falls back to a real GPU.
//!
//! Run with `cargo bench -p ssmap-render --bench paint`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use std::time::{Duration, Instant};

use ssmap_core::{
    Canvas, Color, Command, DistanceUnit, Id, Map, MultiPolygon, PaintStroke, Polygon,
    RoughnessCache, Scale,
};
use ssmap_render::{
    CanvasRenderer, CanvasScene, Fonts, GpuCapabilities, Headless, MapScene, View, wgpu,
};

const FRAMES: u32 = 10;
const SIDE: u32 = 4_000;

fn strokes() -> usize {
    std::env::var("SSMAP_BENCH_STROKES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5_000)
}

fn map_with(strokes: usize) -> Map {
    let mut map = Map::new(Canvas::new(
        SIDE,
        SIDE,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Land".into(),
    })
    .unwrap();
    let side = f64::from(SIDE);
    map.execute(&Command::CreateLandmass {
        id: Id::new(),
        parent: layer,
        index: None,
        geometry: MultiPolygon(vec![Polygon {
            exterior: vec![
                [0.1 * side, 0.1 * side],
                [0.9 * side, 0.1 * side],
                [0.9 * side, 0.9 * side],
                [0.1 * side, 0.9 * side],
            ],
            holes: Vec::new(),
        }]),
        fill: None,
    })
    .unwrap();
    let erase_every: usize = std::env::var("SSMAP_BENCH_ERASE_EVERY")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);
    for i in 0..strokes {
        // A wavy 20-point drag somewhere on the land; every 50th stroke is an eraser.
        let (x, y) = (
            0.15 * side + (i * 7919 % 3_000) as f64,
            0.15 * side + (i * 104_729 % 3_000) as f64,
        );
        let points = (0..20)
            .map(|k| {
                [
                    x + f64::from(k) * 6.0,
                    y + (f64::from(k) * 0.7).sin() * 12.0,
                ]
            })
            .collect();
        map.execute(&Command::AddPaintStroke {
            stroke: PaintStroke {
                id: Id::new(),
                points,
                color: Color::rgb(200, 60, 60),
                size: 24.0,
                softness: 0.5,
                opacity: 0.8,
                erase: erase_every > 0 && i % erase_every == erase_every - 1,
            },
        })
        .unwrap();
    }
    map
}

fn frame(
    gpu: &Headless,
    renderer: &mut CanvasRenderer,
    scene: &CanvasScene,
    view: &View,
) -> Duration {
    let started = Instant::now();
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    renderer.render(&gpu.device, &gpu.queue, &mut encoder, scene, view);
    gpu.queue.submit([encoder.finish()]);
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    started.elapsed()
}

fn measure(gpu: &Headless, label: &str, strokes: usize) {
    let map = map_with(strokes);
    let mut renderer = CanvasRenderer::new(&gpu.device, GpuCapabilities::query(&gpu.adapter));
    let started = Instant::now();
    let content = MapScene::new(Fonts::bundled()).content(&map, &mut RoughnessCache::new());
    let built = started.elapsed();
    let scene = CanvasScene::new(map.canvas()).with_content(content);
    // The whole Canvas in a 1600 x 1200 window, and a part of it at the same size.
    let whole = View::fit([f64::from(SIDE); 2], [1600, 1200], 0.0);
    let zoomed = View {
        origin: [1_000.0, 1_000.0],
        zoom: 0.5,
        size: [1600, 1200],
    };
    for (name, view) in [("whole Canvas", &whole), ("zoomed in", &zoomed)] {
        let first = frame(gpu, &mut renderer, &scene, view);
        let steady: Duration = (0..FRAMES)
            .map(|_| frame(gpu, &mut renderer, &scene, view))
            .sum::<Duration>()
            / FRAMES;
        println!(
            "{label:>14} {name:>12}: scene {built:>9.1?}, first frame {first:>9.1?}, steady {steady:>9.1?}"
        );
    }
}

fn main() {
    let allow_hardware = std::env::var_os("SSMAP_BENCH_ALLOW_HARDWARE").is_some();
    let Some(gpu) = Headless::new(true)
        .ok()
        .or_else(|| allow_hardware.then(|| Headless::new(false).ok()).flatten())
    else {
        println!("skipping: no software adapter (set SSMAP_BENCH_ALLOW_HARDWARE=1 to use a GPU)");
        return;
    };
    let n = strokes();
    measure(&gpu, "no strokes", 0);
    measure(&gpu, &format!("{n} strokes"), n);
}
