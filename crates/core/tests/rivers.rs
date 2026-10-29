//! Rivers through the public API: Commands and their inverses, the Map file, draw order,
//! picking and scatter placement (ADR 0012).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::file::MapFile;
use ssmap_core::scatter::{Placement, Terrain};
use ssmap_core::{
    Affine, Canvas, Color, Command, CommandError, DistanceUnit, Id, Map, MultiPolygon, Node,
    NodeRef, ObjectKind, PickIndex, Polygon, River, RiverPoint, RoughnessCache, Scale,
};

fn map_with_layer() -> (Map, Id) {
    let mut map = Map::new(Canvas::new(
        2048,
        2048,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Water".into(),
    })
    .unwrap();
    (map, layer)
}

fn path() -> Vec<RiverPoint> {
    vec![
        RiverPoint::new([100.0, 100.0], 6.0),
        RiverPoint::new([400.0, 250.0], 14.0),
        RiverPoint::new([800.0, 200.0], 30.0),
    ]
}

fn add_river(map: &mut Map, layer: Id, river: River) -> Id {
    let id = Id::new();
    map.execute(&Command::CreateRiver {
        id,
        parent: layer,
        index: None,
        river: Box::new(river),
    })
    .unwrap();
    map.execute(&Command::SetMoveLocked { id, locked: false })
        .unwrap();
    id
}

fn river(map: &Map, id: Id) -> &River {
    match map.node(id) {
        Some(Node::Object(object)) => match &object.kind {
            ObjectKind::River(river) => river,
            other => panic!("not a River: {other:?}"),
        },
        other => panic!("not an Object: {other:?}"),
    }
}

fn point(index: usize) -> NodeRef {
    NodeRef {
        part: 0,
        ring: 0,
        index,
    }
}

#[test]
fn a_river_has_an_outline_and_undo_removes_it() {
    let (mut map, layer) = map_with_layer();
    let before = map.clone();
    let id = Id::new();
    let inverses = map
        .execute(&Command::CreateRiver {
            id,
            parent: layer,
            index: None,
            river: Box::new(River::new(path())),
        })
        .unwrap();
    let outline = river(&map, id).outline().clone();
    assert_eq!(outline.problem(), None);
    assert!(outline.contains([400.0, 250.0]));
    map.revert(inverses);
    assert_eq!(map.node(id), None);
    assert_eq!(map.children(Some(layer)), before.children(Some(layer)));
}

#[test]
fn an_invalid_river_is_refused() {
    let (mut map, layer) = map_with_layer();
    let one_point = River::new(vec![RiverPoint::new([0.0, 0.0], 5.0)]);
    let err = map
        .execute(&Command::CreateRiver {
            id: Id::new(),
            parent: layer,
            index: None,
            river: Box::new(one_point),
        })
        .unwrap_err();
    assert!(matches!(err, CommandError::InvalidRiver { .. }), "{err}");
}

#[test]
fn moving_a_point_regenerates_the_outline_and_undo_restores_it() {
    let (mut map, layer) = map_with_layer();
    let id = add_river(&mut map, layer, River::new(path()));
    let original = river(&map, id).clone();
    let inverses = map
        .execute(&Command::MoveNodes {
            id,
            nodes: vec![point(1)],
            offset: [0.0, 100.0],
        })
        .unwrap();
    let moved = river(&map, id);
    assert_eq!(moved.path()[1].position, [400.0, 350.0]);
    assert!(moved.outline().contains([400.0, 350.0]));
    assert_ne!(moved.outline(), original.outline());
    map.revert(inverses);
    assert_eq!(river(&map, id), &original);
}

#[test]
fn an_inserted_point_takes_the_width_where_it_lands() {
    let (mut map, layer) = map_with_layer();
    let id = add_river(&mut map, layer, River::new(path()));
    map.execute(&Command::InsertNode {
        id,
        edge: point(1),
        point: [600.0, 225.0],
    })
    .unwrap();
    let added = river(&map, id).path()[2];
    assert_eq!(added.position, [600.0, 225.0]);
    assert!((added.width - 22.0).abs() < 1e-9, "{}", added.width);
}

#[test]
fn a_river_keeps_two_points() {
    let (mut map, layer) = map_with_layer();
    let id = add_river(&mut map, layer, River::new(path()));
    map.execute(&Command::DeleteNodes {
        id,
        nodes: vec![point(1)],
    })
    .unwrap();
    assert_eq!(river(&map, id).path().len(), 2);
    let err = map
        .execute(&Command::DeleteNodes {
            id,
            nodes: vec![point(0)],
        })
        .unwrap_err();
    assert!(matches!(err, CommandError::NodeEdit { .. }), "{err}");
}

#[test]
fn transforming_a_river_moves_its_points_and_scales_its_widths() {
    let (mut map, layer) = map_with_layer();
    let id = add_river(&mut map, layer, River::new(path()));
    map.execute(&Command::Transform {
        ids: vec![id],
        transform: Affine([2.0, 0.0, 0.0, 2.0, 10.0, 0.0]),
    })
    .unwrap();
    let moved = river(&map, id).path();
    assert_eq!(moved[0].position, [210.0, 200.0]);
    assert_eq!(moved[2].width, 60.0);
}

#[test]
fn a_new_seed_changes_the_bends_and_keeps_the_points() {
    let (mut map, layer) = map_with_layer();
    let id = add_river(&mut map, layer, River::new(path()));
    let before = river(&map, id).clone();
    map.execute(&Command::SetRiver {
        id,
        river: Box::new(before.clone().with_seed(before.seed() + 1)),
    })
    .unwrap();
    let after = river(&map, id);
    assert_eq!(after.path(), before.path());
    assert_ne!(after.outline(), before.outline());
}

#[test]
fn rivers_refuse_region_edits_and_roughness() {
    let (mut map, layer) = map_with_layer();
    let id = add_river(&mut map, layer, River::new(path()));
    let region = MultiPolygon(vec![Polygon {
        exterior: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
        holes: Vec::new(),
    }]);
    let err = map.execute(&Command::AddRegion { id, region }).unwrap_err();
    assert_eq!(err, CommandError::RiverOutline(id));
    let err = map
        .execute(&Command::SetRoughness {
            ids: vec![id],
            roughness: None,
        })
        .unwrap_err();
    assert_eq!(err, CommandError::RiverOutline(id));
    // Fill and Shore style apply as they do to a Lake.
    map.execute(&Command::SetFill {
        ids: vec![id],
        fill: Color::rgb(1, 2, 3),
    })
    .unwrap();
    assert_eq!(river(&map, id).fill(), Color::rgb(1, 2, 3));
}

#[test]
fn rivers_draw_with_the_lakes_above_land() {
    let (mut map, layer) = map_with_layer();
    let id = add_river(&mut map, layer, River::new(path()));
    let land = Id::new();
    map.execute(&Command::CreateLandmass {
        id: land,
        parent: layer,
        index: None,
        geometry: MultiPolygon(vec![Polygon {
            exterior: vec![[0.0, 0.0], [1000.0, 0.0], [1000.0, 1000.0], [0.0, 1000.0]],
            holes: Vec::new(),
        }]),
        fill: None,
    })
    .unwrap();
    assert_eq!(map.draw_order(), vec![land, id]);
}

#[test]
fn a_river_is_picked_and_is_not_land_for_scatter() {
    let (mut map, layer) = map_with_layer();
    map.execute(&Command::CreateLandmass {
        id: Id::new(),
        parent: layer,
        index: None,
        geometry: MultiPolygon(vec![Polygon {
            exterior: vec![[0.0, 0.0], [1000.0, 0.0], [1000.0, 1000.0], [0.0, 1000.0]],
            holes: Vec::new(),
        }]),
        fill: None,
    })
    .unwrap();
    let id = add_river(&mut map, layer, River::new(path()));
    let mut cache = RoughnessCache::new();
    let index = PickIndex::build(&map, &mut cache);
    assert_eq!(index.pick([400.0, 250.0]), Some(id));
    let terrain = Terrain::of(&map, &mut cache);
    assert!(!terrain.allows(Placement::Land, [400.0, 250.0]));
    assert!(terrain.allows(Placement::Water, [400.0, 250.0]));
    assert!(terrain.allows(Placement::Land, [400.0, 600.0]));
}

#[test]
fn a_river_survives_saving_and_opening() {
    let (mut map, layer) = map_with_layer();
    let id = add_river(
        &mut map,
        layer,
        River::new(path()).with_seed(9).with_meander(0.8),
    );
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("rivers.ssmap");
    MapFile::create(&file, &map, None).unwrap();
    let (_, opened) = MapFile::open(&file).unwrap();
    assert_eq!(river(&opened, id), river(&map, id));
}

#[test]
fn a_duplicated_river_moves_by_the_offset() {
    let (mut map, layer) = map_with_layer();
    let id = add_river(&mut map, layer, River::new(path()));
    let copy = Id::new();
    map.execute(&Command::Duplicate {
        ids: vec![id],
        new_ids: vec![copy],
        offset: [50.0, 0.0],
    })
    .unwrap();
    assert_eq!(river(&map, copy).path()[0].position, [150.0, 100.0]);
    assert_eq!(river(&map, copy).seed(), river(&map, id).seed());
}

#[test]
fn the_generated_outline_matches_its_snapshot() {
    // Rounded, so the snapshot shows the shape without the last bits of each coordinate.
    let river = River::new(path()).with_seed(3);
    let rounded: Vec<Vec<[f64; 2]>> = river
        .outline()
        .0
        .iter()
        .map(|polygon| {
            polygon
                .exterior
                .iter()
                .step_by(16)
                .map(|p| [(p[0] * 10.0).round() / 10.0, (p[1] * 10.0).round() / 10.0])
                .collect()
        })
        .collect();
    insta::assert_json_snapshot!(rounded);
}
