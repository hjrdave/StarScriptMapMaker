//! Grouping Layers and dissolving Groups.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "existing invariant panics; replace with error handling over time (#351)"
)]

use std::collections::{HashMap, HashSet};

use crate::id::Id;
use crate::map::{Map, Subtree};
use crate::node::{Layer, Node};
use crate::op::Op;

use super::CommandError;
use super::check::{layer, unlocked, unlocked_branch};

pub(super) fn create_group(
    map: &Map,
    id: Id,
    ids: &[Id],
    name: &str,
) -> Result<Vec<Op>, CommandError> {
    if map.contains(id) {
        return Err(CommandError::IdInUse(id));
    }
    let Some(&first) = ids.first() else {
        return Err(CommandError::NothingToGroup);
    };
    let parent = map.parent(first);
    let mut seen = HashSet::new();
    for &member in ids {
        unlocked_branch(map, member)?;
        if !seen.insert(member) {
            return Err(CommandError::DuplicateTarget(member));
        }
        if map.parent(member) != parent {
            return Err(CommandError::NotSiblings(member));
        }
    }
    // Members keep their relative order, and the Group sits where the topmost one was.
    let members: Vec<Id> = map
        .children(parent)
        .iter()
        .copied()
        .filter(|child| seen.contains(child))
        .collect();
    let top = map
        .children(parent)
        .iter()
        .rposition(|child| seen.contains(child))
        .expect("a member is among its parent's children");
    let mut ops = vec![Op::Insert {
        parent,
        index: top + 1,
        subtree: Subtree {
            id,
            node: Node::Layer(Layer::new(name.trim())),
            children: Vec::new(),
        },
    }];
    ops.extend(
        members
            .into_iter()
            .enumerate()
            .map(|(index, member)| Op::Move {
                id: member,
                parent: Some(id),
                index,
            }),
    );
    Ok(ops)
}

pub(super) fn ungroup(map: &Map, ids: &[Id]) -> Result<Vec<Op>, CommandError> {
    let mut seen = HashSet::new();
    for &id in ids {
        layer(map, id)?;
        unlocked(map, id)?;
        if !seen.insert(id) {
            return Err(CommandError::DuplicateTarget(id));
        }
    }
    // Deepest Groups first, so a Group inside another listed Group is dissolved in place.
    let depth = |id: Id| std::iter::successors(map.parent(id), |&p| map.parent(p)).count();
    let mut ordered = ids.to_vec();
    ordered.sort_by_key(|&id| std::cmp::Reverse(depth(id)));
    // Child lists as the planned Ops leave them, loaded only for the parents they touch.
    let mut lists: HashMap<Option<Id>, Vec<Id>> = HashMap::new();
    let mut ops = Vec::new();
    for id in ordered {
        let parent = map.parent(id);
        let contents = lists
            .remove(&Some(id))
            .unwrap_or_else(|| map.children(Some(id)).to_vec());
        let siblings = lists
            .entry(parent)
            .or_insert_with(|| map.children(parent).to_vec());
        let index = siblings
            .iter()
            .position(|&s| s == id)
            .expect("a node is listed by its parent");
        siblings.splice(index..=index, contents.iter().copied());
        for (offset, child) in contents.into_iter().enumerate() {
            ops.push(Op::Move {
                id: child,
                parent,
                index: index + offset,
            });
        }
        ops.push(Op::Remove { id });
    }
    Ok(ops)
}
