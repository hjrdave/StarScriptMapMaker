//! Colour painting (ADR 0017): the Map's paint strokes as vector groups the Canvas draws above
//! the Landmass fills and below what comes after them (Stamps, Labels).
//!
//! Paint is clipped to land with the stencil, as a Layer clipped to a Landmass is, using the
//! roughened outlines the fills use, so it stops at the same coast and never covers a Lake. A
//! stroke is a path stroked with round caps. Its soft edge is a few bands of decreasing
//! opacity, drawn as one group so the bands never blend with each other.
//!
//! An eraser stroke removes the paint drawn before it. The strokes are split into steps at the
//! erasers: the strokes of a step draw only where no later eraser passes, which the stencil
//! decides. An eraser's edge is hard, so its softness is not used.

use std::sync::Arc;

use lyon::math::{Point, point};
use lyon::path::{Path, Winding};
use ssmap_core::{Id, PaintStroke};

use crate::color::{Color, ColorExt as _};
use crate::vector::{FillRule, ObjectRef, Paint, Placement, VectorGroup, VectorShape};

/// How many bands the soft edge of a stroke is drawn in.
const SOFT_BANDS: u32 = 6;

/// What the Canvas draws of the Map's paint.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct PaintLayer {
    /// How many of the content's vector groups draw before the paint.
    pub(crate) after: usize,
    /// Fills of the Landmasses the paint is clipped to.
    pub(crate) land: Vec<VectorGroup>,
    /// Fills of the water over the Landmasses, cut out of them.
    pub(crate) holes: Vec<VectorGroup>,
    /// The strokes, in order, split at the erasers.
    pub(crate) steps: Vec<PaintStep>,
    /// One group per eraser stroke, in order.
    pub(crate) erasers: Vec<VectorGroup>,
    /// The strokes the steps were built from, so an unchanged list is not built again.
    ids: Vec<Id>,
}

/// Paint strokes drawn together, and the erasers that come after them.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct PaintStep {
    /// One group per paint stroke, in order.
    pub(crate) strokes: Vec<VectorGroup>,
    /// The first of the layer's erasers that comes after the strokes. It and every later
    /// eraser remove paint of this step.
    pub(crate) first_eraser: usize,
}

impl PaintLayer {
    /// The paint of `strokes` clipped to `land` minus `holes`, drawn after `after` groups. The
    /// steps are taken from `previous` if it was built from the same strokes, which never
    /// change once added.
    pub(crate) fn new(
        after: usize,
        land: Vec<VectorGroup>,
        holes: Vec<VectorGroup>,
        strokes: &[PaintStroke],
        previous: Option<&PaintLayer>,
    ) -> Self {
        let ids: Vec<Id> = strokes.iter().map(|stroke| stroke.id).collect();
        let (steps, erasers) = match previous {
            Some(previous) if previous.ids == ids => {
                (previous.steps.clone(), previous.erasers.clone())
            }
            _ => steps_of(strokes),
        };
        Self {
            after,
            land,
            holes,
            steps,
            erasers,
            ids,
        }
    }
}

/// Splits `strokes` at the erasers: each step holds the paint between two erasers, and is
/// erased by every eraser from there on. Stretches with no paint have no step. Also returns the
/// erasers.
fn steps_of(strokes: &[PaintStroke]) -> (Vec<PaintStep>, Vec<VectorGroup>) {
    let mut stretches: Vec<Vec<VectorGroup>> = Vec::new();
    let mut erasers: Vec<VectorGroup> = Vec::new();
    let mut pending: Vec<VectorGroup> = Vec::new();
    for stroke in strokes {
        if stroke.erase {
            stretches.push(std::mem::take(&mut pending));
            erasers.push(eraser_group(stroke));
        } else {
            pending.push(paint_group(stroke));
        }
    }
    stretches.push(pending);
    let steps = stretches
        .into_iter()
        .enumerate()
        .filter(|(_, strokes)| !strokes.is_empty())
        .map(|(first_eraser, strokes)| PaintStep {
            strokes,
            first_eraser,
        })
        .collect();
    (steps, erasers)
}

/// Where a stroke's brush went.
enum Trail {
    /// A path, stroked with round caps.
    Line(Arc<Path>),
    /// A press and release in one place. That is a circle: a stroke with no length would
    /// draw nothing.
    Dot(Point),
}

fn trail_of(stroke: &PaintStroke) -> Trail {
    let mut points = stroke
        .points
        .iter()
        .map(|&[x, y]| point(x as f32, y as f32));
    let Some(first) = points.next() else {
        return Trail::Line(Arc::new(Path::new()));
    };
    let mut builder = Path::builder();
    builder.begin(first);
    let mut moved = false;
    for p in points {
        builder.line_to(p);
        moved |= p != first;
    }
    builder.end(false);
    if moved {
        Trail::Line(Arc::new(builder.build()))
    } else {
        Trail::Dot(first)
    }
}

/// The shape of the trail brushed `width` across.
fn stroked(trail: &Trail, color: Color, width: f32) -> VectorShape {
    let (path, paint) = match trail {
        Trail::Line(path) => (path.clone(), Paint::Stroke { width }),
        Trail::Dot(center) => {
            let mut builder = Path::builder();
            builder.add_circle(*center, width / 2.0, Winding::Positive);
            (Arc::new(builder.build()), Paint::Fill(FillRule::NonZero))
        }
    };
    VectorShape {
        path,
        color,
        paint,
        style: ssmap_core::FillStyle::FLAT,
    }
}

/// A group for `stroke`. Strokes never change, so the stroke's id with version 0 names its
/// tessellation in the cache.
fn group_of(stroke: &PaintStroke, shapes: Vec<VectorShape>) -> VectorGroup {
    VectorGroup {
        shapes,
        effect: false,
        object: Some(ObjectRef {
            id: stroke.id,
            version: 0,
        }),
        placement: Placement::Canvas,
    }
}

/// The group a paint stroke draws. Its shapes go from the solid core out through the soft
/// edge, because the first shape that covers a sample wins.
fn paint_group(stroke: &PaintStroke) -> VectorGroup {
    let path = trail_of(stroke);
    let size = stroke.size as f32;
    let core = size * (1.0 - stroke.softness);
    let mut shapes = Vec::new();
    if stroke.softness < 1.0 {
        shapes.push(stroked(&path, stroke.color.faded(stroke.opacity), core));
    }
    if stroke.softness > 0.0 {
        let bands = SOFT_BANDS as f32;
        for band in 1..=SOFT_BANDS {
            let b = band as f32;
            let width = core + (size - core) * b / bands;
            // The middle of the band decides how strong it is.
            let strength = stroke.opacity * (1.0 - (b - 0.5) / bands);
            shapes.push(stroked(&path, stroke.color.faded(strength), width));
        }
    }
    group_of(stroke, shapes)
}

/// The shape an eraser takes paint from; only where it covers matters, not its colour.
fn eraser_group(stroke: &PaintStroke) -> VectorGroup {
    let path = trail_of(stroke);
    group_of(
        stroke,
        vec![stroked(&path, Color::rgb(0, 0, 0), stroke.size as f32)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(erase: bool) -> PaintStroke {
        PaintStroke {
            id: Id::new(),
            points: vec![[0.0, 0.0], [10.0, 0.0]],
            color: Color::rgb(200, 30, 30),
            size: 10.0,
            softness: 0.0,
            opacity: 1.0,
            erase,
        }
    }

    fn widths(group: &VectorGroup) -> Vec<f32> {
        group
            .shapes
            .iter()
            .map(|s| match s.paint {
                Paint::Stroke { width } => width,
                Paint::Fill(_) => 0.0,
            })
            .collect()
    }

    #[test]
    fn erasers_split_the_paint_and_reach_only_what_came_before() {
        let list = [
            stroke(false),
            stroke(true),
            stroke(false),
            stroke(false),
            stroke(true),
        ];
        let (steps, erasers) = steps_of(&list);
        // The last stretch of paint is under only the erasers after it, here one; a trailing
        // eraser alone makes no step.
        assert_eq!(erasers.len(), 2);
        assert_eq!(steps.len(), 2);
        assert_eq!((steps[0].strokes.len(), steps[0].first_eraser), (1, 0));
        assert_eq!((steps[1].strokes.len(), steps[1].first_eraser), (2, 1));
        assert!(steps_of(&[stroke(true)]).0.is_empty());
        assert!(steps_of(&[]).0.is_empty());
    }

    #[test]
    fn a_hard_stroke_is_one_solid_shape() {
        let group = paint_group(&stroke(false));
        assert_eq!(widths(&group), vec![10.0]);
    }

    #[test]
    fn a_soft_stroke_fades_from_its_core_out_to_its_size() {
        let mut soft = stroke(false);
        soft.softness = 0.5;
        let group = paint_group(&soft);
        let w = widths(&group);
        assert_eq!(w.len(), 1 + SOFT_BANDS as usize);
        assert!(w.windows(2).all(|pair| pair[0] < pair[1]), "{w:?}");
        assert!((w[0] - 5.0).abs() < 1e-4 && (w[w.len() - 1] - 10.0).abs() < 1e-4);
        let alphas: Vec<u8> = group.shapes.iter().map(|s| s.color.a).collect();
        assert_eq!(alphas[0], 255);
        assert!(
            alphas.windows(2).all(|pair| pair[0] > pair[1]),
            "{alphas:?}"
        );
        // Fully soft has no solid core.
        soft.softness = 1.0;
        assert_eq!(paint_group(&soft).shapes.len(), SOFT_BANDS as usize);
    }

    #[test]
    fn opacity_scales_every_shape() {
        let mut faint = stroke(false);
        faint.softness = 0.4;
        faint.opacity = 0.5;
        let alphas: Vec<u8> = paint_group(&faint)
            .shapes
            .iter()
            .map(|s| s.color.a)
            .collect();
        assert_eq!(alphas[0], 128);
        assert!(alphas.iter().all(|&a| a <= 128));
    }

    #[test]
    fn a_dot_is_a_filled_circle() {
        let mut dot = stroke(false);
        dot.points = vec![[5.0, 5.0], [5.0, 5.0]];
        let group = paint_group(&dot);
        assert_eq!(group.shapes.len(), 1);
        assert!(matches!(group.shapes[0].paint, Paint::Fill(_)));
        // Its circle is as wide as the brush.
        let Some(lyon::path::Event::Begin { at }) = group.shapes[0].path.iter().next() else {
            panic!("the circle starts somewhere");
        };
        assert!(
            ((at.x - 5.0).hypot(at.y - 5.0) - 5.0).abs() < 1e-3,
            "{at:?}"
        );
    }

    #[test]
    fn the_steps_of_an_unchanged_list_are_kept() {
        let list = [stroke(false), stroke(true)];
        let first = PaintLayer::new(3, Vec::new(), Vec::new(), &list, None);
        let again = PaintLayer::new(4, Vec::new(), Vec::new(), &list, Some(&first));
        assert_eq!(again.steps, first.steps);
        assert_eq!(again.after, 4);
    }
}
