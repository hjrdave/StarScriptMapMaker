//! Landmasses and Lakes: draw order, picking, and transforms that undo exactly.

use ssmap_core::{
    Affine, Canvas, Color, Command, DistanceUnit, History, Id, Map, MultiPolygon, Node, PickIndex,
    Point, Polygon, Rect, Roughness, RoughnessCache, Scale,
};

fn square(min: [f64; 2], size: f64) -> Vec<Point> {
    let [x, y] = min;
    vec![[x, y], [x + size, y], [x + size, y + size], [x, y + size]]
}

fn region(exterior: Vec<Point>, holes: Vec<Vec<Point>>) -> MultiPolygon {
    MultiPolygon(vec![Polygon { exterior, holes }])
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

fn landmass(map: &mut Map, parent: Id, geometry: MultiPolygon) -> Id {
    let id = Id::new();
    run(
        map,
        Command::CreateLandmass {
            id,
            parent,
            index: None,
            geometry,
            fill: None,
        },
    );
    id
}

fn lake(map: &mut Map, parent: Id, index: Option<usize>, geometry: MultiPolygon) -> Id {
    let id = Id::new();
    run(
        map,
        Command::CreateLake {
            id,
            parent,
            index,
            geometry,
            fill: Some(Color::rgb(0, 0, 255)),
        },
    );
    id
}

fn empty_map() -> Map {
    Map::new(Canvas::new(
        1000,
        1000,
        Scale::new(1.0, DistanceUnit::Miles),
    ))
}

/// An island with a hole, a Lake listed beneath it in the tree, and a second Layer on top.
fn world() -> (Map, Id, Id, Id) {
    let mut map = empty_map();
    let land = layer(&mut map, "Land");
    let island = landmass(
        &mut map,
        land,
        region(square([0.0, 0.0], 100.0), vec![square([40.0, 40.0], 20.0)]),
    );
    let water = lake(
        &mut map,
        land,
        Some(0),
        region(square([10.0, 10.0], 20.0), vec![]),
    );
    let top = layer(&mut map, "Top");
    let cover = landmass(&mut map, top, region(square([80.0, 80.0], 40.0), vec![]));
    (map, island, water, cover)
}

#[test]
fn lakes_draw_above_the_land_in_their_layer() {
    let (mut map, island, water, cover) = world();
    assert_eq!(map.draw_order(), vec![island, water, cover]);
    let top = map.parent(cover).unwrap();
    run(
        &mut map,
        Command::SetVisible {
            id: top,
            visible: false,
        },
    );
    assert_eq!(map.draw_order(), vec![island, water]);
}

#[test]
fn picking_finds_the_topmost_fill_and_sees_through_holes() {
    let (map, island, water, cover) = world();
    let index = PickIndex::build(&map, &mut RoughnessCache::new());
    assert!(index.is_current(&map));
    assert_eq!(index.pick([20.0, 15.0]), Some(water));
    assert_eq!(index.pick([5.0, 50.0]), Some(island));
    assert_eq!(index.pick([50.0, 50.0]), None, "the hole shows ocean");
    assert_eq!(index.pick([90.0, 90.0]), Some(cover));
    assert_eq!(index.pick([200.0, 200.0]), None);
    let rect = Rect {
        min: [0.0, 0.0],
        max: [35.0, 35.0],
    };
    assert_eq!(index.intersecting(&rect), vec![island, water]);
}

#[test]
fn locked_objects_are_not_picked() {
    let (mut map, island, water, _) = world();
    run(
        &mut map,
        Command::SetLocked {
            id: water,
            locked: true,
        },
    );
    let index = PickIndex::build(&map, &mut RoughnessCache::new());
    assert_eq!(index.pick([20.0, 15.0]), Some(island));
    let land = map.parent(island).unwrap();
    run(
        &mut map,
        Command::SetLocked {
            id: land,
            locked: true,
        },
    );
    assert!(!index.is_current(&map));
    let index = PickIndex::build(&map, &mut RoughnessCache::new());
    assert_eq!(index.pick([20.0, 15.0]), None);
}

#[test]
fn transforms_undo_to_the_exact_original() {
    let (mut map, island, water, _) = world();
    let before = map.view();
    let mut history = History::default();
    history.begin("Resize", vec![island]).unwrap();
    // A drag re-issues its whole effect each frame from the start.
    for step in 1..=20 {
        history.rewind(&mut map).unwrap();
        let factor = 1.0 + f64::from(step) * 0.037;
        let command = Command::Transform {
            ids: vec![island, water],
            transform: Affine([factor, 0.0, 0.0, -factor, 0.1, 0.3]),
        };
        history.execute(&mut map, &command).unwrap();
    }
    history.commit(vec![island]).unwrap();
    assert_ne!(map.view(), before);
    assert_eq!(history.undo(&mut map).unwrap(), Some(vec![island]));
    assert_eq!(map.view(), before, "bit-for-bit, not approximately");
    history.redo(&mut map).unwrap();
    let Some(Node::Object(object)) = map.node(island) else {
        panic!("a Landmass")
    };
    let first = object.kind.geometry().unwrap().0[0].exterior[1];
    let factor = 1.0 + 20.0 * 0.037;
    assert_eq!(first, [100.0 * factor + 0.1, 0.3]);
}

#[test]
fn transform_refusals_change_nothing() {
    let (mut map, island, water, _) = world();
    let before = map.view();
    let huge = Command::Transform {
        ids: vec![water, island],
        transform: Affine([1e308, 0.0, 0.0, 1e308, 0.0, 0.0]),
    };
    assert!(map.execute(&huge).is_err());
    assert_eq!(map.view(), before);
}

#[test]
fn picking_uses_the_roughened_coastline() {
    let (mut map, island, _, _) = world();
    let mut roughness = RoughnessCache::new();
    let index = PickIndex::build(&map, &mut roughness);
    run(
        &mut map,
        Command::SetRoughness {
            ids: vec![island],
            roughness: Some(Roughness::new(8.0, 25.0, 7)),
        },
    );
    let rough = PickIndex::build(&map, &mut roughness);
    let outline = roughness.outline(&map, island).unwrap();
    // A point beside the smooth left coast (x = 0) that the rough coast puts on the other side.
    let point = (1..100)
        .flat_map(|y| [-3.0, -1.0, 1.0, 3.0].map(|x| [x, f64::from(y)]))
        .find(|&p| outline.contains(p) != (p[0] > 0.0))
        .expect("Roughness moves the coast");
    let expected = outline.contains(point).then_some(island);
    assert_eq!(rough.pick(point), expected, "at {point:?}");
    assert_ne!(index.pick(point), expected, "the smooth index disagrees");
}

#[test]
fn changing_roughness_is_one_undo_step() {
    let (mut map, island, water, _) = world();
    let before = map.view();
    let mut history = History::default();
    history.begin("Roughness", vec![island, water]).unwrap();
    history
        .execute(
            &mut map,
            &Command::SetRoughness {
                ids: vec![island, water],
                roughness: Some(Roughness::new(8.0, 25.0, 7)),
            },
        )
        .unwrap();
    history.commit(vec![island, water]).unwrap();
    assert_ne!(map.view(), before);
    history.undo(&mut map).unwrap();
    assert_eq!(map.view(), before);
    assert_eq!(
        history.undo(&mut map).unwrap(),
        None,
        "nothing else to undo"
    );
}

fn water_stroke(region: MultiPolygon, start: Point, erase: bool) -> ssmap_core::land::BrushStroke {
    ssmap_core::land::BrushStroke {
        region,
        start,
        erase,
    }
}

#[test]
fn the_water_brush_creates_a_lake_then_merges_and_erases() {
    use ssmap_core::land::{BrushError, water_command};

    let mut map = empty_map();
    let water = layer(&mut map, "Water");
    let mut cache = RoughnessCache::default();
    let new_id = Id::new();

    let river = region(square([0.0, 0.0], 20.0), vec![]);
    let command = water_command(
        &map,
        &mut cache,
        &[],
        Some(water),
        water_stroke(river, [10.0, 10.0], false),
        new_id,
    )
    .unwrap()
    .expect("a stroke on bare ground makes a Lake");
    assert!(
        matches!(&command, Command::CreateLake { id, parent, .. } if *id == new_id && *parent == water)
    );
    run(&mut map, command);

    // A stroke starting in the Lake extends it rather than starting another.
    let more = region(square([15.0, 0.0], 20.0), vec![]);
    let command = water_command(
        &map,
        &mut cache,
        &[],
        Some(water),
        water_stroke(more, [18.0, 10.0], false),
        Id::new(),
    )
    .unwrap()
    .unwrap();
    assert!(matches!(&command, Command::AddRegion { id, .. } if *id == new_id));
    run(&mut map, command);

    // Erasing cuts the Lake without any selection.
    let cut = region(square([-5.0, -5.0], 60.0), vec![]);
    let command = water_command(
        &map,
        &mut cache,
        &[],
        None,
        water_stroke(cut, [0.0, 0.0], true),
        Id::new(),
    )
    .unwrap()
    .unwrap();
    assert!(matches!(&command, Command::SubtractRegion { ids, .. } if ids == &vec![new_id]));

    // Erasing away from every Lake does nothing, and painting needs a Layer.
    let far = region(square([500.0, 500.0], 10.0), vec![]);
    let erased = water_command(
        &map,
        &mut cache,
        &[],
        None,
        water_stroke(far.clone(), [505.0, 505.0], true),
        Id::new(),
    );
    assert_eq!(erased, Ok(None));
    let painted = water_command(
        &map,
        &mut cache,
        &[],
        None,
        water_stroke(far, [505.0, 505.0], false),
        Id::new(),
    );
    assert_eq!(painted, Err(BrushError::NoLayer));
}
