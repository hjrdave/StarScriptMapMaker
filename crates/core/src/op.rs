//! Primitive Ops: the only way a Map changes (ADR 0004).
//!
//! Commands compile into Ops. Applying an Op returns its inverse, so undo is the reversed list
//! of inverses. Ops check only the tree's structural rules; Commands do the user-level
//! validation, such as Lock.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::asset::AssetRef;
use crate::background::Background;
use crate::changes::Change;
use crate::color::Color;
use crate::embedded::{Blob, ContentHash, EmbeddedAsset, EmbeddedFont};
use crate::geometry::{MultiPolygon, Point};
use crate::id::Id;
use crate::label::Label;
use crate::map::{Board, InvariantError, Map, MapDefaults, Subtree};
use crate::node::{Membership, Node, ObjectKind, Stamp};
use crate::roughness::Roughness;
use crate::shore::ShoreStyle;

/// One primitive edit to a Map. Commands compile into Ops; applying an Op returns its inverse (ADR 0004).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// Inserts a node and its descendants at `index` among `parent`'s children
    /// (`None` is the top level, which holds only Layers).
    Insert {
        /// The Layer to insert into, or `None` for the top level.
        parent: Option<Id>,
        /// Where to insert among the parent's children.
        index: usize,
        /// The node and its descendants.
        subtree: Subtree,
    },
    /// Removes a node and its descendants.
    Remove {
        /// The node to remove.
        id: Id,
    },
    /// Sets one property of a node.
    SetProperty {
        /// The node to change.
        id: Id,
        /// The property and its new value.
        property: Property,
    },
    /// Replaces the outline of a Landmass or Lake.
    ReplaceGeometry {
        /// The Landmass or Lake to change.
        id: Id,
        /// Its new outline.
        geometry: MultiPolygon,
    },
    /// Moves a node to `index` among `parent`'s children, counted after it leaves its old place.
    Move {
        /// The node to move.
        id: Id,
        /// The Layer to move into, or `None` for the top level.
        parent: Option<Id>,
        /// The new place among the parent's children.
        index: usize,
    },
    /// Sets the Canvas size and the workspace position of its top-left corner. No node
    /// changes: Objects keep their positions and membership.
    ResizeCanvas {
        /// Where the Canvas's top-left corner sits, in workspace units.
        origin: Point,
        /// The new width, in Canvas units.
        width: u32,
        /// The new height, in Canvas units.
        height: u32,
    },
    /// Inserts a board at `index` among the boards. Its id must be new.
    InsertBoard {
        /// Where to insert among the boards.
        index: usize,
        /// The board.
        board: Board,
    },
    /// Removes a board other than the primary. Objects are not touched.
    RemoveBoard {
        /// The board to remove.
        id: Id,
    },
    /// Sets the size and origin of a board. No node changes.
    SetBoardRect {
        /// The board to change.
        id: Id,
        /// Where its top-left corner sits, in workspace units.
        origin: Point,
        /// The new width, in Canvas units.
        width: u32,
        /// The new height, in Canvas units.
        height: u32,
    },
    /// Sets the name of a board, or with `None` returns it to its default name.
    SetBoardName {
        /// The board to change.
        id: Id,
        /// Its new name.
        name: Option<String>,
    },
    /// Replaces the background of a board.
    SetBoardBackground {
        /// The board to change.
        id: Id,
        /// The new background.
        background: Background,
    },
    /// Moves a board to `index` among the boards, counted after it leaves its old place.
    MoveBoard {
        /// The board to move.
        id: Id,
        /// The new place among the boards.
        index: usize,
    },
    /// Makes a board the primary one, the Canvas. No node changes; Commands rewrite the
    /// members with their own Ops.
    SetPrimary {
        /// The new primary board.
        id: Id,
    },
    /// Replaces the Canvas's background.
    SetBackground {
        /// The new background.
        background: Background,
    },
    /// Replaces the Map-wide defaults.
    SetDefaults {
        /// The new Map-wide defaults.
        defaults: MapDefaults,
    },
    /// Stores the embedded file with content hash `hash`, or with `None` removes it.
    SetBlob {
        /// The content hash of the file.
        hash: ContentHash,
        /// The file's bytes, or `None` to remove it.
        data: Option<Blob>,
    },
    /// Sets the embedded record of an Asset, or with `None` removes it. The files it names
    /// are separate [`Op::SetBlob`]s.
    SetEmbeddedAsset {
        /// The Asset the record is for.
        asset: AssetRef,
        /// The new record, or `None` to remove it.
        record: Option<Box<EmbeddedAsset>>,
    },
    /// Sets the embedded record of the font file named `file`, or with `None` removes it.
    SetEmbeddedFont {
        /// The font file's name.
        file: String,
        /// The new record, or `None` to remove it.
        record: Option<Box<EmbeddedFont>>,
    },
}

/// A settable property and its new value.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "name", content = "value", rename_all = "snake_case")]
pub enum Property {
    /// Layers and Objects. An Object's empty name means it has none.
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
    /// Layers and Objects: the Link set, or `None` for no link.
    Link(Option<Id>),
    /// Objects only.
    Membership(Membership),
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

/// Why an Op did not apply. The Map is left unchanged.
#[derive(Clone, PartialEq, Debug, Error)]
pub enum OpError {
    /// No node has this id.
    #[error("no node has id {0}")]
    NotFound(Id),
    /// The node is an Object, and only Layers hold children.
    #[error("{0} is an Object; only Layers hold children")]
    NotALayer(Id),
    /// An Object was put at the top level, which holds only Layers.
    #[error("only Layers can be at the top level")]
    ObjectAtRoot,
    /// The index is past the end of the parent's children.
    #[error("index {index} is past the end of {len} children")]
    IndexOutOfRange {
        /// The index that was asked for.
        index: usize,
        /// How many children the parent has.
        len: usize,
    },
    /// No board has this id.
    #[error("no board has id {0}")]
    NoSuchBoard(Id),
    /// The primary board, the Canvas, cannot be removed.
    #[error("the primary board {0} cannot be removed")]
    RemovePrimaryBoard(Id),
    /// The node was moved into itself or one of its own descendants.
    #[error("cannot move {0} into itself or its own descendant")]
    MoveIntoSelf(Id),
    /// The node's kind does not have this property.
    #[error("{0} has no property {1}")]
    NoSuchProperty(Id, &'static str),
    /// The node has no outline to replace.
    #[error("{0} has no geometry")]
    NoGeometry(Id),
    /// The embedded file's bytes do not match its content hash.
    #[error("the embedded file does not match its content hash {0}")]
    BlobHashMismatch(ContentHash),
    /// The Op would break a structural rule of the Map.
    #[error(transparent)]
    Invariant(#[from] InvariantError),
}

impl Map {
    /// Applies `op` and returns its inverse. On error the Map is unchanged. Outside the
    /// `test-support` feature, only Commands reach this (ADR 0004).
    #[cfg(any(test, feature = "test-support"))]
    pub fn apply(&mut self, op: Op) -> Result<Op, OpError> {
        self.apply_op(op)
    }

    /// The Op applier behind [`Map::apply`], for `core` itself.
    #[allow(
        clippy::expect_used,
        reason = "Map::check verified the id exists; Map::check verified the Object has geometry"
    )]
    pub(crate) fn apply_op(&mut self, op: Op) -> Result<Op, OpError> {
        self.check(&op)?;
        self.revision += 1;
        let change = match &op {
            Op::SetProperty { id, .. } | Op::ReplaceGeometry { id, .. } => Change::Edited(*id),
            Op::Insert { subtree, .. } if subtree.children.is_empty() => {
                Change::Inserted(subtree.id)
            }
            Op::SetDefaults { .. } => Change::Defaults,
            _ => Change::Other,
        };
        let inverse = match op {
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
                    &mut self
                        .entries
                        .get_mut(&id)
                        .expect("Map::check verified the id exists")
                        .node,
                    property,
                );
                if let Property::Link(was) = &old {
                    let now = self.node(id).and_then(Node::link);
                    self.relink(id, *was, now);
                }
                self.touch(Some(id));
                Op::SetProperty { id, property: old }
            }
            Op::ReplaceGeometry { id, geometry } => {
                let Node::Object(object) = &mut self
                    .entries
                    .get_mut(&id)
                    .expect("Map::check verified the id exists")
                    .node
                else {
                    unreachable!("Map::check verified the id is an Object")
                };
                let slot = object
                    .kind
                    .geometry_mut()
                    .expect("Map::check verified the Object has geometry");
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
                self.entries
                    .get_mut(&id)
                    .expect("Map::check verified the id exists")
                    .parent = parent;
                self.touch(Some(id));
                self.touch(old_parent);
                self.touch(parent);
                Op::Move {
                    id,
                    parent: old_parent,
                    index: old_index,
                }
            }
            Op::ResizeCanvas {
                origin,
                width,
                height,
            } => {
                let canvas = self.canvas_mut();
                let old = Op::ResizeCanvas {
                    origin: canvas.origin,
                    width: canvas.width,
                    height: canvas.height,
                };
                canvas.origin = origin;
                canvas.width = width;
                canvas.height = height;
                old
            }
            op @ (Op::InsertBoard { .. }
            | Op::RemoveBoard { .. }
            | Op::SetBoardRect { .. }
            | Op::SetBoardName { .. }
            | Op::SetBoardBackground { .. }
            | Op::MoveBoard { .. }
            | Op::SetPrimary { .. }) => self.apply_board_op(op),
            Op::SetBackground { background } => Op::SetBackground {
                background: std::mem::replace(&mut self.canvas_mut().background, background),
            },
            Op::SetDefaults { defaults } => Op::SetDefaults {
                defaults: std::mem::replace(&mut self.defaults, defaults),
            },
            Op::SetBlob { hash, data } => {
                let old = match data {
                    Some(data) => self.embedded.blobs.insert(hash, data),
                    None => self.embedded.blobs.remove(&hash),
                };
                Op::SetBlob { hash, data: old }
            }
            Op::SetEmbeddedAsset { asset, record } => {
                let old = match record {
                    Some(record) => self.embedded.assets.insert(asset.clone(), *record),
                    None => self.embedded.assets.remove(&asset),
                };
                Op::SetEmbeddedAsset {
                    asset,
                    record: old.map(Box::new),
                }
            }
            Op::SetEmbeddedFont { file, record } => {
                let old = match record {
                    Some(record) => self.embedded.fonts.insert(file.clone(), *record),
                    None => self.embedded.fonts.remove(&file),
                };
                Op::SetEmbeddedFont {
                    file,
                    record: old.map(Box::new),
                }
            }
        };
        self.changes.record(self.revision, change);
        Ok(inverse)
    }

    /// Applies one of the board Ops, which `check` has already passed, and returns its inverse.
    #[allow(clippy::expect_used, reason = "Map::check verified the board exists")]
    fn apply_board_op(&mut self, op: Op) -> Op {
        match op {
            Op::InsertBoard { index, board } => {
                let id = board.id;
                self.boards.insert(index, board);
                Op::RemoveBoard { id }
            }
            Op::RemoveBoard { id } => {
                let index = board_position(&self.boards, id);
                Op::InsertBoard {
                    index,
                    board: self.boards.remove(index),
                }
            }
            Op::SetBoardRect {
                id,
                origin,
                width,
                height,
            } => {
                let canvas = &mut self
                    .board_mut(id)
                    .expect("Map::check verified the board exists")
                    .canvas;
                let old = Op::SetBoardRect {
                    id,
                    origin: canvas.origin,
                    width: canvas.width,
                    height: canvas.height,
                };
                canvas.origin = origin;
                canvas.width = width;
                canvas.height = height;
                old
            }
            Op::SetBoardName { id, name } => {
                let board = self
                    .board_mut(id)
                    .expect("Map::check verified the board exists");
                Op::SetBoardName {
                    id,
                    name: std::mem::replace(&mut board.name, name),
                }
            }
            Op::SetBoardBackground { id, background } => {
                let canvas = &mut self
                    .board_mut(id)
                    .expect("Map::check verified the board exists")
                    .canvas;
                Op::SetBoardBackground {
                    id,
                    background: std::mem::replace(&mut canvas.background, background),
                }
            }
            Op::MoveBoard { id, index } => {
                let old = board_position(&self.boards, id);
                let board = self.boards.remove(old);
                self.boards.insert(index, board);
                Op::MoveBoard { id, index: old }
            }
            Op::SetPrimary { id } => Op::SetPrimary {
                id: std::mem::replace(&mut self.primary, id),
            },
            other => unreachable!("{other:?} is not a board Op"),
        }
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
            Op::SetBlob {
                hash,
                data: Some(data),
            } if data.hash() != *hash => return Err(OpError::BlobHashMismatch(*hash)),
            Op::InsertBoard { index, board } => {
                if self.board(board.id).is_some() {
                    return Err(InvariantError::DuplicateBoard(board.id).into());
                }
                check_index(*index, self.boards.len())?;
            }
            Op::RemoveBoard { id } => {
                self.existing_board(*id)?;
                if *id == self.primary {
                    return Err(OpError::RemovePrimaryBoard(*id));
                }
            }
            Op::SetBoardRect { id, .. }
            | Op::SetBoardName { id, .. }
            | Op::SetBoardBackground { id, .. }
            | Op::SetPrimary { id } => {
                self.existing_board(*id)?;
            }
            Op::MoveBoard { id, index } => {
                self.existing_board(*id)?;
                check_index(*index, self.boards.len() - 1)?;
            }
            Op::SetDefaults { .. }
            | Op::SetBackground { .. }
            | Op::ResizeCanvas { .. }
            | Op::SetBlob { .. }
            | Op::SetEmbeddedAsset { .. }
            | Op::SetEmbeddedFont { .. } => {}
        }
        Ok(())
    }

    fn existing_board(&self, id: Id) -> Result<&Board, OpError> {
        self.board(id).ok_or(OpError::NoSuchBoard(id))
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

#[allow(clippy::expect_used, reason = "a node is listed by its parent")]
fn position(siblings: &[Id], id: Id) -> usize {
    siblings
        .iter()
        .position(|&sibling| sibling == id)
        .expect("a node is listed by its parent")
}

#[allow(clippy::expect_used, reason = "Map::check verified the board exists")]
fn board_position(boards: &[Board], id: Id) -> usize {
    boards
        .iter()
        .position(|board| board.id == id)
        .expect("Map::check verified the board exists")
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
        (Node::Layer(_), Property::Membership(_)) => Err("membership"),
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
        | (
            Node::Object(_),
            Property::Name(_)
            | Property::Locked(_)
            | Property::Link(_)
            | Property::Membership(_)
            | Property::Roughness(_)
            | Property::Shore(_),
        ) => Ok(()),
        (Node::Object(_), Property::Visible(_)) => Err("visible"),
        (Node::Object(_), Property::Opacity(_)) => Err("opacity"),
        (Node::Object(_), Property::Clip(_)) => Err("clip"),
        (Node::Object(_), Property::DepthSort(_)) => Err("depth_sort"),
        (Node::Object(object), Property::Fill(_)) => object.kind.fill().map(|_| ()).ok_or("fill"),
    }
}

/// Sets a property `check_property` accepted and returns its old value.
#[allow(
    clippy::expect_used,
    reason = "Map::check verified the Object has a fill; Map::check verified the Object has roughness; Map::check verified the Object has a shore"
)]
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
        (Node::Layer(layer), Property::Link(link)) => {
            Property::Link(std::mem::replace(&mut layer.link, link))
        }
        (Node::Object(object), Property::Link(link)) => {
            Property::Link(std::mem::replace(&mut object.link, link))
        }
        (Node::Object(object), Property::Name(name)) => {
            let new = (!name.is_empty()).then_some(name);
            Property::Name(std::mem::replace(&mut object.name, new).unwrap_or_default())
        }
        (Node::Object(object), Property::Locked(locked)) => {
            Property::Locked(std::mem::replace(&mut object.locked, locked))
        }
        (Node::Object(object), Property::Membership(membership)) => {
            Property::Membership(std::mem::replace(&mut object.membership, membership))
        }
        (Node::Object(object), Property::Fill(fill)) => {
            let slot = object
                .kind
                .fill_mut()
                .expect("Map::check verified the Object has a fill");
            Property::Fill(std::mem::replace(slot, fill))
        }
        (Node::Object(object), Property::Roughness(roughness)) => {
            let slot = object
                .kind
                .roughness_mut()
                .expect("Map::check verified the Object has roughness");
            Property::Roughness(std::mem::replace(slot, roughness))
        }
        (Node::Object(object), Property::Shore(shore)) => {
            let slot = object
                .kind
                .shore_mut()
                .expect("Map::check verified the Object has a shore");
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
