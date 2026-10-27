//! Commands that create, name, move and set the properties of Layers and Objects.

use crate::geometry::Point;
use crate::id::Id;
use crate::map::{Map, Subtree};
use crate::node::{Layer, Node, Object, ObjectKind};
use crate::op::{Op, Property};

use super::CommandError;
use super::check::{existing, layer, locked_ancestor, set, unlocked, unlocked_branch};

pub(super) fn create_layer(
    map: &Map,
    id: Id,
    parent: Option<Id>,
    index: Option<usize>,
    name: &str,
) -> Result<Vec<Op>, CommandError> {
    if map.contains(id) {
        return Err(CommandError::IdInUse(id));
    }
    if let Some(parent) = parent {
        layer(map, parent)?;
        unlocked(map, parent)?;
    }
    Ok(vec![Op::Insert {
        parent,
        index: index.unwrap_or(map.children(parent).len()),
        subtree: Subtree {
            id,
            node: Node::Layer(Layer::new(name)),
            children: Vec::new(),
        },
    }])
}

pub(super) fn rename_layer(map: &Map, id: Id, name: &str) -> Result<Vec<Op>, CommandError> {
    layer(map, id)?;
    unlocked(map, id)?;
    Ok(vec![set(id, Property::Name(name.to_owned()))])
}

pub(super) fn rename_object(map: &Map, id: Id, name: &str) -> Result<Vec<Op>, CommandError> {
    if !matches!(existing(map, id)?, Node::Object(_)) {
        return Err(CommandError::NotAnObject(id));
    }
    unlocked(map, id)?;
    Ok(vec![set(id, Property::Name(name.trim().to_owned()))])
}

pub(super) fn move_node(
    map: &Map,
    id: Id,
    parent: Option<Id>,
    index: usize,
) -> Result<Vec<Op>, CommandError> {
    unlocked_branch(map, id)?;
    // A Depth-sorted Layer orders its own Stamps, so they can't be reordered there.
    if map.parent(id) == parent
        && let Some(layer) = parent
        && matches!(map.node(layer), Some(Node::Layer(l)) if l.depth_sort)
        && matches!(
            map.node(id),
            Some(Node::Object(Object {
                kind: ObjectKind::Stamp(_),
                ..
            }))
        )
    {
        return Err(CommandError::DepthSorted(id));
    }
    if let Some(parent) = parent {
        layer(map, parent)?;
        unlocked(map, parent)?;
    }
    Ok(vec![Op::Move { id, parent, index }])
}

pub(super) fn set_visible(map: &Map, id: Id, visible: bool) -> Result<Vec<Op>, CommandError> {
    layer(map, id)?;
    unlocked(map, id)?;
    Ok(vec![set(id, Property::Visible(visible))])
}

pub(super) fn set_locked(map: &Map, id: Id, locked: bool) -> Result<Vec<Op>, CommandError> {
    existing(map, id)?;
    // The node's own Lock never blocks changing it, but a locked Layer above does.
    if let Some(layer) = locked_ancestor(map, id) {
        return Err(CommandError::InLockedLayer { target: id, layer });
    }
    Ok(vec![set(id, Property::Locked(locked))])
}

pub(super) fn set_opacity(map: &Map, id: Id, opacity: f32) -> Result<Vec<Op>, CommandError> {
    layer(map, id)?;
    unlocked(map, id)?;
    if !(0.0..=1.0).contains(&opacity) {
        return Err(CommandError::InvalidOpacity(opacity));
    }
    Ok(vec![set(id, Property::Opacity(opacity))])
}

pub(super) fn set_depth_sort(map: &Map, id: Id, depth_sort: bool) -> Result<Vec<Op>, CommandError> {
    layer(map, id)?;
    unlocked(map, id)?;
    Ok(vec![set(id, Property::DepthSort(depth_sort))])
}

pub(super) fn set_clip(map: &Map, id: Id, target: Option<Id>) -> Result<Vec<Op>, CommandError> {
    layer(map, id)?;
    unlocked(map, id)?;
    if let Some(target) = target
        && !matches!(
            existing(map, target)?,
            Node::Object(o) if matches!(o.kind, ObjectKind::Landmass(_))
        )
    {
        return Err(CommandError::NotALandmass(target));
    }
    Ok(vec![set(id, Property::Clip(target))])
}

/// Resizes the Canvas. The size must be at least 1 by 1 and the origin finite.
pub(super) fn resize_canvas(
    origin: Point,
    width: u32,
    height: u32,
) -> Result<Vec<Op>, CommandError> {
    if width == 0 || height == 0 {
        return Err(CommandError::InvalidCanvasSize { width, height });
    }
    if !origin.iter().all(|c| c.is_finite()) {
        return Err(CommandError::InvalidCanvasOrigin(origin));
    }
    Ok(vec![Op::ResizeCanvas {
        origin,
        width,
        height,
    }])
}
