//! The wave marks of a Shore style and the Hand drawn preset (ADR 0016).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::file::MapFile;
use ssmap_core::{
    Canvas, Command, CommandError, DistanceUnit, Id, Map, MultiPolygon, Polygon, Scale, ShoreKind,
    ShoreStyle, ShoreWaves,
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
fn waves_are_off_by_default_and_left_out_of_the_file() {
    for style in [
        ShoreStyle::landmass_default(),
        ShoreStyle::lake_default(),
        ShoreStyle::plain(),
        ShoreStyle::soft_glow(),
    ] {
        assert_eq!(style.waves.density, 0.0);
        assert!(shore_json(&style).get("waves").is_none());
    }
}

#[test]
fn a_style_from_before_waves_reads_with_none() {
    let mut old = shore_json(&ShoreStyle::hand_drawn());
    assert!(old.as_object_mut().unwrap().remove("waves").is_some());
    let style: ShoreStyle = serde_json::from_value(old).unwrap();
    assert_eq!(style.waves, ShoreWaves::default());
}

#[test]
fn the_hand_drawn_preset_has_the_look_and_is_valid() {
    let style = ShoreStyle::hand_drawn();
    assert_eq!(style.problem(), None);
    assert!(style.outline.width > ShoreStyle::soft_glow().outline.width);
    assert_eq!(style.glow.width, 0.0);
    assert_eq!(style.ripples.count, 0);
    assert!(style.lines.taper > 0.0 && style.lines.variation > 0.0);
    assert_eq!(style.waves.density, 0.0, "no wave marks");
    assert_ne!(style.waves.color, ShoreWaves::default().color);
    // It is offered for Landmasses after Soft glow, not for Lakes, and Soft glow is unchanged.
    let names: Vec<_> = ShoreStyle::presets(ShoreKind::Landmass)
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(names, ["Default", "Soft glow", "Hand drawn"]);
    assert_eq!(style.preset_name(ShoreKind::Landmass), Some("Hand drawn"));
    assert_eq!(ShoreStyle::soft_glow().waves, ShoreWaves::default());
    assert_eq!(ShoreStyle::soft_glow().lines.length, 44.0);
}

#[test]
fn waves_round_trip_through_json_and_the_file() {
    let mut style = ShoreStyle::hand_drawn();
    style.waves.seed = 77;
    style.waves.size = 22.0;
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
fn out_of_range_waves_are_problems_and_the_command_refuses_them() {
    let with = |change: &dyn Fn(&mut ShoreWaves)| {
        let mut style = ShoreStyle::hand_drawn();
        change(&mut style.waves);
        style.problem()
    };
    assert!(with(&|w| w.density = -1.0).is_some());
    assert!(with(&|w| w.density = 1000.0).is_some());
    assert!(with(&|w| w.density = f64::NAN).is_some());
    assert!(with(&|w| w.size = 0.0).is_some());
    assert!(with(&|w| w.size = 500.0).is_some());
    assert_eq!(with(&|w| w.density = 0.0), None);

    let (mut map, land) = map_with_land();
    let mut bad = ShoreStyle::hand_drawn();
    bad.waves.size = f64::INFINITY;
    let result = map.execute(&Command::SetShore {
        ids: vec![land],
        shore: Some(bad),
    });
    assert!(matches!(result, Err(CommandError::InvalidShore(_))));
}
