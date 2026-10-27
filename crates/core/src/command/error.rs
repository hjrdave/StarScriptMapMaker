use thiserror::Error;

use crate::geometry::Point;
use crate::id::Id;
use crate::op::OpError;

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
    #[error("{target} is linked to locked {locked}, so nothing linked to it can move")]
    LinkedLocked { target: Id, locked: Id },
    #[error("linking needs at least two different Layers or Objects")]
    NothingToLink,
    #[error("{0} does not share a parent with the other nodes to group")]
    NotSiblings(Id),
    #[error("nothing to group")]
    NothingToGroup,
    #[error("{0} is listed more than once")]
    DuplicateTarget(Id),
    #[error("{0} is in a Layer with Depth sort on, which orders its Stamps")]
    DepthSorted(Id),
    #[error("opacity must be from 0 to 1, not {0}")]
    InvalidOpacity(f32),
    #[error("{0} is not a Landmass or Lake")]
    NoOutline(Id),
    #[error("the outline of {id} is invalid: {problem}")]
    InvalidGeometry { id: Id, problem: String },
    #[error("cannot edit the points of {id}: {problem}")]
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
    #[error("no Label style has id {0}")]
    NoLabelStyle(Id),
    #[error("invalid Label style: {0}")]
    InvalidLabelStyle(&'static str),
    #[error("a Label style named \"{0}\" already exists")]
    LabelStyleNameInUse(String),
    #[error("Label style id {0} is already in use")]
    LabelStyleIdInUse(Id),
    #[error("the default Label style cannot be deleted")]
    DefaultLabelStyle,
    #[error("{0} is not a Stamp")]
    NotAStamp(Id),
    #[error("{0} is not an Object")]
    NotAnObject(Id),
    #[error("the Canvas needs a width and height of at least 1, not {width} by {height}")]
    InvalidCanvasSize { width: u32, height: u32 },
    #[error("the Canvas origin {0:?} is not finite")]
    InvalidCanvasOrigin(Point),
    #[error("the background's texture strength must be 0 to 1 and its scale above 0")]
    InvalidBackground,
    #[error("cannot embed: {0}")]
    InvalidEmbed(String),
    #[error("invalid Stamp: {0}")]
    InvalidStamp(&'static str),
    #[error("Duplicate copies {expected} nodes, so it needs {expected} new ids, not {got}")]
    DuplicateIds { expected: usize, got: usize },
    #[error(transparent)]
    Op(#[from] OpError),
}
