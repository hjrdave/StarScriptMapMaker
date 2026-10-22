//! Link sets: nodes that move together.

use std::collections::HashSet;

use crate::id::Id;
use crate::map::Map;
use crate::node::Node;
use crate::op::{Op, Property};

use super::CommandError;
use super::check::{existing, set, unlocked};

/// Puts `ids` and every node linked to them into the Link set `link_id`.
pub(super) fn link(map: &Map, link_id: Id, ids: &[Id]) -> Result<Vec<Op>, CommandError> {
    let mut listed: Vec<Id> = Vec::with_capacity(ids.len());
    for &id in ids {
        existing(map, id)?;
        unlocked(map, id)?;
        if !listed.contains(&id) {
            listed.push(id);
        }
    }
    if listed.len() < 2 {
        return Err(CommandError::NothingToLink);
    }
    let mut members = listed.clone();
    for &id in &listed {
        for member in map.linked_with(id) {
            if !members.contains(&member) {
                unlocked(map, member)?;
                members.push(member);
            }
        }
    }
    Ok(members
        .into_iter()
        .filter(|&id| map.node(id).and_then(Node::link) != Some(link_id))
        .map(|id| set(id, Property::Link(Some(link_id))))
        .collect())
}

/// Takes `ids` out of their Link sets.
pub(super) fn unlink(map: &Map, ids: &[Id]) -> Result<Vec<Op>, CommandError> {
    let mut ops = Vec::new();
    let mut seen = HashSet::new();
    for &id in ids {
        let node = existing(map, id)?;
        unlocked(map, id)?;
        if node.link().is_some() && seen.insert(id) {
            ops.push(set(id, Property::Link(None)));
        }
    }
    Ok(ops)
}
