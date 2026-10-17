//! Frame-time measurement on a large Map (`--frame-time`, issues #94 and #245).
//!
//! Loads a generated Map like the `large_map` benchmark's (100k Stamps, about 1M coastline
//! vertices), renders a fixed script offscreen (pan and zoom by default, or an editing
//! interaction chosen with `--script`, see [`scripts`]), prints frame-time statistics with the
//! adapter, backend, OS and driver, and exits. It is run by hand per OS on a real GPU; CI only
//! checks it works on a software adapter. `docs/design/performance.md` explains how to read it.

mod baseline;
mod scripts;

use std::collections::BTreeMap;
use std::f64::consts::TAU;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use scripts::{Script, World};
use ssmap_core::asset::{PackLibrary, PackRoots};
use ssmap_core::{
    AssetRef, Canvas, Command, DistanceUnit, Id, Landmass, Layer, Map, MultiPolygon, Node, Object,
    ObjectKind, Op, PlacedStamp, Polygon, Scale, Stamp, Subtree,
};
use ssmap_render::profile::{self, Stage, StageTimes};
use ssmap_render::{
    CanvasRenderer, CanvasScene, Fonts, GpuCapabilities, Headless, MapScene, SpriteStore, View,
};

/// The flag that selects this mode.
pub const FLAG: &str = "--frame-time";
/// Flags that run the reduced scripts against, or into, the checked-in baseline. They imply
/// `--frame-time`.
const CHECK_FLAG: &str = "--check-baseline";
const WRITE_FLAG: &str = "--write-baseline";

const STAMPS: usize = 100_000;
const LANDMASSES: usize = 20;
const VERTICES_PER_LANDMASS: usize = 50_000;
/// The render target, as for a typical editor window.
const TARGET: [u32; 2] = [1280, 800];
/// Frames per phase of the path: zoom in, pan, zoom out.
const PHASE_FRAMES: usize = 60;

/// Exit code when there is no GPU adapter at all.
pub const NO_ADAPTER: i32 = 2;
/// Exit code when `--check-baseline` finds a regression.
pub const REGRESSION: i32 = 3;

/// The reduced run the baseline is measured with: a small Map and a few frames of every script.
const REDUCED_SCALE: f64 = 0.01;
const REDUCED_FRAMES: usize = 10;

enum BaselineMode {
    Check,
    Write,
}

/// Whether the command line asks for the measurement.
pub fn requested(args: &[OsString]) -> bool {
    args.iter()
        .any(|arg| arg == FLAG || arg == CHECK_FLAG || arg == WRITE_FLAG)
}

struct Options {
    /// Force the platform's software adapter.
    software: bool,
    /// Shrinks the Map, for tests and quick runs.
    scale: f64,
    /// What to run, in order.
    scripts: Vec<Script>,
    /// Stops each script after this many frames, for tests and quick runs.
    frames: Option<usize>,
    /// Compare with, or rewrite, the checked-in baseline (issue #338).
    baseline: Option<BaselineMode>,
    baseline_path: PathBuf,
}

fn parse(args: &[OsString]) -> Result<Options, String> {
    let mut options = Options {
        software: false,
        scale: 1.0,
        scripts: vec![Script::PanZoom],
        frames: None,
        baseline: None,
        baseline_path: PathBuf::from(baseline::DEFAULT_PATH),
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
            "--frames" => {
                options.frames = Some(
                    args.next()
                        .and_then(|value| value.parse().ok())
                        .filter(|frames: &usize| *frames > 0)
                        .ok_or("--frames needs a whole number above 0")?,
                );
            }
            "--script" => {
                let name = args.next().unwrap_or_default();
                options.scripts = if name == "all" {
                    Script::ALL.to_vec()
                } else {
                    vec![Script::parse(&name).ok_or_else(|| {
                        format!("--script needs all or one of: {}", scripts::names())
                    })?]
                };
            }
            CHECK_FLAG => options.baseline = Some(BaselineMode::Check),
            WRITE_FLAG => options.baseline = Some(BaselineMode::Write),
            "--baseline-file" => {
                options.baseline_path = args
                    .next()
                    .map(|path| PathBuf::from(&*path))
                    .ok_or("--baseline-file needs a path")?;
            }
            other => return Err(format!("unexpected argument {other}")),
        }
    }
    if options.baseline.is_some() {
        // The reduced scripts on the software adapter, whatever else was asked for.
        options.software = true;
        options.scale = REDUCED_SCALE;
        options.frames = Some(REDUCED_FRAMES);
        options.scripts = Script::ALL.to_vec();
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
    let library = Arc::new(library);
    let asset = library
        .packs()
        .iter()
        .find_map(|pack| {
            let asset = pack.assets.first()?;
            AssetRef::new(pack.id.clone(), asset.id.clone()).ok()
        })
        .unwrap_or_else(|| AssetRef::new("bench", "peak").expect("a valid reference"));
    print!("{adapter}");
    #[cfg(feature = "profiling")]
    let Some(recorder) = profile::Recorder::install() else {
        eprintln!("error: another tracing subscriber is already installed");
        return 1;
    };
    #[cfg(feature = "profiling")]
    let take_stages = || Some(recorder.take());
    #[cfg(not(feature = "profiling"))]
    let take_stages = || None;
    let mut results = BTreeMap::new();
    for &script in &options.scripts {
        let capabilities = GpuCapabilities::query(&gpu.adapter);
        let measured = measure(
            &gpu,
            capabilities,
            &library,
            &asset,
            script,
            &options,
            &take_stages,
        );
        print!("{}", report(script, &measured));
        results.insert(script.name().to_string(), summarise(&measured));
    }
    match options.baseline {
        Some(BaselineMode::Check) => check_baseline(&options, &results),
        Some(BaselineMode::Write) => write_baseline(&options, results),
        None => 0,
    }
}

fn percentiles(frames: &[Duration]) -> baseline::Percentiles {
    let stats = Stats::new(frames);
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    baseline::Percentiles {
        p50: ms(stats.p50),
        p95: ms(stats.p95),
        p99: ms(stats.p99),
    }
}

/// The numbers the baseline keeps: the frame and the three coarse stages, which exist with or
/// without the `profiling` feature.
fn summarise(measured: &Measured) -> baseline::ScriptBaseline {
    baseline::ScriptBaseline {
        frame: percentiles(&measured.total),
        stages: BTreeMap::from([
            ("edit".to_string(), percentiles(&measured.edit)),
            ("scene".to_string(), percentiles(&measured.scene)),
            ("render".to_string(), percentiles(&measured.render)),
        ]),
    }
}

fn check_baseline(options: &Options, results: &BTreeMap<String, baseline::ScriptBaseline>) -> i32 {
    let path = &options.baseline_path;
    let expected = std::fs::read_to_string(path)
        .map_err(|err| err.to_string())
        .and_then(|text| {
            serde_json::from_str::<baseline::Baseline>(&text).map_err(|err| err.to_string())
        });
    let expected = match expected {
        Ok(expected) => expected,
        Err(err) => {
            eprintln!("error: cannot read the baseline {}: {err}", path.display());
            return 1;
        }
    };
    let text = baseline::diff(&expected, results);
    if baseline::compare(&expected, results).is_empty() {
        print!("\n{text}");
        0
    } else {
        eprint!("\n{text}");
        REGRESSION
    }
}

fn write_baseline(options: &Options, scripts: BTreeMap<String, baseline::ScriptBaseline>) -> i32 {
    let new = baseline::Baseline {
        note: format!(
            "Reduced scripts on the software adapter (--check-baseline). Refresh with \
             --write-baseline; see docs/design/performance.md. Written on {} {}.",
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
        scale: REDUCED_SCALE,
        frames: REDUCED_FRAMES,
        scripts,
    };
    let json = serde_json::to_string_pretty(&new).expect("the baseline serialises");
    if let Err(err) = std::fs::write(&options.baseline_path, json + "\n") {
        eprintln!(
            "error: cannot write {}: {err}",
            options.baseline_path.display()
        );
        return 1;
    }
    println!("\nwrote {}", options.baseline_path.display());
    0
}

/// The timings of one script, one entry per measured frame.
struct Measured {
    total: Vec<Duration>,
    /// Applying the frame's input to the Map: brush geometry, Commands, picking.
    edit: Vec<Duration>,
    /// Rebuilding the scene from the Map: roughness, tessellation.
    scene: Vec<Duration>,
    /// Upload and draw, until the GPU is done.
    render: Vec<Duration>,
    /// The six stages of each frame, when built with the `profiling` feature.
    stages: Vec<StageTimes>,
}

fn measure(
    gpu: &Headless,
    capabilities: GpuCapabilities,
    library: &Arc<PackLibrary>,
    asset: &AssetRef,
    script: Script,
    options: &Options,
    take_stages: &dyn Fn() -> Option<StageTimes>,
) -> Measured {
    let (map, layer) = large_map(options.scale, asset.clone());
    let mut world = World::new(map, layer, LANDMASSES, script);
    let mut renderer = CanvasRenderer::new(&gpu.device, capabilities);
    renderer.set_sprites(SpriteStore::synchronous(library.clone()));
    let mut scene = MapScene::new(Fonts::bundled());
    let canvas_size = [
        f64::from(world.map.canvas.width),
        f64::from(world.map.canvas.height),
    ];
    let views = scripts::views(script, canvas_size);
    let mut draw = |world: &mut World, view: &View| -> (Duration, Duration) {
        let started = Instant::now();
        let content = scene.content(&world.map, &mut world.cache);
        let canvas = CanvasScene::new(&world.map.canvas).with_content(content);
        let built = started.elapsed();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        renderer.render(&gpu.device, &gpu.queue, &mut encoder, &canvas, view);
        {
            let _draw = profile::stage(Stage::Draw);
            gpu.queue.submit([encoder.finish()]);
            gpu.device
                .poll(ssmap_render::wgpu::PollType::wait_indefinitely())
                .expect("the GPU finishes its work");
        }
        (built, started.elapsed() - built)
    };
    // The first frame uploads the Map's geometry and Stamps and roughens every outline, which
    // is not what the script measures.
    draw(&mut world, &views[0]);
    take_stages();
    let mut measured = Measured {
        total: Vec::new(),
        edit: Vec::new(),
        scene: Vec::new(),
        render: Vec::new(),
        stages: Vec::new(),
    };
    let frames = scripts::frames(script).min(options.frames.unwrap_or(usize::MAX));
    for frame in 0..frames {
        let view = views.get(frame + 1).unwrap_or(&views[views.len() - 1]);
        let started = Instant::now();
        {
            let _input = profile::stage(Stage::Input);
            scripts::edit(script, frame, &mut world);
        }
        let edit = started.elapsed();
        let (scene, render) = draw(&mut world, view);
        measured.total.push(started.elapsed());
        measured.stages.extend(take_stages());
        measured.edit.push(edit);
        measured.scene.push(scene);
        measured.render.push(render);
    }
    measured
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
    pub p50: Duration,
    pub p95: Duration,
    pub p99: Duration,
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
            p50: at((count * 50).div_ceil(100).saturating_sub(1)),
            p95: at((count * 95).div_ceil(100).saturating_sub(1)),
            p99: at((count * 99).div_ceil(100).saturating_sub(1)),
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

/// The 60 fps budget at p95 (issue #245), and the 120 Hz stretch goal.
const BUDGET: Duration = Duration::from_micros(16_600);
const STRETCH_BUDGET: Duration = Duration::from_micros(8_300);
/// No frame of a continuous gesture should take longer than this.
const HITCH: Duration = Duration::from_millis(33);

fn report(script: Script, measured: &Measured) -> String {
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let stats = Stats::new(&measured.total);
    let verdict = |ok: bool| if ok { "met" } else { "missed" };
    let mut text = format!(
        "\nscript: {}\nframes: {}\nmean: {:.2} ms\np50: {:.2} ms\np95: {:.2} ms\n\
         p99: {:.2} ms\nworst: {:.2} ms\n",
        script.name(),
        stats.frames,
        ms(stats.mean),
        ms(stats.p50),
        ms(stats.p95),
        ms(stats.p99),
        ms(stats.worst)
    );
    let slow = measured
        .total
        .iter()
        .filter(|&&frame| frame > HITCH)
        .count();
    let _ = writeln!(
        text,
        "budget: 60 fps (p95 under 16.6 ms) {}, 120 fps (p95 under 8.3 ms) {}, \
         {slow} frames over 33 ms",
        verdict(stats.p95 < BUDGET),
        verdict(stats.p95 < STRETCH_BUDGET),
    );
    let _ = writeln!(text, "stages (ms)      p50      p95      p99    worst");
    let rows: Vec<(&str, Vec<Duration>)> = if measured.stages.is_empty() {
        vec![
            ("edit", measured.edit.clone()),
            ("scene", measured.scene.clone()),
            ("render", measured.render.clone()),
        ]
    } else {
        Stage::ALL
            .into_iter()
            .map(|stage| {
                let times = measured.stages.iter().map(|frame| frame[stage]);
                (stage.name(), times.collect())
            })
            .collect()
    };
    for (name, frames) in &rows {
        let stats = Stats::new(frames);
        let _ = writeln!(
            text,
            "  {name:<12} {:>8.2} {:>8.2} {:>8.2} {:>8.2}",
            ms(stats.p50),
            ms(stats.p95),
            ms(stats.p99),
            ms(stats.worst)
        );
    }
    if let Some(share) = coverage(measured) {
        let _ = writeln!(
            text,
            "stages add up to {:.1}% of the frame time",
            share * 100.0
        );
    }
    text
}

/// How much of the summed frame time the stages account for, `None` without stage timing.
/// A little under 1: setup between the stages is in none of them.
fn coverage(measured: &Measured) -> Option<f64> {
    if measured.stages.is_empty() {
        return None;
    }
    let staged: Duration = measured.stages.iter().map(StageTimes::total).sum();
    let total: Duration = measured.total.iter().sum();
    (!total.is_zero()).then(|| staged.as_secs_f64() / total.as_secs_f64())
}

/// A Map like the `large_map` benchmark's, scaled by `scale`, and its Layer, which holds the
/// landmasses and then the Stamps.
fn large_map(scale: f64, asset: AssetRef) -> (Map, Id) {
    let mut map = Map::new(Canvas::new(
        16_384,
        16_384,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    let layer = insert(&mut map, None, Node::Layer(Layer::new("Bench")));
    for i in 0..LANDMASSES {
        let geometry = coastline(
            scripts::centre_of(i),
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
    (map, layer)
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
        assert_eq!(stats.p50, Duration::from_millis(50));
        assert_eq!(stats.p95, Duration::from_millis(95));
        assert_eq!(stats.p99, Duration::from_millis(99));
        assert_eq!(stats.worst, Duration::from_millis(100));
        assert_eq!(stats.mean, Duration::from_micros(50_500));
        assert_eq!(Stats::new(&[]).worst, Duration::ZERO);
    }

    #[test]
    fn the_report_lists_every_stage_and_they_add_up_to_the_frame() {
        let ms = Duration::from_millis;
        let mut measured = Measured {
            total: Vec::new(),
            edit: Vec::new(),
            scene: Vec::new(),
            render: Vec::new(),
            stages: Vec::new(),
        };
        for frame in 0..10 {
            let mut stages = StageTimes::default();
            stages[Stage::Input] = ms(1);
            stages[Stage::Update] = ms(2);
            stages[Stage::Roughness] = ms(3);
            stages[Stage::Tessellation] = ms(1);
            stages[Stage::Upload] = ms(1);
            stages[Stage::Draw] = ms(2 + frame % 2);
            // Setup between the stages is in none of them.
            measured
                .total
                .push(stages.total() + Duration::from_micros(100));
            measured.stages.push(stages);
        }
        let text = report(Script::Paint, &measured);
        for stage in Stage::ALL {
            assert!(text.contains(&format!("  {:<12} ", stage.name())), "{text}");
        }
        let share = coverage(&measured).unwrap();
        assert!((0.95..=1.0).contains(&share), "{share}");
        assert!(text.contains("stages add up to 99."), "{text}");
    }

    #[test]
    fn without_stage_timing_the_report_has_three_stages() {
        let measured = Measured {
            total: vec![Duration::from_millis(5)],
            edit: vec![Duration::from_millis(1)],
            scene: vec![Duration::from_millis(2)],
            render: vec![Duration::from_millis(2)],
            stages: Vec::new(),
        };
        assert_eq!(coverage(&measured), None);
        let text = report(Script::Paint, &measured);
        assert!(
            text.contains("  edit ") && text.contains("  render "),
            "{text}"
        );
        assert!(!text.contains("add up"), "{text}");
    }

    #[test]
    fn the_path_is_the_same_every_run() {
        assert_eq!(path([16_384.0; 2]), path([16_384.0; 2]));
        assert_eq!(path([16_384.0; 2]).len(), 3 * PHASE_FRAMES + 1);
    }

    #[test]
    fn every_script_has_a_name_that_parses() {
        for script in Script::ALL {
            assert_eq!(Script::parse(script.name()), Some(script));
        }
        assert_eq!(Script::parse("nope"), None);
    }

    #[test]
    fn every_script_runs_without_a_gpu() {
        let asset = AssetRef::new("bench", "peak").unwrap();
        for script in Script::ALL {
            let (map, layer) = large_map(0.02, asset.clone());
            let mut world = World::new(map, layer, LANDMASSES, script);
            let before = world.map.revision();
            for frame in 0..scripts::frames(script).min(scripts::EDIT_FRAMES) {
                scripts::edit(script, frame, &mut world);
            }
            let changed = world.map.revision() != before;
            assert_eq!(
                changed,
                !matches!(script, Script::PanZoom | Script::Marquee),
                "{}",
                script.name()
            );
            assert!(!scripts::views(script, [16_384.0; 2]).is_empty());
        }
    }

    #[test]
    fn the_edit_views_frame_the_edited_landmass() {
        let view = scripts::views(Script::Paint, [16_384.0; 2])[0];
        let middle = view.to_canvas([640.0, 400.0]);
        let target = scripts::centre_of(12);
        assert!((middle[0] - target[0]).abs() < 1e-6 && (middle[1] - target[1]).abs() < 1e-6);
    }
}
