//! Save, open and Command apply on a large Map (spec §11, ADR 0005).
//!
//! The Map has 100k Stamps and about 1M coastline vertices. Criterion posts the trend
//! numbers; afterwards a guard times save and open once and fails above 2× their targets, so
//! the nightly run breaks only on a real regression. `SSMAP_BENCH_SCALE` (default 1.0)
//! shrinks the Map for quick local runs, and turns the guard off below 1.0.

use std::f64::consts::TAU;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use criterion::{BatchSize, Criterion};
use ssmap_core::file::MapFile;
use ssmap_core::{
    AssetRef, Canvas, Command, DistanceUnit, Id, Landmass, Layer, Map, MultiPolygon, Node, Object,
    ObjectKind, Op, PlacedStamp, Polygon, Scale, Stamp, Subtree,
};

const STAMPS: usize = 100_000;
const LANDMASSES: usize = 20;
const VERTICES_PER_LANDMASS: usize = 50_000;

/// ADR 0005 targets.
const SAVE_TARGET: Duration = Duration::from_millis(200);
/// Reading the file is the part of open-to-interactive that lives in the core.
const OPEN_TARGET: Duration = Duration::from_secs(3);

struct Fixture {
    map: Map,
    layer: Id,
    coast: Id,
}

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

fn placed(count: usize) -> Vec<PlacedStamp> {
    let asset = AssetRef::new("bench", "peak").unwrap();
    (0..count)
        .map(|i| PlacedStamp {
            id: Id::new(),
            stamp: Stamp::new(
                asset.clone(),
                [(i % 400) as f64 * 20.0, (i / 400) as f64 * 20.0],
            ),
        })
        .collect()
}

fn fixture(scale: f64) -> Fixture {
    let mut map = Map::new(Canvas::new(
        16_384,
        16_384,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    let layer = insert(&mut map, None, Node::Layer(Layer::new("Bench")));
    let mut coast = layer;
    for i in 0..LANDMASSES {
        let geometry = coastline(
            [(i % 5) as f64 * 3_000.0, (i / 5) as f64 * 3_000.0],
            (VERTICES_PER_LANDMASS as f64 * scale) as usize,
        );
        let object = Object::new(ObjectKind::Landmass(Landmass {
            geometry,
            ..Default::default()
        }));
        coast = insert(&mut map, Some(layer), Node::Object(object));
    }
    map.execute(&Command::PlaceStamps {
        parent: layer,
        stamps: placed((STAMPS as f64 * scale) as usize),
    })
    .unwrap();
    Fixture { map, layer, coast }
}

fn saved(dir: &Path, map: &Map) -> PathBuf {
    let path = dir.join("large.ssmap");
    MapFile::create(&path, map, None).unwrap();
    path
}

fn nudge(coast: Id) -> Command {
    Command::Translate {
        ids: vec![coast],
        offset: [1.0, 0.0],
    }
}

fn benches(c: &mut Criterion, Fixture { map, layer, coast }: &Fixture, path: &Path) {
    let (layer, coast) = (*layer, *coast);
    let mut group = c.benchmark_group("large_map");
    group.sample_size(10);

    group.bench_function("save_full", |b| {
        let target = path.with_file_name("full.ssmap");
        b.iter(|| MapFile::create(&target, black_box(map), None).unwrap());
    });
    group.bench_function("save_incremental", |b| {
        let (mut file, mut edited) = MapFile::open(path).unwrap();
        b.iter(|| {
            edited.execute(&nudge(coast)).unwrap();
            file.save(black_box(&edited), None).unwrap()
        });
    });
    group.bench_function("open", |b| {
        b.iter(|| MapFile::open(black_box(path)).unwrap());
    });
    group.bench_function("apply_translate_coastline", |b| {
        let mut edited = map.clone();
        b.iter(|| edited.execute(&nudge(coast)).unwrap());
    });
    group.bench_function("apply_place_100_stamps", |b| {
        let mut edited = map.clone();
        b.iter_batched(
            || placed(100),
            |stamps| {
                edited
                    .execute(&Command::PlaceStamps {
                        parent: layer,
                        stamps,
                    })
                    .unwrap()
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

/// Times one save after a small edit and one open, and fails above 2× their targets.
fn guard(path: &Path, coast: Id) {
    let (mut file, mut edited) = MapFile::open(path).unwrap();
    edited.execute(&nudge(coast)).unwrap();
    let started = Instant::now();
    file.save(&edited, None).unwrap();
    let save = started.elapsed();

    let started = Instant::now();
    black_box(MapFile::open(path).unwrap());
    let open = started.elapsed();

    println!(
        "guard: save {save:?} (target {SAVE_TARGET:?}), open {open:?} (target {OPEN_TARGET:?})"
    );
    let mut failed = false;
    for (name, took, target) in [("save", save, SAVE_TARGET), ("open", open, OPEN_TARGET)] {
        if took > target * 2 {
            eprintln!("error: {name} took {took:?}, over 2x the {target:?} target");
            failed = true;
        }
    }
    if failed {
        std::process::exit(1);
    }
}

fn main() {
    let scale = scale();
    let fixture = fixture(scale);
    let dir = tempfile::tempdir().unwrap();
    let path = saved(dir.path(), &fixture.map);

    let mut criterion = Criterion::default().configure_from_args();
    benches(&mut criterion, &fixture, &path);
    criterion.final_summary();
    if scale >= 1.0 {
        guard(&path, fixture.coast);
    }
}
