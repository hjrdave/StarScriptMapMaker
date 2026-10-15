use ssmap_core::{
    Canvas, DistanceUnit, Id, Landmass, Layer, Map, MapView, MultiPolygon, Node, Object,
    ObjectKind, Op, OpError, Polygon, Property, Scale, Stamp, Subtree,
};

fn empty_map() -> Map {
    Map::new(Canvas::new(
        1024,
        768,
        Scale::new(1.0, DistanceUnit::Kilometres),
    ))
}

fn layer(name: &str) -> Subtree {
    Subtree {
        id: Id::new(),
        node: Node::Layer(Layer::new(name)),
        children: Vec::new(),
    }
}

fn object(kind: ObjectKind) -> Subtree {
    Subtree {
        id: Id::new(),
        node: Node::Object(Object::new(kind)),
        children: Vec::new(),
    }
}

fn insert(map: &mut Map, parent: Option<Id>, subtree: Subtree) -> Id {
    let id = subtree.id;
    let index = map.children(parent).len();
    map.apply(Op::Insert {
        parent,
        index,
        subtree,
    })
    .unwrap();
    id
}

#[test]
fn ids_are_uuid_v7() {
    assert_eq!(Id::new().as_uuid().get_version_num(), 7);
}

#[test]
fn objects_cannot_sit_at_the_top_level() {
    let mut map = empty_map();
    let err = map
        .apply(Op::Insert {
            parent: None,
            index: 0,
            subtree: object(ObjectKind::Stamp(Stamp::default())),
        })
        .unwrap_err();
    assert_eq!(err, OpError::ObjectAtRoot);
}

#[test]
fn objects_cannot_hold_children() {
    let mut map = empty_map();
    let land = insert(&mut map, None, layer("Land"));
    let stamp = insert(
        &mut map,
        Some(land),
        object(ObjectKind::Stamp(Stamp::default())),
    );
    let err = map
        .apply(Op::Move {
            id: land,
            parent: Some(stamp),
            index: 0,
        })
        .unwrap_err();
    assert_eq!(err, OpError::NotALayer(stamp));
}

#[test]
fn a_layer_cannot_move_into_its_own_descendant() {
    let mut map = empty_map();
    let outer = insert(&mut map, None, layer("Outer"));
    let inner = insert(&mut map, Some(outer), layer("Inner"));
    let err = map
        .apply(Op::Move {
            id: outer,
            parent: Some(inner),
            index: 0,
        })
        .unwrap_err();
    assert_eq!(err, OpError::MoveIntoSelf(outer));
}

#[test]
fn objects_only_have_the_locked_property() {
    let mut map = empty_map();
    let land = insert(&mut map, None, layer("Land"));
    let stamp = insert(
        &mut map,
        Some(land),
        object(ObjectKind::Stamp(Stamp::default())),
    );
    assert_eq!(
        map.apply(Op::SetProperty {
            id: stamp,
            property: Property::Opacity(0.5),
        }),
        Err(OpError::NoSuchProperty(stamp, "opacity"))
    );
    map.apply(Op::SetProperty {
        id: stamp,
        property: Property::Locked(true),
    })
    .unwrap();
    assert!(map.node(stamp).unwrap().locked());
}

#[test]
fn only_landmasses_and_lakes_have_geometry() {
    let mut map = empty_map();
    let land = insert(&mut map, None, layer("Land"));
    let stamp = insert(
        &mut map,
        Some(land),
        object(ObjectKind::Stamp(Stamp::default())),
    );
    assert_eq!(
        map.apply(Op::ReplaceGeometry {
            id: stamp,
            geometry: MultiPolygon::default(),
        }),
        Err(OpError::NoGeometry(stamp))
    );
}

#[test]
fn touched_nodes_get_a_new_version_and_others_keep_theirs() {
    let mut map = empty_map();
    let land = insert(&mut map, None, layer("Land"));
    let island = insert(
        &mut map,
        Some(land),
        object(ObjectKind::Landmass(Landmass::default())),
    );
    let other = insert(
        &mut map,
        Some(land),
        object(ObjectKind::Landmass(Landmass::default())),
    );
    let island_before = map.version(island).unwrap();
    let other_before = map.version(other).unwrap();

    let geometry = MultiPolygon(vec![Polygon {
        exterior: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
        holes: Vec::new(),
    }]);
    let inverse = map
        .apply(Op::ReplaceGeometry {
            id: island,
            geometry,
        })
        .unwrap();
    let after_edit = map.version(island).unwrap();
    assert!(after_edit > island_before);
    assert_eq!(map.version(other), Some(other_before));

    // Undo restores content but still counts as a change, so caches never see a stale version.
    map.apply(inverse).unwrap();
    assert!(map.version(island).unwrap() > after_edit);
}

#[test]
fn moving_touches_both_parents() {
    let mut map = empty_map();
    let a = insert(&mut map, None, layer("A"));
    let b = insert(&mut map, None, layer("B"));
    let stamp = insert(
        &mut map,
        Some(a),
        object(ObjectKind::Stamp(Stamp::default())),
    );
    let before = (map.version(a).unwrap(), map.version(b).unwrap());
    map.apply(Op::Move {
        id: stamp,
        parent: Some(b),
        index: 0,
    })
    .unwrap();
    assert!(map.version(a).unwrap() > before.0);
    assert!(map.version(b).unwrap() > before.1);
    assert_eq!(map.parent(stamp), Some(b));
}

#[test]
fn removing_a_layer_returns_its_whole_branch() {
    let mut map = empty_map();
    let land = insert(&mut map, None, layer("Land"));
    let group = insert(&mut map, Some(land), layer("Group"));
    let stamp = insert(
        &mut map,
        Some(group),
        object(ObjectKind::Stamp(Stamp::default())),
    );
    let inverse = map.apply(Op::Remove { id: group }).unwrap();
    assert!(!map.contains(stamp));
    let Op::Insert { subtree, .. } = &inverse else {
        panic!("removal inverts to insertion");
    };
    assert_eq!(subtree.children[0].id, stamp);
    map.apply(inverse).unwrap();
    assert_eq!(map.parent(stamp), Some(group));
}

#[test]
fn from_view_rejects_duplicate_ids() {
    let layer = layer("Land");
    let view = MapView {
        canvas: empty_map().canvas,
        defaults: Default::default(),
        layers: vec![layer.clone(), layer],
        embedded: Default::default(),
        export_settings: Default::default(),
    };
    assert!(Map::from_view(view).is_err());
}

#[test]
fn the_read_view_has_a_json_schema() {
    let schema = serde_json::to_value(schemars::schema_for!(MapView)).unwrap();
    assert_eq!(schema["title"], "MapView");
    let op_schema = serde_json::to_value(schemars::schema_for!(Op)).unwrap();
    assert_eq!(op_schema["title"], "Op");
}
