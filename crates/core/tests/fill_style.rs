//! Landmass Fill style: flat, grain, mottle or both (ADR 0015).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::file::MapFile;
use ssmap_core::{
    Canvas, Color, Command, CommandError, DistanceUnit, FillStyle, Grain, Id, Map, Mottle,
    MultiPolygon, Node, ObjectKind, Polygon, Property, Scale,
};

fn square() -> MultiPolygon {
    MultiPolygon(vec![Polygon {
        exterior: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
        holes: vec![],
    }])
}

/// A Map with one Layer holding a Landmass and a Lake.
fn map_with_land_and_lake() -> (Map, Id, Id) {
    let mut map = Map::new(Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles)));
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Land".into(),
    })
    .unwrap();
    let (land, lake) = (Id::new(), Id::new());
    map.execute(&Command::CreateLandmass {
        id: land,
        parent: layer,
        index: None,
        geometry: square(),
        fill: None,
    })
    .unwrap();
    map.execute(&Command::CreateLake {
        id: lake,
        parent: layer,
        index: None,
        geometry: square(),
        fill: None,
    })
    .unwrap();
    (map, land, lake)
}

fn grained() -> FillStyle {
    FillStyle {
        grain: Some(Grain::DEFAULT),
        mottle: None,
    }
}

fn mottled() -> FillStyle {
    FillStyle {
        grain: None,
        mottle: Some(Mottle::DEFAULT),
    }
}

fn both() -> FillStyle {
    FillStyle {
        grain: Some(Grain::DEFAULT),
        mottle: Some(Mottle::DEFAULT),
    }
}

fn style_of(map: &Map, id: Id) -> FillStyle {
    match map.node(id) {
        Some(Node::Object(object)) => object.kind.fill_style(),
        _ => panic!("not an Object"),
    }
}

fn set(map: &mut Map, ids: Vec<Id>, style: FillStyle) -> Result<Vec<ssmap_core::Op>, CommandError> {
    map.execute(&Command::SetFillStyle { ids, style })
}

#[test]
fn a_new_landmass_is_flat() {
    let (map, land, lake) = map_with_land_and_lake();
    assert_eq!(style_of(&map, land), FillStyle::FLAT);
    assert_eq!(style_of(&map, lake), FillStyle::FLAT);
}

#[test]
fn each_style_can_be_set_and_undone() {
    for style in [
        grained(),
        mottled(),
        both(),
        FillStyle {
            grain: Some(Grain {
                amount: 0.4,
                size: 3.0,
            }),
            mottle: Some(Mottle {
                color: Color::rgb(10, 20, 30),
                scale: 400.0,
                contrast: 0.8,
                seed: 77,
            }),
        },
    ] {
        let (mut map, land, _) = map_with_land_and_lake();
        let inverses = set(&mut map, vec![land], style).unwrap();
        assert_eq!(style_of(&map, land), style);
        map.revert(inverses);
        assert_eq!(style_of(&map, land), FillStyle::FLAT);
    }
}

#[test]
fn switching_one_part_off_keeps_the_other() {
    let (mut map, land, _) = map_with_land_and_lake();
    set(&mut map, vec![land], both()).unwrap();
    set(&mut map, vec![land], grained()).unwrap();
    assert_eq!(style_of(&map, land), grained());
    set(&mut map, vec![land], FillStyle::FLAT).unwrap();
    assert_eq!(style_of(&map, land), FillStyle::FLAT);
}

#[test]
fn it_is_left_out_of_the_file_when_flat_and_each_part_when_off() {
    let (mut map, land, _) = map_with_land_and_lake();
    let json = |map: &Map| match map.node(land) {
        Some(Node::Object(object)) => serde_json::to_value(&object.kind).unwrap(),
        _ => panic!("not an Object"),
    };
    assert!(json(&map).get("fill_style").is_none());
    set(&mut map, vec![land], grained()).unwrap();
    assert!(json(&map)["fill_style"].get("grain").is_some());
    assert!(json(&map)["fill_style"].get("mottle").is_none());
    set(&mut map, vec![land], both()).unwrap();
    let back: ObjectKind = serde_json::from_value(json(&map)).unwrap();
    assert_eq!(back.fill_style(), both());
}

#[test]
fn a_file_from_before_fill_styles_reads_as_flat() {
    let kind: ObjectKind = serde_json::from_value(serde_json::json!({
        "kind": "landmass",
        "geometry": [{"exterior": [[0, 0], [10, 0], [10, 10], [0, 10]]}]
    }))
    .unwrap();
    assert_eq!(kind.fill_style(), FillStyle::FLAT);
}

#[test]
fn only_landmasses_have_a_fill_style() {
    let (mut map, land, lake) = map_with_land_and_lake();
    assert_eq!(
        set(&mut map, vec![lake], both()).unwrap_err(),
        CommandError::NotALandmass(lake)
    );
    // Nothing changed, not even for the Landmass listed beside the Lake.
    assert!(set(&mut map, vec![land, lake], both()).is_err());
    assert_eq!(style_of(&map, land), FillStyle::FLAT);
}

#[test]
fn values_out_of_range_are_refused() {
    let (mut map, land, _) = map_with_land_and_lake();
    let bad_grain = |amount, size| FillStyle {
        grain: Some(Grain { amount, size }),
        mottle: None,
    };
    let bad_mottle = |scale, contrast| FillStyle {
        grain: None,
        mottle: Some(Mottle {
            scale,
            contrast,
            ..Mottle::DEFAULT
        }),
    };
    for style in [
        bad_grain(-0.1, 2.0),
        bad_grain(1.5, 2.0),
        bad_grain(0.5, 0.0),
        bad_grain(0.5, 99.0),
        bad_grain(f64::NAN, 2.0),
        bad_mottle(1.0, 0.5),
        bad_mottle(99_999.0, 0.5),
        bad_mottle(150.0, 1.5),
        bad_mottle(f64::NAN, 0.5),
    ] {
        let result = set(&mut map, vec![land], style);
        assert!(
            matches!(result, Err(CommandError::InvalidFillStyle(_))),
            "{style:?}"
        );
    }
    assert_eq!(style_of(&map, land), FillStyle::FLAT);
}

#[test]
fn a_locked_landmass_refuses_the_change() {
    let (mut map, land, _) = map_with_land_and_lake();
    map.execute(&Command::SetLocked {
        id: land,
        locked: true,
    })
    .unwrap();
    assert!(set(&mut map, vec![land], both()).is_err());
    assert_eq!(style_of(&map, land), FillStyle::FLAT);
}

#[test]
fn a_layer_has_no_fill_style() {
    let (mut map, ..) = map_with_land_and_lake();
    let layer = map.children(None)[0];
    let result = map.apply(ssmap_core::Op::SetProperty {
        id: layer,
        property: Property::FillStyle(FillStyle::FLAT),
    });
    assert!(result.is_err());
}

#[test]
fn it_survives_saving_and_opening_the_file() {
    let (mut map, land, _) = map_with_land_and_lake();
    let style = FillStyle {
        grain: Some(Grain {
            amount: 0.6,
            size: 1.5,
        }),
        mottle: Some(Mottle {
            color: Color::rgb(1, 2, 3),
            scale: 321.0,
            contrast: 0.25,
            seed: 4_000_000_000,
        }),
    };
    set(&mut map, vec![land], style).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("map.ssmap");
    MapFile::create(&file_path, &map, None).unwrap();
    let (_, reopened) = MapFile::open(&file_path).unwrap();
    assert_eq!(style_of(&reopened, land), style);
}
