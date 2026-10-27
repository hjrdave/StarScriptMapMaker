//! The live preview of a land brush stroke: the rough shore shown while painting is the one the
//! stroke commits.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::land::{BrushStroke, brush_command, difference, preview_area, signed_area};
use ssmap_core::{
    Canvas, Command, DistanceUnit, Id, Map, MultiPolygon, Polygon, Roughness, RoughnessCache, Scale,
};

fn square(min: [f64; 2], size: f64) -> MultiPolygon {
    let [x, y] = min;
    MultiPolygon(vec![Polygon {
        exterior: vec![[x, y], [x + size, y], [x + size, y + size], [x, y + size]],
        holes: Vec::new(),
    }])
}

fn area(region: &MultiPolygon) -> f64 {
    region
        .0
        .iter()
        .flat_map(Polygon::rings)
        .map(signed_area)
        .sum()
}

/// A rough island with a fixed seed, alone in its Layer.
fn rough_island() -> (Map, Id, Id) {
    let mut map = Map::new(Canvas::new(
        1000,
        1000,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    let (land, island) = (Id::new(), Id::new());
    for command in [
        Command::CreateLayer {
            id: land,
            parent: None,
            index: None,
            name: "Land".into(),
        },
        Command::CreateLandmass {
            id: island,
            parent: land,
            index: None,
            geometry: square([100.0, 100.0], 200.0),
            fill: None,
        },
        Command::SetRoughness {
            ids: vec![island],
            roughness: Some(Roughness::new(8.0, 25.0, 7)),
        },
    ] {
        map.execute(&command).unwrap();
    }
    (map, land, island)
}

/// A stroke across the island's right-hand coast.
fn stroke(erase: bool) -> BrushStroke {
    BrushStroke {
        region: square([260.0, 150.0], 80.0),
        start: [270.0, 160.0],
        erase,
    }
}

#[test]
fn the_live_preview_is_the_committed_shore() {
    for erase in [false, true] {
        let (mut map, land, island) = rough_island();
        let mut cache = RoughnessCache::new();
        let before = cache.outline(&map, island).unwrap();
        let command = brush_command(
            &map,
            &mut cache,
            &[island],
            Some(land),
            stroke(erase),
            Id::new(),
        )
        .unwrap()
        .unwrap();
        let preview = preview_area(&map, &mut cache, &command).expect("the stroke changes land");
        map.execute(&command).unwrap();
        let after = cache.outline(&map, island).unwrap();
        // What the committed stroke added to the rough shore, or cut from it.
        let changed = if erase {
            difference(&before, &after)
        } else {
            difference(&after, &before)
        };
        assert!(area(&changed) > 0.0, "erase = {erase}");
        assert!(
            (area(&preview) - area(&changed)).abs() < 1e-6 * area(&before),
            "erase = {erase}: preview {} vs committed {}",
            area(&preview),
            area(&changed),
        );
        // And it sits where the committed shore is: nothing of it lies outside the change.
        assert!(area(&difference(&preview, &changed)) < 1e-6 * area(&before));
    }
}
