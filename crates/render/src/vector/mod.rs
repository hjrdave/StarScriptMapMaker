//! Vector fills and strokes behind a trait, so the renderer can be swapped (ADR 0001).

mod cache;
mod chunk;
mod renderer;

use std::sync::Arc;

use lyon::path::Path;
use ssmap_core::Id;

pub use lyon::tessellation::FillRule;

use crate::color::Color;

pub use cache::{TessellationCache, TessellationRebuild};
pub(crate) use chunk::APPLY_BUDGET;
pub(crate) use renderer::LyonRenderer;

/// Where an Object is drawn and which edge clips it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Placement {
    /// A member of the Canvas, clipped at its edge.
    #[default]
    Canvas,
    /// A member of the Art board at this place among the scene's Art boards
    /// ([`CanvasScene::art_boards`](crate::CanvasScene::art_boards)), clipped at its edge. A
    /// scene without that Art board (export) leaves the Object out.
    Board(usize),
    /// Off every board: drawn in full around them, and left out of export.
    OffCanvas,
}

/// How a path is painted.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Paint {
    Fill(FillRule),
    /// A stroke centred on the path, `width` Canvas units across, with round joins and caps.
    Stroke {
        width: f32,
    },
}

/// One painted path in Canvas units.
#[derive(Clone, Debug)]
pub struct VectorShape {
    /// Shared, so a group is cheap to copy when part of the content is replaced.
    pub path: Arc<Path>,
    pub color: Color,
    pub paint: Paint,
}

impl PartialEq for VectorShape {
    fn eq(&self, other: &Self) -> bool {
        self.color == other.color
            && self.paint == other.paint
            && self.path.iter().eq(other.path.iter())
    }
}

/// Shapes painted as one, such as a Label's fill and its halo. Every sample takes the colour
/// of the first shape in the group that covers it, so the group never blends with itself:
/// a translucent stroke doesn't darken where it overlaps itself, and a halo listed after the
/// fill shows only around it.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct VectorGroup {
    pub shapes: Vec<VectorShape>,
    /// A Shore effect (ripples or inner shading), which a frame drawn mid-gesture skips.
    pub effect: bool,
    /// The Object the group draws, whose tessellation is cached as a whole. Groups of no
    /// Object (Clip masks, the backdrop) are tessellated afresh every time.
    pub object: Option<ObjectRef>,
    /// Which board's edge clips the group, or none: an Object off the boards shows in full
    /// around them, and is left out of export.
    pub placement: Placement,
}

/// An Object at the version its groups were built from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ObjectRef {
    pub id: Id,
    pub version: u64,
}

impl VectorGroup {
    pub fn fill(path: Path, color: Color, rule: FillRule) -> Self {
        Self {
            shapes: vec![VectorShape {
                path: Arc::new(path),
                color,
                paint: Paint::Fill(rule),
            }],
            effect: false,
            object: None,
            placement: Placement::Canvas,
        }
    }

    /// Marks the group as drawing `object`.
    pub fn of_object(mut self, object: ObjectRef) -> Self {
        self.object = Some(object);
        self
    }

    /// Marks the group as a Shore effect.
    pub fn with_effect(mut self) -> Self {
        self.effect = true;
        self
    }

    /// Whether drawing the group's triangles plainly could cover a sample twice. Fill
    /// tessellation never overlaps itself; strokes and stacked shapes do.
    fn may_overlap(&self) -> bool {
        match self.shapes.as_slice() {
            [] => false,
            [shape] => matches!(shape.paint, Paint::Stroke { .. }),
            _ => true,
        }
    }
}

/// What a frame draws of the groups: those that reach `bounds`, and Shore effects only if
/// `effects`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DrawFilter {
    /// The visible Canvas area as `[min_x, min_y, max_x, max_y]`.
    pub bounds: [f32; 4],
    pub effects: bool,
}

impl DrawFilter {
    pub(crate) fn everything() -> Self {
        Self {
            bounds: [f32::MIN, f32::MIN, f32::MAX, f32::MAX],
            effects: true,
        }
    }
}

/// Whether two `[min_x, min_y, max_x, max_y]` boxes overlap.
pub(crate) fn overlaps(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}
