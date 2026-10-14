//! Commands: the user-level edits the UI and agents issue (ADR 0004).
//!
//! A Command checks the Map's user-level rules (Lock, references, value ranges), compiles to
//! primitive Ops, and applies them all or none.

use std::collections::HashSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::color::Color;
use crate::geometry::{Affine, MultiPolygon};
use crate::id::Id;
use crate::map::{Map, MapDefaults, Subtree};
use crate::node::{Label, Lake, Landmass, Layer, Node, Object, ObjectKind, Stamp};
use crate::op::{Op, OpError, Property};
use crate::roughness::Roughness;

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
        stamp: Stamp,
    },
    /// Replaces every field of a Stamp as one edit: Asset, placement, tint, fill or opacity.
    SetStamp {
        id: Id,
        stamp: Stamp,
    },
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
    #[error("opacity must be from 0 to 1, not {0}")]
    InvalidOpacity(f32),
    #[error("{0} is not a Landmass or Lake")]
    NoOutline(Id),
    #[error("the outline of {id} is invalid: {problem}")]
    InvalidGeometry { id: Id, problem: String },
    #[error("the transform {0:?} is not finite or collapses shapes")]
    InvalidTransform([f64; 6]),
    #[error("Roughness needs an amount of 0 or more and a scale above 0, not {amount} and {scale}")]
    InvalidRoughness { amount: f64, scale: f64 },
    #[error("{0} is not a Label")]
    NotALabel(Id),
    #[error("{0} is not a Stamp")]
    NotAStamp(Id),
    #[error("invalid Stamp: {0}")]
    InvalidStamp(&'static str),
    #[error("invalid Label: {0}")]
    InvalidLabel(&'static str),
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
            Command::Delete { .. } => "Delete",
            Command::CreateLandmass { .. } => "Create Landmass",
            Command::CreateLake { .. } => "Create Lake",
            Command::Translate { .. } => "Move",
            Command::Transform { .. } => "Transform",
            Command::SetFill { .. } => "Change Fill",
            Command::SetRoughness { .. } => "Change Roughness",
            Command::SetDefaultRoughness { .. } => "Change Default Roughness",
            Command::CreateLabel { .. } => "Add Label",
            Command::SetLabel { .. } => "Edit Label",
            Command::CreateStamp { .. } => "Add Stamp",
            Command::SetStamp { .. } => "Edit Stamp",
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
                        node: Node::Object(Object::new(ObjectKind::Stamp(stamp.clone()))),
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
                Ok(vec![set(*id, Property::Stamp(Box::new(stamp.clone())))])
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

fn valid_roughness(roughness: &Roughness) -> Result<(), CommandError> {
    let Roughness { amount, scale, .. } = *roughness;
    if amount.is_finite() && amount >= 0.0 && scale.is_finite() && scale > 0.0 {
        Ok(())
    } else {
        Err(CommandError::InvalidRoughness { amount, scale })
    }
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
