//! The glow and water lines of a Shore style (ADR 0016).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::file::MapFile;
use ssmap_core::{
    Canvas, Color, Command, CommandError, DistanceUnit, Id, Map, MultiPolygon, Polygon, Scale,
    ShoreGlow, ShoreKind, ShoreLines, ShoreStyle,
};

fn square() -> MultiPolygon {
    MultiPolygon(vec![Polygon {
        exterior: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
        holes: vec![],
    }])
}

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
        geometry: square(),
        fill: None,
    })
    .unwrap();
    (map, land)
}

fn shore_json(style: &ShoreStyle) -> serde_json::Value {
    serde_json::to_value(style).unwrap()
}

#[test]
fn the_existing_styles_have_neither_glow_nor_lines() {
    for style in [
        ShoreStyle::landmass_default(),
        ShoreStyle::lake_default(),
        ShoreStyle::plain(),
    ] {
        assert_eq!(style.glow.width, 0.0);
        assert_eq!(style.lines.length, 0.0);
    }
}

#[test]
fn the_soft_glow_preset_has_both_and_is_valid() {
    let style = ShoreStyle::soft_glow();
    assert!(style.glow.width > 0.0 && style.lines.length > 0.0);
    assert_eq!(style.ripples.count, 0);
    assert_eq!(style.problem(), None);
}

#[test]
fn an_off_glow_and_off_lines_are_left_out_of_the_file() {
    let json = shore_json(&ShoreStyle::landmass_default());
    assert!(json.get("glow").is_none() && json.get("lines").is_none());
    let json = shore_json(&ShoreStyle::soft_glow());
    assert!(json.get("glow").is_some() && json.get("lines").is_some());
    let back: ShoreStyle = serde_json::from_value(json).unwrap();
    assert_eq!(back, ShoreStyle::soft_glow());
}

#[test]
fn a_shore_style_from_before_glow_and_lines_reads_as_having_neither() {
    let mut json = shore_json(&ShoreStyle::landmass_default());
    json.as_object_mut().unwrap().remove("glow");
    let style: ShoreStyle = serde_json::from_value(json).unwrap();
    assert_eq!(style, ShoreStyle::landmass_default());
}

#[test]
fn out_of_range_glow_and_lines_are_problems() {
    let with = |change: &dyn Fn(&mut ShoreStyle)| {
        let mut style = ShoreStyle::soft_glow();
        change(&mut style);
        style.problem()
    };
    assert!(with(&|s| s.glow.width = -1.0).is_some());
    assert!(with(&|s| s.glow.width = 9_999.0).is_some());
    assert!(with(&|s| s.glow.width = f64::NAN).is_some());
    assert!(with(&|s| s.lines.length = -1.0).is_some());
    assert!(with(&|s| s.lines.length = 9_999.0).is_some());
    assert!(with(&|s| s.lines.spacing = 0.0).is_some());
    assert!(with(&|s| s.lines.spacing = 0.5).is_some());
    assert!(with(&|s| s.lines.spacing = f64::INFINITY).is_some());
    assert!(with(&|s| s.lines.width = -0.1).is_some());
    assert!(with(&|s| s.lines.width = 99.0).is_some());
    assert_eq!(with(&|s| s.glow.width = 0.0), None);
    assert_eq!(with(&|s| s.lines.length = 0.0), None);
}

#[test]
fn a_landmass_can_take_the_soft_glow_and_undo_it() {
    let (mut map, land) = map_with_land();
    let inverses = map
        .execute(&Command::SetShore {
            ids: vec![land],
            shore: Some(ShoreStyle::soft_glow()),
        })
        .unwrap();
    assert_eq!(map.shore(land), Some(&ShoreStyle::soft_glow()));
    map.revert(inverses);
    assert_eq!(map.shore(land), Some(&ShoreStyle::landmass_default()));
}

#[test]
fn an_invalid_glow_is_refused_by_the_command() {
    let (mut map, land) = map_with_land();
    let mut bad = ShoreStyle::soft_glow();
    bad.lines.spacing = 0.1;
    let result = map.execute(&Command::SetShore {
        ids: vec![land],
        shore: Some(bad.clone()),
    });
    assert!(matches!(result, Err(CommandError::InvalidShore(_))));
    let result = map.execute(&Command::SetDefaultShore {
        kind: ShoreKind::Landmass,
        shore: bad,
    });
    assert!(matches!(result, Err(CommandError::InvalidShore(_))));
}

#[test]
fn the_map_wide_default_and_a_landmass_override_survive_the_file() {
    let (mut map, land) = map_with_land();
    let own = ShoreStyle {
        glow: ShoreGlow {
            width: 12.0,
            color: Color::rgba(1, 2, 3, 4),
        },
        lines: ShoreLines {
            spacing: 7.0,
            length: 30.0,
            width: 1.5,
            color: Color::rgba(9, 8, 7, 6),
        },
        ..ShoreStyle::landmass_default()
    };
    map.execute(&Command::SetShore {
        ids: vec![land],
        shore: Some(own.clone()),
    })
    .unwrap();
    map.execute(&Command::SetDefaultShore {
        kind: ShoreKind::Lake,
        shore: ShoreStyle::soft_glow(),
    })
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("map.ssmap");
    MapFile::create(&path, &map, None).unwrap();
    let (_, reopened) = MapFile::open(&path).unwrap();
    assert_eq!(reopened.shore(land), Some(&own));
    assert_eq!(
        reopened.defaults().shore_for(ShoreKind::Lake),
        &ShoreStyle::soft_glow()
    );
}

#[test]
fn every_preset_is_valid_and_named_once_and_lakes_have_no_water_lines() {
    for kind in [ShoreKind::Landmass, ShoreKind::Lake] {
        let presets = ShoreStyle::presets(kind);
        assert!(!presets.is_empty());
        for (i, (name, style)) in presets.iter().enumerate() {
            assert_eq!(style.problem(), None, "{name}");
            assert_eq!(style.preset_name(kind), Some(*name));
            assert!(presets[..i].iter().all(|(n, s)| n != name && s != style));
        }
    }
    assert!(
        ShoreStyle::presets(ShoreKind::Lake)
            .iter()
            .all(|(_, s)| s.lines.length == 0.0)
    );
    let names: Vec<_> = ShoreStyle::presets(ShoreKind::Landmass)
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(names, ["Default", "Soft glow"]);
}

#[test]
fn a_style_matching_no_preset_has_no_name() {
    let mut style = ShoreStyle::soft_glow();
    assert_eq!(style.preset_name(ShoreKind::Landmass), Some("Soft glow"));
    assert_eq!(style.preset_name(ShoreKind::Lake), None);
    style.outline.width += 1.0;
    assert_eq!(style.preset_name(ShoreKind::Landmass), None);
}
