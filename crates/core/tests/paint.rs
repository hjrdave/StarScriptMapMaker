//! Paint strokes: Commands and undo, simplification, validation and the file round trip.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::file::{FileError, MapFile};
use ssmap_core::{
    Canvas, Color, Command, CommandError, DistanceUnit, History, Id, InvariantError,
    MAX_PAINT_STROKES, MAX_RAW_STROKE_POINTS, MAX_STROKE_POINTS, MAX_STROKE_SIZE, Map, MapView, Op,
    OpError, PaintStroke, STROKE_TOLERANCE, Scale, Selection, StrokeError, simplify_path,
};

fn new_map() -> Map {
    Map::new(Canvas::new(100, 80, Scale::new(1.0, DistanceUnit::Miles)))
}

fn stroke(points: Vec<[f64; 2]>) -> PaintStroke {
    PaintStroke {
        id: Id::new(),
        points,
        color: Color::rgb(200, 30, 30),
        size: 12.0,
        softness: 0.5,
        opacity: 0.8,
        erase: false,
    }
}

fn eraser(points: Vec<[f64; 2]>) -> PaintStroke {
    PaintStroke {
        erase: true,
        ..stroke(points)
    }
}

fn add(map: &mut Map, stroke: &PaintStroke) {
    map.execute(&Command::AddPaintStroke {
        stroke: stroke.clone(),
    })
    .unwrap();
}

#[test]
fn a_new_map_has_no_strokes_and_a_stroke_lands_on_top() {
    let mut map = new_map();
    assert!(map.paint_strokes().is_empty());
    let a = stroke(vec![[0.0, 0.0], [10.0, 10.0]]);
    let b = stroke(vec![[5.0, 0.0], [5.0, 30.0]]);
    add(&mut map, &a);
    add(&mut map, &b);
    let ids: Vec<Id> = map.paint_strokes().iter().map(|s| s.id).collect();
    assert_eq!(ids, vec![a.id, b.id]);
    assert_eq!(map.paint_stroke(b.id), Some(&b));
}

#[test]
fn every_paint_command_is_one_undo_step() {
    let mut map = new_map();
    let mut history = History::default();
    let first = stroke(vec![[0.0, 0.0], [10.0, 10.0]]);
    let second = stroke(vec![[0.0, 5.0], [10.0, 5.0]]);
    let rub = eraser(vec![[3.0, 0.0], [3.0, 10.0]]);
    let commands = [
        Command::AddPaintStroke {
            stroke: first.clone(),
        },
        Command::AddPaintStroke {
            stroke: second.clone(),
        },
        Command::AddPaintStroke { stroke: rub },
        Command::DeletePaintStrokes {
            ids: vec![first.id],
        },
        Command::ClearPaint,
    ];
    let mut views = vec![map.view()];
    for command in &commands {
        history
            .execute_step(&mut map, command, Selection::default())
            .unwrap();
        views.push(map.view());
    }
    assert!(map.paint_strokes().is_empty());
    for expected in views.iter().rev().skip(1) {
        history.undo(&mut map).unwrap();
        assert_eq!(&map.view(), expected);
    }
    for expected in views.iter().skip(1) {
        history.redo(&mut map).unwrap();
        assert_eq!(&map.view(), expected);
    }
}

#[test]
fn undo_restores_deleted_strokes_in_their_places() {
    let mut map = new_map();
    let strokes: Vec<PaintStroke> = (0..4)
        .map(|i| stroke(vec![[f64::from(i), 0.0], [f64::from(i), 9.0]]))
        .collect();
    for s in &strokes {
        add(&mut map, s);
    }
    let before = map.view();
    let undo = map
        .execute(&Command::DeletePaintStrokes {
            ids: vec![strokes[1].id, strokes[3].id, strokes[1].id],
        })
        .unwrap();
    assert_eq!(map.paint_strokes().len(), 2);
    map.revert(undo);
    assert_eq!(map.view(), before);
}

#[test]
fn clearing_an_empty_map_does_nothing() {
    let mut map = new_map();
    assert!(map.execute(&Command::ClearPaint).unwrap().is_empty());
}

#[test]
fn an_eraser_is_a_stroke_that_keeps_its_place_in_the_order() {
    let mut map = new_map();
    add(&mut map, &stroke(vec![[0.0, 0.0], [10.0, 0.0]]));
    let rub = eraser(vec![[5.0, -5.0], [5.0, 5.0]]);
    let command = Command::AddPaintStroke {
        stroke: rub.clone(),
    };
    assert_eq!(command.label(), "Erase Paint");
    map.execute(&command).unwrap();
    assert!(map.paint_strokes()[1].erase);
    assert_eq!(map.paint_strokes()[1].id, rub.id);
    let paint = Command::AddPaintStroke {
        stroke: stroke(vec![[0.0, 0.0]]),
    };
    assert_eq!(paint.label(), "Paint");
}

#[test]
fn duplicate_and_missing_ids_are_refused() {
    let mut map = new_map();
    let s = stroke(vec![[0.0, 0.0], [4.0, 4.0]]);
    add(&mut map, &s);
    let before = map.view();
    assert_eq!(
        map.execute(&Command::AddPaintStroke { stroke: s.clone() })
            .unwrap_err(),
        CommandError::StrokeIdInUse(s.id)
    );
    let missing = Id::new();
    assert_eq!(
        map.execute(&Command::DeletePaintStrokes {
            ids: vec![s.id, missing]
        })
        .unwrap_err(),
        CommandError::NoSuchStroke(missing)
    );
    assert_eq!(map.view(), before);
}

#[test]
fn a_long_path_is_simplified_when_committed() {
    let mut map = new_map();
    // A gentle arc drawn with many points.
    let path: Vec<[f64; 2]> = (0..=2000)
        .map(|i| {
            let x = f64::from(i) * 0.05;
            [x, 10.0 * (x / 15.0).sin()]
        })
        .collect();
    let s = stroke(path.clone());
    add(&mut map, &s);
    let stored = &map.paint_strokes()[0];
    assert!(
        stored.points.len() < path.len() / 10,
        "{}",
        stored.points.len()
    );
    assert_eq!(stored.points.first(), path.first());
    assert_eq!(stored.points.last(), path.last());
    assert_eq!(
        (stored.color, stored.size, stored.softness, stored.opacity),
        (s.color, s.size, s.softness, s.opacity)
    );
}

#[test]
fn simplifying_keeps_every_dropped_point_within_the_tolerance() {
    let path: Vec<[f64; 2]> = (0..=500)
        .map(|i| {
            let t = f64::from(i) * 0.1;
            [t * 3.0, 20.0 * t.sin() + (t * 7.0).cos()]
        })
        .collect();
    let tolerance = 0.75;
    let simple = simplify_path(&path, tolerance);
    assert!(simple.len() < path.len());
    for p in &path {
        let d = simple
            .windows(2)
            .map(|w| distance_to_segment(*p, w[0], w[1]))
            .fold(f64::INFINITY, f64::min);
        assert!(
            d <= tolerance + 1e-9,
            "{p:?} is {d} from the simplified path"
        );
    }
    // Every kept point is one of the originals, in order.
    let mut rest = path.iter();
    for kept in &simple {
        assert!(rest.any(|p| p == kept));
    }
}

fn distance_to_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    };
    (p[0] - (a[0] + t * dx)).hypot(p[1] - (a[1] + t * dy))
}

#[test]
fn a_single_point_and_a_closed_loop_survive_simplifying() {
    assert_eq!(simplify_path(&[[1.0, 2.0]], 1.0), vec![[1.0, 2.0]]);
    let mut ring: Vec<[f64; 2]> = (0..64)
        .map(|i| {
            let a = f64::from(i) / 64.0 * std::f64::consts::TAU;
            [50.0 * a.cos(), 50.0 * a.sin()]
        })
        .collect();
    ring.push(ring[0]);
    let simple = simplify_path(&ring, STROKE_TOLERANCE);
    assert!(simple.len() > 8, "a loop keeps its shape");
    assert_eq!(simple.first(), simple.last());
}

#[test]
fn bad_strokes_are_refused_and_leave_the_map_alone() {
    let mut map = new_map();
    let base = stroke(vec![[0.0, 0.0], [5.0, 5.0]]);
    let cases: Vec<(PaintStroke, StrokeError)> = vec![
        (
            PaintStroke {
                points: vec![],
                ..base.clone()
            },
            StrokeError::NoPoints,
        ),
        (
            PaintStroke {
                points: vec![[0.0, 0.0], [f64::NAN, 1.0], [5.0, 5.0]],
                ..base.clone()
            },
            StrokeError::NonFinitePoint,
        ),
        (
            PaintStroke {
                points: vec![[0.0, f64::INFINITY]],
                ..base.clone()
            },
            StrokeError::NonFinitePoint,
        ),
        (
            PaintStroke {
                size: 0.0,
                ..base.clone()
            },
            StrokeError::Size(0.0),
        ),
        (
            PaintStroke {
                size: MAX_STROKE_SIZE + 1.0,
                ..base.clone()
            },
            StrokeError::Size(MAX_STROKE_SIZE + 1.0),
        ),
        (
            PaintStroke {
                softness: 1.5,
                ..base.clone()
            },
            StrokeError::Softness(1.5),
        ),
        (
            PaintStroke {
                opacity: -0.1,
                ..base.clone()
            },
            StrokeError::Opacity(-0.1),
        ),
        (
            PaintStroke {
                opacity: f32::NAN,
                ..base.clone()
            },
            StrokeError::Opacity(f32::NAN),
        ),
    ];
    for (bad, _) in &cases {
        let err = map
            .execute(&Command::AddPaintStroke {
                stroke: bad.clone(),
            })
            .unwrap_err();
        assert!(
            matches!(err, CommandError::InvalidStroke(_)),
            "{bad:?}: {err:?}"
        );
    }
    // NaN never equals itself, so match the others by value.
    for (bad, expected) in cases
        .iter()
        .filter(|(_, e)| !matches!(e, StrokeError::Opacity(v) if v.is_nan()))
    {
        assert_eq!(
            map.execute(&Command::AddPaintStroke {
                stroke: bad.clone()
            })
            .unwrap_err(),
            CommandError::InvalidStroke(expected.clone())
        );
    }
    assert!(map.paint_strokes().is_empty());
}

#[test]
fn the_point_caps_apply_to_raw_and_stored_paths() {
    let mut map = new_map();
    // Too many raw points are refused before any work is done.
    let raw: Vec<[f64; 2]> = (0..=MAX_RAW_STROKE_POINTS)
        .map(|i| [i as f64, 0.0])
        .collect();
    assert!(matches!(
        map.execute(&Command::AddPaintStroke {
            stroke: stroke(raw)
        }),
        Err(CommandError::InvalidStroke(
            StrokeError::TooManyPoints { .. }
        ))
    ));
    // A path that is still too long after simplifying is refused as stored.
    let zigzag: Vec<[f64; 2]> = (0..=MAX_STROKE_POINTS)
        .map(|i| [i as f64, if i % 2 == 0 { 0.0 } else { 20.0 }])
        .collect();
    assert!(matches!(
        map.execute(&Command::AddPaintStroke {
            stroke: stroke(zigzag)
        }),
        Err(CommandError::InvalidStroke(
            StrokeError::TooManyPoints { .. }
        ))
    ));
    assert!(map.paint_strokes().is_empty());
}

#[test]
fn the_stroke_count_is_capped() {
    let mut map = new_map();
    let ops: Vec<Op> = (0..MAX_PAINT_STROKES)
        .map(|index| Op::InsertStroke {
            index,
            stroke: stroke(vec![[0.0, 0.0]]),
        })
        .collect();
    for op in ops {
        map.apply(op).unwrap();
    }
    assert_eq!(
        map.execute(&Command::AddPaintStroke {
            stroke: stroke(vec![[1.0, 1.0]])
        })
        .unwrap_err(),
        CommandError::TooManyStrokes(MAX_PAINT_STROKES)
    );
    let mut view = map.view();
    view.strokes.push(stroke(vec![[1.0, 1.0]]));
    assert!(matches!(
        Map::from_view(view),
        Err(InvariantError::TooManyStrokes(_))
    ));
}

#[test]
fn the_stroke_ops_check_their_targets_and_invert() {
    let mut map = new_map();
    let s = stroke(vec![[0.0, 0.0]]);
    assert!(matches!(
        map.apply(Op::InsertStroke {
            index: 1,
            stroke: s.clone()
        }),
        Err(OpError::IndexOutOfRange { .. })
    ));
    let inverse = map
        .apply(Op::InsertStroke {
            index: 0,
            stroke: s.clone(),
        })
        .unwrap();
    assert_eq!(inverse, Op::RemoveStroke { id: s.id });
    assert!(matches!(
        map.apply(Op::InsertStroke {
            index: 0,
            stroke: s.clone()
        }),
        Err(OpError::Invariant(InvariantError::DuplicateStroke(_)))
    ));
    let back = map.apply(inverse).unwrap();
    assert_eq!(
        back,
        Op::InsertStroke {
            index: 0,
            stroke: s.clone()
        }
    );
    assert!(map.paint_strokes().is_empty());
    assert_eq!(
        map.apply(Op::RemoveStroke { id: s.id }),
        Err(OpError::NoSuchStroke(s.id))
    );
}

#[test]
fn from_view_rejects_bad_and_duplicate_strokes() {
    let good = stroke(vec![[0.0, 0.0], [1.0, 1.0]]);
    let view = |strokes: Vec<PaintStroke>| MapView {
        strokes,
        ..new_map().view()
    };
    assert!(Map::from_view(view(vec![good.clone()])).is_ok());
    assert!(matches!(
        Map::from_view(view(vec![good.clone(), good.clone()])),
        Err(InvariantError::DuplicateStroke(_))
    ));
    let bad = PaintStroke {
        size: -1.0,
        ..stroke(vec![[0.0, 0.0]])
    };
    assert!(matches!(
        Map::from_view(view(vec![bad])),
        Err(InvariantError::InvalidStroke(..))
    ));
}

#[test]
fn strokes_round_trip_through_the_file_and_an_empty_map_has_no_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("paint.ssmap");
    let mut map = new_map();
    let (mut file, _) = MapFile::create(&path, &map, None).unwrap();
    assert!(!has_strokes_key(&path));

    add(
        &mut map,
        &stroke(vec![[0.0, 0.0], [10.0, 4.0], [20.0, 0.0]]),
    );
    map.execute(&Command::AddPaintStroke {
        stroke: eraser(vec![[5.0, -5.0], [5.0, 5.0]]),
    })
    .unwrap();
    file.save(&map, None).unwrap();
    assert!(has_strokes_key(&path));
    drop(file);

    let (mut file, reopened) = MapFile::open(&path).unwrap();
    assert_eq!(reopened.paint_strokes(), map.paint_strokes());
    assert_eq!(reopened.view(), map.view());

    map.execute(&Command::ClearPaint).unwrap();
    file.save(&map, None).unwrap();
    drop(file);
    assert!(!has_strokes_key(&path));
    let (_, again) = MapFile::open(&path).unwrap();
    assert!(again.paint_strokes().is_empty());
}

#[test]
fn save_as_keeps_the_strokes() {
    let dir = tempfile::tempdir().unwrap();
    let mut map = new_map();
    add(&mut map, &stroke(vec![[1.0, 1.0], [30.0, 9.0]]));
    let (mut file, _) = MapFile::create(dir.path().join("a.ssmap"), &map, None).unwrap();
    file.save_as(dir.path().join("b.ssmap"), &map, None)
        .unwrap();
    let (_, copy) = MapFile::open(dir.path().join("b.ssmap")).unwrap();
    assert_eq!(copy.paint_strokes(), map.paint_strokes());
}

#[test]
fn a_file_saved_before_paint_opens_with_none_and_a_damaged_stroke_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.ssmap");
    let map = new_map();
    MapFile::create(&path, &map, None).unwrap();
    // Pretend an older version saved it: no `strokes` key and format 1.18.
    set_meta(&path, "format_version", "1.18");
    let (_, opened) = MapFile::open(&path).unwrap();
    assert!(opened.paint_strokes().is_empty());

    // A stroke with a size out of range is a damaged file, not a Map.
    let bad = serde_json::to_string(&[PaintStroke {
        size: 0.0,
        ..stroke(vec![[0.0, 0.0]])
    }])
    .unwrap();
    set_meta(&path, "strokes", &bad);
    assert!(matches!(MapFile::open(&path), Err(FileError::Damaged(_))));
    set_meta(&path, "strokes", "not json");
    assert!(matches!(MapFile::open(&path), Err(FileError::Damaged(_))));
}

#[test]
fn the_view_leaves_strokes_out_when_empty() {
    let mut map = new_map();
    let json = serde_json::to_value(map.view()).unwrap();
    assert!(json.get("strokes").is_none());
    add(&mut map, &stroke(vec![[0.0, 0.0]]));
    let json = serde_json::to_value(map.view()).unwrap();
    assert_eq!(json["strokes"].as_array().unwrap().len(), 1);
    // `erase` is left out for paint.
    assert!(json["strokes"][0].get("erase").is_none());
}

fn has_strokes_key(path: &std::path::Path) -> bool {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.query_row("SELECT COUNT(*) FROM meta WHERE key = 'strokes'", [], |r| {
        r.get::<_, i64>(0)
    })
    .unwrap()
        == 1
}

fn set_meta(path: &std::path::Path, key: &str, value: &str) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2) \
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        [key, value],
    )
    .unwrap();
}
