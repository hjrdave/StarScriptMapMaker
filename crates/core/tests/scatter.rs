//! The scatter brush, Stamp eraser and Shuffle variant (spec §4, ADR 0003, ADR 0004).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::scatter::{
    Placement, ScatterBrush, ScatterStroke, Terrain, erase_command, scatter_command,
    shuffle_commands,
};
use ssmap_core::{
    AssetRef, Canvas, Command, DistanceUnit, Id, Map, MultiPolygon, Node, ObjectKind, Point,
    Polygon, RoughnessCache, Scale, Stamp,
};

fn square(min: f64, max: f64) -> Vec<Point> {
    vec![[min, min], [max, min], [max, max], [min, max]]
}

fn region(exterior: Vec<Point>) -> MultiPolygon {
    MultiPolygon(vec![Polygon {
        exterior,
        holes: Vec::new(),
    }])
}

fn asset(name: &str) -> AssetRef {
    format!("pack/{name}").parse().unwrap()
}

fn layer(map: &mut Map, name: &str) -> Id {
    let id = Id::new();
    map.execute(&Command::CreateLayer {
        id,
        parent: None,
        index: None,
        name: name.into(),
    })
    .unwrap();
    id
}

/// Land over (0,0)-(400,400) with a Lake over (100,100)-(200,200); the rest is ocean.
fn island() -> (Map, Id, Id, Id) {
    let mut map = Map::new(Canvas::new(
        1000,
        1000,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    let terrain = layer(&mut map, "Terrain");
    let land = Id::new();
    map.execute(&Command::CreateLandmass {
        id: land,
        parent: terrain,
        index: None,
        geometry: region(square(0.0, 400.0)),
        fill: None,
    })
    .unwrap();
    map.execute(&Command::CreateLake {
        id: Id::new(),
        parent: terrain,
        index: None,
        geometry: region(square(100.0, 200.0)),
        fill: None,
    })
    .unwrap();
    let mountains = layer(&mut map, "Mountains");
    (map, terrain, land, mountains)
}

fn brush(placement: Placement) -> ScatterBrush {
    ScatterBrush {
        density: 40.0,
        spacing: 8.0,
        rotation_jitter: 15.0,
        placement,
        ..ScatterBrush::default()
    }
}

fn sequential_ids() -> impl FnMut() -> Id {
    let mut n = 0u128;
    move || {
        n += 1;
        Id::from_uuid(uuid_from(n))
    }
}

fn uuid_from(n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(0x0000_0000_0000_7000_8000_0000_0000_0000 + n)
}

/// Scatters over a wide stroke across land, Lake and ocean.
fn scatter(map: &Map, layer: Id, placement: Placement, seed: u64) -> Option<Command> {
    let terrain = Terrain::of(map, &mut RoughnessCache::new());
    let base = Stamp::new(asset("peak-1"), [0.0, 0.0]);
    let variants = [asset("peak-1"), asset("peak-2"), asset("peak-3")];
    let points = [[50.0, 150.0], [250.0, 150.0], [450.0, 150.0]];
    scatter_command(
        &brush(placement),
        ScatterStroke {
            points: &points,
            radius: 80.0,
            parent: layer,
            base: &base,
            variants: &variants,
        },
        &terrain,
        seed,
        sequential_ids(),
    )
}

fn placed(command: &Command) -> Vec<Stamp> {
    let Command::PlaceStamps { stamps, .. } = command else {
        panic!("not PlaceStamps");
    };
    stamps.iter().map(|p| p.stamp.clone()).collect()
}

#[test]
fn a_replayed_command_script_places_identical_stamps() {
    let (map, _, _, mountains) = island();
    let command = scatter(&map, mountains, Placement::Land, 7).unwrap();
    assert_eq!(
        command,
        scatter(&map, mountains, Placement::Land, 7).unwrap()
    );
    assert_ne!(
        command,
        scatter(&map, mountains, Placement::Land, 8).unwrap()
    );

    // Through JSON, as a recorded script, onto two copies of the same Map.
    let script = serde_json::to_string(&command).unwrap();
    let replayed: Command = serde_json::from_str(&script).unwrap();
    let (mut a, mut b) = (map.clone(), map);
    a.execute(&command).unwrap();
    b.execute(&replayed).unwrap();
    assert_eq!(a.view(), b.view());
    assert_eq!(a.children(Some(mountains)).len(), placed(&command).len());
}

#[test]
fn a_stroke_is_one_command_with_varied_stamps() {
    let (map, _, _, mountains) = island();
    let stamps = placed(&scatter(&map, mountains, Placement::Land, 1).unwrap());
    assert!(stamps.len() > 10);
    assert!(stamps.iter().any(|s| s.flip) && stamps.iter().any(|s| !s.flip));
    assert!(stamps.iter().any(|s| s.asset == asset("peak-2")));
    assert!(stamps.iter().any(|s| s.asset == asset("peak-3")));
    assert!(stamps.iter().all(|s| (0.8..=1.2).contains(&s.scale)));
    assert!(stamps.iter().any(|s| s.rotation != 0.0));
    assert!(stamps.iter().all(|s| s.rotation.abs() <= 15.0));
    for (i, a) in stamps.iter().enumerate() {
        for b in &stamps[i + 1..] {
            let d = ((a.position[0] - b.position[0]).powi(2)
                + (a.position[1] - b.position[1]).powi(2))
            .sqrt();
            assert!(d >= 8.0, "spacing");
        }
    }
}

#[test]
fn land_only_never_places_on_water_or_in_a_lake() {
    let (map, _, _, mountains) = island();
    for seed in 0..20 {
        for stamp in placed(&scatter(&map, mountains, Placement::Land, seed).unwrap()) {
            let [x, y] = stamp.position;
            assert!(
                (0.0..=400.0).contains(&x) && (0.0..=400.0).contains(&y),
                "ocean"
            );
            assert!(
                !((100.0..=200.0).contains(&x) && (100.0..=200.0).contains(&y)),
                "lake"
            );
        }
    }
}

#[test]
fn water_only_and_anywhere() {
    let (map, _, _, mountains) = island();
    let water = placed(&scatter(&map, mountains, Placement::Water, 3).unwrap());
    assert!(water.iter().all(|s| {
        let [x, y] = s.position;
        !((0.0..=400.0).contains(&x) && (0.0..=400.0).contains(&y))
            || ((100.0..=200.0).contains(&x) && (100.0..=200.0).contains(&y))
    }));
    let anywhere = placed(&scatter(&map, mountains, Placement::Anywhere, 3).unwrap());
    assert!(anywhere.len() > water.len());
}

#[test]
fn nothing_is_placed_when_the_constraint_allows_nowhere() {
    let (map, _, _, mountains) = island();
    let terrain = Terrain::of(&map, &mut RoughnessCache::new());
    let base = Stamp::new(asset("peak-1"), [0.0, 0.0]);
    let points = [[800.0, 800.0]];
    let command = scatter_command(
        &brush(Placement::Land),
        ScatterStroke {
            points: &points,
            radius: 50.0,
            parent: mountains,
            base: &base,
            variants: &[],
        },
        &terrain,
        1,
        Id::new,
    );
    assert_eq!(command, None);
}

#[test]
fn editing_a_coastline_afterwards_moves_no_stamps() {
    let (mut map, _, land, mountains) = island();
    map.execute(&scatter(&map, mountains, Placement::Land, 5).unwrap())
        .unwrap();
    let before = positions(&map, mountains);
    map.execute(&Command::SetMoveLocked {
        id: land,
        locked: false,
    })
    .unwrap();
    map.execute(&Command::SubtractRegion {
        ids: vec![land],
        region: region(square(0.0, 300.0)),
    })
    .unwrap();
    map.execute(&Command::Translate {
        ids: vec![land],
        offset: [50.0, 50.0],
    })
    .unwrap();
    assert_eq!(positions(&map, mountains), before);
}

fn positions(map: &Map, layer: Id) -> Vec<Point> {
    map.children(Some(layer))
        .iter()
        .filter_map(|&id| match map.node(id) {
            Some(Node::Object(o)) => match &o.kind {
                ObjectKind::Stamp(s) => Some(s.position),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[test]
fn the_eraser_removes_stamps_in_the_active_layer_only() {
    let (mut map, _, _, mountains) = island();
    let other = layer(&mut map, "Trees");
    for parent in [mountains, other] {
        for x in [10.0, 50.0, 300.0] {
            map.execute(&Command::CreateStamp {
                id: Id::new(),
                parent,
                index: None,
                stamp: Box::new(Stamp::new(asset("peak-1"), [x, 20.0])),
            })
            .unwrap();
        }
    }
    let command = erase_command(&map, mountains, &[[0.0, 20.0], [60.0, 20.0]], 15.0).unwrap();
    map.execute(&command).unwrap();
    let left: Vec<f64> = positions(&map, mountains).iter().map(|p| p[0]).collect();
    assert_eq!(left, vec![300.0]);
    assert_eq!(positions(&map, other).len(), 3);
    assert_eq!(
        erase_command(&map, mountains, &[[600.0, 600.0]], 15.0),
        None
    );
}

#[test]
fn shuffle_variant_rerolls_within_the_group() {
    let (mut map, _, _, mountains) = island();
    let group = vec![asset("peak-1"), asset("peak-2"), asset("peak-3")];
    let mut ids = Vec::new();
    for x in 0..30 {
        let id = Id::new();
        map.execute(&Command::CreateStamp {
            id,
            parent: mountains,
            index: None,
            stamp: Box::new(Stamp::new(asset("peak-1"), [f64::from(x), 0.0])),
        })
        .unwrap();
        ids.push(id);
    }
    // A lone Stamp of an Asset with no group is left alone.
    let lone = Id::new();
    map.execute(&Command::CreateStamp {
        id: lone,
        parent: mountains,
        index: None,
        stamp: Box::new(Stamp::new(asset("tower"), [0.0, 0.0])),
    })
    .unwrap();
    ids.push(lone);

    let group_of = |a: &AssetRef| {
        if group.contains(a) {
            group.clone()
        } else {
            Vec::new()
        }
    };
    let commands = shuffle_commands(&map, &ids, group_of, 9);
    assert_eq!(commands, shuffle_commands(&map, &ids, group_of, 9));
    assert!(!commands.is_empty());
    for command in &commands {
        map.execute(command).unwrap();
    }
    let assets: Vec<AssetRef> = ids
        .iter()
        .filter_map(|&id| match map.node(id) {
            Some(Node::Object(o)) => match &o.kind {
                ObjectKind::Stamp(s) => Some(s.asset.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert!(
        assets
            .iter()
            .all(|a| *a == asset("tower") || group.contains(a))
    );
    assert!(assets.contains(&asset("peak-2")) && assets.contains(&asset("peak-3")));
    assert_eq!(assets.last(), Some(&asset("tower")));
}
