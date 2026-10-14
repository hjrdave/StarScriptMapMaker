//! Generating Silhouettes for outline art (ADR 0003).
//!
//! Everything the art's ink encloses, plus the ink itself, is inside. The outside is found by
//! filling inward from the art's outer edge through every pixel that is not ink. Small breaks
//! in an outline would let that fill leak in, so the ink is first thickened by `gap` pixels,
//! and the outside is grown back by the same amount afterwards (a morphological closing).

use crate::raster::Raster;

/// Alpha at or above which a pixel counts as ink.
const INK_ALPHA: u8 = 64;

/// The Silhouette of `art`: opaque white inside, transparent outside, at the art's size.
/// `gap` is the widest break in an outline, in pixels, that still counts as closed.
pub fn generate(art: &Raster, gap: u32) -> Raster {
    // A transparent margin lets the outer fill reach around ink that touches the edge.
    let margin = gap + 1;
    let width = (art.width + 2 * margin) as usize;
    let height = (art.height + 2 * margin) as usize;
    let mut ink = vec![false; width * height];
    for y in 0..art.height {
        for x in 0..art.width {
            let i = (y + margin) as usize * width + (x + margin) as usize;
            ink[i] = art.pixel(x, y)[3] >= INK_ALPHA;
        }
    }

    let blocked = dilate(&ink, width, height, gap as usize);
    let outside = flood_from_edges(&blocked, width, height);
    let outside = dilate(&outside, width, height, gap as usize);

    let mut silhouette = Raster::new(art.width, art.height);
    for y in 0..art.height {
        for x in 0..art.width {
            let i = (y + margin) as usize * width + (x + margin) as usize;
            if ink[i] || !outside[i] {
                let o = ((y * art.width + x) * 4) as usize;
                silhouette.pixels[o..o + 4].copy_from_slice(&[255; 4]);
            }
        }
    }
    silhouette
}

/// Every pixel reachable from the grid's edge through unblocked pixels, 4-connected.
fn flood_from_edges(blocked: &[bool], width: usize, height: usize) -> Vec<bool> {
    let mut reached = vec![false; width * height];
    let mut stack = Vec::new();
    let visit = |i: usize, reached: &mut Vec<bool>, stack: &mut Vec<usize>| {
        if !blocked[i] && !reached[i] {
            reached[i] = true;
            stack.push(i);
        }
    };
    for x in 0..width {
        visit(x, &mut reached, &mut stack);
        visit((height - 1) * width + x, &mut reached, &mut stack);
    }
    for y in 0..height {
        visit(y * width, &mut reached, &mut stack);
        visit(y * width + width - 1, &mut reached, &mut stack);
    }
    while let Some(i) = stack.pop() {
        let (x, y) = (i % width, i / width);
        if x > 0 {
            visit(i - 1, &mut reached, &mut stack);
        }
        if x + 1 < width {
            visit(i + 1, &mut reached, &mut stack);
        }
        if y > 0 {
            visit(i - width, &mut reached, &mut stack);
        }
        if y + 1 < height {
            visit(i + width, &mut reached, &mut stack);
        }
    }
    reached
}

/// Grows set pixels by `radius` in every direction (a square), in two separable passes.
fn dilate(mask: &[bool], width: usize, height: usize, radius: usize) -> Vec<bool> {
    if radius == 0 {
        return mask.to_vec();
    }
    let pass =
        |source: &[bool], len: usize, lines: usize, index: &dyn Fn(usize, usize) -> usize| {
            let mut out = vec![false; source.len()];
            for line in 0..lines {
                // Distance since the last set pixel, scanning forwards then backwards.
                let mut since = usize::MAX;
                for at in 0..len {
                    let i = index(line, at);
                    since = if source[i] {
                        0
                    } else {
                        since.saturating_add(1)
                    };
                    out[i] = since <= radius;
                }
                since = usize::MAX;
                for at in (0..len).rev() {
                    let i = index(line, at);
                    since = if source[i] {
                        0
                    } else {
                        since.saturating_add(1)
                    };
                    out[i] |= since <= radius;
                }
            }
            out
        };
    let rows = pass(mask, width, height, &|y, x| y * width + x);
    pass(&rows, height, width, &|x, y| y * width + x)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A raster from rows of `#` (ink) and `.` (empty).
    fn raster(rows: &[&str]) -> Raster {
        let mut raster = Raster::new(rows[0].len() as u32, rows.len() as u32);
        for (y, row) in rows.iter().enumerate() {
            for (x, c) in row.chars().enumerate() {
                if c == '#' {
                    let i = (y * row.len() + x) * 4;
                    raster.pixels[i..i + 4].copy_from_slice(&[0, 0, 0, 255]);
                }
            }
        }
        raster
    }

    fn picture(raster: &Raster) -> Vec<String> {
        (0..raster.height)
            .map(|y| {
                (0..raster.width)
                    .map(|x| {
                        if raster.pixel(x, y)[3] == 255 {
                            '#'
                        } else {
                            '.'
                        }
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_closed_outline_fills() {
        let art = raster(&[
            ".......", //
            ".#####.", //
            ".#...#.", //
            ".#.#.#.", //
            ".#####.", //
            ".......", //
        ]);
        assert_eq!(
            picture(&generate(&art, 0)),
            [
                ".......", ".#####.", ".#####.", ".#####.", ".#####.", "......."
            ]
        );
    }

    #[test]
    fn ink_touching_the_edge_still_fills() {
        let art = raster(&["####", "#..#", "####"]);
        assert_eq!(picture(&generate(&art, 0)), ["####", "####", "####"]);
    }

    #[test]
    fn small_gaps_close_and_open_shapes_stay_open() {
        let art = raster(&[
            "........", //
            ".######.", //
            ".#....#.", //
            ".#....#.", //
            ".###.##.", //
            "........", //
        ]);
        // Without closing, the one-pixel break lets the outside in.
        assert_eq!(picture(&generate(&art, 0)), picture(&art));
        assert_eq!(
            picture(&generate(&art, 1)),
            [
                "........", //
                ".######.", //
                ".######.", //
                ".######.", //
                ".######.", //
                "........", //
            ]
        );
    }
}
