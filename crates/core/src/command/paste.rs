//! Duplicating and pasting nodes, with their Links, Clips and names.

use std::collections::{HashMap, HashSet};

use crate::geometry::Affine;
use crate::id::Id;
use crate::map::{Map, Subtree};
use crate::node::{Lake, Landmass, Node, ObjectKind};
use crate::op::Op;

use super::check::{existing, layer, unlocked};
use super::{CommandError, is_zero};

/// The name a copy of `name` gets: "Pine copy", then "Pine copy 2", the first not in `taken`.
fn copy_name(name: &str, taken: &HashSet<String>) -> String {
    let base = match name.rsplit_once(" copy") {
        Some((base, "")) => base,
        Some((base, n))
            if n.strip_prefix(' ')
                .is_some_and(|n| n.parse::<u32>().is_ok()) =>
        {
            base
        }
        _ => name,
    };
    let first = format!("{base} copy");
    if !taken.contains(&first) {
        return first;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base} copy {n}");
        if !taken.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// The name a node shows in its Layer, if it has one.
fn own_name(node: &Node) -> Option<&str> {
    match node {
        Node::Layer(layer) => Some(&layer.name),
        Node::Object(object) => object.name.as_deref(),
    }
}

pub(super) fn duplicate(
    map: &Map,
    ids: &[Id],
    new_ids: &[Id],
    offset: [f64; 2],
) -> Result<Vec<Op>, CommandError> {
    if !offset.iter().all(|v| v.is_finite()) {
        return Err(CommandError::InvalidTransform(Affine::translate(offset).0));
    }
    let translate = Affine::translate(offset);
    let listed: HashSet<Id> = ids.iter().copied().collect();
    let mut sources = Vec::with_capacity(ids.len());
    let mut seen = HashSet::new();
    for &id in ids {
        existing(map, id)?;
        let mut above = map.parent(id);
        let mut inside = false;
        while let Some(ancestor) = above {
            inside |= listed.contains(&ancestor);
            above = map.parent(ancestor);
        }
        if !inside && seen.insert(id) {
            sources.push(id);
        }
    }
    // Every copied node, in the order `new_ids` names them.
    let mut order = Vec::new();
    for &root in &sources {
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            order.push(id);
            pending.extend(map.children(Some(id)).iter().rev());
        }
    }
    if new_ids.len() != order.len() {
        return Err(CommandError::DuplicateIds {
            expected: order.len(),
            got: new_ids.len(),
        });
    }
    let mut fresh = HashSet::new();
    for &id in new_ids {
        if map.contains(id) || !fresh.insert(id) {
            return Err(CommandError::IdInUse(id));
        }
    }
    let renamed: HashMap<Id, Id> = order.iter().copied().zip(new_ids.iter().copied()).collect();

    // The children of each parent as the copies go in, so each lands just above its original.
    let mut siblings: HashMap<Option<Id>, Vec<Id>> = HashMap::new();
    let mut names: HashMap<Option<Id>, HashSet<String>> = HashMap::new();
    let mut ops = Vec::with_capacity(sources.len());
    for source in sources {
        let parent = map.parent(source);
        if let Some(parent) = parent {
            unlocked(map, parent)?;
        }
        let children = siblings
            .entry(parent)
            .or_insert_with(|| map.children(parent).to_vec());
        let taken = names.entry(parent).or_insert_with(|| {
            children
                .iter()
                .filter_map(|&id| map.node(id).and_then(own_name).map(str::to_owned))
                .collect()
        });
        let position = children
            .iter()
            .position(|&id| id == source)
            .ok_or(CommandError::NotFound(source))?;
        let mut subtree = copied_subtree(map, source, &renamed, &translate, offset)?;
        match &mut subtree.node {
            Node::Layer(layer) => {
                layer.locked = false;
                layer.name = copy_name(&layer.name, taken);
                taken.insert(layer.name.clone());
            }
            Node::Object(object) => {
                object.locked = false;
                if let Some(name) = &object.name {
                    let name = copy_name(name, taken);
                    taken.insert(name.clone());
                    object.name = Some(name);
                }
            }
        }
        children.insert(position + 1, subtree.id);
        ops.push(Op::Insert {
            parent,
            index: position + 1,
            subtree,
        });
    }
    Ok(ops)
}

pub(super) fn paste(
    map: &Map,
    parent: Option<Id>,
    index: Option<usize>,
    subtrees: &[Subtree],
    new_ids: &[Id],
    offset: [f64; 2],
) -> Result<Vec<Op>, CommandError> {
    if !offset.iter().all(|v| v.is_finite()) {
        return Err(CommandError::InvalidTransform(Affine::translate(offset).0));
    }
    if let Some(parent) = parent {
        layer(map, parent)?;
        unlocked(map, parent)?;
    }
    let siblings = map.children(parent);
    let index = index.unwrap_or(siblings.len());
    let mut order = Vec::new();
    for root in subtrees {
        let mut pending = vec![root];
        while let Some(tree) = pending.pop() {
            order.push(tree.id);
            pending.extend(tree.children.iter().rev());
        }
    }
    if new_ids.len() != order.len() {
        return Err(CommandError::DuplicateIds {
            expected: order.len(),
            got: new_ids.len(),
        });
    }
    let mut fresh = HashSet::new();
    for &id in new_ids {
        if map.contains(id) || !fresh.insert(id) {
            return Err(CommandError::IdInUse(id));
        }
    }
    let renamed: HashMap<Id, Id> = order.iter().copied().zip(new_ids.iter().copied()).collect();
    let translate = Affine::translate(offset);
    let mut taken: HashSet<String> = siblings
        .iter()
        .filter_map(|&id| map.node(id).and_then(own_name).map(str::to_owned))
        .collect();
    let mut ops = Vec::with_capacity(subtrees.len());
    for (n, source) in subtrees.iter().enumerate() {
        let mut subtree = pasted_subtree(map, source, &renamed, &translate, offset)?;
        match &mut subtree.node {
            Node::Layer(layer) => {
                if taken.contains(&layer.name) {
                    layer.name = copy_name(&layer.name, &taken);
                }
                taken.insert(layer.name.clone());
            }
            Node::Object(object) => {
                if let Some(name) = &object.name {
                    let name = if taken.contains(name) {
                        copy_name(name, &taken)
                    } else {
                        name.clone()
                    };
                    taken.insert(name.clone());
                    object.name = Some(name);
                }
            }
        }
        ops.push(Op::Insert {
            parent,
            index: index + n,
            subtree,
        });
    }
    let mut trees: Vec<&mut Subtree> = ops
        .iter_mut()
        .filter_map(|op| match op {
            Op::Insert { subtree, .. } => Some(subtree),
            _ => None,
        })
        .collect();
    relink_pasted(&mut trees);
    Ok(ops)
}

/// Keeps Links among pasted nodes: the nodes of one Link set that were pasted together form
/// a new set, named by the first of them; one pasted without the rest of its set is unlinked.
fn relink_pasted(trees: &mut [&mut Subtree]) {
    fn collect(tree: &Subtree, sets: &mut HashMap<Id, Vec<Id>>) {
        if let Some(set) = tree.node.link() {
            sets.entry(set).or_default().push(tree.id);
        }
        for child in &tree.children {
            collect(child, sets);
        }
    }
    fn apply(tree: &mut Subtree, sets: &HashMap<Id, Vec<Id>>) {
        let link = tree.node.link().and_then(|set| {
            let members = &sets[&set];
            (members.len() > 1).then(|| members[0])
        });
        tree.node.set_link(link);
        for child in &mut tree.children {
            apply(child, sets);
        }
    }
    let mut sets = HashMap::new();
    for tree in trees.iter() {
        collect(tree, &mut sets);
    }
    for tree in trees.iter_mut() {
        apply(tree, &sets);
    }
}

/// `source` with new ids, moved by `offset`, its Clips resolved against the pasted ids.
fn pasted_subtree(
    map: &Map,
    source: &Subtree,
    renamed: &HashMap<Id, Id>,
    translate: &Affine,
    offset: [f64; 2],
) -> Result<Subtree, CommandError> {
    let mut node = source.node.clone();
    if let Node::Layer(layer) = &mut node
        && let Some(clip) = layer.clip
    {
        let still_a_landmass = matches!(
            map.node(clip),
            Some(Node::Object(o)) if matches!(o.kind, ObjectKind::Landmass(_))
        );
        layer.clip = match renamed.get(&clip) {
            Some(&new) => Some(new),
            None if still_a_landmass => Some(clip),
            None => None,
        };
    }
    let id = renamed[&source.id];
    if let Node::Object(object) = &mut node
        && !is_zero(&offset)
    {
        match &mut object.kind {
            ObjectKind::Landmass(Landmass { geometry, .. })
            | ObjectKind::Lake(Lake { geometry, .. }) => {
                *geometry = geometry.transformed(translate);
                if let Some(problem) = geometry.problem() {
                    return Err(CommandError::InvalidGeometry { id, problem });
                }
            }
            ObjectKind::River(river) => **river = river.transformed(translate),
            ObjectKind::Stamp(stamp) => {
                stamp.position = [stamp.position[0] + offset[0], stamp.position[1] + offset[1]];
            }
            ObjectKind::Label(label) => {
                label.position = [label.position[0] + offset[0], label.position[1] + offset[1]];
            }
        }
    }
    let children = source
        .children
        .iter()
        .map(|child| pasted_subtree(map, child, renamed, translate, offset))
        .collect::<Result<_, _>>()?;
    Ok(Subtree { id, node, children })
}

/// A copy of the subtree at `id` with new ids, its outlines and positions moved by `offset`.
fn copied_subtree(
    map: &Map,
    id: Id,
    renamed: &HashMap<Id, Id>,
    translate: &Affine,
    offset: [f64; 2],
) -> Result<Subtree, CommandError> {
    let mut node = map.node(id).ok_or(CommandError::NotFound(id))?.clone();
    // A copy joins a Link set only when the whole set is copied, and then links to the copies.
    node.set_link(
        map.link_members(id)
            .filter(|members| members.iter().all(|member| renamed.contains_key(member)))
            .and_then(|members| members.iter().min())
            .map(|first| renamed[first]),
    );
    match &mut node {
        Node::Layer(layer) => {
            if let Some(clip) = layer.clip {
                layer.clip = Some(renamed.get(&clip).copied().unwrap_or(clip));
            }
        }
        Node::Object(object) if !is_zero(&offset) => match &mut object.kind {
            ObjectKind::Landmass(Landmass { geometry, .. })
            | ObjectKind::Lake(Lake { geometry, .. }) => {
                *geometry = geometry.transformed(translate);
                if let Some(problem) = geometry.problem() {
                    return Err(CommandError::InvalidGeometry { id, problem });
                }
            }
            ObjectKind::River(river) => **river = river.transformed(translate),
            ObjectKind::Stamp(stamp) => {
                stamp.position = [stamp.position[0] + offset[0], stamp.position[1] + offset[1]];
            }
            ObjectKind::Label(label) => {
                label.position = [label.position[0] + offset[0], label.position[1] + offset[1]];
            }
        },
        Node::Object(_) => {}
    }
    let children = map
        .children(Some(id))
        .iter()
        .map(|&child| copied_subtree(map, child, renamed, translate, offset))
        .collect::<Result<_, _>>()?;
    Ok(Subtree {
        id: renamed[&id],
        node,
        children,
    })
}
