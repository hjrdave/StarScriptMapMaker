//! Commands: the user-level edits the UI and agents issue (ADR 0004).
//!
//! A Command checks the Map's user-level rules (Lock, references, value ranges), compiles to
//! primitive Ops, and applies them all or none.

use std::collections::HashSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::color::Color;
use crate::geometry::{Affine, MultiPolygon, NodeRef, Point};
use crate::id::Id;
use crate::land;
use crate::map::{Map, MapDefaults, Subtree};
use crate::node::{Label, Lake, Landmass, Layer, Node, Object, ObjectKind, Stamp};
use crate::op::{Op, OpError, Property};
use crate::roughness::Roughness;
use crate::shore::{ShoreKind, ShoreStyle};

/// One edit to a Map. Commands name every id they touch or create, so replaying one gives
/// the same Map every time.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// Creates an empty Layer with the given id, inside `parent` (`None` for the top level).
    /// `index` counts from the bottom of the Stacking order; omitted, the Layer goes on top.
    CreateLayer {
        id: Id,
        parent: Option<Id>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        name: String,
    },
    RenameLayer {
        id: Id,
        name: String,
    },
    /// Moves a Layer or Object to `index` among `parent`'s children, counted after it leaves
    /// its old place.
    Move {
        id: Id,
        parent: Option<Id>,
        index: usize,
    },
    SetVisible {
        id: Id,
        visible: bool,
    },
    /// Locks or unlocks a Layer or Object. Unlocking is refused while a Layer above it is locked.
    SetLocked {
        id: Id,
        locked: bool,
    },
    SetOpacity {
        id: Id,
        opacity: f32,
    },
    /// Switches Depth sort on or off for a Layer: its direct Stamps then draw by Anchor
    /// height (ADR 0003).
    SetDepthSort {
        id: Id,
        depth_sort: bool,
    },
    /// Deletes Layers and Objects with everything inside them, and clears Clips that pointed
    /// at anything deleted.
    Delete {
        ids: Vec<Id>,
    },
    /// Creates a Landmass with the given id and outline inside Layer `parent`. `index` counts
    /// from the bottom of the Stacking order; omitted, the Landmass goes on top. Without a
    /// `fill`, it takes the default land colour.
    CreateLandmass {
        id: Id,
        parent: Id,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        geometry: MultiPolygon,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fill: Option<Color>,
    },
    /// Creates a Lake, as `create_landmass` does a Landmass.
    CreateLake {
        id: Id,
        parent: Id,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        geometry: MultiPolygon,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fill: Option<Color>,
    },
    /// Moves the outlines of Landmasses and Lakes by `offset` Canvas units.
    Translate {
        ids: Vec<Id>,
        offset: [f64; 2],
    },
    /// Applies an affine transform to the outlines of Landmasses and Lakes, as dragging a
    /// resize handle does. It must not collapse them to a line or a point.
    Transform {
        ids: Vec<Id>,
        transform: Affine,
    },
    /// Adds `region` to the outline of a Landmass or Lake, as a land brush or lasso stroke
    /// does. Parts it joins become one part; a hole it covers fills in.
    AddRegion {
        id: Id,
        region: MultiPolygon,
    },
    /// Cuts `region` out of the outlines of Landmasses and Lakes. One it splits keeps every
    /// part, one it leaves with nothing is deleted (clearing Clips to it), and one it misses
    /// is left alone.
    SubtractRegion {
        ids: Vec<Id>,
        region: MultiPolygon,
    },
    /// Moves nodes of a Landmass or Lake by `offset` Canvas units. If that makes the outline
    /// cross itself, it is repaired.
    MoveNodes {
        id: Id,
        nodes: Vec<NodeRef>,
        offset: [f64; 2],
    },
    /// Adds a node at `point` on the edge that runs from `edge` to the next point of its ring.
    InsertNode {
        id: Id,
        edge: NodeRef,
        point: Point,
    },
    /// Deletes nodes. A ring cannot be left with fewer than three points.
    DeleteNodes {
        id: Id,
        nodes: Vec<NodeRef>,
    },
    /// Merges Landmasses into one outline. The result keeps the topmost one's id, place and
    /// properties; the others are removed, and Layers clipped to them clip to the result.
    Union {
        ids: Vec<Id>,
    },
    /// Cuts the topmost of the Landmasses out of the others and removes it. A Landmass left
    /// with nothing is deleted, clearing Clips to it.
    Subtract {
        ids: Vec<Id>,
    },
    /// Turns each part of a Landmass into a Landmass of its own, in the same Layer. The first
    /// part keeps `id`; `new_ids` name the rest, in order, so there must be one fewer than
    /// there are parts.
    SplitApart {
        id: Id,
        new_ids: Vec<Id>,
    },
    /// Sets the fill colour of Landmasses and Lakes.
    SetFill {
        ids: Vec<Id>,
        fill: Color,
    },
    /// Sets the Roughness of Landmasses and Lakes, or with `None` returns them to the Map-wide
    /// default.
    SetRoughness {
        ids: Vec<Id>,
        roughness: Option<Roughness>,
    },
    /// Sets the Roughness of every Landmass and Lake without its own.
    SetDefaultRoughness {
        roughness: Roughness,
    },
    /// Sets the Shore style of Landmasses and Lakes, or with `None` returns them to the
    /// Map-wide default for their kind.
    SetShore {
        ids: Vec<Id>,
        shore: Option<ShoreStyle>,
    },
    /// Sets the Map-wide Shore style of every Landmass, or every Lake, without its own.
    SetDefaultShore {
        kind: ShoreKind,
        shore: ShoreStyle,
    },
    /// Places a Label with the given id inside Layer `parent`. `index` counts from the bottom
    /// of the Stacking order; omitted, the Label goes on top.
    CreateLabel {
        id: Id,
        parent: Id,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        label: Box<Label>,
    },
    /// Replaces every field of a Label, as one edit: text, placement, typography or colour.
    SetLabel {
        id: Id,
        label: Box<Label>,
    },
    /// Places a Stamp with the given id inside Layer `parent`. `index` counts from the bottom
    /// of the Stacking order; omitted, the Stamp goes on top.
    CreateStamp {
        id: Id,
        parent: Id,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        stamp: Box<Stamp>,
    },
    /// Replaces every field of a Stamp, as one edit: its Asset, placement, colours or opacity.
    SetStamp {
        id: Id,
        stamp: Box<Stamp>,
    },
    /// Places many Stamps inside Layer `parent` on top, in the order given, as one edit. A
    /// brush resolves its randomness first, so this names every Stamp it creates.
    PlaceStamps {
        parent: Id,
        stamps: Vec<PlacedStamp>,
    },
}

/// One Stamp of a [`Command::PlaceStamps`], with the id it gets.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PlacedStamp {
    pub id: Id,
    pub stamp: Stamp,
}

#[derive(Clone, PartialEq, Debug, Error)]
pub enum CommandError {
    #[error("nothing has id {0}")]
    NotFound(Id),
    #[error("{0} is not a Layer")]
    NotALayer(Id),
    #[error("id {0} is already in use")]
    IdInUse(Id),
    #[error("{0} is locked")]
    Locked(Id),
    #[error("{target} is inside locked Layer {layer}")]
    InLockedLayer { target: Id, layer: Id },
    #[error("{target} contains locked {locked}")]
    ContainsLocked { target: Id, locked: Id },
    #[error("{0} is in a Layer with Depth sort on, which orders its Stamps")]
    DepthSorted(Id),
    #[error("opacity must be from 0 to 1, not {0}")]
    InvalidOpacity(f32),
    #[error("{0} is not a Landmass or Lake")]
    NoOutline(Id),
    #[error("the outline of {id} is invalid: {problem}")]
    InvalidGeometry { id: Id, problem: String },
    #[error("cannot edit the nodes of {id}: {problem}")]
    NodeEdit { id: Id, problem: String },
    #[error("{0} is not a Landmass")]
    NotALandmass(Id),
    #[error("{command} needs at least two Landmasses")]
    NeedsTwoLandmasses { command: &'static str },
    #[error("{id} has {parts} parts, so Split Apart needs {} new ids, not {got}", .parts - 1)]
    SplitIds { id: Id, parts: usize, got: usize },
    #[error("{0} is already one part")]
    NothingToSplit(Id),
    #[error("the Shore style is invalid: {0}")]
    InvalidShore(String),
    #[error("the transform {0:?} is not finite or collapses shapes")]
    InvalidTransform([f64; 6]),
    #[error("Roughness needs an amount of 0 or more and a scale above 0, not {amount} and {scale}")]
    InvalidRoughness { amount: f64, scale: f64 },
    #[error("{0} is not a Label")]
    NotALabel(Id),
    #[error("invalid Label: {0}")]
    InvalidLabel(&'static str),
    #[error("{0} is not a Stamp")]
    NotAStamp(Id),
    #[error("invalid Stamp: {0}")]
    InvalidStamp(&'static str),
    #[error(transparent)]
    Op(#[from] OpError),
}

impl Command {
    /// A short name for menus and the undo history.
    pub fn label(&self) -> &'static str {
        match self {
            Command::CreateLayer { .. } => "Create Layer",
            Command::RenameLayer { .. } => "Rename Layer",
            Command::Move { .. } => "Move",
            Command::SetVisible { .. } => "Change Visibility",
            Command::SetLocked { .. } => "Change Lock",
            Command::SetOpacity { .. } => "Change Opacity",
            Command::SetDepthSort { .. } => "Change Depth Sort",
            Command::Delete { .. } => "Delete",
            Command::CreateLandmass { .. } => "Create Landmass",
            Command::CreateLake { .. } => "Create Lake",
            Command::Translate { .. } => "Move",
            Command::Transform { .. } => "Transform",
            Command::AddRegion { .. } => "Add Region",
            Command::SubtractRegion { .. } => "Subtract Region",
            Command::MoveNodes { .. } => "Move Nodes",
            Command::InsertNode { .. } => "Add Node",
            Command::DeleteNodes { .. } => "Delete Nodes",
            Command::Union { .. } => "Union",
            Command::Subtract { .. } => "Subtract",
            Command::SplitApart { .. } => "Split Apart",
            Command::SetFill { .. } => "Change Fill",
            Command::SetRoughness { .. } => "Change Roughness",
            Command::SetDefaultRoughness { .. } => "Change Default Roughness",
            Command::SetShore { .. } => "Change Shore Style",
            Command::SetDefaultShore { .. } => "Change Default Shore Style",
            Command::CreateLabel { .. } => "Add Label",
            Command::SetLabel { .. } => "Edit Label",
            Command::CreateStamp { .. } => "Add Stamp",
            Command::SetStamp { .. } => "Edit Stamp",
            Command::PlaceStamps { .. } => "Place Stamps",
        }
    }

    /// Checks the Command against `map` and returns the Ops it applies, in order.
    pub fn compile(&self, map: &Map) -> Result<Vec<Op>, CommandError> {
        match self {
            Command::CreateLayer {
                id,
                parent,
                index,
                name,
            } => {
                if map.contains(*id) {
                    return Err(CommandError::IdInUse(*id));
                }
                if let Some(parent) = *parent {
                    layer(map, parent)?;
                    unlocked(map, parent)?;
                }
                Ok(vec![Op::Insert {
                    parent: *parent,
                    index: index.unwrap_or(map.children(*parent).len()),
                    subtree: Subtree {
                        id: *id,
                        node: Node::Layer(Layer::new(name.clone())),
                        children: Vec::new(),
                    },
                }])
            }
            Command::RenameLayer { id, name } => {
                layer(map, *id)?;
                unlocked(map, *id)?;
                Ok(vec![set(*id, Property::Name(name.clone()))])
            }
            Command::Move { id, parent, index } => {
                unlocked_branch(map, *id)?;
                // A Depth-sorted Layer orders its own Stamps, so they can't be reordered there.
                if map.parent(*id) == *parent
                    && let Some(layer) = *parent
                    && matches!(map.node(layer), Some(Node::Layer(l)) if l.depth_sort)
                    && matches!(
                        map.node(*id),
                        Some(Node::Object(Object {
                            kind: ObjectKind::Stamp(_),
                            ..
                        }))
                    )
                {
                    return Err(CommandError::DepthSorted(*id));
                }
                if let Some(parent) = *parent {
                    layer(map, parent)?;
                    unlocked(map, parent)?;
                }
                Ok(vec![Op::Move {
                    id: *id,
                    parent: *parent,
                    index: *index,
                }])
            }
            Command::SetVisible { id, visible } => {
                layer(map, *id)?;
                unlocked(map, *id)?;
                Ok(vec![set(*id, Property::Visible(*visible))])
            }
            Command::SetLocked { id, locked } => {
                existing(map, *id)?;
                // The node's own Lock never blocks changing it, but a locked Layer above does.
                if let Some(layer) = locked_ancestor(map, *id) {
                    return Err(CommandError::InLockedLayer { target: *id, layer });
                }
                Ok(vec![set(*id, Property::Locked(*locked))])
            }
            Command::SetOpacity { id, opacity } => {
                layer(map, *id)?;
                unlocked(map, *id)?;
                if !(0.0..=1.0).contains(opacity) {
                    return Err(CommandError::InvalidOpacity(*opacity));
                }
                Ok(vec![set(*id, Property::Opacity(*opacity))])
            }
            Command::SetDepthSort { id, depth_sort } => {
                layer(map, *id)?;
                unlocked(map, *id)?;
                Ok(vec![set(*id, Property::DepthSort(*depth_sort))])
            }
            Command::Delete { ids } => delete(map, ids),
            Command::CreateLandmass {
                id,
                parent,
                index,
                geometry,
                fill,
            } => {
                let landmass = Landmass {
                    fill: fill.unwrap_or(Landmass::DEFAULT_FILL),
                    ..Landmass::new(geometry.clone())
                };
                create_object(map, *id, *parent, *index, ObjectKind::Landmass(landmass))
            }
            Command::CreateLake {
                id,
                parent,
                index,
                geometry,
                fill,
            } => {
                let lake = Lake {
                    fill: fill.unwrap_or(Lake::DEFAULT_FILL),
                    ..Lake::new(geometry.clone())
                };
                create_object(map, *id, *parent, *index, ObjectKind::Lake(lake))
            }
            Command::Translate { ids, offset } => transform(map, ids, &Affine::translate(*offset)),
            Command::Transform { ids, transform: t } => transform(map, ids, t),
            Command::AddRegion { id, region } => {
                let [id] = outlined(map, &[*id])?[..] else {
                    unreachable!("one id in, one id out")
                };
                valid_region(id, region)?;
                let geometry = land::union(outline(map, id), region);
                if let Some(problem) = geometry.problem() {
                    return Err(CommandError::InvalidGeometry { id, problem });
                }
                Ok(vec![Op::ReplaceGeometry { id, geometry }])
            }
            Command::SubtractRegion { ids, region } => {
                let ids = outlined(map, ids)?;
                if let Some(&first) = ids.first() {
                    valid_region(first, region)?;
                }
                let mut ops = Vec::new();
                let mut emptied = Vec::new();
                for id in ids {
                    let current = outline(map, id);
                    if !land::intersects(current, region) {
                        continue;
                    }
                    let geometry = land::difference(current, region);
                    if geometry.is_empty() {
                        emptied.push(id);
                    } else {
                        ops.push(Op::ReplaceGeometry { id, geometry });
                    }
                }
                if !emptied.is_empty() {
                    ops.extend(delete(map, &emptied)?);
                }
                Ok(ops)
            }
            Command::MoveNodes { id, nodes, offset } => node_edit(map, *id, |outline| {
                land::move_nodes(outline, nodes, *offset)
            }),
            Command::InsertNode { id, edge, point } => node_edit(map, *id, |outline| {
                land::insert_node(outline, *edge, *point)
            }),
            Command::DeleteNodes { id, nodes } => {
                node_edit(map, *id, |outline| land::delete_nodes(outline, nodes))
            }
            Command::Union { ids } => union_landmasses(map, ids),
            Command::Subtract { ids } => subtract_landmasses(map, ids),
            Command::SplitApart { id, new_ids } => split_apart(map, *id, new_ids),
            Command::SetFill { ids, fill } => {
                let ids = outlined(map, ids)?;
                Ok(ids
                    .into_iter()
                    .map(|id| set(id, Property::Fill(*fill)))
                    .collect())
            }
            Command::SetRoughness { ids, roughness } => {
                if let Some(roughness) = roughness {
                    valid_roughness(roughness)?;
                }
                let ids = outlined(map, ids)?;
                Ok(ids
                    .into_iter()
                    .map(|id| set(id, Property::Roughness(roughness.clone())))
                    .collect())
            }
            Command::SetShore { ids, shore } => {
                if let Some(problem) = shore.as_ref().and_then(ShoreStyle::problem) {
                    return Err(CommandError::InvalidShore(problem));
                }
                let ids = outlined(map, ids)?;
                Ok(ids
                    .into_iter()
                    .map(|id| set(id, Property::Shore(shore.clone())))
                    .collect())
            }
            Command::SetDefaultShore { kind, shore } => {
                if let Some(problem) = shore.problem() {
                    return Err(CommandError::InvalidShore(problem));
                }
                let mut defaults = map.defaults().clone();
                match kind {
                    ShoreKind::Landmass => defaults.shore = shore.clone(),
                    ShoreKind::Lake => defaults.lake_shore = shore.clone(),
                }
                Ok(vec![Op::SetDefaults { defaults }])
            }
            Command::SetDefaultRoughness { roughness } => {
                valid_roughness(roughness)?;
                Ok(vec![Op::SetDefaults {
                    defaults: MapDefaults {
                        roughness: roughness.clone(),
                        ..map.defaults().clone()
                    },
                }])
            }
            Command::CreateLabel {
                id,
                parent,
                index,
                label,
            } => {
                if map.contains(*id) {
                    return Err(CommandError::IdInUse(*id));
                }
                layer(map, *parent)?;
                unlocked(map, *parent)?;
                check_label(label)?;
                Ok(vec![Op::Insert {
                    parent: Some(*parent),
                    index: index.unwrap_or(map.children(Some(*parent)).len()),
                    subtree: Subtree {
                        id: *id,
                        node: Node::Object(Object::new(ObjectKind::Label(label.clone()))),
                        children: Vec::new(),
                    },
                }])
            }
            Command::SetLabel { id, label } => {
                match existing(map, *id)? {
                    Node::Object(Object {
                        kind: ObjectKind::Label(_),
                        ..
                    }) => {}
                    _ => return Err(CommandError::NotALabel(*id)),
                }
                unlocked(map, *id)?;
                check_label(label)?;
                Ok(vec![set(*id, Property::Label(label.clone()))])
            }
            Command::CreateStamp {
                id,
                parent,
                index,
                stamp,
            } => {
                if map.contains(*id) {
                    return Err(CommandError::IdInUse(*id));
                }
                layer(map, *parent)?;
                unlocked(map, *parent)?;
                check_stamp(stamp)?;
                Ok(vec![Op::Insert {
                    parent: Some(*parent),
                    index: index.unwrap_or(map.children(Some(*parent)).len()),
                    subtree: Subtree {
                        id: *id,
                        node: Node::Object(Object::new(ObjectKind::Stamp((**stamp).clone()))),
                        children: Vec::new(),
                    },
                }])
            }
            Command::SetStamp { id, stamp } => {
                match existing(map, *id)? {
                    Node::Object(Object {
                        kind: ObjectKind::Stamp(_),
                        ..
                    }) => {}
                    _ => return Err(CommandError::NotAStamp(*id)),
                }
                unlocked(map, *id)?;
                check_stamp(stamp)?;
                Ok(vec![set(*id, Property::Stamp(stamp.clone()))])
            }
            Command::PlaceStamps { parent, stamps } => {
                layer(map, *parent)?;
                unlocked(map, *parent)?;
                let start = map.children(Some(*parent)).len();
                let mut seen = HashSet::new();
                let mut ops = Vec::with_capacity(stamps.len());
                for (offset, placed) in stamps.iter().enumerate() {
                    if map.contains(placed.id) || !seen.insert(placed.id) {
                        return Err(CommandError::IdInUse(placed.id));
                    }
                    check_stamp(&placed.stamp)?;
                    ops.push(Op::Insert {
                        parent: Some(*parent),
                        index: start + offset,
                        subtree: Subtree {
                            id: placed.id,
                            node: Node::Object(Object::new(ObjectKind::Stamp(
                                placed.stamp.clone(),
                            ))),
                            children: Vec::new(),
                        },
                    });
                }
                Ok(ops)
            }
        }
    }
}

impl Map {
    /// Applies a Command all or nothing and returns the inverses of its Ops, in the order they
    /// were applied.
    pub fn execute(&mut self, command: &Command) -> Result<Vec<Op>, CommandError> {
        self.execute_recording(command, None)
    }

    /// Like `execute`, and also appends the Ops it applied to `applied`. A failed Command
    /// leaves `applied` as it was.
    pub(crate) fn execute_recording(
        &mut self,
        command: &Command,
        mut applied: Option<&mut Vec<Op>>,
    ) -> Result<Vec<Op>, CommandError> {
        let ops = command.compile(self)?;
        let recorded = applied.as_ref().map_or(0, |applied| applied.len());
        let mut inverses = Vec::with_capacity(ops.len());
        for op in ops {
            if let Some(applied) = applied.as_deref_mut() {
                applied.push(op.clone());
            }
            match self.apply(op) {
                Ok(inverse) => inverses.push(inverse),
                Err(err) => {
                    self.revert(inverses);
                    if let Some(applied) = applied {
                        applied.truncate(recorded);
                    }
                    return Err(err.into());
                }
            }
        }
        Ok(inverses)
    }

    /// Undoes Ops from their inverses (as `execute` returns them) and returns the Ops that
    /// redo them, in the same form.
    pub fn revert(&mut self, inverses: Vec<Op>) -> Vec<Op> {
        inverses
            .into_iter()
            .rev()
            .map(|inverse| self.apply(inverse).expect("an inverse always applies"))
            .collect()
    }
}

fn delete(map: &Map, ids: &[Id]) -> Result<Vec<Op>, CommandError> {
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

fn create_object(
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
    let geometry = kind.geometry().expect("Landmasses and Lakes have outlines");
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

fn transform(map: &Map, ids: &[Id], transform: &Affine) -> Result<Vec<Op>, CommandError> {
    if !transform.is_invertible() {
        return Err(CommandError::InvalidTransform(transform.0));
    }
    let ids = outlined(map, ids)?;
    let mut ops = Vec::with_capacity(ids.len());
    for id in ids {
        let geometry = outline(map, id).transformed(transform);
        // Huge factors can overflow to infinity even when the transform itself is finite.
        if let Some(problem) = geometry.problem() {
            return Err(CommandError::InvalidGeometry { id, problem });
        }
        ops.push(Op::ReplaceGeometry { id, geometry });
    }
    Ok(ops)
}

/// The distinct, unlocked Landmasses among `ids`, topmost first.
fn landmasses_topmost_first(
    map: &Map,
    ids: &[Id],
    command: &'static str,
) -> Result<Vec<Id>, CommandError> {
    let ids = outlined(map, ids)?;
    for &id in &ids {
        if !matches!(map.node(id), Some(Node::Object(o)) if matches!(o.kind, ObjectKind::Landmass(_)))
        {
            return Err(CommandError::NotALandmass(id));
        }
    }
    if ids.len() < 2 {
        return Err(CommandError::NeedsTwoLandmasses { command });
    }
    let order = map.draw_order();
    let rank = |id: Id| order.iter().position(|&o| o == id);
    let mut ids = ids;
    ids.sort_by_key(|&id| std::cmp::Reverse(rank(id)));
    Ok(ids)
}

fn union_landmasses(map: &Map, ids: &[Id]) -> Result<Vec<Op>, CommandError> {
    let ids = landmasses_topmost_first(map, ids, "Union")?;
    let (&keeper, others) = ids.split_first().expect("two or more");
    let mut geometry = outline(map, keeper).clone();
    for &other in others {
        geometry = land::union(&geometry, outline(map, other));
    }
    if let Some(problem) = geometry.problem() {
        return Err(CommandError::InvalidGeometry {
            id: keeper,
            problem,
        });
    }
    let mut ops = vec![Op::ReplaceGeometry {
        id: keeper,
        geometry,
    }];
    let mut clipped: Vec<Id> = map
        .ids()
        .filter(|&id| matches!(map.node(id), Some(Node::Layer(l)) if l.clip.is_some_and(|c| others.contains(&c))))
        .collect();
    clipped.sort();
    for id in clipped {
        unlocked(map, id)?;
        ops.push(set(id, Property::Clip(Some(keeper))));
    }
    ops.extend(others.iter().map(|&id| Op::Remove { id }));
    Ok(ops)
}

fn subtract_landmasses(map: &Map, ids: &[Id]) -> Result<Vec<Op>, CommandError> {
    let ids = landmasses_topmost_first(map, ids, "Subtract")?;
    let (&cutter, targets) = ids.split_first().expect("two or more");
    let region = outline(map, cutter).clone();
    let mut ops = Vec::new();
    let mut gone = vec![cutter];
    for &id in targets {
        let current = outline(map, id);
        if !land::intersects(current, &region) {
            continue;
        }
        let geometry = land::difference(current, &region);
        if geometry.is_empty() {
            gone.push(id);
        } else {
            ops.push(Op::ReplaceGeometry { id, geometry });
        }
    }
    ops.extend(delete(map, &gone)?);
    Ok(ops)
}

fn split_apart(map: &Map, id: Id, new_ids: &[Id]) -> Result<Vec<Op>, CommandError> {
    let [id] = outlined(map, &[id])?[..] else {
        unreachable!("one id in, one id out")
    };
    let Some(Node::Object(object)) = map.node(id) else {
        unreachable!("checked")
    };
    let ObjectKind::Landmass(landmass) = &object.kind else {
        return Err(CommandError::NotALandmass(id));
    };
    let parts = &landmass.geometry.0;
    if parts.len() < 2 {
        return Err(CommandError::NothingToSplit(id));
    }
    if new_ids.len() != parts.len() - 1 {
        return Err(CommandError::SplitIds {
            id,
            parts: parts.len(),
            got: new_ids.len(),
        });
    }
    for (i, &new) in new_ids.iter().enumerate() {
        if map.contains(new) || new == id || new_ids[..i].contains(&new) {
            return Err(CommandError::IdInUse(new));
        }
    }
    let parent = map.parent(id).expect("Objects live in Layers");
    let position = map
        .children(Some(parent))
        .iter()
        .position(|&c| c == id)
        .expect("a node is listed by its parent");
    let mut ops = vec![Op::ReplaceGeometry {
        id,
        geometry: MultiPolygon(vec![parts[0].clone()]),
    }];
    for (k, (&new, part)) in new_ids.iter().zip(&parts[1..]).enumerate() {
        let mut copy = object.clone();
        if let ObjectKind::Landmass(l) = &mut copy.kind {
            l.geometry = MultiPolygon(vec![part.clone()]);
        }
        ops.push(Op::Insert {
            parent: Some(parent),
            index: position + 1 + k,
            subtree: Subtree {
                id: new,
                node: Node::Object(copy),
                children: Vec::new(),
            },
        });
    }
    Ok(ops)
}

/// Replaces the outline of one Landmass or Lake with the result of a node edit.
fn node_edit(
    map: &Map,
    id: Id,
    edit: impl FnOnce(&MultiPolygon) -> Result<MultiPolygon, land::NodeEditError>,
) -> Result<Vec<Op>, CommandError> {
    let [id] = outlined(map, &[id])?[..] else {
        unreachable!("one id in, one id out")
    };
    let geometry = edit(outline(map, id)).map_err(|e| CommandError::NodeEdit {
        id,
        problem: e.to_string(),
    })?;
    if let Some(problem) = geometry.problem() {
        return Err(CommandError::InvalidGeometry { id, problem });
    }
    Ok(vec![Op::ReplaceGeometry { id, geometry }])
}

/// Checks that every id is an unlocked Landmass or Lake, and drops repeats.
fn outlined(map: &Map, ids: &[Id]) -> Result<Vec<Id>, CommandError> {
    let mut seen = HashSet::new();
    let mut unique = Vec::with_capacity(ids.len());
    for &id in ids {
        match existing(map, id)? {
            Node::Object(object) if object.kind.geometry().is_some() => {}
            _ => return Err(CommandError::NoOutline(id)),
        }
        unlocked(map, id)?;
        if seen.insert(id) {
            unique.push(id);
        }
    }
    Ok(unique)
}

/// The outline of an id `outlined` accepted.
fn outline(map: &Map, id: Id) -> &MultiPolygon {
    match map.node(id) {
        Some(Node::Object(object)) => object.kind.geometry().expect("checked"),
        _ => unreachable!("checked"),
    }
}

/// A region to add or subtract must itself be a valid outline. Errors name `id`, the Object
/// being edited.
fn valid_region(id: Id, region: &MultiPolygon) -> Result<(), CommandError> {
    match region.problem() {
        Some(problem) => Err(CommandError::InvalidGeometry {
            id,
            problem: format!("the region: {problem}"),
        }),
        None => Ok(()),
    }
}

fn valid_roughness(roughness: &Roughness) -> Result<(), CommandError> {
    let Roughness { amount, scale, .. } = *roughness;
    if amount.is_finite() && amount >= 0.0 && scale.is_finite() && scale > 0.0 {
        Ok(())
    } else {
        Err(CommandError::InvalidRoughness { amount, scale })
    }
}

/// Checks a Label's values are drawable.
fn check_label(label: &Label) -> Result<(), CommandError> {
    let invalid = |reason| Err(CommandError::InvalidLabel(reason));
    if label.text.is_empty() {
        return invalid("the text is empty");
    }
    if !label.position.iter().all(|v| v.is_finite()) || !label.rotation.is_finite() {
        return invalid("the position and rotation must be finite");
    }
    if !(label.size.is_finite() && label.size > 0.0) {
        return invalid("the size must be greater than 0");
    }
    if !label.letter_spacing.is_finite() {
        return invalid("the letter spacing must be finite");
    }
    if !(label.line_height.is_finite() && label.line_height > 0.0) {
        return invalid("the line height must be greater than 0");
    }
    if !(1..=1000).contains(&label.font.weight) {
        return invalid("the font weight must be from 1 to 1000");
    }
    if !(label.halo.width.is_finite() && label.halo.width >= 0.0) {
        return invalid("the halo width must be 0 or more");
    }
    if !(0.0..=1.0).contains(&label.opacity) {
        return Err(CommandError::InvalidOpacity(label.opacity));
    }
    Ok(())
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

fn set(id: Id, property: Property) -> Op {
    Op::SetProperty { id, property }
}

fn existing(map: &Map, id: Id) -> Result<&Node, CommandError> {
    map.node(id).ok_or(CommandError::NotFound(id))
}

fn layer(map: &Map, id: Id) -> Result<&Layer, CommandError> {
    match existing(map, id)? {
        Node::Layer(layer) => Ok(layer),
        Node::Object(_) => Err(CommandError::NotALayer(id)),
    }
}

/// The nearest locked Layer above `id`.
fn locked_ancestor(map: &Map, id: Id) -> Option<Id> {
    std::iter::successors(map.parent(id), |&p| map.parent(p))
        .find(|&ancestor| map.node(ancestor).is_some_and(Node::locked))
}

/// Fails if `id` or any Layer above it is locked.
fn unlocked(map: &Map, id: Id) -> Result<(), CommandError> {
    if existing(map, id)?.locked() {
        return Err(CommandError::Locked(id));
    }
    match locked_ancestor(map, id) {
        Some(layer) => Err(CommandError::InLockedLayer { target: id, layer }),
        None => Ok(()),
    }
}

/// Fails if `id`, a Layer above it, or anything inside it is locked.
fn unlocked_branch(map: &Map, id: Id) -> Result<(), CommandError> {
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

fn collect_branch(map: &Map, id: Id, out: &mut HashSet<Id>) {
    out.insert(id);
    for &child in map.children(Some(id)) {
        collect_branch(map, child, out);
    }
}
