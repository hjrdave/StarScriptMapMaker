//! Depth sort: a Layer draws its direct Stamps by Anchor height, lower in front (ADR 0003).

use ssmap_core::{Canvas, Command, CommandError, DistanceUnit, Id, Map, Scale, Stamp};

fn stamp(y: f64) -> Stamp {
    Stamp::new("pack/asset".parse().unwrap(), [0.0, y])
}

/// A Layer holding Stamps at the given heights, in that tree order.
fn map_with(heights: &[f64]) -> (Map, Id, Vec<Id>) {
    let mut map = Map::new(Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles)));
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Mountains".into(),
    })
    .unwrap();
    let ids = heights
        .iter()
        .map(|&y| {
            let id = Id::new();
            map.execute(&Command::CreateStamp {
                id,
                parent: layer,
                index: None,
                stamp: Box::new(stamp(y)),
            })
            .unwrap();
            id
        })
        .collect();
    (map, layer, ids)
}

fn depth_sort(map: &mut Map, id: Id, on: bool) {
    map.execute(&Command::SetDepthSort { id, depth_sort: on })
        .unwrap();
}

#[test]
fn stamps_draw_by_anchor_height_lower_in_front() {
    let (mut map, layer, ids) = map_with(&[30.0, 10.0, 20.0]);
    assert_eq!(map.draw_order(), ids, "tree order without Depth sort");
    depth_sort(&mut map, layer, true);
    assert_eq!(map.draw_order(), vec![ids[1], ids[2], ids[0]]);
    assert_eq!(map.ordered_children(layer), vec![ids[1], ids[2], ids[0]]);
}

#[test]
fn moving_a_stamp_re_sorts_it() {
    let (mut map, layer, ids) = map_with(&[10.0, 20.0, 30.0]);
    depth_sort(&mut map, layer, true);
    assert_eq!(map.draw_order(), ids);
    map.execute(&Command::SetStamp {
        id: ids[0],
        stamp: Box::new(stamp(25.0)),
    })
    .unwrap();
    assert_eq!(map.draw_order(), vec![ids[1], ids[0], ids[2]]);
}

#[test]
fn layers_and_other_objects_keep_tree_order() {
    let (mut map, layer, ids) = map_with(&[30.0, 10.0]);
    let inner = Id::new();
    map.execute(&Command::CreateLayer {
        id: inner,
        parent: Some(layer),
        index: Some(1),
        name: "Inner".into(),
    })
    .unwrap();
    let inner_stamp = Id::new();
    map.execute(&Command::CreateStamp {
        id: inner_stamp,
        parent: inner,
        index: None,
        stamp: Box::new(stamp(99.0)),
    })
    .unwrap();
    depth_sort(&mut map, layer, true);
    // The nested Layer stays between the two Stamps; only the Stamps swap places.
    assert_eq!(map.ordered_children(layer), vec![ids[1], inner, ids[0]]);
    assert_eq!(map.draw_order(), vec![ids[1], inner_stamp, ids[0]]);
}

#[test]
fn stamps_cannot_be_reordered_by_hand_in_a_sorted_layer() {
    let (mut map, layer, ids) = map_with(&[10.0, 20.0]);
    depth_sort(&mut map, layer, true);
    let result = map.execute(&Command::Move {
        id: ids[0],
        parent: Some(layer),
        index: 1,
    });
    assert!(matches!(result, Err(CommandError::DepthSorted(_))));
    depth_sort(&mut map, layer, false);
    map.execute(&Command::Move {
        id: ids[0],
        parent: Some(layer),
        index: 1,
    })
    .unwrap();
}

#[test]
fn toggling_depth_sort_undoes_cleanly() {
    let (mut map, layer, ids) = map_with(&[30.0, 10.0]);
    let before = map.view();
    let mut history = ssmap_core::History::new(ssmap_core::DEFAULT_BUDGET_BYTES);
    let toggle = Command::SetDepthSort {
        id: layer,
        depth_sort: true,
    };
    history.execute_step(&mut map, &toggle, vec![]).unwrap();
    assert_eq!(map.draw_order(), vec![ids[1], ids[0]]);
    history.undo(&mut map).unwrap();
    assert_eq!(map.draw_order(), ids);
    assert_eq!(map.view(), before);
    history.redo(&mut map).unwrap();
    assert_eq!(map.draw_order(), vec![ids[1], ids[0]]);
}
