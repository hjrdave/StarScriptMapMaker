use geo::Validation;

use super::*;
use crate::geometry::NodeRef;

fn square(min: f64, max: f64) -> MultiPolygon {
    MultiPolygon(vec![Polygon {
        exterior: vec![[min, min], [max, min], [max, max], [min, max]],
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

fn assert_oriented(region: &MultiPolygon) {
    for polygon in &region.0 {
        assert!(
            signed_area(&polygon.exterior) > 0.0,
            "exterior is counter-clockwise"
        );
        for hole in &polygon.holes {
            assert!(signed_area(hole) < 0.0, "hole is clockwise");
        }
    }
}

#[test]
fn a_click_is_a_disc_and_a_drag_a_capsule() {
    let disc = stroke_region(&[[50.0, 50.0]], 10.0);
    assert_eq!(disc.0.len(), 1);
    let expected = std::f64::consts::PI * 100.0;
    assert!((area(&disc) - expected).abs() < expected * 0.02);

    let capsule = stroke_region(&[[0.0, 0.0], [100.0, 0.0]], 10.0);
    let expected = 100.0 * 20.0 + std::f64::consts::PI * 100.0;
    assert!((area(&capsule) - expected).abs() < expected * 0.02);
    assert!(capsule.contains([-9.0, 0.0]) && capsule.contains([109.0, 0.0]));
    assert!(!capsule.contains([50.0, 11.0]));
}

#[test]
fn a_committed_stroke_is_lighter_than_its_preview() {
    let points: Vec<Point> = (0..200)
        .map(|i| {
            let t = f64::from(i) / 10.0;
            [t * 20.0, t.sin() * 40.0]
        })
        .collect();
    let preview = stroke_region(&points, 15.0);
    let committed = brush_region(&points, 15.0, simplify_tolerance(15.0, 1.0));
    let count = |r: &MultiPolygon| r.0.iter().map(|p| p.exterior.len()).sum::<usize>();
    assert!(count(&committed) < count(&preview));
    assert!(committed.problem().is_none());
    assert_oriented(&committed);
    assert!((area(&committed) - area(&preview)).abs() < area(&preview) * 0.05);
}

#[test]
fn union_and_difference_keep_outlines_valid() {
    let joined = union(&square(0.0, 10.0), &square(5.0, 15.0));
    assert_eq!(joined.0.len(), 1);
    assert!((area(&joined) - 175.0).abs() < 1e-9);
    assert_oriented(&joined);

    let holed = difference(&square(0.0, 10.0), &square(4.0, 6.0));
    assert_eq!(holed.0[0].holes.len(), 1);
    assert_oriented(&holed);

    // A cut across the middle leaves two parts of one outline.
    let band = MultiPolygon(vec![Polygon {
        exterior: vec![[4.0, -1.0], [6.0, -1.0], [6.0, 11.0], [4.0, 11.0]],
        holes: Vec::new(),
    }]);
    let split = difference(&square(0.0, 10.0), &band);
    assert_eq!(split.0.len(), 2);
    assert_oriented(&split);

    assert!(difference(&square(0.0, 10.0), &square(-1.0, 11.0)).is_empty());
}

#[test]
fn clockwise_input_comes_out_counter_clockwise() {
    let mut clockwise = square(0.0, 10.0);
    clockwise.0[0].exterior.reverse();
    assert!(signed_area(&clockwise.0[0].exterior) < 0.0);
    assert_oriented(&union(&clockwise, &square(20.0, 30.0)));
}

#[test]
fn intersects_needs_shared_area_not_just_shared_bounds() {
    let l_shape = MultiPolygon(vec![Polygon {
        exterior: vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 2.0],
            [2.0, 2.0],
            [2.0, 10.0],
            [0.0, 10.0],
        ],
        holes: Vec::new(),
    }]);
    assert!(intersects(&l_shape, &square(1.0, 3.0)));
    assert!(!intersects(&l_shape, &square(5.0, 9.0)));
    assert!(!intersects(&l_shape, &square(20.0, 30.0)));
}

fn node(part: usize, ring: usize, index: usize) -> NodeRef {
    NodeRef { part, ring, index }
}

#[test]
fn moving_nodes_keeps_indices_when_the_outline_stays_valid() {
    let moved = move_nodes(&square(0.0, 10.0), &[node(0, 0, 2)], [5.0, 5.0]).unwrap();
    assert_eq!(moved.0[0].exterior[2], [15.0, 15.0]);
    assert_eq!(moved.0[0].exterior.len(), 4);
    // Naming a node twice moves it once.
    let moved = move_nodes(&square(0.0, 10.0), &[node(0, 0, 2); 2], [5.0, 0.0]).unwrap();
    assert_eq!(moved.0[0].exterior[2], [15.0, 10.0]);
    assert_eq!(
        move_nodes(&square(0.0, 10.0), &[node(0, 0, 9)], [1.0, 1.0]),
        Err(NodeEditError::NoSuchNode(node(0, 0, 9)))
    );
}

#[test]
fn dragging_a_node_across_the_outline_is_repaired() {
    // Dragging a corner past the opposite one ties the square into a bow-tie.
    let bowtie = move_nodes(&square(0.0, 10.0), &[node(0, 0, 2)], [-20.0, 0.0]).unwrap();
    assert!(to_geo(&bowtie).is_valid());
    assert_oriented(&bowtie);
}

#[test]
fn insert_adds_a_node_after_the_edge_start_and_the_last_edge_wraps() {
    let inserted = insert_node(&square(0.0, 10.0), node(0, 0, 0), [5.0, -3.0]).unwrap();
    assert_eq!(inserted.0[0].exterior[1], [5.0, -3.0]);
    assert_eq!(inserted.0[0].exterior.len(), 5);
    let closing = insert_node(&square(0.0, 10.0), node(0, 0, 3), [-2.0, 5.0]).unwrap();
    assert_eq!(closing.0[0].exterior[4], [-2.0, 5.0]);
    assert_oriented(&closing);
}

#[test]
fn delete_refuses_to_leave_a_ring_with_fewer_than_three_points() {
    let pentagon = MultiPolygon(vec![Polygon {
        exterior: vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [12.0, 6.0],
            [5.0, 10.0],
            [-2.0, 6.0],
        ],
        holes: Vec::new(),
    }]);
    let fewer = delete_nodes(&pentagon, &[node(0, 0, 1), node(0, 0, 3), node(0, 0, 3)]).unwrap();
    assert_eq!(fewer.0[0].exterior.len(), 3);
    assert_eq!(
        delete_nodes(&pentagon, &[node(0, 0, 0), node(0, 0, 1), node(0, 0, 2)]),
        Err(NodeEditError::TooFewPoints)
    );
    assert_eq!(
        delete_nodes(&square(0.0, 10.0), &[node(0, 0, 0), node(0, 0, 2)]),
        Err(NodeEditError::TooFewPoints)
    );
}

#[test]
fn a_big_coastline_stays_quick_to_edit() {
    let n = 50_000;
    let ring: Vec<Point> = (0..n)
        .map(|i| {
            let t = f64::from(i) / f64::from(n) * std::f64::consts::TAU;
            let r = 10_000.0 + 400.0 * (t * 37.0).sin();
            [r * t.cos(), r * t.sin()]
        })
        .collect();
    let coast = MultiPolygon(vec![Polygon {
        exterior: ring,
        holes: Vec::new(),
    }]);
    let started = std::time::Instant::now();
    let edited = move_nodes(&coast, &[node(0, 0, 25_000)], [30.0, 0.0]).unwrap();
    assert_eq!(edited.0[0].exterior.len(), 50_000);
    let elapsed = started.elapsed();
    // Generous for debug builds; a drag runs this every frame.
    assert!(elapsed.as_millis() < 1500, "took {elapsed:?}");
}

#[test]
fn offset_grows_and_shrinks_outlines() {
    let grown = offset(&square(0.0, 10.0), 2.0);
    assert!(grown.contains([-1.0, 5.0]) && !grown.contains([-3.0, 5.0]));
    let shrunk = offset(&square(0.0, 10.0), -2.0);
    assert!(shrunk.contains([5.0, 5.0]) && !shrunk.contains([1.0, 5.0]));
    assert!(offset(&square(0.0, 10.0), -6.0).is_empty());
    // A hole grows when the land around it shrinks.
    let ring = difference(&square(0.0, 30.0), &square(10.0, 20.0));
    let inset = offset(&ring, -2.0);
    assert!(!inset.contains([8.5, 15.0]) && inset.contains([7.0, 15.0]));
}

#[test]
fn a_lasso_loop_becomes_a_valid_region_even_when_it_crosses_itself() {
    let square = lasso_region(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], 0.0);
    assert_eq!(square.0.len(), 1);
    assert_oriented(&square);
    assert!((area(&square) - 100.0).abs() < 1e-9);
    // A bow-tie is two triangles.
    let bowtie = lasso_region(&[[0.0, 0.0], [10.0, 10.0], [10.0, 0.0], [0.0, 10.0]], 0.0);
    assert_eq!(bowtie.0.len(), 2);
    assert_oriented(&bowtie);
    assert!(lasso_region(&[[0.0, 0.0], [5.0, 5.0]], 0.0).is_empty());
    assert!(lasso_region(&[[0.0, 0.0], [5.0, 5.0], [10.0, 10.0]], 0.0).is_empty());
}

/// How far a region's outline strays from a circle about `centre`: the spread between its
/// nearest and farthest points, as a share of the farthest.
fn out_of_round(region: &MultiPolygon, centre: Point) -> f64 {
    let distances: Vec<f64> = region
        .0
        .iter()
        .flat_map(|polygon| polygon.exterior.iter())
        .map(|p| (p[0] - centre[0]).hypot(p[1] - centre[1]))
        .collect();
    let far = distances.iter().copied().fold(0.0, f64::max);
    let near = distances.iter().copied().fold(f64::INFINITY, f64::min);
    (far - near) / far
}

fn stored_points(region: &MultiPolygon) -> usize {
    region
        .0
        .iter()
        .flat_map(Polygon::rings)
        .map(<[Point]>::len)
        .sum()
}

#[test]
fn irregularity_zero_is_the_round_stroke() {
    let points = [[0.0, 0.0], [80.0, 30.0], [160.0, 0.0]];
    let tolerance = simplify_tolerance(30.0, 1.0);
    let round = brush_region(&points, 30.0, tolerance);
    for irregularity in [0.0, -0.2, f64::NAN] {
        assert_eq!(
            irregular_region(&points, 30.0, tolerance, irregularity, 7),
            round
        );
    }
}

#[test]
fn an_irregular_click_is_a_lumpy_blob() {
    let tolerance = simplify_tolerance(60.0, 1.0);
    // A dab: a click that moved a few units, as a lone point paints nothing.
    let dab = [[98.0, 100.0], [102.0, 100.0]];
    let round = irregular_region(&dab, 60.0, tolerance, 0.0, 3);
    let blob = irregular_region(&dab, 60.0, tolerance, 0.2, 3);
    assert!(!round.is_empty() && !blob.is_empty());
    assert!(out_of_round(&round, [100.0, 100.0]) < 0.05);
    assert!(out_of_round(&blob, [100.0, 100.0]) > 0.2);
    assert!(blob.problem().is_none());
    assert_oriented(&blob);
}

#[test]
fn the_seed_picks_the_shape() {
    let points = [[0.0, 0.0], [120.0, 40.0]];
    let tolerance = simplify_tolerance(40.0, 1.0);
    let shape = |seed| irregular_region(&points, 40.0, tolerance, 0.25, seed);
    assert_eq!(shape(11), shape(11));
    assert_ne!(shape(11), shape(12));
}

#[test]
fn an_irregular_stroke_stays_light_and_valid() {
    let points: Vec<Point> = (0..200)
        .map(|i| {
            let t = f64::from(i) / 10.0;
            [t * 20.0, t.sin() * 40.0]
        })
        .collect();
    let tolerance = simplify_tolerance(15.0, 1.0);
    let round = brush_region(&points, 15.0, tolerance);
    for seed in 0..8 {
        let region = irregular_region(&points, 15.0, tolerance, MAX_IRREGULARITY, seed);
        assert!(region.problem().is_none());
        assert_oriented(&region);
        assert!(stored_points(&region) < stored_points(&round) * 4);
        // Pushed out and in about evenly, it covers about as much as the round stroke.
        assert!((area(&region) - area(&round)).abs() < area(&round) * 0.25);
    }
}

#[test]
fn an_irregular_erase_cuts_an_uneven_bite() {
    let points = [[50.0, 120.0], [150.0, 120.0]];
    let tolerance = simplify_tolerance(30.0, 1.0);
    let round = difference(&square(0.0, 200.0), &brush_region(&points, 30.0, tolerance));
    let ragged = difference(
        &square(0.0, 200.0),
        &irregular_region(&points, 30.0, tolerance, 0.3, 5),
    );
    assert!(ragged.problem().is_none());
    assert_oriented(&ragged);
    assert_eq!(ragged.0.len(), 1);
    assert!(stored_points(&ragged) > stored_points(&round));
}
