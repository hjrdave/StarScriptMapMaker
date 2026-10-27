//! The tree's rows and the pure questions asked of the Map: which rows are visible, where a drop
//! lands, what a node is called.

use std::collections::HashSet;

use ssmap_core::{Command, Id, Map, Node, Object, ObjectKind};

use crate::icons::Icon;

/// One visible row: a node and how deep it sits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Row {
    pub id: Id,
    pub depth: usize,
}

/// Where a dragged row would land, relative to the row under the pointer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DropTarget {
    /// Just above the row in the list, which is just above it in the Stacking order.
    Above(Id),
    Below(Id),
    /// On top of everything inside a Layer.
    Into(Id),
}

/// The visible rows, top of the Stacking order first, skipping collapsed Layers' contents.
pub fn rows(map: &Map, collapsed: &HashSet<Id>) -> Vec<Row> {
    fn walk(
        map: &Map,
        parent: Option<Id>,
        depth: usize,
        collapsed: &HashSet<Id>,
        out: &mut Vec<Row>,
    ) {
        let children = match parent {
            Some(layer) => map.ordered_children(layer),
            None => map.children(None).to_vec(),
        };
        for &id in children.iter().rev() {
            out.push(Row { id, depth });
            if map.node(id).is_some_and(Node::is_layer) && !collapsed.contains(&id) {
                walk(map, Some(id), depth + 1, collapsed, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(map, None, 0, collapsed, &mut out);
    out
}

/// The Move Command for dropping `dragged` on `target`, or `None` if it would do nothing
/// or move a Layer into itself.
pub fn move_command(map: &Map, dragged: Id, target: DropTarget) -> Option<Command> {
    let position = |id: Id| {
        map.children(map.parent(id))
            .iter()
            .position(|&sibling| sibling == id)
    };
    let (parent, mut index) = match target {
        DropTarget::Above(row) => (map.parent(row), position(row)? + 1),
        DropTarget::Below(row) => (map.parent(row), position(row)?),
        DropTarget::Into(layer) => (Some(layer), map.children(Some(layer)).len()),
    };
    if let Some(parent) = parent
        && (map.is_ancestor_or_self(dragged, parent) || !map.accepts_children(parent))
    {
        // Into itself, or onto a locked Group (itself or through one above it).
        return None;
    }
    // Move counts the index after the dragged node leaves its old place.
    let old_parent = map.parent(dragged);
    let old_index = position(dragged)?;
    if old_parent == parent && is_depth_sorted_stamp(map, dragged) {
        return None;
    }
    if old_parent == parent && old_index < index {
        index -= 1;
    }
    if old_parent == parent && old_index == index {
        return None;
    }
    Some(Command::Move {
        id: dragged,
        parent,
        index,
    })
}

/// A Stamp inside a Layer with Depth sort on, whose place among its siblings is not its to pick.
pub fn is_depth_sorted_stamp(map: &Map, id: Id) -> bool {
    matches!(
        map.node(id),
        Some(Node::Object(Object {
            kind: ObjectKind::Stamp(_),
            ..
        }))
    ) && matches!(
        map.parent(id).and_then(|p| map.node(p)),
        Some(Node::Layer(layer)) if layer.depth_sort
    )
}

/// Every Landmass, topmost first as the tree lists them.
pub fn landmasses(map: &Map) -> Vec<Id> {
    rows(map, &HashSet::new())
        .into_iter()
        .map(|row| row.id)
        .filter(|&id| is_landmass(map, id))
        .collect()
}

fn is_landmass(map: &Map, id: Id) -> bool {
    matches!(
        map.node(id),
        Some(Node::Object(object)) if matches!(object.kind, ObjectKind::Landmass(_))
    )
}

/// The Landmass a Layer should be clipped to: the one Object `selection` holds, if it is a
/// Landmass.
pub fn selected_landmass(map: &Map, selection: &[Id]) -> Option<Id> {
    match selection {
        &[id] if is_landmass(map, id) => Some(id),
        _ => None,
    }
}

/// A Landmass as a Clip target is named by its place in [`landmasses`], as Landmasses have no
/// names of their own.
pub fn landmass_name(map: &Map, id: Id) -> String {
    match landmasses(map).iter().position(|&l| l == id) {
        Some(index) => format!("Landmass {}", index + 1),
        None => "Missing Landmass".to_owned(),
    }
}

pub fn node_name(map: &Map, id: Id) -> String {
    match map.node(id) {
        Some(Node::Layer(layer)) if layer.name.trim().is_empty() => "Layer".to_owned(),
        Some(Node::Layer(layer)) => layer.name.clone(),
        Some(Node::Object(Object {
            name: Some(name), ..
        })) => name.clone(),
        Some(Node::Object(object)) => match &object.kind {
            ObjectKind::Landmass(_) => "Landmass".to_owned(),
            ObjectKind::Lake(_) => "Lake".to_owned(),
            ObjectKind::Stamp(_) => "Stamp".to_owned(),
            // A Label goes by its first line.
            ObjectKind::Label(label) => label.text.lines().next().unwrap_or("Label").to_owned(),
        },
        None => String::new(),
    }
}

/// How a row looks, apart from selection and hover.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RowState {
    /// A hidden Layer: slashed eye, name italic and dim.
    pub hidden: bool,
    /// Locked itself: filled padlock.
    pub locked: bool,
    /// Locked only through a Layer above: dashed padlock, dim name, not toggleable here.
    pub locked_by_parent: bool,
}

impl RowState {
    pub fn of(map: &Map, id: Id) -> Self {
        let Some(node) = map.node(id) else {
            return Self {
                hidden: false,
                locked: false,
                locked_by_parent: false,
            };
        };
        let locked = node.locked();
        Self {
            hidden: matches!(node, Node::Layer(layer) if !layer.visible),
            locked,
            locked_by_parent: !locked && map.is_locked(id),
        }
    }

    /// Hidden names go italic, so the state is never colour alone.
    pub fn italic(self) -> bool {
        self.hidden
    }

    pub(super) fn dim(self) -> bool {
        self.hidden || self.locked_by_parent
    }
}

/// How many Objects and Layers the Map holds.
pub fn counts(map: &Map) -> (usize, usize) {
    let layers = map
        .ids()
        .filter(|&id| map.node(id).is_some_and(Node::is_layer))
        .count();
    (map.ids().count() - layers, layers)
}

/// "3 Objects · 2 Layers".
pub fn footer_text(objects: usize, layers: usize) -> String {
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    format!(
        "{} · {}",
        plural(objects, "Object", "Objects"),
        plural(layers, "Layer", "Layers")
    )
}

pub(super) fn kind_icon(node: &Node) -> Icon {
    match node {
        Node::Layer(_) => Icon::Layer,
        Node::Object(object) => match object.kind {
            ObjectKind::Landmass(_) => Icon::Landmass,
            ObjectKind::Lake(_) => Icon::Lake,
            ObjectKind::Stamp(_) => Icon::Stamp,
            ObjectKind::Label(_) => Icon::LabelKind,
        },
    }
}
