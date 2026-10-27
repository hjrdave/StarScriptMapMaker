//! Commands: the user-level edits the UI and agents issue (ADR 0004).
//!
//! A Command checks the Map's user-level rules (Lock, references, value ranges), compiles to
//! primitive Ops, and applies them all or none. This file holds the `Command` enum and the
//! `compile` dispatch; the checks and Ops for each concern live in the submodules.

mod board;
mod check;
mod defaults;
mod embed;
mod error;
mod execute;
mod group;
mod label;
mod land;
mod layer;
mod link;
mod object;
mod paste;
mod stamp;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::background::Background;
use crate::color::Color;
use crate::embedded::{EmbedAsset, EmbedFont};
use crate::geometry::{Affine, MultiPolygon, NodeRef, Point};
use crate::id::Id;
use crate::label::{Label, LabelStyle};
use crate::land as outlines;
use crate::map::{Map, Subtree};
use crate::node::{Membership, Stamp};
use crate::op::Op;
use crate::roughness::Roughness;
use crate::shore::{ShoreKind, ShoreStyle};

use land::node_edit;

pub use error::CommandError;

/// One edit to a Map. Commands name every id they touch or create, so replaying one gives
/// the same Map every time.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// Creates an empty Layer with the given id, inside `parent` (`None` for the top level).
    /// `index` counts from the bottom of the Stacking order; omitted, the Layer goes on top.
    CreateLayer {
        /// The id the new Layer gets.
        id: Id,
        /// The Layer to create it in, or `None` for the top level.
        parent: Option<Id>,
        /// Where the node goes among its siblings, counted from the bottom of the Stacking order.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        /// The new Layer's name.
        name: String,
    },
    /// Renames a Layer.
    RenameLayer {
        /// The Layer.
        id: Id,
        /// The new name.
        name: String,
    },
    /// Names an Object. An empty name clears it, back to the kind's default.
    RenameObject {
        /// The id of the Layer or Object this Command acts on.
        id: Id,
        /// The new name.
        name: String,
    },
    /// Moves a Layer or Object to `index` among `parent`'s children, counted after it leaves
    /// its old place.
    Move {
        /// The id of the Layer or Object this Command acts on.
        id: Id,
        /// The Layer to move into, or `None` for the top level.
        parent: Option<Id>,
        /// The new place among `parent`'s children.
        index: usize,
    },
    /// Shows or hides a Layer.
    SetVisible {
        /// The Layer.
        id: Id,
        /// Whether the Layer is shown.
        visible: bool,
    },
    /// Locks or unlocks a Layer or Object. Unlocking is refused while a Layer above it is locked.
    SetLocked {
        /// The id of the Layer or Object this Command acts on.
        id: Id,
        /// Whether it is locked.
        locked: bool,
    },
    /// Sets the opacity of a Layer or Object, from 0 (clear) to 1 (opaque).
    SetOpacity {
        /// The Layer or Object.
        id: Id,
        /// The opacity, from 0 to 1.
        opacity: f32,
    },
    /// Switches Depth sort on or off for a Layer: its direct Stamps then draw by Anchor
    /// height (ADR 0003).
    SetDepthSort {
        /// The id of the Layer or Object this Command acts on.
        id: Id,
        /// Whether Depth sort is on.
        depth_sort: bool,
    },
    /// Clips a Layer to the Landmass `target`, or clears its Clip with `None`.
    SetClip {
        /// The id of the Layer or Object this Command acts on.
        id: Id,
        /// The Landmass to clip to, or `None` to clear the Clip.
        target: Option<Id>,
    },
    /// Wraps Layers and Objects that share a parent in a new Group (a Layer named `name`
    /// with the given id). The Group takes the place of the topmost member, and the members
    /// keep their relative order inside it.
    CreateGroup {
        /// The id the new Group gets.
        id: Id,
        /// The members to wrap, which must share a parent.
        ids: Vec<Id>,
        /// The Group's name.
        name: String,
    },
    /// Dissolves Groups: their contents move up into the parent, in the Group's place and in
    /// order, and the empty Group is removed.
    Ungroup {
        /// The Groups to dissolve.
        ids: Vec<Id>,
    },
    /// Links Layers and Objects into the Link set `id`, so moving or transforming one moves
    /// or transforms the others. Any set a listed node already belongs to joins it whole.
    /// The nodes keep their own places in the Stacking order, in any Layer.
    Link {
        /// The id of the Link set.
        id: Id,
        /// The nodes to link.
        ids: Vec<Id>,
    },
    /// Takes Layers and Objects out of their Link sets.
    Unlink {
        /// The nodes to take out of their Link sets.
        ids: Vec<Id>,
    },
    /// Deletes Layers and Objects with everything inside them, and clears Clips that pointed
    /// at anything deleted.
    Delete {
        /// The nodes to delete.
        ids: Vec<Id>,
    },
    /// Creates a Landmass with the given id and outline inside Layer `parent`. `index` counts
    /// from the bottom of the Stacking order; omitted, the Landmass goes on top. Without a
    /// `fill`, it takes the default land colour.
    CreateLandmass {
        /// The id the new Landmass gets.
        id: Id,
        /// The Layer to create it in.
        parent: Id,
        /// Where the node goes among its siblings, counted from the bottom of the Stacking order.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        /// The outline of the Landmass.
        geometry: MultiPolygon,
        /// The fill colour, or `None` for the default land colour.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fill: Option<Color>,
    },
    /// Creates a Lake, as `create_landmass` does a Landmass.
    CreateLake {
        /// The id the new Lake gets.
        id: Id,
        /// The Layer to create it in.
        parent: Id,
        /// Where the node goes among its siblings, counted from the bottom of the Stacking order.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        /// The outline of the Lake.
        geometry: MultiPolygon,
        /// The fill colour, or `None` for the default water colour.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fill: Option<Color>,
    },
    /// Moves the outlines of Landmasses and Lakes by `offset` Canvas units.
    Translate {
        /// The Landmasses and Lakes to move.
        ids: Vec<Id>,
        /// How far to move them, in Canvas units, as `[x, y]`.
        offset: [f64; 2],
    },
    /// Applies an affine transform to the outlines of Landmasses and Lakes, as dragging a
    /// resize handle does. It must not collapse them to a line or a point.
    Transform {
        /// The Landmasses and Lakes to transform.
        ids: Vec<Id>,
        /// The affine transform to apply.
        transform: Affine,
    },
    /// Adds `region` to the outline of a Landmass or Lake, as a land brush or lasso stroke
    /// does. Parts it joins become one part; a hole it covers fills in.
    AddRegion {
        /// The Landmass or Lake to add to.
        id: Id,
        /// The area to add.
        region: MultiPolygon,
    },
    /// Cuts `region` out of the outlines of Landmasses and Lakes. One it splits keeps every
    /// part, one it leaves with nothing is deleted (clearing Clips to it), and one it misses
    /// is left alone.
    SubtractRegion {
        /// The Landmasses and Lakes to cut.
        ids: Vec<Id>,
        /// The area to cut out.
        region: MultiPolygon,
    },
    /// Moves nodes of a Landmass or Lake by `offset` Canvas units. If that makes the outline
    /// cross itself, it is repaired.
    MoveNodes {
        /// The Landmass or Lake the nodes belong to.
        id: Id,
        /// The nodes to move.
        nodes: Vec<NodeRef>,
        /// How far to move them, in Canvas units, as `[x, y]`.
        offset: [f64; 2],
    },
    /// Adds a node at `point` on the edge that runs from `edge` to the next point of its ring.
    InsertNode {
        /// The Landmass or Lake to add the node to.
        id: Id,
        /// The node the edge starts from.
        edge: NodeRef,
        /// Where the new node goes.
        point: Point,
    },
    /// Deletes nodes. A ring cannot be left with fewer than three points.
    DeleteNodes {
        /// The Landmass or Lake the nodes belong to.
        id: Id,
        /// The nodes to delete.
        nodes: Vec<NodeRef>,
    },
    /// Merges Landmasses into one outline. The result keeps the topmost one's id, place and
    /// properties; the others are removed, and Layers clipped to them clip to the result.
    Union {
        /// The Landmasses to merge.
        ids: Vec<Id>,
    },
    /// Cuts the topmost of the Landmasses out of the others and removes it. A Landmass left
    /// with nothing is deleted, clearing Clips to it.
    Subtract {
        /// The Landmasses to work on; the topmost is cut out of the rest.
        ids: Vec<Id>,
    },
    /// Turns each part of a Landmass into a Landmass of its own, in the same Layer. The first
    /// part keeps `id`; `new_ids` name the rest, in order, so there must be one fewer than
    /// there are parts.
    SplitApart {
        /// The Landmass to split.
        id: Id,
        /// The ids for every part after the first.
        new_ids: Vec<Id>,
    },
    /// Resizes the Canvas to `width` by `height` with its top-left corner at the workspace
    /// position `origin`. Objects keep their positions, size and membership, even when they
    /// end up entirely outside the Canvas. Locks don't matter: no Object is touched.
    ResizeCanvas {
        /// Where the Canvas's top-left corner sits, in workspace units.
        origin: Point,
        /// The new width, in Canvas units.
        width: u32,
        /// The new height, in Canvas units.
        height: u32,
    },
    /// Creates an Art board with the given id, at `origin`, `width` by `height`, with the
    /// default background. `index` counts from the first board; omitted, it goes last.
    CreateBoard {
        /// The id the new board gets.
        id: Id,
        /// Its name; `None` shows as "Art board N".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// Where its top-left corner sits, in workspace units.
        origin: Point,
        /// Its width, in Canvas units.
        width: u32,
        /// Its height, in Canvas units.
        height: u32,
        /// Where it goes among the boards.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
    },
    /// Moves and resizes a board (the Canvas or an Art board). No Object changes position or
    /// membership, even when it ends up entirely outside the board.
    ResizeBoard {
        /// The board to change.
        id: Id,
        /// Where its top-left corner sits, in workspace units.
        origin: Point,
        /// The new width, in Canvas units.
        width: u32,
        /// The new height, in Canvas units.
        height: u32,
    },
    /// Renames a board, or with `None` returns it to its default name. Names need not be
    /// unique.
    RenameBoard {
        /// The board to rename.
        id: Id,
        /// The new name.
        name: Option<String>,
    },
    /// Deletes an Art board. Its Objects are kept and go off-board. The Canvas is refused.
    DeleteBoard {
        /// The board to delete.
        id: Id,
    },
    /// Moves a board to `index` among the boards, counted after it leaves its old place.
    ReorderBoard {
        /// The board to move.
        id: Id,
        /// The new place among the boards.
        index: usize,
    },
    /// Makes a board the Canvas. The old Canvas becomes an Art board, and every Object stays on
    /// the board it was on.
    SetPrimaryBoard {
        /// The new primary board.
        id: Id,
    },
    /// Sets whether Objects belong to the Canvas or sit off it. Only a drag
    /// changes membership; resizing the Canvas never does.
    SetMembership {
        /// The Objects to change.
        ids: Vec<Id>,
        /// Whether they belong to the Canvas or sit off it.
        membership: Membership,
    },
    /// Sets the fill colour of Landmasses and Lakes.
    SetFill {
        /// The Landmasses and Lakes to colour.
        ids: Vec<Id>,
        /// The new fill colour.
        fill: Color,
    },
    /// Sets the Roughness of Landmasses and Lakes, or with `None` returns them to the Map-wide
    /// default.
    SetRoughness {
        /// The Landmasses and Lakes to change.
        ids: Vec<Id>,
        /// Their Roughness, or `None` to follow the Map-wide default.
        roughness: Option<Roughness>,
    },
    /// Replaces the Canvas's background: the water colour and the texture over it. The
    /// template the Map was made from is left as it is.
    SetBackground {
        /// The new background.
        background: Background,
    },
    /// Sets the Roughness of every Landmass and Lake without its own.
    SetDefaultRoughness {
        /// The new Map-wide Roughness.
        roughness: Roughness,
    },
    /// Sets the Shore style of Landmasses and Lakes, or with `None` returns them to the
    /// Map-wide default for their kind.
    SetShore {
        /// The Landmasses and Lakes to change.
        ids: Vec<Id>,
        /// Their Shore style, or `None` to follow the Map-wide default.
        shore: Option<ShoreStyle>,
    },
    /// Sets the Map-wide Shore style of every Landmass, or every Lake, without its own.
    SetDefaultShore {
        /// Whether this is the default for Landmasses or for Lakes.
        kind: ShoreKind,
        /// The new default Shore style.
        shore: ShoreStyle,
    },
    /// Places a Label with the given id inside Layer `parent`. `index` counts from the bottom
    /// of the Stacking order; omitted, the Label goes on top.
    CreateLabel {
        /// The id the new Label gets.
        id: Id,
        /// The Layer to place it in.
        parent: Id,
        /// Where the node goes among its siblings, counted from the bottom of the Stacking order.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        /// The Label to place.
        label: Box<Label>,
    },
    /// Replaces every field of a Label, as one edit: text, placement, typography or colour.
    SetLabel {
        /// The Label to replace.
        id: Id,
        /// Its new content.
        label: Box<Label>,
    },
    /// Adds a Label style, last in the list. Its name must be new.
    CreateLabelStyle {
        /// The style to add.
        style: LabelStyle,
    },
    /// Renames a Label style. The new name must not belong to another style.
    RenameLabelStyle {
        /// The Label style to rename.
        id: Id,
        /// Its new name.
        name: String,
    },
    /// Replaces the settings of a Label style, and so restyles every Label that follows it
    /// and does not override them, as one edit. The style keeps its name.
    SetLabelStyle {
        /// The style as it should be, found by its id.
        style: LabelStyle,
    },
    /// Deletes a Label style other than the default. Labels that follow it get its values as
    /// overrides and move to the default style, so nothing changes on the Canvas.
    DeleteLabelStyle {
        /// The Label style to delete.
        id: Id,
    },
    /// Places a Stamp with the given id inside Layer `parent`. `index` counts from the bottom
    /// of the Stacking order; omitted, the Stamp goes on top.
    CreateStamp {
        /// The id the new Stamp gets.
        id: Id,
        /// The Layer to place it in.
        parent: Id,
        /// Where the node goes among its siblings, counted from the bottom of the Stacking order.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        /// The Stamp to place.
        stamp: Box<Stamp>,
    },
    /// Replaces every field of a Stamp, as one edit: its Asset, placement, colours or opacity.
    SetStamp {
        /// The Stamp to replace.
        id: Id,
        /// Its new content.
        stamp: Box<Stamp>,
    },
    /// Places many Stamps inside Layer `parent` on top, in the order given, as one edit. A
    /// brush resolves its randomness first, so this names every Stamp it creates.
    PlaceStamps {
        /// The Layer to place them in.
        parent: Id,
        /// The Stamps to place, each with its id.
        stamps: Vec<PlacedStamp>,
    },
    /// Copies Layers and Objects, each with everything inside it, to just above the original.
    /// Ids that sit inside another listed id come with it. `new_ids` names every copied node:
    /// the listed ids in order, each followed by its descendants in tree order. Copies are
    /// named "... copy", are unlocked, and keep every other setting (a Landmass keeps its
    /// Roughness, so its coast is identical). A Clip to a copied Landmass follows the copy.
    /// `offset` moves the copies by that many Canvas units, as an Option-drag does.
    Duplicate {
        /// The nodes to copy.
        ids: Vec<Id>,
        /// The ids of the copies, in the order described above.
        new_ids: Vec<Id>,
        /// How far to move the copies, in Canvas units, as `[x, y]`.
        #[serde(default, skip_serializing_if = "is_zero")]
        offset: [f64; 2],
    },
    /// Pastes copied branches of the tree into Layer `parent` (the root when `None`) at
    /// `index` (the top when `None`), as one edit. `subtrees` are the branches as copied, with
    /// their original ids; `new_ids` names every pasted node as in `Duplicate`: each branch in
    /// order, a node followed by its descendants in tree order. Nodes keep their names,
    /// visibility, lock state and settings, and a name already used beside them gets " copy"
    /// added. `offset` moves every Object by that many Canvas units. A Clip follows its
    /// Landmass when that is pasted too, stays if the Landmass is still in the Map, and is
    /// dropped otherwise.
    Paste {
        /// The Layer to paste into, or `None` for the top level.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<Id>,
        /// Where to paste among its children, or `None` for the top.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        /// The branches to paste, with their original ids.
        subtrees: Vec<Subtree>,
        /// The ids of the pasted nodes, in the order described above.
        new_ids: Vec<Id>,
        /// How far to move every Object, in Canvas units, as `[x, y]`.
        #[serde(default, skip_serializing_if = "is_zero")]
        offset: [f64; 2],
    },
    /// Embeds Assets and font files in the Map (ADR 0005), as saving does for everything the
    /// Map uses. Files are stored once per content hash. An Asset or font file already
    /// embedded is left as it is: the embedded copy wins.
    EmbedAssets {
        /// The Assets to embed.
        assets: Vec<EmbedAsset>,
        /// The font files to embed.
        fonts: Vec<EmbedFont>,
    },
    /// Replaces the embedded copies of Assets already embedded with fresh ones read from an
    /// installed pack ("Update from pack"). Files nothing refers to afterwards are dropped.
    UpdateFromPack {
        /// The fresh copies of the Assets to replace.
        assets: Vec<EmbedAsset>,
    },
}

fn is_zero(offset: &[f64; 2]) -> bool {
    *offset == [0.0, 0.0]
}

/// One Stamp of a [`Command::PlaceStamps`], with the id it gets.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PlacedStamp {
    /// The id the Stamp gets.
    pub id: Id,
    /// The Stamp to place.
    pub stamp: Stamp,
}

impl Command {
    /// A short name for menus and the undo history.
    pub fn label(&self) -> &'static str {
        match self {
            Command::CreateLayer { .. } => "Create Layer",
            Command::RenameLayer { .. } => "Rename Layer",
            Command::RenameObject { .. } => "Rename Object",
            Command::Move { .. } => "Move",
            Command::SetVisible { .. } => "Change Visibility",
            Command::SetLocked { .. } => "Change Lock",
            Command::SetOpacity { .. } => "Change Opacity",
            Command::SetDepthSort { .. } => "Change Depth Sort",
            Command::SetClip {
                target: Some(_), ..
            } => "Clip to Landmass",
            Command::SetClip { target: None, .. } => "Clear Clip",
            Command::CreateGroup { .. } => "Group",
            Command::Ungroup { .. } => "Ungroup",
            Command::Link { .. } => "Link",
            Command::Unlink { .. } => "Unlink",
            Command::Delete { .. } => "Delete",
            Command::CreateLandmass { .. } => "Create Landmass",
            Command::CreateLake { .. } => "Create Lake",
            Command::Translate { .. } => "Move",
            Command::Transform { .. } => "Transform",
            Command::AddRegion { .. } => "Add Region",
            Command::SubtractRegion { .. } => "Subtract Region",
            Command::MoveNodes { .. } => "Move Points",
            Command::InsertNode { .. } => "Add Point",
            Command::DeleteNodes { .. } => "Delete Points",
            Command::Union { .. } => "Union",
            Command::Subtract { .. } => "Subtract",
            Command::SplitApart { .. } => "Split Apart",
            Command::ResizeCanvas { .. } => "Resize Canvas",
            Command::CreateBoard { .. } => "Add Art Board",
            Command::ResizeBoard { .. } => "Resize Board",
            Command::RenameBoard { .. } => "Rename Board",
            Command::DeleteBoard { .. } => "Delete Board",
            Command::ReorderBoard { .. } => "Reorder Boards",
            Command::SetPrimaryBoard { .. } => "Change Canvas",
            Command::SetMembership { .. } => "Move On or Off Canvas",
            Command::SetFill { .. } => "Change Fill",
            Command::SetRoughness { .. } => "Change Roughness",
            Command::SetBackground { .. } => "Change Background",
            Command::SetDefaultRoughness { .. } => "Change Default Roughness",
            Command::SetShore { .. } => "Change Shore Style",
            Command::SetDefaultShore { .. } => "Change Default Shore Style",
            Command::CreateLabel { .. } => "Add Label",
            Command::SetLabel { .. } => "Edit Label",
            Command::CreateLabelStyle { .. } => "Add Label Style",
            Command::RenameLabelStyle { .. } => "Rename Label Style",
            Command::SetLabelStyle { .. } => "Edit Label Style",
            Command::DeleteLabelStyle { .. } => "Delete Label Style",
            Command::CreateStamp { .. } => "Add Stamp",
            Command::SetStamp { .. } => "Edit Stamp",
            Command::PlaceStamps { .. } => "Place Stamps",
            Command::Duplicate { .. } => "Duplicate",
            Command::Paste { .. } => "Paste",
            Command::EmbedAssets { .. } => "Embed Assets",
            Command::UpdateFromPack { .. } => "Update from Pack",
        }
    }

    /// Checks the Command against `map` and returns the Ops it applies, in order.
    pub(crate) fn compile(&self, map: &Map) -> Result<Vec<Op>, CommandError> {
        match self {
            Command::CreateLayer {
                id,
                parent,
                index,
                name,
            } => layer::create_layer(map, *id, *parent, *index, name),
            Command::RenameLayer { id, name } => layer::rename_layer(map, *id, name),
            Command::RenameObject { id, name } => layer::rename_object(map, *id, name),
            Command::Move { id, parent, index } => layer::move_node(map, *id, *parent, *index),
            Command::SetVisible { id, visible } => layer::set_visible(map, *id, *visible),
            Command::SetLocked { id, locked } => layer::set_locked(map, *id, *locked),
            Command::SetOpacity { id, opacity } => layer::set_opacity(map, *id, *opacity),
            Command::SetDepthSort { id, depth_sort } => {
                layer::set_depth_sort(map, *id, *depth_sort)
            }
            Command::SetClip { id, target } => layer::set_clip(map, *id, *target),
            Command::CreateGroup { id, ids, name } => group::create_group(map, *id, ids, name),
            Command::Ungroup { ids } => group::ungroup(map, ids),
            Command::Link { id, ids } => link::link(map, *id, ids),
            Command::Unlink { ids } => link::unlink(map, ids),
            Command::Delete { ids } => object::delete(map, ids),
            Command::CreateLandmass {
                id,
                parent,
                index,
                geometry,
                fill,
            } => object::create_landmass(map, *id, *parent, *index, geometry, *fill),
            Command::CreateLake {
                id,
                parent,
                index,
                geometry,
                fill,
            } => object::create_lake(map, *id, *parent, *index, geometry, *fill),
            Command::Duplicate {
                ids,
                new_ids,
                offset,
            } => paste::duplicate(map, ids, new_ids, *offset),
            Command::Paste {
                parent,
                index,
                subtrees,
                new_ids,
                offset,
            } => paste::paste(map, *parent, *index, subtrees, new_ids, *offset),
            Command::Translate { ids, offset } => {
                object::transform(map, ids, &Affine::translate(*offset))
            }
            Command::Transform { ids, transform: t } => object::transform(map, ids, t),
            Command::AddRegion { id, region } => land::add_region(map, *id, region),
            Command::SubtractRegion { ids, region } => land::subtract_region(map, ids, region),
            Command::MoveNodes { id, nodes, offset } => node_edit(map, *id, |outline| {
                outlines::move_nodes(outline, nodes, *offset)
            }),
            Command::InsertNode { id, edge, point } => node_edit(map, *id, |outline| {
                outlines::insert_node(outline, *edge, *point)
            }),
            Command::DeleteNodes { id, nodes } => {
                node_edit(map, *id, |outline| outlines::delete_nodes(outline, nodes))
            }
            Command::Union { ids } => land::union_landmasses(map, ids),
            Command::Subtract { ids } => land::subtract_landmasses(map, ids),
            Command::SplitApart { id, new_ids } => land::split_apart(map, *id, new_ids),
            Command::ResizeCanvas {
                origin,
                width,
                height,
            } => layer::resize_canvas(*origin, *width, *height),
            Command::CreateBoard {
                id,
                name,
                origin,
                width,
                height,
                index,
            } => board::create_board(map, *id, name.as_deref(), *origin, *width, *height, *index),
            Command::ResizeBoard {
                id,
                origin,
                width,
                height,
            } => board::resize_board(map, *id, *origin, *width, *height),
            Command::RenameBoard { id, name } => board::rename_board(map, *id, name.as_deref()),
            Command::DeleteBoard { id } => board::delete_board(map, *id),
            Command::ReorderBoard { id, index } => board::reorder_board(map, *id, *index),
            Command::SetPrimaryBoard { id } => board::set_primary_board(map, *id),
            Command::SetMembership { ids, membership } => {
                object::set_membership(map, ids, *membership)
            }
            Command::SetFill { ids, fill } => land::set_fill(map, ids, *fill),
            Command::SetRoughness { ids, roughness } => land::set_roughness(map, ids, roughness),
            Command::SetShore { ids, shore } => land::set_shore(map, ids, shore),
            Command::SetDefaultShore { kind, shore } => {
                defaults::set_default_shore(map, *kind, shore)
            }
            Command::SetBackground { background } => defaults::set_background(background),
            Command::SetDefaultRoughness { roughness } => {
                defaults::set_default_roughness(map, roughness)
            }
            Command::CreateLabel {
                id,
                parent,
                index,
                label,
            } => label::create_label(map, *id, *parent, *index, label),
            Command::SetLabel { id, label } => label::set_label(map, *id, label),
            Command::CreateLabelStyle { style } => label::create_label_style(map, style),
            Command::RenameLabelStyle { id, name } => label::rename_label_style(map, *id, name),
            Command::SetLabelStyle { style } => label::set_label_style(map, style),
            Command::DeleteLabelStyle { id } => label::delete_label_style(map, *id),
            Command::CreateStamp {
                id,
                parent,
                index,
                stamp,
            } => stamp::create_stamp(map, *id, *parent, *index, stamp),
            Command::SetStamp { id, stamp } => stamp::set_stamp(map, *id, stamp),
            Command::PlaceStamps { parent, stamps } => stamp::place_stamps(map, *parent, stamps),
            Command::EmbedAssets { assets, fonts } => embed::embed_assets(map, assets, fonts),
            Command::UpdateFromPack { assets } => embed::update_from_pack(map, assets),
        }
    }
}
