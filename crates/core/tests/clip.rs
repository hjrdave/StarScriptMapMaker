//! Clip: a Layer restricted to a Landmass, and how it follows the Landmass's fate.

use ssmap_core::{
    Canvas, Command, CommandError, DistanceUnit, History, Id, Map, MultiPolygon, Node, Point,
    Polygon, Scale,
};

fn square(min: [f64; 2], size: f64) -> MultiPolygon {
    let [x, y] = min;
    let exterior: Vec<Point> = vec![[x, y], [x + size, y], [x + size, y + size], [x, y + size]];
    MultiPolygon(vec![Polygon {
        exterior,
        holes: vec![],
    }])
}

fn run(map: &mut Map, command: Command) {
    map.execute(&command)
        .unwrap_or_else(|err| panic!("{command:?}: {err}"));
}

fn layer(map: &mut Map, name: &str) -> Id {
    let id = Id::new();
    run(
        map,
        Command::CreateLayer {
            id,
            parent: None,
            index: None,
            name: name.into(),
        },
    );
    id
}

fn landmass(map: &mut Map, parent: Id, min: [f64; 2]) -> Id {
    let id = Id::new();
    run(
        map,
        Command::CreateLandmass {
            id,
            parent,
            index: None,
            geometry: square(min, 40.0),
            fill: None,
        },
    );
    id
}

fn clip_of(map: &Map, layer: Id) -> Option<Id> {
    match map.node(layer) {
        Some(Node::Layer(layer)) => layer.clip,
        _ => panic!("not a Layer"),
    }
}

/// A Map with two Landmasses and a Layer clipped to the first.
fn clipped() -> (Map, Id, Id, Id) {
    let mut map = Map::new(Canvas::new(200, 200, Scale::new(1.0, DistanceUnit::Miles)));
    let land = layer(&mut map, "Land");
    let first = landmass(&mut map, land, [0.0, 0.0]);
    let second = landmass(&mut map, land, [30.0, 0.0]);
    let art = layer(&mut map, "Art");
    run(
        &mut map,
        Command::SetClip {
            id: art,
            target: Some(first),
        },
    );
    (map, art, first, second)
}

#[test]
fn a_clip_names_a_landmass() {
    let (mut map, art, first, _) = clipped();
    assert_eq!(clip_of(&map, art), Some(first));
    run(
        &mut map,
        Command::SetClip {
            id: art,
            target: None,
        },
    );
    assert_eq!(clip_of(&map, art), None);

    let err = map
        .execute(&Command::SetClip {
            id: art,
            target: Some(art),
        })
        .unwrap_err();
    assert_eq!(err, CommandError::NotALandmass(art));
}

#[test]
fn deleting_the_target_clears_the_clip_and_undo_restores_both() {
    let (mut map, art, first, _) = clipped();
    let before = map.view();
    let mut history = History::default();
    history
        .execute_step(&mut map, &Command::Delete { ids: vec![first] }, vec![])
        .unwrap();
    assert!(map.node(first).is_none());
    assert_eq!(clip_of(&map, art), None);
    history.undo(&mut map).unwrap();
    assert!(map.node(first).is_some());
    assert_eq!(clip_of(&map, art), Some(first));
    assert_eq!(map.view(), before);
}

#[test]
fn union_retargets_the_clip() {
    let (mut map, art, first, second) = clipped();
    let mut history = History::default();
    history
        .execute_step(
            &mut map,
            &Command::Union {
                ids: vec![first, second],
            },
            vec![],
        )
        .unwrap();
    let keeper = clip_of(&map, art).expect("still clipped");
    assert!(map.node(keeper).is_some());
    history.undo(&mut map).unwrap();
    assert_eq!(clip_of(&map, art), Some(first));
}
