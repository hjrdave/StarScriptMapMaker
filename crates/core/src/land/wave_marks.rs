//! Small wavy marks scattered over the open water beyond a coast (ADR 0016): the wave marks of
//! a Shore style.

use super::shore_lines::unit_stream;
use crate::geometry::MultiPolygon;

/// At most this many marks, however large the outline, to bound the geometry.
pub const MAX_WAVE_MARKS: usize = 4000;

/// At most this many grid cells are tried; a larger outline gets a coarser grid.
const MAX_CELLS: f64 = 40_000.0;

/// How far past the nearest a mark can sit, as a multiple of its size.
const BAND: f64 = 6.0;

/// Points along one mark.
const MARK_POINTS: usize = 9;

/// One wave mark: an open line of points, drawn from the first to the last.
pub type WaveMark = Vec<[f64; 2]>;

/// The wave marks beside `outline`: about `density` for every 10,000 square units of water,
/// each `size` long, in a band of open water that starts `clearance` from the coast (so past
/// the water lines) and reaches `6 * size` further out. A mark never touches land.
///
/// Marks sit one to a cell of a grid fixed to the Canvas, jittered by `seed`, so what is drawn
/// at a place depends only on the seed and that place, not on the viewport or tile (ADR 0007).
/// Holes are land here, because a mark in a Lake would sit on another coast.
pub fn wave_marks(
    outline: &MultiPolygon,
    density: f64,
    size: f64,
    seed: u32,
    clearance: f64,
) -> Vec<WaveMark> {
    let valid = [density, size, clearance].iter().all(|v| v.is_finite())
        && density > 0.0
        && size > 0.0
        && clearance >= 0.0;
    let (true, Some(bounds)) = (valid, outline.bounds()) else {
        return Vec::new();
    };
    let near = clearance + size * 0.6;
    let far = near + size * BAND;
    let (min, max) = (
        [bounds.min[0] - far, bounds.min[1] - far],
        [bounds.max[0] + far, bounds.max[1] + far],
    );
    let area = (max[0] - min[0]) * (max[1] - min[1]);
    let zones: Vec<Zone> = outline
        .0
        .iter()
        .map(|p| Zone::of(&p.exterior, far))
        .collect();
    // The water in reach: about a band along every coast, and no more than the padded bounds.
    let reach: f64 = zones
        .iter()
        .map(|z| (z.perimeter * (far - near)).min(z.area()))
        .sum();
    // Coarsen the grid so the count stays under the cap without dropping marks by position.
    let cell = (10_000.0 / density)
        .sqrt()
        .max((area / MAX_CELLS).sqrt())
        .max((reach / (MAX_WAVE_MARKS as f64 * 0.5)).sqrt());
    let columns = (min[0] / cell).floor() as i64..=(max[0] / cell).ceil() as i64;
    let rows = (min[1] / cell).floor() as i64..=(max[1] / cell).ceil() as i64;
    let mut marks = Vec::new();
    for j in rows {
        for i in columns.clone() {
            let mut unit = unit_stream(seed, [i as f64, j as f64]);
            let centre = [(i as f64 + unit()) * cell, (j as f64 + unit()) * cell];
            let (length, flip, phase) = (size * (0.7 + 0.3 * unit()), unit() < 0.5, unit());
            if !zones.iter().any(|z| z.contains(centre))
                || !in_band(outline, &zones, centre, near, far)
            {
                continue;
            }
            marks.push(mark(centre, length, flip, phase));
        }
    }
    marks
}

/// A wavy line about `length` long, centred on `centre`.
fn mark(centre: [f64; 2], length: f64, flip: bool, phase: f64) -> WaveMark {
    let amplitude = length * 0.18 * if flip { -1.0 } else { 1.0 };
    (0..MARK_POINTS)
        .map(|k| {
            let t = k as f64 / (MARK_POINTS - 1) as f64;
            let wave = (std::f64::consts::TAU * (1.5 * t + phase * 0.25)).sin();
            [centre[0] + (t - 0.5) * length, centre[1] + amplitude * wave]
        })
        .collect()
}

/// The bounds of one exterior ring, padded by the reach of the marks.
struct Zone {
    min: [f64; 2],
    max: [f64; 2],
    perimeter: f64,
}

impl Zone {
    fn of(ring: &[[f64; 2]], pad: f64) -> Self {
        let mut zone = Zone {
            min: [f64::INFINITY; 2],
            max: [f64::NEG_INFINITY; 2],
            perimeter: 0.0,
        };
        for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            zone.min = [zone.min[0].min(a[0] - pad), zone.min[1].min(a[1] - pad)];
            zone.max = [zone.max[0].max(a[0] + pad), zone.max[1].max(a[1] + pad)];
            zone.perimeter += (b[0] - a[0]).hypot(b[1] - a[1]);
        }
        zone
    }

    fn area(&self) -> f64 {
        (self.max[0] - self.min[0]) * (self.max[1] - self.min[1])
    }

    fn contains(&self, p: [f64; 2]) -> bool {
        (self.min[0]..=self.max[0]).contains(&p[0]) && (self.min[1]..=self.max[1]).contains(&p[1])
    }
}

/// Whether `at` is in water, at least `near` and at most `far` from the coast.
fn in_band(outline: &MultiPolygon, zones: &[Zone], at: [f64; 2], near: f64, far: f64) -> bool {
    let mut nearest = f64::INFINITY;
    for (polygon, zone) in outline.0.iter().zip(zones) {
        // A polygon whose padded bounds miss `at` is neither under it nor within `far`.
        if !zone.contains(at) {
            continue;
        }
        let ring = &polygon.exterior;
        let mut inside = false;
        for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            if (a[1] <= at[1]) != (b[1] <= at[1])
                && at[0] < a[0] + (at[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0])
            {
                inside = !inside;
            }
            // Skip the distance for an edge wholly beyond `far` along one axis.
            let (lo, hi) = (a[0].min(b[0]), a[0].max(b[0]));
            let (low, high) = (a[1].min(b[1]), a[1].max(b[1]));
            if lo - at[0] > far || at[0] - hi > far || low - at[1] > far || at[1] - high > far {
                continue;
            }
            nearest = nearest.min(distance_to_segment(at, *a, *b));
            if nearest < near {
                return false;
            }
        }
        if inside {
            return false;
        }
    }
    nearest <= far
}

fn distance_to_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length_squared = dx * dx + dy * dy;
    let t = if length_squared == 0.0 {
        0.0
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length_squared).clamp(0.0, 1.0)
    };
    (p[0] - (a[0] + t * dx)).hypot(p[1] - (a[1] + t * dy))
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

    fn island() -> MultiPolygon {
        MultiPolygon(vec![square([0.0, 0.0], [200.0, 200.0])])
    }

    fn on_land(p: [f64; 2]) -> bool {
        (0.0..=200.0).contains(&p[0]) && (0.0..=200.0).contains(&p[1])
    }

    #[test]
    fn nothing_is_drawn_for_zero_density_or_an_empty_outline() {
        assert!(wave_marks(&island(), 0.0, 14.0, 1, 0.0).is_empty());
        assert!(wave_marks(&island(), f64::NAN, 14.0, 1, 0.0).is_empty());
        assert!(wave_marks(&island(), 5.0, 0.0, 1, 0.0).is_empty());
        assert!(wave_marks(&MultiPolygon::default(), 5.0, 14.0, 1, 0.0).is_empty());
    }

    #[test]
    fn marks_fill_the_water_around_the_land_and_never_touch_it() {
        let outline = island();
        let marks = wave_marks(&outline, 8.0, 14.0, 3, 20.0);
        assert!(marks.len() > 10, "{}", marks.len());
        for mark in &marks {
            assert_eq!(mark.len(), MARK_POINTS);
            for &p in mark {
                assert!(!on_land(p), "{p:?}");
                // Past the clearance, so beyond the water lines.
                let dx = (p[0] - p[0].clamp(0.0, 200.0)).abs();
                let dy = (p[1] - p[1].clamp(0.0, 200.0)).abs();
                assert!(dx.hypot(dy) >= 20.0, "{p:?}");
            }
        }
    }

    #[test]
    fn the_density_sets_how_many_marks() {
        let outline = island();
        let few = wave_marks(&outline, 2.0, 10.0, 1, 0.0).len();
        let many = wave_marks(&outline, 20.0, 10.0, 1, 0.0).len();
        assert!(many > few * 3, "{few} {many}");
    }

    #[test]
    fn the_same_seed_gives_the_same_marks_and_another_seed_other_ones() {
        let outline = island();
        let a = wave_marks(&outline, 8.0, 14.0, 5, 10.0);
        assert_eq!(a, wave_marks(&outline, 8.0, 14.0, 5, 10.0));
        assert_ne!(a, wave_marks(&outline, 8.0, 14.0, 6, 10.0));
    }

    #[test]
    fn a_mark_depends_on_its_place_not_on_the_outline_around_it() {
        // A second island far away adds marks but leaves the first island's alone.
        let alone = island();
        let both = MultiPolygon(vec![
            square([0.0, 0.0], [200.0, 200.0]),
            square([2000.0, 0.0], [2200.0, 200.0]),
        ]);
        let a = wave_marks(&alone, 8.0, 14.0, 5, 10.0);
        let b = wave_marks(&both, 8.0, 14.0, 5, 10.0);
        assert!(b.len() > a.len());
        assert!(a.iter().all(|mark| b.contains(mark)));
    }

    #[test]
    fn marks_stay_out_of_a_lake_inside_the_land() {
        let mut donut = square([0.0, 0.0], [400.0, 400.0]);
        donut.holes.push(vec![
            [100.0, 100.0],
            [100.0, 300.0],
            [300.0, 300.0],
            [300.0, 100.0],
        ]);
        let marks = wave_marks(&MultiPolygon(vec![donut]), 30.0, 10.0, 2, 0.0);
        assert!(!marks.is_empty());
        assert!(
            marks
                .iter()
                .flatten()
                .all(|p| { !((0.0..=400.0).contains(&p[0]) && (0.0..=400.0).contains(&p[1])) })
        );
    }

    #[test]
    fn a_huge_outline_is_capped() {
        let outline = MultiPolygon(vec![square([0.0, 0.0], [1_000_000.0, 1_000_000.0])]);
        let marks = wave_marks(&outline, 50.0, 100.0, 1, 0.0);
        assert!(marks.len() <= MAX_WAVE_MARKS);
        assert!(!marks.is_empty());
    }

    #[test]
    fn a_capped_outline_still_has_marks_across_its_whole_extent() {
        let outline = MultiPolygon(vec![square([0.0, 0.0], [100_000.0, 100_000.0])]);
        let marks = wave_marks(&outline, 50.0, 400.0, 1, 0.0);
        assert!(marks.len() <= MAX_WAVE_MARKS);
        let (mut top, mut bottom) = (0, 0);
        for p in marks.iter().flatten() {
            if p[1] < 50_000.0 {
                top += 1;
            } else {
                bottom += 1;
            }
        }
        assert!(top > 0 && bottom > 0, "{top} {bottom}");
    }
}
