//! Lookups and lock checks shared by every Command's compile step.

use std::collections::HashSet;

use crate::id::Id;
use crate::map::Map;
use crate::node::{Category, Layer, Node};
use crate::op::{Op, Property};

use super::CommandError;

pub(super) fn set(id: Id, property: Property) -> Op {
    Op::SetProperty { id, property }
}

pub(super) fn existing(map: &Map, id: Id) -> Result<&Node, CommandError> {
    map.node(id).ok_or(CommandError::NotFound(id))
}

pub(super) fn layer(map: &Map, id: Id) -> Result<&Layer, CommandError> {
    match existing(map, id)? {
        Node::Layer(layer) => Ok(layer),
        Node::Object(_) => Err(CommandError::NotALayer(id)),
    }
}

/// Fails unless the Layer `parent` holds `category`, the category of `target` (the new or
/// moved Object or Layer).
pub(super) fn fits(
    map: &Map,
    parent: Id,
    target: Id,
    category: Category,
) -> Result<(), CommandError> {
    let layer_category = layer(map, parent)?.category;
    if layer_category == category {
        Ok(())
    } else {
        Err(CommandError::WrongCategory {
            target,
            target_category: category,
            layer: parent,
            layer_category,
        })
    }
}

/// The category of a node already in the Map: a Layer's own, or an Object's by kind.
pub(super) fn category_of(map: &Map, id: Id) -> Result<Category, CommandError> {
    Ok(match existing(map, id)? {
        Node::Layer(layer) => layer.category,
        Node::Object(object) => object.kind.category(),
    })
}

/// The nearest locked Layer above `id`.
pub(super) fn locked_ancestor(map: &Map, id: Id) -> Option<Id> {
    map.locked_ancestor(id)
}

/// Fails if `id` or any Layer above it is locked.
pub(super) fn unlocked(map: &Map, id: Id) -> Result<(), CommandError> {
    if existing(map, id)?.locked() {
        return Err(CommandError::Locked(id));
    }
    match locked_ancestor(map, id) {
        Some(layer) => Err(CommandError::InLockedLayer { target: id, layer }),
        None => Ok(()),
    }
}

/// Fails if `id`, a Layer above it, or anything inside it is locked.
pub(super) fn unlocked_branch(map: &Map, id: Id) -> Result<(), CommandError> {
    unlocked(map, id)?;
    let mut stack = map.children(Some(id)).to_vec();
    while let Some(child) = stack.pop() {
        if map.node(child).is_some_and(Node::locked) {
            return Err(CommandError::ContainsLocked {
                target: id,
                locked: child,
            });
        }
        stack.extend_from_slice(map.children(Some(child)));
    }
    Ok(())
}

pub(super) fn collect_branch(map: &Map, id: Id, out: &mut HashSet<Id>) {
    out.insert(id);
    for &child in map.children(Some(id)) {
        collect_branch(map, child, out);
    }
}
