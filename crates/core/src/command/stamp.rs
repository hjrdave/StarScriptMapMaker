//! Stamp Commands.

use std::collections::HashSet;

use crate::id::Id;
use crate::map::{Map, Subtree};
use crate::node::{Node, Object, ObjectKind, Stamp};
use crate::op::{Op, Property};

use super::check::{existing, layer, set, unlocked};
use super::{CommandError, PlacedStamp};

pub(super) fn create_stamp(
    map: &Map,
    id: Id,
    parent: Id,
    index: Option<usize>,
    stamp: &Stamp,
) -> Result<Vec<Op>, CommandError> {
    if map.contains(id) {
        return Err(CommandError::IdInUse(id));
    }
    layer(map, parent)?;
    unlocked(map, parent)?;
    check_stamp(stamp)?;
    Ok(vec![Op::Insert {
        parent: Some(parent),
        index: index.unwrap_or(map.children(Some(parent)).len()),
        subtree: Subtree {
            id,
            node: Node::Object(Object::new(ObjectKind::Stamp(stamp.clone()))),
            children: Vec::new(),
        },
    }])
}

pub(super) fn set_stamp(map: &Map, id: Id, stamp: &Stamp) -> Result<Vec<Op>, CommandError> {
    match existing(map, id)? {
        Node::Object(Object {
            kind: ObjectKind::Stamp(_),
            ..
        }) => {}
        _ => return Err(CommandError::NotAStamp(id)),
    }
    unlocked(map, id)?;
    check_stamp(stamp)?;
    Ok(vec![set(id, Property::Stamp(Box::new(stamp.clone())))])
}

pub(super) fn place_stamps(
    map: &Map,
    parent: Id,
    stamps: &[PlacedStamp],
) -> Result<Vec<Op>, CommandError> {
    layer(map, parent)?;
    unlocked(map, parent)?;
    let start = map.children(Some(parent)).len();
    let mut seen = HashSet::new();
    let mut ops = Vec::with_capacity(stamps.len());
    for (offset, placed) in stamps.iter().enumerate() {
        if map.contains(placed.id) || !seen.insert(placed.id) {
            return Err(CommandError::IdInUse(placed.id));
        }
        check_stamp(&placed.stamp)?;
        ops.push(Op::Insert {
            parent: Some(parent),
            index: start + offset,
            subtree: Subtree {
                id: placed.id,
                node: Node::Object(Object::new(ObjectKind::Stamp(placed.stamp.clone()))),
                children: Vec::new(),
            },
        });
    }
    Ok(ops)
}

/// Checks a Stamp's values are drawable.
fn check_stamp(stamp: &Stamp) -> Result<(), CommandError> {
    let invalid = |reason| Err(CommandError::InvalidStamp(reason));
    if !stamp.position.iter().all(|v| v.is_finite()) || !stamp.rotation.is_finite() {
        return invalid("the position and rotation must be finite");
    }
    if !(stamp.scale.is_finite() && stamp.scale > 0.0) {
        return invalid("the scale must be greater than 0");
    }
    if !(0.0..=1.0).contains(&stamp.opacity) {
        return Err(CommandError::InvalidOpacity(stamp.opacity));
    }
    Ok(())
}
