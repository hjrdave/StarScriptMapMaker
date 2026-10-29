//! Creating, deleting and transforming Objects and the Layers that hold them.

use std::collections::HashSet;

use crate::color::Color;
use crate::geometry::{Affine, MultiPolygon};
use crate::id::Id;
use crate::map::{Map, Subtree};
use crate::node::{Lake, Landmass, Membership, Node, Object, ObjectKind};
use crate::op::{Op, Property};

use super::CommandError;
use super::check::{collect_branch, existing, layer, set, unlocked, unlocked_branch};
use super::land::{outline, outlined};

pub(super) fn delete(map: &Map, ids: &[Id]) -> Result<Vec<Op>, CommandError> {
    for &id in ids {
        unlocked_branch(map, id)?;
    }
    // Deleting a Layer deletes what it holds, so drop ids already inside another one.
    let requested: HashSet<Id> = ids.iter().copied().collect();
    let mut roots: Vec<Id> = Vec::new();
    for &id in ids {
        let inside_another = std::iter::successors(map.parent(id), |&p| map.parent(p))
            .any(|ancestor| requested.contains(&ancestor));
        if !inside_another && !roots.contains(&id) {
            roots.push(id);
        }
    }
    let mut removed = HashSet::new();
    for &root in &roots {
        collect_branch(map, root, &mut removed);
    }

    // Clips pointing into the deleted branches are cleared in the same Undo step.
    let mut ops = Vec::new();
    let mut clipped: Vec<Id> = map
        .ids()
        .filter(|id| !removed.contains(id))
        .filter(|&id| matches!(map.node(id), Some(Node::Layer(l)) if l.clip.is_some_and(|c| removed.contains(&c))))
        .collect();
    clipped.sort();
    for id in clipped {
        unlocked(map, id)?;
        ops.push(set(id, Property::Clip(None)));
    }
    ops.extend(roots.into_iter().map(|id| Op::Remove { id }));
    Ok(ops)
}

pub(super) fn create_object(
    map: &Map,
    id: Id,
    parent: Id,
    index: Option<usize>,
    kind: ObjectKind,
) -> Result<Vec<Op>, CommandError> {
    if map.contains(id) {
        return Err(CommandError::IdInUse(id));
    }
    layer(map, parent)?;
    unlocked(map, parent)?;
    let geometry = kind.geometry().ok_or(CommandError::NoOutline(id))?;
    if let Some(problem) = geometry.problem() {
        return Err(CommandError::InvalidGeometry { id, problem });
    }
    Ok(vec![Op::Insert {
        parent: Some(parent),
        index: index.unwrap_or(map.children(Some(parent)).len()),
        subtree: Subtree {
            id,
            node: Node::Object(Object::new(kind)),
            children: Vec::new(),
        },
    }])
}

pub(super) fn transform(
    map: &Map,
    ids: &[Id],
    transform: &Affine,
) -> Result<Vec<Op>, CommandError> {
    if !transform.is_invertible() {
        return Err(CommandError::InvalidTransform(transform.0));
    }
    let mut ids = outlined(map, ids)?;
    // Linked nodes take the same transform; a locked one blocks it for them all.
    let along = map.linked_movers(&ids)?;
    ids.extend(along.into_iter().filter(
        |&id| matches!(map.node(id), Some(Node::Object(o)) if o.kind.geometry().is_some()),
    ));
    if let Some(&id) = ids
        .iter()
        .find(|&&id| matches!(map.node(id), Some(Node::Object(o)) if o.move_locked))
    {
        return Err(CommandError::MoveLocked(id));
    }
    let mut ops = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(river) = super::river::river(map, id) {
            let moved = river.transformed(transform);
            if let Some(problem) = moved.problem() {
                return Err(CommandError::InvalidRiver { id, problem });
            }
            ops.push(set(id, Property::River(Box::new(moved))));
            continue;
        }
        let geometry = outline(map, id).transformed(transform);
        // Huge factors can overflow to infinity even when the transform itself is finite.
        if let Some(problem) = geometry.problem() {
            return Err(CommandError::InvalidGeometry { id, problem });
        }
        ops.push(Op::ReplaceGeometry { id, geometry });
    }
    Ok(ops)
}

/// Creates a Landmass in Layer `parent`. Without a `fill` it takes the Map's land colour.
pub(super) fn create_landmass(
    map: &Map,
    id: Id,
    parent: Id,
    index: Option<usize>,
    geometry: &MultiPolygon,
    fill: Option<Color>,
) -> Result<Vec<Op>, CommandError> {
    let landmass = Landmass {
        fill: fill.unwrap_or(map.defaults.land_fill),
        ..Landmass::new(geometry.clone())
    };
    create_object(map, id, parent, index, ObjectKind::Landmass(landmass))
}

/// Creates a Lake in Layer `parent`. Without a `fill` it takes the default water colour.
pub(super) fn create_lake(
    map: &Map,
    id: Id,
    parent: Id,
    index: Option<usize>,
    geometry: &MultiPolygon,
    fill: Option<Color>,
) -> Result<Vec<Op>, CommandError> {
    let lake = Lake {
        fill: fill.unwrap_or(Lake::DEFAULT_FILL),
        ..Lake::new(geometry.clone())
    };
    create_object(map, id, parent, index, ObjectKind::Lake(lake))
}

/// Sets whether Objects belong to the Canvas or sit off it. Layers are refused, and so are
/// locked Objects.
pub(super) fn set_membership(
    map: &Map,
    ids: &[Id],
    membership: Membership,
) -> Result<Vec<Op>, CommandError> {
    let membership = match membership {
        Membership::Board(board) if board == map.primary_board() => Membership::Canvas,
        Membership::Board(board) if map.board(board).is_none() => {
            return Err(CommandError::NoSuchBoard(board));
        }
        other => other,
    };
    let mut seen = HashSet::new();
    let mut ops = Vec::with_capacity(ids.len());
    for &id in ids {
        if !matches!(existing(map, id)?, Node::Object(_)) {
            return Err(CommandError::NotAnObject(id));
        }
        unlocked(map, id)?;
        if seen.insert(id) {
            ops.push(set(id, Property::Membership(membership)));
        }
    }
    Ok(ops)
}
