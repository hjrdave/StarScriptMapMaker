//! The taper, variation and seed of the water lines of a Shore style (ADR 0016).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::file::MapFile;
use ssmap_core::{
    Canvas, Command, CommandError, DistanceUnit, Id, Map, MultiPolygon, Polygon, Scale, ShoreStyle,
};

fn map_with_land() -> (Map, Id) {
    let mut map = Map::new(Canvas::new(400, 400, Scale::new(1.0, DistanceUnit::Miles)));
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Land".into(),
    })
    .unwrap();
    let land = Id::new();
    map.execute(&Command::CreateLandmass {
        id: land,
        parent: layer,
        index: None,
        geometry: MultiPolygon(vec![Polygon {
            exterior: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
            holes: vec![],
        }]),
        fill: None,
    })
    .unwrap();
    (map, land)
}

fn shore_json(style: &ShoreStyle) -> serde_json::Value {
    serde_json::to_value(style).unwrap()
}

#[test]
fn taper_variation_and_seed_are_off_by_default_and_left_out_of_the_file() {
    let soft = ShoreStyle::soft_glow().lines;
    assert_eq!((soft.taper, soft.variation, soft.seed), (0.0, 0.0, 0));
    let json = shore_json(&ShoreStyle::soft_glow());
    let lines = json.get("lines").unwrap().as_object().unwrap();
    assert!(!lines.contains_key("taper"));
    assert!(!lines.contains_key("variation"));
    assert!(!lines.contains_key("seed"));
}

#[test]
fn lines_from_before_taper_variation_and_seed_read_with_defaults() {
    let mut old = shore_json(&ShoreStyle::soft_glow());
    let lines = old.get_mut("lines").unwrap().as_object_mut().unwrap();
    lines.remove("taper");
    lines.remove("variation");
    lines.remove("seed");
    let style: ShoreStyle = serde_json::from_value(old).unwrap();
    assert_eq!(style, ShoreStyle::soft_glow());
}

#[test]
fn taper_variation_and_seed_round_trip_through_json_and_the_file() {
    let mut style = ShoreStyle::soft_glow();
    style.lines.taper = 1.0;
    style.lines.variation = 0.5;
    style.lines.seed = 42;
    let back: ShoreStyle = serde_json::from_value(shore_json(&style)).unwrap();
    assert_eq!(back, style);

    let (mut map, land) = map_with_land();
    map.execute(&Command::SetShore {
        ids: vec![land],
        shore: Some(style.clone()),
    })
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("map.ssmap");
    MapFile::create(&path, &map, None).unwrap();
    let (_, reopened) = MapFile::open(&path).unwrap();
    assert_eq!(reopened.shore(land), Some(&style));
}

#[test]
fn out_of_range_taper_and_variation_are_problems_and_the_command_refuses_them() {
    let with = |change: &dyn Fn(&mut ShoreStyle)| {
        let mut style = ShoreStyle::soft_glow();
        change(&mut style);
        style.problem()
    };
    assert!(with(&|s| s.lines.taper = -0.1).is_some());
    assert!(with(&|s| s.lines.taper = 1.1).is_some());
    assert!(with(&|s| s.lines.taper = f64::NAN).is_some());
    assert!(with(&|s| s.lines.variation = -0.1).is_some());
    assert!(with(&|s| s.lines.variation = 2.0).is_some());
    assert_eq!(with(&|s| s.lines.variation = 1.0), None);
    assert_eq!(with(&|s| s.lines.taper = 0.0), None);

    let (mut map, land) = map_with_land();
    let mut bad = ShoreStyle::soft_glow();
    bad.lines.taper = 3.0;
    let result = map.execute(&Command::SetShore {
        ids: vec![land],
        shore: Some(bad),
    });
    assert!(matches!(result, Err(CommandError::InvalidShore(_))));
}
