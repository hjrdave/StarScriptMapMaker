//! The Commands that add, erase and delete paint strokes (ADR 0017).
//!
//! Strokes belong to the Map, not to a Layer or Object, so Locks don't apply.

use std::collections::HashSet;

use crate::id::Id;
use crate::map::Map;
use crate::op::Op;
use crate::paint::{MAX_PAINT_STROKES, PaintStroke};

use super::CommandError;

/// Adds `stroke`, paint or eraser, after every other stroke, with its path simplified.
pub(super) fn add_stroke(map: &Map, stroke: &PaintStroke) -> Result<Vec<Op>, CommandError> {
    if map.paint_stroke(stroke.id).is_some() {
        return Err(CommandError::StrokeIdInUse(stroke.id));
    }
    if map.paint_strokes().len() >= MAX_PAINT_STROKES {
        return Err(CommandError::TooManyStrokes(MAX_PAINT_STROKES));
    }
    stroke.validate_settings()?;
    let stroke = stroke.clone().committed()?;
    Ok(vec![Op::InsertStroke {
        index: map.paint_strokes().len(),
        stroke,
    }])
}

/// Deletes the strokes in `ids`, each once. Each must exist.
pub(super) fn delete_strokes(map: &Map, ids: &[Id]) -> Result<Vec<Op>, CommandError> {
    let mut seen = HashSet::new();
    let mut ops = Vec::new();
    for &id in ids {
        if map.paint_stroke(id).is_none() {
            return Err(CommandError::NoSuchStroke(id));
        }
        if seen.insert(id) {
            ops.push(Op::RemoveStroke { id });
        }
    }
    Ok(ops)
}

/// Deletes every stroke.
pub(super) fn clear(map: &Map) -> Result<Vec<Op>, CommandError> {
    Ok(map
        .paint_strokes()
        .iter()
        .map(|stroke| Op::RemoveStroke { id: stroke.id })
        .collect())
}
