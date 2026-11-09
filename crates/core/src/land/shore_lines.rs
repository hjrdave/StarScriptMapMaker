//! Rows of short horizontal lines beside a coast (ADR 0016): the water lines of a Shore style.

use crate::geometry::MultiPolygon;
use crate::shore::MIN_LINE_SPACING;

/// At most this many rows, however tall the outline, to bound the geometry.
pub const MAX_LINE_ROWS: usize = 4000;

/// One water line: where it starts at the coast and where it ends out on the water.
pub type ShoreLine = [[f64; 2]; 2];

/// The water lines beside `outline`: in rows `spacing` apart, one line out from each side of
/// every stretch of land that a row crosses, up to `length` long. A line stops short of other
/// land, and where two coasts face each other across less than `2 * length` of water each
/// takes half, so the lines never double up. Holes are ignored, because a Lake inside land
/// has its own coast.
///
/// Rows sit half a gap in from the bottom of the outline, so lines move with the land. The
/// lines run from the coast outward, so a caller can fade them along their length.
pub fn shore_lines(outline: &MultiPolygon, spacing: f64, length: f64) -> Vec<ShoreLine> {
    let valid = spacing.is_finite() && length.is_finite() && length > 0.0;
    let (true, Some(bounds)) = (valid, outline.bounds()) else {
        return Vec::new();
    };
    let height = bounds.max[1] - bounds.min[1];
    // Far taller than it is worth drawing: spread the rows out instead.
    let spacing = spacing
        .max(MIN_LINE_SPACING)
        .max(height / MAX_LINE_ROWS as f64);
    let mut lines = Vec::new();
    let mut y = bounds.min[1] + spacing / 2.0;
    while y < bounds.max[1] {
        row(outline, y, length, &mut lines);
        y += spacing;
    }
    lines
}

/// The lines of one row, at height `y`.
fn row(outline: &MultiPolygon, y: f64, length: f64, lines: &mut Vec<ShoreLine>) {
    let land = land_at(outline, y);
    for (i, &(start, end)) in land.iter().enumerate() {
        // The gap to the next stretch of land on each side, or none past the last.
        let before = i.checked_sub(1).map(|j| start - land[j].1);
        let after = land.get(i + 1).map(|next| next.0 - end);
        let reach = |gap: Option<f64>| gap.map_or(length, |gap| length.min(gap / 2.0));
        let (left, right) = (reach(before), reach(after));
        if left > f64::EPSILON {
            lines.push([[start, y], [start - left, y]]);
        }
        if right > f64::EPSILON {
            lines.push([[end, y], [end + right, y]]);
        }
    }
}

/// The stretches of land at height `y`, as sorted `(from, to)` pairs of x, with overlapping
/// ones merged. Only exteriors count.
fn land_at(outline: &MultiPolygon, y: f64) -> Vec<(f64, f64)> {
    let mut stretches = Vec::new();
    for polygon in &outline.0 {
        let ring = &polygon.exterior;
        let mut crossings: Vec<f64> = ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .filter(|(a, b)| (a[1] <= y) != (b[1] <= y))
            .map(|(a, b)| a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]))
            .collect();
        crossings.sort_by(f64::total_cmp);
        stretches.extend(crossings.chunks_exact(2).map(|pair| (pair[0], pair[1])));
    }
    stretches.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::with_capacity(stretches.len());
    for (start, end) in stretches {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Polygon;

    fn square(min: [f64; 2], max: [f64; 2]) -> Polygon {
        Polygon {
            exterior: vec![min, [max[0], min[1]], max, [min[0], max[1]]],
            holes: Vec::new(),
        }
    }

    fn region(parts: Vec<Polygon>) -> MultiPolygon {
        MultiPolygon(parts)
    }

    #[test]
    fn a_square_gets_a_line_out_of_each_side_in_every_row() {
        let outline = region(vec![square([0.0, 0.0], [100.0, 100.0])]);
        let lines = shore_lines(&outline, 10.0, 20.0);
        // Rows at 5, 15 ... 95, two lines each.
        assert_eq!(lines.len(), 20);
        let ys: Vec<f64> = lines.iter().map(|l| l[0][1]).collect();
        assert!(ys.iter().all(|y| (y - 5.0) % 10.0 == 0.0));
        assert!(lines.contains(&[[0.0, 5.0], [-20.0, 5.0]]));
        assert!(lines.contains(&[[100.0, 95.0], [120.0, 95.0]]));
        // Every line is horizontal and `length` long.
        assert!(lines.iter().all(|l| l[0][1] == l[1][1]));
        assert!(
            lines
                .iter()
                .all(|l| ((l[1][0] - l[0][0]).abs() - 20.0).abs() < 1e-9)
        );
    }

    #[test]
    fn nothing_is_drawn_for_a_zero_length_or_an_empty_outline() {
        let outline = region(vec![square([0.0, 0.0], [100.0, 100.0])]);
        assert!(shore_lines(&outline, 10.0, 0.0).is_empty());
        assert!(shore_lines(&outline, 10.0, f64::NAN).is_empty());
        assert!(shore_lines(&MultiPolygon::default(), 10.0, 20.0).is_empty());
    }

    #[test]
    fn facing_coasts_share_the_water_between_them() {
        // 30 of water between the squares: with a length of 20, each side takes 15.
        let outline = region(vec![
            square([0.0, 0.0], [100.0, 100.0]),
            square([130.0, 0.0], [230.0, 100.0]),
        ]);
        let lines = shore_lines(&outline, 50.0, 20.0);
        assert!(lines.contains(&[[100.0, 25.0], [115.0, 25.0]]));
        assert!(lines.contains(&[[130.0, 25.0], [115.0, 25.0]]));
        // The outer sides keep the full length.
        assert!(lines.contains(&[[0.0, 25.0], [-20.0, 25.0]]));
        assert!(lines.contains(&[[230.0, 25.0], [250.0, 25.0]]));
        // And with plenty of water, nobody is cut short.
        let far = region(vec![
            square([0.0, 0.0], [100.0, 100.0]),
            square([300.0, 0.0], [400.0, 100.0]),
        ]);
        assert!(shore_lines(&far, 50.0, 20.0).contains(&[[100.0, 25.0], [120.0, 25.0]]));
    }

    #[test]
    fn a_hole_gets_no_lines() {
        let mut donut = square([0.0, 0.0], [100.0, 100.0]);
        donut
            .holes
            .push(vec![[30.0, 30.0], [30.0, 70.0], [70.0, 70.0], [70.0, 30.0]]);
        let lines = shore_lines(&region(vec![donut]), 10.0, 20.0);
        assert!(lines.iter().all(|l| l[0][0] <= 0.0 || l[0][0] >= 100.0));
    }

    #[test]
    fn overlapping_parts_count_as_one_stretch() {
        let outline = region(vec![
            square([0.0, 0.0], [60.0, 100.0]),
            square([40.0, 0.0], [100.0, 100.0]),
        ]);
        let lines = shore_lines(&outline, 50.0, 10.0);
        // One stretch from 0 to 100 per row, so two lines per row and none inside.
        assert_eq!(lines.len(), 4);
        assert!(lines.contains(&[[0.0, 25.0], [-10.0, 25.0]]));
        assert!(lines.contains(&[[100.0, 25.0], [110.0, 25.0]]));
    }

    #[test]
    fn a_slanted_coast_gives_lines_that_start_on_it() {
        // A triangle: the right side leans in towards the top.
        let outline = region(vec![Polygon {
            exterior: vec![[0.0, 0.0], [100.0, 0.0], [0.0, 100.0]],
            holes: Vec::new(),
        }]);
        let lines = shore_lines(&outline, 20.0, 15.0);
        // At y = 10 the right coast is at x = 90, at y = 50 it is at x = 50.
        assert!(
            lines
                .iter()
                .any(|l| l[0][1] == 10.0 && (l[0][0] - 90.0).abs() < 1e-9)
        );
        assert!(
            lines
                .iter()
                .any(|l| l[0][1] == 50.0 && (l[0][0] - 50.0).abs() < 1e-9)
        );
    }

    #[test]
    fn a_very_tall_outline_is_capped_in_rows() {
        let outline = region(vec![square([0.0, 0.0], [10.0, 1_000_000.0])]);
        let lines = shore_lines(&outline, 1.0, 5.0);
        assert!(lines.len() <= 2 * MAX_LINE_ROWS + 2, "{}", lines.len());
        assert!(!lines.is_empty());
    }

    #[test]
    fn rows_follow_the_land_when_it_moves() {
        let a = region(vec![square([0.0, 0.0], [100.0, 100.0])]);
        let b = region(vec![square([500.0, 37.0], [600.0, 137.0])]);
        let shift = |lines: Vec<ShoreLine>| -> Vec<ShoreLine> {
            lines
                .into_iter()
                .map(|[p, q]| [[p[0] - 500.0, p[1] - 37.0], [q[0] - 500.0, q[1] - 37.0]])
                .collect()
        };
        let (a, b) = (shore_lines(&a, 10.0, 20.0), shore_lines(&b, 10.0, 20.0));
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(shift(b)) {
            for k in 0..2 {
                assert!((x[k][0] - y[k][0]).abs() < 1e-9 && (x[k][1] - y[k][1]).abs() < 1e-9);
            }
        }
    }
}
