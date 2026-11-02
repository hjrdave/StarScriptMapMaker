//! Layer categories (ADR 0013): an Object goes only into a Layer of its own category, a Group
//! holds one category, and files from before categories get theirs when opened.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::file::MapFile;
use ssmap_core::{
    Canvas, Category, Command, CommandError, DistanceUnit, Id, Label, Layer, Map, MultiPolygon,
    Node, Polygon, Scale, Stamp, Subtree,
};

fn new_map() -> Map {
    Map::new(Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles)))
}

fn layer_in(map: &mut Map, parent: Option<Id>, name: &str, category: Category) -> Id {
    let id = Id::new();
    map.execute(&Command::CreateLayer {
        id,
        parent,
        index: None,
        name: name.into(),
        category,
    })
    .unwrap();
    id
}

fn square() -> MultiPolygon {
    MultiPolygon(vec![Polygon {
        exterior: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
        holes: Vec::new(),
    }])
}

/// The commands that put one Object of each kind into `parent`, with the category each belongs to.
fn creates(parent: Id) -> Vec<(Category, Command)> {
    vec![
        (
            Category::Labels,
            Command::CreateLabel {
                id: Id::new(),
                parent,
                index: None,
                label: Box::new(Label {
                    text: "Hello".into(),
                    ..Label::default()
                }),
            },
        ),
        (
            Category::Assets,
            Command::CreateStamp {
                id: Id::new(),
                parent,
                index: None,
                stamp: Box::new(Stamp::default()),
            },
        ),
        (
            Category::Land,
            Command::CreateLandmass {
                id: Id::new(),
                parent,
                index: None,
                geometry: square(),
                fill: None,
            },
        ),
        (
            Category::Land,
            Command::CreateLake {
                id: Id::new(),
                parent,
                index: None,
                geometry: square(),
                fill: None,
            },
        ),
    ]
}

#[test]
fn an_object_goes_only_into_a_layer_of_its_category_for_every_pair() {
    for layer_category in Category::ALL {
        let mut map = new_map();
        let layer = layer_in(&mut map, None, "Layer", layer_category);
        for (object_category, command) in creates(layer) {
            let before = map.view();
            let result = map.execute(&command);
            if object_category == layer_category {
                assert!(result.is_ok(), "{object_category} into {layer_category}");
            } else {
                assert!(
                    matches!(result, Err(CommandError::WrongCategory { .. })),
                    "{object_category} into {layer_category}: {result:?}"
                );
                assert_eq!(map.view(), before, "a refusal changes nothing");
            }
        }
    }
}

#[test]
fn the_refusal_names_both_categories() {
    let mut map = new_map();
    let stamps = layer_in(&mut map, None, "Trees", Category::Assets);
    let (_, land) = creates(stamps).remove(2);
    let err = map.execute(&land).unwrap_err();
    assert!(
        err.to_string().contains("is Land and cannot go into"),
        "{err}"
    );
    assert!(err.to_string().contains("holds Assets"), "{err}");
}

#[test]
fn moving_an_object_or_a_layer_is_refused_into_another_category_and_allowed_into_its_own() {
    let mut map = new_map();
    let assets = layer_in(&mut map, None, "Assets", Category::Assets);
    let more_assets = layer_in(&mut map, None, "More assets", Category::Assets);
    let land = layer_in(&mut map, None, "Land", Category::Land);
    let stamp = Id::new();
    map.execute(&Command::CreateStamp {
        id: stamp,
        parent: assets,
        index: None,
        stamp: Box::new(Stamp::default()),
    })
    .unwrap();

    let into = |id, parent| Command::Move {
        id,
        parent: Some(parent),
        index: 0,
    };
    assert!(map.execute(&into(stamp, more_assets)).is_ok());
    let err = map.execute(&into(stamp, land)).unwrap_err();
    assert!(matches!(err, CommandError::WrongCategory { .. }), "{err}");

    // A Layer nests only in a Group of its own category.
    let group = layer_in(&mut map, None, "Group", Category::Assets);
    assert!(map.execute(&into(more_assets, group)).is_ok());
    let err = map.execute(&into(land, group)).unwrap_err();
    assert!(matches!(err, CommandError::WrongCategory { .. }), "{err}");
    let err = map
        .execute(&Command::CreateLayer {
            id: Id::new(),
            parent: Some(group),
            index: None,
            name: "Nested".into(),
            category: Category::Labels,
        })
        .unwrap_err();
    assert!(matches!(err, CommandError::WrongCategory { .. }), "{err}");
}

#[test]
fn a_group_takes_its_members_category_and_cannot_mix_two() {
    let mut map = new_map();
    let a = layer_in(&mut map, None, "A", Category::Assets);
    let b = layer_in(&mut map, None, "B", Category::Assets);
    let l = layer_in(&mut map, None, "L", Category::Labels);
    let group = Id::new();
    map.execute(&Command::CreateGroup {
        id: group,
        ids: vec![a, b],
        name: "Forest".into(),
    })
    .unwrap();
    assert!(matches!(
        map.node(group),
        Some(Node::Layer(Layer {
            category: Category::Assets,
            ..
        }))
    ));
    let err = map
        .execute(&Command::CreateGroup {
            id: Id::new(),
            ids: vec![group, l],
            name: "Mixed".into(),
        })
        .unwrap_err();
    assert!(matches!(err, CommandError::MixedCategories(_)), "{err}");
}

#[test]
fn a_paste_goes_only_into_a_layer_of_its_category() {
    let mut map = new_map();
    let assets = layer_in(&mut map, None, "Assets", Category::Assets);
    let land = layer_in(&mut map, None, "Land", Category::Land);
    let branch = Subtree {
        id: Id::new(),
        node: Node::Object(ssmap_core::Object::new(ssmap_core::ObjectKind::Stamp(
            Stamp::default(),
        ))),
        children: Vec::new(),
    };
    let paste = |parent| Command::Paste {
        parent: Some(parent),
        index: None,
        subtrees: vec![branch.clone()],
        new_ids: vec![Id::new()],
        offset: [0.0; 2],
    };
    assert!(map.execute(&paste(assets)).is_ok());
    let err = map.execute(&paste(land)).unwrap_err();
    assert!(matches!(err, CommandError::WrongCategory { .. }), "{err}");
}

#[test]
fn undo_and_redo_are_not_blocked_by_the_rule() {
    let mut map = new_map();
    let assets = layer_in(&mut map, None, "Assets", Category::Assets);
    let stamp = Id::new();
    let inverse = map
        .execute(&Command::CreateStamp {
            id: stamp,
            parent: assets,
            index: None,
            stamp: Box::new(Stamp::default()),
        })
        .unwrap();
    let with = map.view();
    map.revert(inverse);
    assert!(map.node(stamp).is_none());
    // The Ops of the recorded step still apply, as redo does.
    let redo = map
        .execute(&Command::CreateStamp {
            id: stamp,
            parent: assets,
            index: None,
            stamp: Box::new(Stamp::default()),
        })
        .unwrap();
    drop(redo);
    assert_eq!(map.view(), with);
}

#[test]
fn a_layers_category_survives_a_save_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cats.ssmap");
    let mut map = new_map();
    layer_in(&mut map, None, "Names", Category::Labels);
    layer_in(&mut map, None, "Trees", Category::Assets);
    MapFile::create(&path, &map, None).unwrap();
    let (_, reopened) = MapFile::open(&path).unwrap();
    assert_eq!(reopened.view(), map.view());
}

#[test]
fn a_file_from_before_categories_gets_them_when_opened() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.ssmap");
    let mut map = new_map();
    // Stacking order from the bottom: Labels, Stamps, a mixed Layer holding both.
    let names = layer_in(&mut map, None, "Names", Category::Labels);
    let trees = layer_in(&mut map, None, "Trees", Category::Assets);
    let both = layer_in(&mut map, None, "Both", Category::Land);
    map.execute(&creates(names).remove(0).1).unwrap();
    map.execute(&creates(trees).remove(1).1).unwrap();
    for (_, command) in creates(both)
        .into_iter()
        .filter(|(c, _)| *c == Category::Land)
    {
        map.execute(&command).unwrap();
    }
    MapFile::create(&path, &map, None).unwrap();

    // Make it look like format 1.11: no category in any Layer's props.
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "UPDATE nodes SET props = json_remove(props, '$.category') WHERE kind = 'layer';
         UPDATE meta SET value = '1.11' WHERE key = 'format_version';",
    )
    .unwrap();
    drop(conn);

    let (_, opened) = MapFile::open(&path).unwrap();
    let category = |id| match opened.node(id) {
        Some(Node::Layer(layer)) => layer.category,
        _ => panic!("a Layer"),
    };
    assert_eq!(category(names), Category::Labels);
    assert_eq!(category(trees), Category::Assets);
    assert_eq!(category(both), Category::Land);
    // Put in the fixed order, lowest first: Land, Assets, Labels on top.
    assert_eq!(opened.children(None), &[both, trees, names]);
    opened.validate().unwrap();
}
