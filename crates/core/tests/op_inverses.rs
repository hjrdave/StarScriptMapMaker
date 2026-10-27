//! "Apply an Op, then its inverse, restores the Map" for random trees and random Ops (ADR 0004).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use proptest::prelude::*;
use ssmap_core::{
    ArtBoard, Background, Canvas, Color, DistanceUnit, Halo, Id, Label, Lake, Landmass, Layer, Map,
    MapDefaults, MapView, Membership, MultiPolygon, Node, Object, ObjectKind, Op, Point, Polygon,
    Property, Roughness, Scale, Stamp, Subtree,
};

fn canvas() -> Canvas {
    Canvas::new(4096, 3072, Scale::new(0.5, DistanceUnit::Miles))
}

fn point() -> impl Strategy<Value = Point> {
    [-1e4..1e4f64, -1e4..1e4f64]
}

fn geometry() -> impl Strategy<Value = MultiPolygon> {
    let ring = || prop::collection::vec(point(), 3..6);
    let polygon = (ring(), prop::collection::vec(ring(), 0..2))
        .prop_map(|(exterior, holes)| Polygon { exterior, holes });
    prop::collection::vec(polygon, 0..3).prop_map(MultiPolygon)
}

fn roughness() -> impl Strategy<Value = Roughness> {
    (0.0..50.0f64, 0.5..500.0f64, any::<u32>())
        .prop_map(|(amount, scale, seed)| Roughness::new(amount, scale, seed))
}

fn label() -> impl Strategy<Value = Label> {
    (
        "[A-Za-zÆá\n ]{1,12}",
        point(),
        -360.0..360.0f64,
        1.0..200.0f64,
        any::<bool>(),
        (0.0..=1.0f32, 0.0..8.0f64),
    )
        .prop_map(
            |(text, position, rotation, size, all_caps, (opacity, halo))| {
                let mut label = Label {
                    text,
                    position,
                    rotation,
                    size: Some(size),
                    all_caps: Some(all_caps),
                    opacity: Some(opacity),
                    ..Label::default()
                };
                label.halo = Some(Halo {
                    width: halo,
                    ..Halo::default()
                });
                label
            },
        )
}

fn object() -> impl Strategy<Value = Object> {
    let kind = prop_oneof![
        (geometry(), prop::option::of(roughness())).prop_map(|(geometry, roughness)| {
            ObjectKind::Landmass(Landmass {
                geometry,
                roughness,
                ..Default::default()
            })
        }),
        (geometry(), prop::option::of(roughness())).prop_map(|(geometry, roughness)| {
            ObjectKind::Lake(Lake {
                geometry,
                roughness,
                ..Default::default()
            })
        }),
        Just(ObjectKind::Stamp(Stamp::default())),
        label().prop_map(|label| ObjectKind::Label(Box::new(label))),
    ];
    (any::<bool>(), kind).prop_map(|(locked, kind)| Object {
        locked,
        ..Object::new(kind)
    })
}

fn layer() -> impl Strategy<Value = Layer> {
    ("[a-z ]{0,8}", any::<bool>(), any::<bool>(), 0.0..=1.0f32).prop_map(
        |(name, visible, locked, opacity)| Layer {
            name,
            visible,
            locked,
            opacity,
            ..Layer::new(String::new())
        },
    )
}

/// A Layer with a random branch of Layers and Objects beneath it. Ids are fresh UUIDv7s.
fn layer_subtree() -> impl Strategy<Value = Subtree> {
    let leaf = prop_oneof![
        object().prop_map(|object| leaf(Node::Object(object))),
        layer().prop_map(|layer| leaf(Node::Layer(layer))),
    ];
    let branch = leaf.prop_recursive(3, 24, 4, |inner| {
        (layer(), prop::collection::vec(inner, 0..4)).prop_map(|(layer, children)| Subtree {
            id: Id::new(),
            node: Node::Layer(layer),
            children,
        })
    });
    (layer(), prop::collection::vec(branch, 0..4)).prop_map(|(layer, children)| Subtree {
        id: Id::new(),
        node: Node::Layer(layer),
        children,
    })
}

fn leaf(node: Node) -> Subtree {
    Subtree {
        id: Id::new(),
        node,
        children: Vec::new(),
    }
}

fn art_board() -> impl Strategy<Value = ArtBoard> {
    ("[a-z ]{0,6}", point(), 1..200u32, 1..200u32).prop_map(|(name, origin, width, height)| {
        ArtBoard::new(
            Id::new(),
            name,
            origin,
            width,
            height,
            Background::default(),
        )
    })
}

fn map() -> impl Strategy<Value = Map> {
    (
        prop::collection::vec(layer_subtree(), 0..4),
        roughness(),
        prop::collection::vec(art_board(), 0..3),
        prop::collection::vec((any::<usize>(), any::<usize>()), 0..6),
    )
        .prop_map(|(layers, roughness, art_boards, picks)| {
            let mut map = Map::from_view(MapView {
                canvas: canvas(),
                art_boards,
                defaults: MapDefaults {
                    roughness,
                    ..Default::default()
                },
                layers,
                embedded: Default::default(),
                export_settings: Default::default(),
            })
            .expect("generated trees are valid");
            // Some Objects belong to Art boards, some to the Canvas, some to neither.
            let boards: Vec<Id> = map.art_boards().iter().map(|board| board.id).collect();
            let objects: Vec<Id> = ids(&map)
                .into_iter()
                .filter(|&id| matches!(map.node(id), Some(Node::Object(_))))
                .collect();
            if !boards.is_empty() && !objects.is_empty() {
                for (object, board) in picks {
                    let membership = match board % (boards.len() + 1) {
                        n if n == boards.len() => Membership::OffCanvas,
                        n => Membership::Board(boards[n]),
                    };
                    map.apply(Op::SetProperty {
                        id: objects[object % objects.len()],
                        property: Property::Membership(membership),
                    })
                    .expect("the board exists");
                }
            }
            map
        })
}

/// An Op described by selectors, resolved against whatever Map it meets. Selectors that miss
/// produce invalid Ops on purpose, which must fail without changing the Map.
#[derive(Clone, Debug)]
enum OpSpec {
    Insert {
        parent: usize,
        index: usize,
        subtree: Subtree,
    },
    InsertObject {
        parent: usize,
        index: usize,
        object: Object,
    },
    Remove {
        node: usize,
    },
    SetProperty {
        node: usize,
        property: Property,
    },
    ReplaceGeometry {
        node: usize,
        geometry: MultiPolygon,
    },
    Move {
        node: usize,
        parent: usize,
        index: usize,
    },
    Clip {
        layer: usize,
        target: usize,
    },
    SetDefaults {
        roughness: Roughness,
    },
    Board(BoardSpec),
    Membership {
        node: usize,
        board: usize,
    },
    Unknown,
}

/// A board Op described by selectors; `board` picks the Canvas, an Art board or, one past
/// them, an id the Map does not have.
#[derive(Clone, Debug)]
enum BoardSpec {
    Add {
        index: usize,
        board: ArtBoard,
    },
    Remove {
        board: usize,
    },
    Resize {
        board: usize,
        origin: Point,
        width: u32,
        height: u32,
    },
    Background {
        board: usize,
    },
    Rename {
        board: usize,
        name: String,
    },
    Move {
        board: usize,
        index: usize,
    },
    Primary {
        board: usize,
    },
}

fn board_spec() -> impl Strategy<Value = BoardSpec> {
    let sel = || any::<usize>();
    prop_oneof![
        (sel(), art_board()).prop_map(|(index, board)| BoardSpec::Add { index, board }),
        sel().prop_map(|board| BoardSpec::Remove { board }),
        (sel(), point(), 1..200u32, 1..200u32).prop_map(|(board, origin, width, height)| {
            BoardSpec::Resize {
                board,
                origin,
                width,
                height,
            }
        }),
        sel().prop_map(|board| BoardSpec::Background { board }),
        (sel(), "[a-z ]{0,6}").prop_map(|(board, name)| BoardSpec::Rename { board, name }),
        (sel(), sel()).prop_map(|(board, index)| BoardSpec::Move { board, index }),
        sel().prop_map(|board| BoardSpec::Primary { board }),
    ]
}

impl BoardSpec {
    fn resolve(self, map: &Map) -> Op {
        let boards: Vec<Id> = std::iter::once(map.canvas().id)
            .chain(map.art_boards().iter().map(|board| board.id))
            .collect();
        let pick = |sel: usize| {
            boards
                .get(sel % (boards.len() + 1))
                .copied()
                .unwrap_or_default()
        };
        // Indexes run up to two past the end, to cover out-of-range errors.
        let index = |sel: usize| sel % (map.art_boards().len() + 3);
        match self {
            BoardSpec::Add { index: i, board } => Op::AddBoard {
                index: index(i),
                board,
            },
            BoardSpec::Remove { board } => Op::RemoveBoard { id: pick(board) },
            BoardSpec::Resize {
                board,
                origin,
                width,
                height,
            } => Op::ResizeBoard {
                id: pick(board),
                origin,
                width,
                height,
            },
            BoardSpec::Background { board } => Op::SetBoardBackground {
                id: pick(board),
                background: Background {
                    water: Color::rgb(1, 2, 3),
                    ..Background::default()
                },
            },
            BoardSpec::Rename { board, name } => Op::RenameBoard {
                id: pick(board),
                name,
            },
            BoardSpec::Move { board, index: i } => Op::MoveBoard {
                id: pick(board),
                index: index(i),
            },
            BoardSpec::Primary { board } => Op::SetPrimaryBoard { id: pick(board) },
        }
    }
}

fn property() -> impl Strategy<Value = Property> {
    prop_oneof![
        "[a-z ]{0,8}".prop_map(Property::Name),
        any::<bool>().prop_map(Property::Visible),
        any::<bool>().prop_map(Property::Locked),
        (0.0..=1.0f32).prop_map(Property::Opacity),
        Just(Property::Clip(None)),
        any::<[u8; 4]>().prop_map(|[r, g, b, a]| Property::Fill(Color::rgba(r, g, b, a))),
        prop::option::of(roughness()).prop_map(Property::Roughness),
        label().prop_map(|label| Property::Label(Box::new(label))),
    ]
}

fn op_spec() -> impl Strategy<Value = OpSpec> {
    let sel = || any::<usize>();
    prop_oneof![
        (sel(), sel(), layer_subtree()).prop_map(|(parent, index, subtree)| OpSpec::Insert {
            parent,
            index,
            subtree
        }),
        (sel(), sel(), object()).prop_map(|(parent, index, object)| OpSpec::InsertObject {
            parent,
            index,
            object
        }),
        sel().prop_map(|node| OpSpec::Remove { node }),
        (sel(), property()).prop_map(|(node, property)| OpSpec::SetProperty { node, property }),
        (sel(), geometry()).prop_map(|(node, geometry)| OpSpec::ReplaceGeometry { node, geometry }),
        (sel(), sel(), sel()).prop_map(|(node, parent, index)| OpSpec::Move {
            node,
            parent,
            index
        }),
        (sel(), sel()).prop_map(|(layer, target)| OpSpec::Clip { layer, target }),
        roughness().prop_map(|roughness| OpSpec::SetDefaults { roughness }),
        board_spec().prop_map(OpSpec::Board),
        (sel(), sel()).prop_map(|(node, board)| OpSpec::Membership { node, board }),
        Just(OpSpec::Unknown),
    ]
}

/// Every id in the Map, in tree order.
fn ids(map: &Map) -> Vec<Id> {
    fn walk(map: &Map, parent: Option<Id>, out: &mut Vec<Id>) {
        for &id in map.children(parent) {
            out.push(id);
            walk(map, Some(id), out);
        }
    }
    let mut out = Vec::new();
    walk(map, None, &mut out);
    out
}

impl OpSpec {
    fn resolve(self, map: &Map) -> Op {
        let ids = ids(map);
        // Selecting one past the last id picks the top level.
        let parent = |sel: usize| ids.get(sel % (ids.len() + 1)).copied();
        // Indexes may run up to two past the end, to cover out-of-range errors.
        let index = |sel: usize, parent: Option<Id>| sel % (map.children(parent).len() + 3);
        let node = |sel: usize| ids.get(sel % ids.len().max(1)).copied().unwrap_or_default();
        match self {
            OpSpec::Insert {
                parent: p,
                index: i,
                subtree,
            } => {
                let parent = parent(p);
                let index = index(i, parent);
                Op::Insert {
                    parent,
                    index,
                    subtree,
                }
            }
            OpSpec::InsertObject {
                parent: p,
                index: i,
                object,
            } => {
                let parent = parent(p);
                let index = index(i, parent);
                Op::Insert {
                    parent,
                    index,
                    subtree: leaf(Node::Object(object)),
                }
            }
            OpSpec::Remove { node: n } => Op::Remove { id: node(n) },
            OpSpec::SetProperty { node: n, property } => Op::SetProperty {
                id: node(n),
                property,
            },
            OpSpec::ReplaceGeometry { node: n, geometry } => {
                // Prefer nodes that have geometry, so most of these Ops succeed.
                let outlined: Vec<Id> = ids
                    .iter()
                    .copied()
                    .filter(|&id| match map.node(id) {
                        Some(Node::Object(object)) => object.kind.geometry().is_some(),
                        _ => false,
                    })
                    .collect();
                let id = match outlined.len() {
                    0 => node(n),
                    len => outlined[n % len],
                };
                Op::ReplaceGeometry { id, geometry }
            }
            OpSpec::Move {
                node: n,
                parent: p,
                index: i,
            } => {
                let parent = parent(p);
                Op::Move {
                    id: node(n),
                    parent,
                    index: index(i, parent),
                }
            }
            OpSpec::SetDefaults { roughness } => Op::SetDefaults {
                defaults: MapDefaults {
                    roughness,
                    ..Default::default()
                },
            },
            OpSpec::Clip { layer, target } => {
                let pick = |kind: fn(&Node) -> bool, sel: usize| {
                    let matching: Vec<Id> = ids
                        .iter()
                        .copied()
                        .filter(|&id| map.node(id).is_some_and(kind))
                        .collect();
                    matching.get(sel % matching.len().max(1)).copied()
                };
                let is_landmass = |node: &Node| matches!(node, Node::Object(o) if matches!(o.kind, ObjectKind::Landmass(_)));
                Op::SetProperty {
                    id: pick(Node::is_layer, layer).unwrap_or_default(),
                    property: Property::Clip(pick(is_landmass, target)),
                }
            }
            OpSpec::Board(spec) => spec.resolve(map),
            OpSpec::Membership { node: n, board } => {
                let boards: Vec<Id> = map.art_boards().iter().map(|b| b.id).collect();
                // One past the boards: off the boards; two past: a board the Map lacks.
                let membership = match board % (boards.len() + 3) {
                    n if n < boards.len() => Membership::Board(boards[n]),
                    n if n == boards.len() => Membership::OffCanvas,
                    n if n == boards.len() + 1 => Membership::Canvas,
                    _ => Membership::Board(Id::new()),
                };
                Op::SetProperty {
                    id: node(n),
                    property: Property::Membership(membership),
                }
            }
            OpSpec::Unknown => Op::Remove { id: Id::new() },
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn op_then_inverse_restores_the_map(mut map in map(), spec in op_spec()) {
        let before = map.view();
        let op = spec.resolve(&map);
        if let Ok(inverse) = map.apply(op) {
            map.validate().unwrap();
            map.apply(inverse).expect("an inverse always applies");
            map.validate().unwrap();
        }
        prop_assert_eq!(map.view(), before);
    }

    #[test]
    fn undoing_a_sequence_restores_the_map(
        mut map in map(),
        specs in prop::collection::vec(op_spec(), 1..24),
    ) {
        let before = map.view();
        let mut inverses = Vec::new();
        for spec in specs {
            let op = spec.resolve(&map);
            if let Ok(inverse) = map.apply(op) {
                inverses.push(inverse);
            }
            map.validate().unwrap();
        }
        for inverse in inverses.into_iter().rev() {
            map.apply(inverse).expect("an inverse always applies");
        }
        prop_assert_eq!(map.view(), before);
    }

    #[test]
    fn failed_ops_leave_versions_alone(mut map in map(), spec in op_spec()) {
        let ids = ids(&map);
        let versions: Vec<_> = ids.iter().map(|&id| map.version(id)).collect();
        let revision = map.revision();
        if map.apply(spec.resolve(&map)).is_err() {
            prop_assert_eq!(map.revision(), revision);
            let after: Vec<_> = ids.iter().map(|&id| map.version(id)).collect();
            prop_assert_eq!(after, versions);
        }
    }

    #[test]
    fn ops_round_trip_through_json(map in map(), spec in op_spec()) {
        let op = spec.resolve(&map);
        let json = serde_json::to_string(&op).unwrap();
        prop_assert_eq!(serde_json::from_str::<Op>(&json).unwrap(), op);
    }

    #[test]
    fn views_round_trip_through_json(map in map()) {
        let view = map.view();
        let json = serde_json::to_string(&view).unwrap();
        let back: MapView = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(Map::from_view(back).unwrap().view(), view);
    }
}
