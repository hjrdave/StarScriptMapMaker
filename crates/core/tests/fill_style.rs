//! Landmass Fill style: flat or grain (ADR 0015).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::file::MapFile;
use ssmap_core::{
    Canvas, Command, CommandError, DistanceUnit, FillStyle, Grain, Id, Map, MultiPolygon, Node,
    ObjectKind, Polygon, Property, Scale,
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
    assert_eq!(style_of(&map, land), FillStyle::Flat);
    assert_eq!(style_of(&map, lake), FillStyle::Flat);
}

#[test]
fn grain_can_be_set_and_undone() {
    let (mut map, land, _) = map_with_land_and_lake();
    let grain = FillStyle::Grain(Grain {
        amount: 0.4,
        size: 3.0,
    });
    let inverses = set(&mut map, vec![land], grain).unwrap();
    assert_eq!(style_of(&map, land), grain);
    map.revert(inverses);
    assert_eq!(style_of(&map, land), FillStyle::Flat);
}

#[test]
fn it_is_left_out_of_the_file_when_flat() {
    let (mut map, land, _) = map_with_land_and_lake();
    let json = |map: &Map| match map.node(land) {
        Some(Node::Object(object)) => serde_json::to_value(&object.kind).unwrap(),
        _ => panic!("not an Object"),
    };
    assert!(json(&map).get("fill_style").is_none());
    set(&mut map, vec![land], FillStyle::Grain(Grain::DEFAULT)).unwrap();
    assert_eq!(json(&map)["fill_style"]["kind"], "grain");
    let back: ObjectKind = serde_json::from_value(json(&map)).unwrap();
    assert_eq!(back.fill_style(), FillStyle::Grain(Grain::DEFAULT));
}

#[test]
fn a_file_from_before_fill_styles_reads_as_flat() {
    let kind: ObjectKind = serde_json::from_value(serde_json::json!({
        "kind": "landmass",
        "geometry": [{"exterior": [[0, 0], [10, 0], [10, 10], [0, 10]]}]
    }))
    .unwrap();
    assert_eq!(kind.fill_style(), FillStyle::Flat);
}

#[test]
fn only_landmasses_have_a_fill_style() {
    let (mut map, land, lake) = map_with_land_and_lake();
    let grain = FillStyle::Grain(Grain::DEFAULT);
    assert_eq!(
        set(&mut map, vec![lake], grain).unwrap_err(),
        CommandError::NotALandmass(lake)
    );
    // Nothing changed, not even for the Landmass listed beside the Lake.
    assert!(set(&mut map, vec![land, lake], grain).is_err());
    assert_eq!(style_of(&map, land), FillStyle::Flat);
}

#[test]
fn values_out_of_range_are_refused() {
    let (mut map, land, _) = map_with_land_and_lake();
    for (amount, size) in [
        (-0.1, 2.0),
        (1.5, 2.0),
        (0.5, 0.0),
        (0.5, 99.0),
        (f64::NAN, 2.0),
    ] {
        let result = set(
            &mut map,
            vec![land],
            FillStyle::Grain(Grain { amount, size }),
        );
        assert!(
            matches!(result, Err(CommandError::InvalidFillStyle(_))),
            "{amount} {size}"
        );
    }
    assert_eq!(style_of(&map, land), FillStyle::Flat);
}

#[test]
fn a_locked_landmass_refuses_the_change() {
    let (mut map, land, _) = map_with_land_and_lake();
    map.execute(&Command::SetLocked {
        id: land,
        locked: true,
    })
    .unwrap();
    assert!(set(&mut map, vec![land], FillStyle::Grain(Grain::DEFAULT)).is_err());
    assert_eq!(style_of(&map, land), FillStyle::Flat);
}

#[test]
fn a_layer_has_no_fill_style() {
    let (mut map, ..) = map_with_land_and_lake();
    let layer = map.children(None)[0];
    let result = map.apply(ssmap_core::Op::SetProperty {
        id: layer,
        property: Property::FillStyle(FillStyle::Flat),
    });
    assert!(result.is_err());
}

#[test]
fn it_survives_saving_and_opening_the_file() {
    let (mut map, land, _) = map_with_land_and_lake();
    let grain = FillStyle::Grain(Grain {
        amount: 0.6,
        size: 1.5,
    });
    set(&mut map, vec![land], grain).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("map.ssmap");
    MapFile::create(&file_path, &map, None).unwrap();
    let (_, reopened) = MapFile::open(&file_path).unwrap();
    assert_eq!(style_of(&reopened, land), grain);
}
