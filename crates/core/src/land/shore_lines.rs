//! Rows of short horizontal lines beside a coast (ADR 0016): the water lines of a Shore style.

use crate::geometry::MultiPolygon;
use crate::shore::MIN_LINE_SPACING;

/// At most this many rows, however tall the outline, to bound the geometry.
pub const MAX_LINE_ROWS: usize = 4000;

/// One water line: where it starts at the coast and where it ends out on the water.
pub type ShoreLine = [[f64; 2]; 2];

/// A water line after it has been made uneven: its two ends and how heavy it is.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DrawnLine {
    /// Where it starts at the coast.
    pub from: [f64; 2],
    /// Where it ends out on the water.
    pub to: [f64; 2],
    /// A factor on the line width, from 0 to 1.
    pub weight: f64,
}

/// Makes `lines` look hand drawn: with `variation` above 0, some are left out, and the rest
/// are shorter and lighter by different amounts. What happens to a line depends only on `seed`
/// and where the line is on the Canvas, so Canvas, export and tiled export agree (ADR 0007).
/// A variation of 0 returns every line whole at full weight.
pub fn hand_drawn(lines: &[ShoreLine], variation: f64, seed: u32) -> Vec<DrawnLine> {
    let variation = if variation.is_finite() {
        variation.clamp(0.0, 1.0)
    } else {
        0.0
    };
    lines
        .iter()
        .filter_map(|&[from, to]| {
            if variation == 0.0 {
                return Some(DrawnLine {
                    from,
                    to,
                    weight: 1.0,
                });
            }
            let mut unit = unit_stream(seed, from);
            // Gaps: up to 40% of the lines are missing at full variation.
            if unit() < variation * 0.4 {
                return None;
            }
            let length = 1.0 - variation * 0.7 * unit();
            let weight = 1.0 - variation * 0.6 * unit();
            Some(DrawnLine {
                from,
                to: [from[0] + (to[0] - from[0]) * length, to[1]],
                weight,
            })
        })
        .collect()
}

/// A stream of numbers in `[0, 1)` fixed by `seed` and a Canvas position. The position is
/// rounded to a tenth of a unit so the same line always lands on the same numbers.
pub(super) fn unit_stream(seed: u32, at: [f64; 2]) -> impl FnMut() -> f64 {
    let mut state = u64::from(seed)
        ^ ((at[0] * 10.0).round() as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ ((at[1] * 10.0).round() as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    move || {
        // splitmix64
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }
}

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

    fn some_lines() -> Vec<ShoreLine> {
        (0..200)
            .map(|i| [[0.0, f64::from(i) * 5.0], [-30.0, f64::from(i) * 5.0]])
            .collect()
    }

    #[test]
    fn no_variation_keeps_every_line_whole() {
        let lines = some_lines();
        let drawn = hand_drawn(&lines, 0.0, 7);
        assert_eq!(drawn.len(), lines.len());
        assert!(drawn.iter().zip(&lines).all(|(d, l)| {
            d.from == l[0] && d.to == l[1] && (d.weight - 1.0).abs() < f64::EPSILON
        }));
    }

    #[test]
    fn variation_leaves_gaps_and_shortens_and_lightens_lines() {
        let lines = some_lines();
        let drawn = hand_drawn(&lines, 0.8, 7);
        assert!(drawn.len() < lines.len() && drawn.len() > lines.len() / 2);
        assert!(
            drawn.iter().all(|d| {
                (d.to[0] - d.from[0]).abs() <= 30.0 && d.weight > 0.0 && d.weight <= 1.0
            })
        );
        assert!(drawn.iter().any(|d| d.weight < 0.9));
        assert!(drawn.iter().any(|d| (d.to[0] - d.from[0]).abs() < 25.0));
        assert!(drawn.windows(2).any(|w| w[0].weight != w[1].weight));
    }

    #[test]
    fn the_same_seed_gives_the_same_lines_and_another_seed_other_ones() {
        let lines = some_lines();
        assert_eq!(hand_drawn(&lines, 0.5, 3), hand_drawn(&lines, 0.5, 3));
        assert_ne!(hand_drawn(&lines, 0.5, 3), hand_drawn(&lines, 0.5, 4));
    }

    #[test]
    fn a_line_is_drawn_the_same_whatever_else_is_drawn_with_it() {
        let lines = some_lines();
        let whole = hand_drawn(&lines, 0.6, 9);
        let part = hand_drawn(&lines[50..100], 0.6, 9);
        assert!(part.iter().all(|d| whole.contains(d)));
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
