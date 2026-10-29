//! River Commands: creating and replacing Rivers, and editing the points of their paths.
//! A River's points are its nodes, all in part 0, ring 0, from source to mouth.

use crate::geometry::{NodeRef, Point};
use crate::id::Id;
use crate::map::{Map, Subtree};
use crate::node::{Node, Object, ObjectKind};
use crate::op::{Op, Property};
use crate::river::{River, RiverPoint};

use super::CommandError;
use super::check::{existing, layer, set, unlocked};

pub(super) fn create_river(
    map: &Map,
    id: Id,
    parent: Id,
    index: Option<usize>,
    river: &River,
) -> Result<Vec<Op>, CommandError> {
    if map.contains(id) {
        return Err(CommandError::IdInUse(id));
    }
    layer(map, parent)?;
    unlocked(map, parent)?;
    check_river(id, river)?;
    Ok(vec![Op::Insert {
        parent: Some(parent),
        index: index.unwrap_or(map.children(Some(parent)).len()),
        subtree: Subtree {
            id,
            node: Node::Object(Object::new(ObjectKind::River(Box::new(river.clone())))),
            children: Vec::new(),
        },
    }])
}

pub(super) fn set_river(map: &Map, id: Id, river: &River) -> Result<Vec<Op>, CommandError> {
    existing_river(map, id)?;
    unlocked(map, id)?;
    check_river(id, river)?;
    Ok(vec![set(id, Property::River(Box::new(river.clone())))])
}

/// The River `id`, if it is one.
pub(super) fn river(map: &Map, id: Id) -> Option<&River> {
    match map.node(id)? {
        Node::Object(Object {
            kind: ObjectKind::River(river),
            ..
        }) => Some(river),
        _ => None,
    }
}

/// Moves points of a River's path by `offset`.
pub(super) fn move_points(
    map: &Map,
    id: Id,
    nodes: &[NodeRef],
    offset: Point,
) -> Result<Vec<Op>, CommandError> {
    edit_path(map, id, |path| {
        let mut moved = vec![false; path.len()];
        for &node in nodes {
            let index = point_index(node, path.len())?;
            // A point named twice still moves once.
            if !std::mem::replace(&mut moved[index], true) {
                let p = &mut path[index].position;
                *p = [p[0] + offset[0], p[1] + offset[1]];
            }
        }
        Ok(())
    })
}

/// Adds a point at `point` after the point `edge`, as wide as the River is there.
pub(super) fn insert_point(
    map: &Map,
    id: Id,
    edge: NodeRef,
    point: Point,
) -> Result<Vec<Op>, CommandError> {
    edit_path(map, id, |path| {
        let index = point_index(edge, path.len())?;
        let (a, b) = (path[index], path.get(index + 1).copied());
        let width = b.map_or(a.width, |b| {
            let share = share_along(a.position, b.position, point);
            a.width + (b.width - a.width) * share
        });
        path.insert(index + 1, RiverPoint::new(point, width));
        Ok(())
    })
}

/// Deletes points of a River's path. A River keeps at least two.
pub(super) fn delete_points(map: &Map, id: Id, nodes: &[NodeRef]) -> Result<Vec<Op>, CommandError> {
    edit_path(map, id, |path| {
        let mut doomed = Vec::with_capacity(nodes.len());
        for &node in nodes {
            doomed.push(point_index(node, path.len())?);
        }
        doomed.sort_unstable_by(|a, b| b.cmp(a));
        doomed.dedup();
        if path.len() - doomed.len() < 2 {
            return Err("a River needs at least two points".into());
        }
        for index in doomed {
            path.remove(index);
        }
        Ok(())
    })
}

/// Replaces the path of River `id` with the result of `edit` on a copy of it.
fn edit_path(
    map: &Map,
    id: Id,
    edit: impl FnOnce(&mut Vec<RiverPoint>) -> Result<(), String>,
) -> Result<Vec<Op>, CommandError> {
    let river = existing_river(map, id)?;
    unlocked(map, id)?;
    let mut path = river.path().to_vec();
    edit(&mut path).map_err(|problem| CommandError::NodeEdit { id, problem })?;
    let edited = river.clone().with_path(path);
    check_river(id, &edited)?;
    Ok(vec![set(id, Property::River(Box::new(edited)))])
}

fn point_index(node: NodeRef, len: usize) -> Result<usize, String> {
    if node.part == 0 && node.ring == 0 && node.index < len {
        Ok(node.index)
    } else {
        Err(format!(
            "a River has no point at part {}, ring {}, point {}",
            node.part, node.ring, node.index
        ))
    }
}

/// How far along from `a` to `b` the point nearest `p` is, from 0 to 1.
fn share_along(a: Point, b: Point, p: Point) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length_squared = dx * dx + dy * dy;
    if length_squared == 0.0 {
        return 0.0;
    }
    (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length_squared).clamp(0.0, 1.0)
}

fn existing_river(map: &Map, id: Id) -> Result<&River, CommandError> {
    existing(map, id)?;
    river(map, id).ok_or(CommandError::NotARiver(id))
}

fn check_river(id: Id, river: &River) -> Result<(), CommandError> {
    match river.problem() {
        Some(problem) => Err(CommandError::InvalidRiver { id, problem }),
        None => Ok(()),
    }
}
