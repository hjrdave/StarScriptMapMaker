//! Coastline Roughness (ADR 0002).

use ssmap_core::{
    Canvas, DistanceUnit, Id, Landmass, Layer, Map, MultiPolygon, Node, Object, ObjectKind, Op,
    Polygon, Roughness, RoughnessCache, Scale, Subtree, roughen,
};

fn insert(map: &mut Map, parent: Option<Id>, node: Node) -> Id {
    let id = Id::new();
    let index = map.children(parent).len();
    map.apply(Op::Insert {
        parent,
        index,
        subtree: Subtree {
            id,
            node,
            children: Vec::new(),
        },
    })
    .unwrap();
    id
}

fn landmass(geometry: MultiPolygon) -> Node {
    Node::Object(Object::new(ObjectKind::Landmass(Landmass {
        geometry,
        ..Default::default()
    })))
}

fn island(dx: f64) -> MultiPolygon {
    MultiPolygon(vec![Polygon {
        exterior: vec![
            [dx + 0.1, 0.3],
            [dx + 412.75, -20.125],
            [dx + 530.0, 260.0 / 3.0],
            [dx + 380.5, 410.0],
            [dx - 12.0, 300.0 + 1e-9],
        ],
        holes: vec![vec![
            [dx + 150.0, 150.0],
            [dx + 150.0, 230.5],
            [dx + 260.0, 199.0],
        ]],
    }])
}

/// FNV-1a over the bits of every coordinate, in order.
fn fingerprint(outline: &MultiPolygon) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut feed = |value: f64| {
        for byte in value.to_bits().to_le_bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    for polygon in &outline.0 {
        for ring in std::iter::once(&polygon.exterior).chain(&polygon.holes) {
            feed(ring.len() as f64);
            for point in ring {
                feed(point[0]);
                feed(point[1]);
            }
        }
    }
    hash
}

/// The same input gives a bit-identical outline on every OS. CI runs this on Linux, Windows and
/// macOS; if the algorithm changes on purpose, update the expected values.
#[test]
fn roughened_outlines_are_bit_identical_everywhere() {
    let cases = [
        (Roughness::new(6.0, 40.0, 1), 0xadd2_efb4_920e_01e0_u64),
        (Roughness::new(0.75, 3.5, 42), 0x555d_33b7_51ea_cbfe),
        (Roughness::new(25.0, 300.0, u32::MAX), 0x594b_a984_0f67_c902),
    ];
    let got: Vec<u64> = cases
        .iter()
        .map(|(roughness, _)| fingerprint(&roughen(&island(0.0), roughness)))
        .collect();
    let expected: Vec<u64> = cases.iter().map(|(_, hash)| *hash).collect();
    assert_eq!(got, expected, "got {got:#018x?}");
}

#[test]
fn the_cache_recomputes_only_changed_objects() {
    let mut map = Map::new(Canvas::new(
        2048,
        2048,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    let layer = insert(&mut map, None, Node::Layer(Layer::new("Land")));
    let west = insert(&mut map, Some(layer), landmass(island(0.0)));
    let east = insert(&mut map, Some(layer), landmass(island(1000.0)));
    let roughness = Roughness::new(6.0, 40.0, 1);
    let mut cache = RoughnessCache::new();

    let west_first = cache.outline(&map, west, &roughness).unwrap();
    let east_first = cache.outline(&map, east, &roughness).unwrap();
    assert_eq!(cache.computed(), 2);
    assert_eq!(*west_first, roughen(&island(0.0), &roughness));

    // Asking again computes nothing.
    cache.outline(&map, west, &roughness).unwrap();
    cache.outline(&map, east, &roughness).unwrap();
    assert_eq!(cache.computed(), 2);

    // Editing one island recomputes only that island.
    map.apply(Op::ReplaceGeometry {
        id: east,
        geometry: island(1100.0),
    })
    .unwrap();
    let west_again = cache.outline(&map, west, &roughness).unwrap();
    let east_again = cache.outline(&map, east, &roughness).unwrap();
    assert_eq!(cache.computed(), 3);
    assert!(std::sync::Arc::ptr_eq(&west_first, &west_again));
    assert_ne!(east_first, east_again);

    // New settings recompute.
    cache
        .outline(&map, west, &Roughness::new(6.0, 40.0, 2))
        .unwrap();
    assert_eq!(cache.computed(), 4);

    // Layers have no outline; removed Objects are forgotten.
    assert!(cache.outline(&map, layer, &roughness).is_none());
    map.apply(Op::Remove { id: east }).unwrap();
    assert!(cache.outline(&map, east, &roughness).is_none());
    cache.retain(&map);
}
