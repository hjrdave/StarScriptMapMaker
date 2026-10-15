//! The Map: a Canvas and one ordered tree of Layers and Objects.

use std::collections::{HashMap, HashSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::embedded::Embedded;
use crate::extra::Extra;
use crate::id::Id;
use crate::label::{LabelStyles, ResolvedLabel};
use crate::node::{Node, Object, ObjectKind};
use crate::roughness::Roughness;
use crate::shore::{ShoreKind, ShoreStyle};

/// The bounded drawing area of a Map.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Canvas {
    /// Width in nominal pixels (Canvas units).
    pub width: u32,
    /// Height in nominal pixels (Canvas units).
    pub height: u32,
    pub scale: Scale,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Canvas {
    pub fn new(width: u32, height: u32, scale: Scale) -> Self {
        Self {
            width,
            height,
            scale,
            extra: Extra::new(),
        }
    }
}

/// How much real-world distance one Canvas unit stands for.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Scale {
    pub distance_per_unit: f64,
    pub unit: DistanceUnit,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Scale {
    pub fn new(distance_per_unit: f64, unit: DistanceUnit) -> Self {
        Self {
            distance_per_unit,
            unit,
            extra: Extra::new(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DistanceUnit {
    Miles,
    Kilometres,
}

/// Map-wide defaults for Objects that don't override them.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct MapDefaults {
    /// The Roughness of every Landmass and Lake without its own.
    #[serde(default)]
    pub roughness: Roughness,
    /// The Shore style of every Landmass without its own.
    #[serde(default = "ShoreStyle::landmass_default")]
    pub shore: ShoreStyle,
    /// The Shore style of every Lake without its own.
    #[serde(default = "ShoreStyle::lake_default")]
    pub lake_shore: ShoreStyle,
    /// The Label styles Labels refer to. Files saved before they existed get the default one.
    #[serde(default)]
    pub label_styles: LabelStyles,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for MapDefaults {
    fn default() -> Self {
        Self {
            roughness: Roughness::default(),
            shore: ShoreStyle::landmass_default(),
            lake_shore: ShoreStyle::lake_default(),
            label_styles: LabelStyles::default(),
            extra: Extra::new(),
        }
    }
}

impl MapDefaults {
    /// The Shore style default for a kind of Object.
    pub fn shore_for(&self, kind: ShoreKind) -> &ShoreStyle {
        match kind {
            ShoreKind::Landmass => &self.shore,
            ShoreKind::Lake => &self.lake_shore,
        }
    }
}

/// A node and all its descendants, in Stacking order (first child draws lowest).
///
/// This is both the shape of the read view and the payload Ops use to insert and restore
/// whole branches of the tree.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Subtree {
    pub id: Id,
    #[serde(flatten)]
    pub node: Node,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Subtree>,
}

/// A serialisable snapshot of a whole Map, for agents, tests and debugging.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct MapView {
    pub canvas: Canvas,
    #[serde(default)]
    pub defaults: MapDefaults,
    /// The top-level Layers, first drawn lowest.
    pub layers: Vec<Subtree>,
    /// The Assets and fonts embedded in the Map.
    #[serde(default, skip_serializing_if = "Embedded::is_empty")]
    pub embedded: Embedded,
}

#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub(crate) node: Node,
    pub(crate) parent: Option<Id>,
    pub(crate) children: Vec<Id>,
    pub(crate) version: u64,
}

/// One Map document. It is changed only by applying [`Op`](crate::Op)s.
#[derive(Clone, Debug)]
pub struct Map {
    pub canvas: Canvas,
    /// Changed only by [`Op::SetDefaults`](crate::Op::SetDefaults).
    pub(crate) defaults: MapDefaults,
    /// Changed only by the embed Ops.
    pub(crate) embedded: Embedded,
    pub(crate) root: Vec<Id>,
    pub(crate) entries: HashMap<Id, Entry>,
    /// Increases with every applied Op; touched nodes take its new value as their version.
    pub(crate) revision: u64,
}

impl Map {
    pub fn new(canvas: Canvas) -> Self {
        Self {
            canvas,
            defaults: MapDefaults::default(),
            embedded: Embedded::default(),
            root: Vec::new(),
            entries: HashMap::new(),
            revision: 0,
        }
    }

    /// Builds a Map from a read view, as when loading.
    pub fn from_view(view: MapView) -> Result<Self, InvariantError> {
        let mut map = Self::new(view.canvas);
        map.defaults = view.defaults;
        map.embedded = view.embedded;
        for subtree in view.layers {
            if !subtree.node.is_layer() {
                return Err(InvariantError::ObjectAtRoot(subtree.id));
            }
            map.root.push(subtree.id);
            map.insert_subtree(None, subtree)?;
        }
        Ok(map)
    }

    pub fn view(&self) -> MapView {
        MapView {
            canvas: self.canvas.clone(),
            defaults: self.defaults.clone(),
            layers: self.root.iter().map(|&id| self.subtree(id)).collect(),
            embedded: self.embedded.clone(),
        }
    }

    /// The Assets and fonts embedded in the Map.
    pub fn embedded(&self) -> &Embedded {
        &self.embedded
    }

    pub fn defaults(&self) -> &MapDefaults {
        &self.defaults
    }

    /// The Roughness a Landmass or Lake is drawn with: its own, or the Map-wide default.
    /// `None` for anything without an outline.
    pub fn roughness(&self, id: Id) -> Option<&Roughness> {
        let Node::Object(Object { kind, .. }) = self.node(id)? else {
            return None;
        };
        kind.geometry()?;
        Some(kind.roughness().unwrap_or(&self.defaults.roughness))
    }

    /// The Shore style a Landmass or Lake is drawn with: its own, or the Map-wide default for
    /// its kind. `None` for anything without an outline.
    pub fn shore(&self, id: Id) -> Option<&ShoreStyle> {
        let Node::Object(Object { kind, .. }) = self.node(id)? else {
            return None;
        };
        let default = self.defaults.shore_for(kind.shore_kind()?);
        Some(kind.shore().unwrap_or(default))
    }

    /// The Label styles of the Map.
    pub fn label_styles(&self) -> &LabelStyles {
        &self.defaults.label_styles
    }

    /// A Label with its style applied. `None` for anything that is not a Label.
    pub fn resolved_label(&self, id: Id) -> Option<ResolvedLabel> {
        match self.node(id)? {
            Node::Object(Object {
                kind: ObjectKind::Label(label),
                ..
            }) => Some(label.resolve(&self.defaults.label_styles)),
            _ => None,
        }
    }

    /// The Labels that follow the style `style`, in no particular order.
    pub fn labels_using(&self, style: Id) -> Vec<Id> {
        self.entries
            .iter()
            .filter(|(_, entry)| {
                matches!(
                    &entry.node,
                    Node::Object(Object { kind: ObjectKind::Label(label), .. }) if label.style == style
                )
            })
            .map(|(&id, _)| id)
            .collect()
    }

    pub fn contains(&self, id: Id) -> bool {
        self.entries.contains_key(&id)
    }

    pub fn node(&self, id: Id) -> Option<&Node> {
        self.entries.get(&id).map(|entry| &entry.node)
    }

    /// The parent Layer of `id`, or `None` for a top-level Layer or an unknown id.
    pub fn parent(&self, id: Id) -> Option<Id> {
        self.entries.get(&id).and_then(|entry| entry.parent)
    }

    /// The children of a Layer in Stacking order, or the top-level Layers for `None`.
    pub fn children(&self, parent: Option<Id>) -> &[Id] {
        match parent {
            None => &self.root,
            Some(id) => self.entries.get(&id).map_or(&[], |entry| &entry.children),
        }
    }

    /// Every Layer and Object id, in no particular order.
    pub fn ids(&self) -> impl Iterator<Item = Id> + '_ {
        self.entries.keys().copied()
    }

    /// Changes whenever an Op touches the node; derived caches key on it.
    pub fn version(&self, id: Id) -> Option<u64> {
        self.entries.get(&id).map(|entry| entry.version)
    }

    /// The number of Ops applied so far.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The node and all its descendants. Panics if `id` is unknown.
    pub fn subtree(&self, id: Id) -> Subtree {
        let entry = &self.entries[&id];
        Subtree {
            id,
            node: entry.node.clone(),
            children: entry.children.iter().map(|&c| self.subtree(c)).collect(),
        }
    }

    /// `ancestor` itself, or any Layer above `id`.
    pub fn is_ancestor_or_self(&self, ancestor: Id, id: Id) -> bool {
        let mut current = Some(id);
        while let Some(node) = current {
            if node == ancestor {
                return true;
            }
            current = self.parent(node);
        }
        false
    }

    /// The children of `layer` in the order they draw, first lowest. Normally the tree order.
    /// With Depth sort on, the Stamps take the places the Stamps held, ordered by Anchor
    /// height with lower in front (ties keep tree order); Layers and other Objects stay put.
    pub fn ordered_children(&self, layer: Id) -> Vec<Id> {
        let mut children = self.children(Some(layer)).to_vec();
        if !matches!(self.node(layer), Some(Node::Layer(l)) if l.depth_sort) {
            return children;
        }
        let anchor_y = |id: Id| match self.node(id) {
            Some(Node::Object(Object {
                kind: ObjectKind::Stamp(stamp),
                ..
            })) => Some(stamp.position[1]),
            _ => None,
        };
        let slots: Vec<usize> = (0..children.len())
            .filter(|&i| anchor_y(children[i]).is_some())
            .collect();
        let mut stamps: Vec<Id> = slots.iter().map(|&i| children[i]).collect();
        stamps.sort_by(|&a, &b| {
            let (a, b) = (anchor_y(a).unwrap_or(0.0), anchor_y(b).unwrap_or(0.0));
            a.total_cmp(&b)
        });
        for (slot, id) in slots.into_iter().zip(stamps) {
            children[slot] = id;
        }
        children
    }

    /// The Objects that draw, first lowest: the tree in Stacking order, skipping hidden
    /// Layers, except that a Layer's Lakes draw above all its other content, so water always
    /// lies over the land around it.
    pub fn draw_order(&self) -> Vec<Id> {
        let mut out = Vec::new();
        for &layer in &self.root {
            self.collect_draw_order(layer, &mut out);
        }
        out
    }

    fn collect_draw_order(&self, layer: Id, out: &mut Vec<Id>) {
        if !matches!(self.node(layer), Some(Node::Layer(l)) if l.visible) {
            return;
        }
        let mut lakes = Vec::new();
        for child in self.ordered_children(layer) {
            match self.node(child) {
                Some(Node::Layer(_)) => self.collect_draw_order(child, out),
                Some(Node::Object(object)) if matches!(object.kind, ObjectKind::Lake(_)) => {
                    lakes.push(child);
                }
                Some(Node::Object(_)) => out.push(child),
                None => {}
            }
        }
        out.extend(lakes);
    }

    /// `id` is locked itself or through a Layer above it.
    pub fn is_locked(&self, id: Id) -> bool {
        std::iter::successors(Some(id), |&n| self.parent(n))
            .any(|n| self.node(n).is_some_and(Node::locked))
    }

    /// Checks the tree's structural rules. A Map built only through Ops always passes.
    pub fn validate(&self) -> Result<(), InvariantError> {
        let mut seen = HashSet::new();
        let mut stack: Vec<(Option<Id>, Id)> = self.root.iter().map(|&id| (None, id)).collect();
        for &id in &self.root {
            match self.entries.get(&id) {
                Some(entry) if !entry.node.is_layer() => {
                    return Err(InvariantError::ObjectAtRoot(id));
                }
                _ => {}
            }
        }
        while let Some((parent, id)) = stack.pop() {
            if !seen.insert(id) {
                return Err(InvariantError::DuplicateId(id));
            }
            let entry = self.entries.get(&id).ok_or(InvariantError::Missing(id))?;
            if entry.parent != parent {
                return Err(InvariantError::WrongParent(id));
            }
            if !entry.node.is_layer() && !entry.children.is_empty() {
                return Err(InvariantError::ObjectWithChildren(id));
            }
            stack.extend(entry.children.iter().map(|&child| (Some(id), child)));
        }
        if seen.len() != self.entries.len() {
            let orphan = self.entries.keys().find(|id| !seen.contains(id));
            return Err(InvariantError::Orphan(*orphan.expect("an unseen entry")));
        }
        Ok(())
    }

    /// Checks that `subtree` could be inserted: fresh, unique ids and no Object with children.
    pub(crate) fn check_insertable(&self, subtree: &Subtree) -> Result<(), InvariantError> {
        let mut ids = HashSet::new();
        let mut stack = vec![subtree];
        while let Some(tree) = stack.pop() {
            if self.contains(tree.id) || !ids.insert(tree.id) {
                return Err(InvariantError::DuplicateId(tree.id));
            }
            if !tree.node.is_layer() && !tree.children.is_empty() {
                return Err(InvariantError::ObjectWithChildren(tree.id));
            }
            stack.extend(&tree.children);
        }
        Ok(())
    }

    /// Adds entries for `subtree` under `parent` without touching the parent's child list.
    fn insert_subtree(
        &mut self,
        parent: Option<Id>,
        subtree: Subtree,
    ) -> Result<(), InvariantError> {
        self.check_insertable(&subtree)?;
        self.insert_entries(parent, subtree);
        Ok(())
    }

    /// Adds entries for an already-checked subtree and stamps them with the current revision.
    pub(crate) fn insert_entries(&mut self, parent: Option<Id>, subtree: Subtree) {
        let Subtree { id, node, children } = subtree;
        let child_ids = children.iter().map(|child| child.id).collect();
        self.entries.insert(
            id,
            Entry {
                node,
                parent,
                children: child_ids,
                version: self.revision,
            },
        );
        for child in children {
            self.insert_entries(Some(id), child);
        }
    }

    /// Removes the entries of `id` and its descendants, returning them as a Subtree.
    pub(crate) fn remove_entries(&mut self, id: Id) -> Subtree {
        let entry = self.entries.remove(&id).expect("a known id");
        Subtree {
            id,
            node: entry.node,
            children: entry
                .children
                .into_iter()
                .map(|child| self.remove_entries(child))
                .collect(),
        }
    }

    pub(crate) fn children_mut(&mut self, parent: Option<Id>) -> &mut Vec<Id> {
        match parent {
            None => &mut self.root,
            Some(id) => &mut self.entries.get_mut(&id).expect("a known id").children,
        }
    }

    pub(crate) fn touch(&mut self, id: Option<Id>) {
        if let Some(entry) = id.and_then(|id| self.entries.get_mut(&id)) {
            entry.version = self.revision;
        }
    }
}

/// A broken structural rule of the Map tree.
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum InvariantError {
    #[error("id {0} appears more than once")]
    DuplicateId(Id),
    #[error("Object {0} is at the top level; only Layers can be")]
    ObjectAtRoot(Id),
    #[error("Object {0} has children; only Layers can")]
    ObjectWithChildren(Id),
    #[error("node {0} is listed as a child but missing")]
    Missing(Id),
    #[error("node {0} records the wrong parent")]
    WrongParent(Id),
    #[error("node {0} is not reachable from the top level")]
    Orphan(Id),
}
