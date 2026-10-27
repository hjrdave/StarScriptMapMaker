use thiserror::Error;

use crate::geometry::Point;
use crate::id::Id;
use crate::op::OpError;

/// Why a Command was refused. The Map is left unchanged.
#[derive(Clone, PartialEq, Debug, Error)]
pub enum CommandError {
    /// No Layer or Object has this id.
    #[error("nothing has id {0}")]
    NotFound(Id),
    /// The node is an Object where a Layer is needed.
    #[error("{0} is not a Layer")]
    NotALayer(Id),
    /// A new node was given an id that already exists.
    #[error("id {0} is already in use")]
    IdInUse(Id),
    /// The node is locked.
    #[error("{0} is locked")]
    Locked(Id),
    /// The node sits inside a locked Layer.
    #[error("{target} is inside locked Layer {layer}")]
    InLockedLayer {
        /// The node the Command acts on.
        target: Id,
        /// The locked Layer above it.
        layer: Id,
    },
    /// The Layer holds a locked node.
    #[error("{target} contains locked {locked}")]
    ContainsLocked {
        /// The Layer the Command acts on.
        target: Id,
        /// The locked node inside it.
        locked: Id,
    },
    /// The node is linked to a locked node, so its Link set cannot move.
    #[error("{target} is linked to locked {locked}, so nothing linked to it can move")]
    LinkedLocked {
        /// The node the Command acts on.
        target: Id,
        /// The locked node in its Link set.
        locked: Id,
    },
    /// Fewer than two different nodes were given to link.
    #[error("linking needs at least two different Layers or Objects")]
    NothingToLink,
    /// The node does not share a parent with the others being grouped.
    #[error("{0} does not share a parent with the other nodes to group")]
    NotSiblings(Id),
    /// No nodes were given to group.
    #[error("nothing to group")]
    NothingToGroup,
    /// The same id was listed more than once.
    #[error("{0} is listed more than once")]
    DuplicateTarget(Id),
    /// The node is a Stamp in a Layer with Depth sort on, which orders its Stamps.
    #[error("{0} is in a Layer with Depth sort on, which orders its Stamps")]
    DepthSorted(Id),
    /// The opacity is outside 0 to 1.
    #[error("opacity must be from 0 to 1, not {0}")]
    InvalidOpacity(f32),
    /// The node is not a Landmass or Lake, so it has no outline.
    #[error("{0} is not a Landmass or Lake")]
    NoOutline(Id),
    /// The outline is not a valid shape.
    #[error("the outline of {id} is invalid: {problem}")]
    InvalidGeometry {
        /// The Landmass or Lake with the bad outline.
        id: Id,
        /// What is wrong with it.
        problem: String,
    },
    /// A node edit could not be made.
    #[error("cannot edit the points of {id}: {problem}")]
    NodeEdit {
        /// The Landmass or Lake being edited.
        id: Id,
        /// Why the edit failed.
        problem: String,
    },
    /// The node is not a Landmass.
    #[error("{0} is not a Landmass")]
    NotALandmass(Id),
    /// The Command needs at least two Landmasses.
    #[error("{command} needs at least two Landmasses")]
    NeedsTwoLandmasses {
        /// The name of the Command.
        command: &'static str,
    },
    /// Split Apart was given the wrong number of new ids.
    #[error("{id} has {parts} parts, so Split Apart needs {} new ids, not {got}", .parts - 1)]
    SplitIds {
        /// The Landmass being split.
        id: Id,
        /// How many parts it has.
        parts: usize,
        /// How many new ids were given.
        got: usize,
    },
    /// The Landmass is already a single part.
    #[error("{0} is already one part")]
    NothingToSplit(Id),
    /// The Shore style has a value out of range.
    #[error("the Shore style is invalid: {0}")]
    InvalidShore(String),
    /// The transform is not finite, or collapses shapes to a line or a point.
    #[error("the transform {0:?} is not finite or collapses shapes")]
    InvalidTransform([f64; 6]),
    /// The Roughness amount or scale is out of range.
    #[error("Roughness needs an amount of 0 or more and a scale above 0, not {amount} and {scale}")]
    InvalidRoughness {
        /// The amount that was given.
        amount: f64,
        /// The scale that was given.
        scale: f64,
    },
    /// The node is not a Label.
    #[error("{0} is not a Label")]
    NotALabel(Id),
    /// The Label has a value out of range.
    #[error("invalid Label: {0}")]
    InvalidLabel(&'static str),
    /// No Label style has this id.
    #[error("no Label style has id {0}")]
    NoLabelStyle(Id),
    /// The Label style has a value out of range.
    #[error("invalid Label style: {0}")]
    InvalidLabelStyle(&'static str),
    /// Another Label style already has this name.
    #[error("a Label style named \"{0}\" already exists")]
    LabelStyleNameInUse(String),
    /// A Label style already has this id.
    #[error("Label style id {0} is already in use")]
    LabelStyleIdInUse(Id),
    /// The default Label style cannot be deleted.
    #[error("the default Label style cannot be deleted")]
    DefaultLabelStyle,
    /// The node is not a Stamp.
    #[error("{0} is not a Stamp")]
    NotAStamp(Id),
    /// The node is a Layer where an Object is needed.
    #[error("{0} is not an Object")]
    NotAnObject(Id),
    /// The Canvas would be smaller than 1 by 1.
    #[error("the Canvas needs a width and height of at least 1, not {width} by {height}")]
    InvalidCanvasSize {
        /// The width that was given.
        width: u32,
        /// The height that was given.
        height: u32,
    },
    /// The Canvas origin is not finite.
    #[error("the Canvas origin {0:?} is not finite")]
    InvalidCanvasOrigin(Point),
    /// The Canvas and the Art boards have no board with this id.
    #[error("no board has id {0}")]
    NoSuchBoard(Id),
    /// The Canvas is the primary board and cannot be deleted. Make another board the Canvas
    /// first.
    #[error("{0} is the Canvas and cannot be deleted")]
    CannotDeleteCanvas(Id),
    /// The Canvas is always the lowest board.
    #[error("{0} is the Canvas and is always the lowest board")]
    CannotReorderCanvas(Id),
    /// The background's texture strength or scale is out of range.
    #[error("the background's texture strength must be 0 to 1 and its scale above 0")]
    InvalidBackground,
    /// The Assets or fonts to embed are not valid.
    #[error("cannot embed: {0}")]
    InvalidEmbed(String),
    /// The Stamp has a value out of range.
    #[error("invalid Stamp: {0}")]
    InvalidStamp(&'static str),
    /// Duplicate was given the wrong number of new ids.
    #[error("Duplicate copies {expected} nodes, so it needs {expected} new ids, not {got}")]
    DuplicateIds {
        /// How many new ids it needs.
        expected: usize,
        /// How many were given.
        got: usize,
    },
    /// An Op the Command compiled to did not apply.
    #[error(transparent)]
    Op(#[from] OpError),
}
