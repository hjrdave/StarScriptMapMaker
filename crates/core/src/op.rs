//! Primitive Ops: the only way a Map changes (ADR 0004).
//!
//! Commands compile into Ops. Applying an Op returns its inverse, so undo is the reversed list
//! of inverses. Ops check only the tree's structural rules; Commands do the user-level
//! validation, such as Lock.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::color::Color;
use crate::geometry::MultiPolygon;
use crate::id::Id;
use crate::map::{InvariantError, Map, MapDefaults, Subtree};
use crate::node::{Label, Node, ObjectKind, Stamp};
use crate::roughness::Roughness;
use crate::shore::ShoreStyle;

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// Inserts a node and its descendants at `index` among `parent`'s children
    /// (`None` is the top level, which holds only Layers).
    Insert {
        parent: Option<Id>,
        index: usize,
        subtree: Subtree,
    },
    /// Removes a node and its descendants.
    Remove { id: Id },
    /// Sets one property of a node.
    SetProperty { id: Id, property: Property },
    /// Replaces the outline of a Landmass or Lake.
    ReplaceGeometry { id: Id, geometry: MultiPolygon },
    /// Moves a node to `index` among `parent`'s children, counted after it leaves its old place.
    Move {
        id: Id,
        parent: Option<Id>,
        index: usize,
    },
    /// Replaces the Map-wide defaults.
    SetDefaults { defaults: MapDefaults },
}

/// A settable property and its new value.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "name", content = "value", rename_all = "snake_case")]
pub enum Property {
    /// Layers only.
    Name(String),
    /// Layers only.
    Visible(bool),
    /// Layers and Objects.
    Locked(bool),
    /// Layers only.
    Opacity(f32),
    /// Layers only. Ops don't check the target, so that inverses always apply; Commands
    /// keep it pointing at a Landmass.
    Clip(Option<Id>),
    /// Layers only.
    DepthSort(bool),
    /// Landmasses and Lakes only.
    Fill(Color),
    /// Landmasses and Lakes only. `None` uses the Map-wide default.
    Roughness(Option<Roughness>),
    /// Landmasses and Lakes only.
    Shore(Option<ShoreStyle>),
    /// Label Objects only: every field of the Label at once.
    Label(Box<Label>),
    /// Stamp Objects only: every field of the Stamp at once.
    Stamp(Box<Stamp>),
}

#[derive(Clone, PartialEq, Debug, Error)]
pub enum OpError {
    #[error("no node has id {0}")]
    NotFound(Id),
    #[error("{0} is an Object; only Layers hold children")]
    NotALayer(Id),
    #[error("only Layers can be at the top level")]
    ObjectAtRoot,
    #[error("index {index} is past the end of {len} children")]
    IndexOutOfRange { index: usize, len: usize },
    #[error("cannot move {0} into itself or its own descendant")]
    MoveIntoSelf(Id),
    #[error("{0} has no property {1}")]
    NoSuchProperty(Id, &'static str),
    #[error("{0} has no geometry")]
    NoGeometry(Id),
    #[error(transparent)]
    Invariant(#[from] InvariantError),
}

impl Map {
    /// Applies `op` and returns its inverse. On error the Map is unchanged.
    pub fn apply(&mut self, op: Op) -> Result<Op, OpError> {
        self.check(&op)?;
        self.revision += 1;
        Ok(match op {
            Op::Insert {
                parent,
                index,
                subtree,
            } => {
                let id = subtree.id;
                self.insert_entries(parent, subtree);
                self.children_mut(parent).insert(index, id);
                self.touch(parent);
                Op::Remove { id }
            }
            Op::Remove { id } => {
                let parent = self.parent(id);
                let siblings = self.children_mut(parent);
                let index = position(siblings, id);
                siblings.remove(index);
                let subtree = self.remove_entries(id);
                self.touch(parent);
                Op::Insert {
                    parent,
                    index,
                    subtree,
                }
            }
            Op::SetProperty { id, property } => {
                let old = set_property(
                    &mut self.entries.get_mut(&id).expect("checked").node,
                    property,
                );
                self.touch(Some(id));
                Op::SetProperty { id, property: old }
            }
            Op::ReplaceGeometry { id, geometry } => {
                let Node::Object(object) = &mut self.entries.get_mut(&id).expect("checked").node
                else {
                    unreachable!("checked")
                };
                let slot = object.kind.geometry_mut().expect("checked");
                let old = std::mem::replace(slot, geometry);
                self.touch(Some(id));
                Op::ReplaceGeometry { id, geometry: old }
            }
            Op::Move { id, parent, index } => {
                let old_parent = self.parent(id);
                let siblings = self.children_mut(old_parent);
                let old_index = position(siblings, id);
                siblings.remove(old_index);
                self.children_mut(parent).insert(index, id);
                self.entries.get_mut(&id).expect("checked").parent = parent;
                self.touch(Some(id));
                self.touch(old_parent);
                self.touch(parent);
                Op::Move {
                    id,
                    parent: old_parent,
                    index: old_index,
                }
            }
            Op::SetDefaults { defaults } => Op::SetDefaults {
                defaults: std::mem::replace(&mut self.defaults, defaults),
            },
        })
    }

    /// Checks everything that could make `op` fail, so `apply` never changes the Map halfway.
    fn check(&self, op: &Op) -> Result<(), OpError> {
        match op {
            Op::Insert {
                parent,
                index,
                subtree,
            } => {
                self.check_destination(*parent, subtree.node.is_layer())?;
                check_index(*index, self.children(*parent).len())?;
                self.check_insertable(subtree)?;
            }
            Op::Remove { id } => {
                self.existing(*id)?;
            }
            Op::SetProperty { id, property } => {
                check_property(self.existing(*id)?, property)
                    .map_err(|name| OpError::NoSuchProperty(*id, name))?;
            }
            Op::ReplaceGeometry { id, .. } => match self.existing(*id)? {
                Node::Object(object) if object.kind.geometry().is_some() => {}
                _ => return Err(OpError::NoGeometry(*id)),
            },
            Op::Move { id, parent, index } => {
                let node = self.existing(*id)?;
                self.check_destination(*parent, node.is_layer())?;
                if let Some(parent) = *parent
                    && self.is_ancestor_or_self(*id, parent)
                {
                    return Err(OpError::MoveIntoSelf(*id));
                }
                let mut len = self.children(*parent).len();
                if self.parent(*id) == *parent {
                    len -= 1;
                }
                check_index(*index, len)?;
            }
            Op::SetDefaults { .. } => {}
        }
        Ok(())
    }

    fn existing(&self, id: Id) -> Result<&Node, OpError> {
        self.node(id).ok_or(OpError::NotFound(id))
    }

    fn check_destination(&self, parent: Option<Id>, is_layer: bool) -> Result<(), OpError> {
        match parent {
            None if !is_layer => Err(OpError::ObjectAtRoot),
            None => Ok(()),
            Some(parent) if self.existing(parent)?.is_layer() => Ok(()),
            Some(parent) => Err(OpError::NotALayer(parent)),
        }
    }
}

fn position(siblings: &[Id], id: Id) -> usize {
    siblings
        .iter()
        .position(|&sibling| sibling == id)
        .expect("a node is listed by its parent")
}

fn check_index(index: usize, len: usize) -> Result<(), OpError> {
    if index > len {
        return Err(OpError::IndexOutOfRange { index, len });
    }
    Ok(())
}

/// Fails with the property's name if `node` does not have it.
fn check_property(node: &Node, property: &Property) -> Result<(), &'static str> {
    match (node, property) {
        (Node::Layer(_), Property::Fill(_)) => Err("fill"),
        (Node::Layer(_), Property::Label(_)) => Err("label"),
        (Node::Layer(_), Property::Stamp(_)) => Err("stamp"),
        (Node::Layer(_), Property::Roughness(_)) => Err("roughness"),
        (Node::Layer(_), Property::Shore(_)) => Err("shore"),
        (Node::Object(object), Property::Shore(_)) if object.kind.geometry().is_none() => {
            Err("shore")
        }
        (Node::Object(object), Property::Roughness(_)) if object.kind.geometry().is_none() => {
            Err("roughness")
        }
        (Node::Object(object), Property::Label(_)) => match object.kind {
            ObjectKind::Label(_) => Ok(()),
            _ => Err("label"),
        },
        (Node::Object(object), Property::Stamp(_)) => match object.kind {
            ObjectKind::Stamp(_) => Ok(()),
            _ => Err("stamp"),
        },
        (Node::Layer(_), _)
        | (Node::Object(_), Property::Locked(_) | Property::Roughness(_) | Property::Shore(_)) => {
            Ok(())
        }
        (Node::Object(_), Property::Name(_)) => Err("name"),
        (Node::Object(_), Property::Visible(_)) => Err("visible"),
        (Node::Object(_), Property::Opacity(_)) => Err("opacity"),
        (Node::Object(_), Property::Clip(_)) => Err("clip"),
        (Node::Object(_), Property::DepthSort(_)) => Err("depth_sort"),
        (Node::Object(object), Property::Fill(_)) => object.kind.fill().map(|_| ()).ok_or("fill"),
    }
}

/// Sets a property `check_property` accepted and returns its old value.
fn set_property(node: &mut Node, property: Property) -> Property {
    match (node, property) {
        (Node::Layer(layer), Property::Name(name)) => {
            Property::Name(std::mem::replace(&mut layer.name, name))
        }
        (Node::Layer(layer), Property::Visible(visible)) => {
            Property::Visible(std::mem::replace(&mut layer.visible, visible))
        }
        (Node::Layer(layer), Property::Locked(locked)) => {
            Property::Locked(std::mem::replace(&mut layer.locked, locked))
        }
        (Node::Layer(layer), Property::Opacity(opacity)) => {
            Property::Opacity(std::mem::replace(&mut layer.opacity, opacity))
        }
        (Node::Layer(layer), Property::Clip(clip)) => {
            Property::Clip(std::mem::replace(&mut layer.clip, clip))
        }
        (Node::Layer(layer), Property::DepthSort(on)) => {
            Property::DepthSort(std::mem::replace(&mut layer.depth_sort, on))
        }
        (Node::Object(object), Property::Locked(locked)) => {
            Property::Locked(std::mem::replace(&mut object.locked, locked))
        }
        (Node::Object(object), Property::Fill(fill)) => {
            let slot = object.kind.fill_mut().expect("checked");
            Property::Fill(std::mem::replace(slot, fill))
        }
        (Node::Object(object), Property::Roughness(roughness)) => {
            let slot = object.kind.roughness_mut().expect("checked");
            Property::Roughness(std::mem::replace(slot, roughness))
        }
        (Node::Object(object), Property::Shore(shore)) => {
            let slot = object.kind.shore_mut().expect("checked");
            Property::Shore(std::mem::replace(slot, shore))
        }
        (Node::Object(object), Property::Label(label)) => match &mut object.kind {
            ObjectKind::Label(old) => Property::Label(std::mem::replace(old, label)),
            _ => unreachable!("rejected by check_property"),
        },
        (Node::Object(object), Property::Stamp(stamp)) => match &mut object.kind {
            ObjectKind::Stamp(old) => Property::Stamp(Box::new(std::mem::replace(old, *stamp))),
            _ => unreachable!("rejected by check_property"),
        },
        (_, _) => unreachable!("rejected by check_property"),
    }
}
