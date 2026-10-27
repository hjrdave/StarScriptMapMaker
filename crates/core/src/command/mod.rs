//! Commands: the user-level edits the UI and agents issue (ADR 0004).
//!
//! A Command checks the Map's user-level rules (Lock, references, value ranges), compiles to
//! primitive Ops, and applies them all or none. This file holds the `Command` enum and the
//! `compile` dispatch; the checks and Ops for each concern live in the submodules.

mod check;
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

use std::collections::HashSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::background::Background;
use crate::color::Color;
use crate::embedded::{EmbedAsset, EmbedFont};
use crate::geometry::{Affine, MultiPolygon, NodeRef, Point};
use crate::id::Id;
use crate::label::{Label, LabelStyle};
use crate::land as outlines;
use crate::map::{Map, MapDefaults, Subtree};
use crate::node::{Lake, Landmass, Membership, Node, ObjectKind, Stamp};
use crate::op::{Op, Property};
use crate::roughness::Roughness;
use crate::shore::{ShoreKind, ShoreStyle};

use check::{existing, set, unlocked};
use land::{node_edit, valid_roughness};

pub use error::CommandError;

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
    /// Names an Object. An empty name clears it, back to the kind's default.
    RenameObject {
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
    /// Clips a Layer to the Landmass `target`, or clears its Clip with `None`.
    SetClip {
        id: Id,
        target: Option<Id>,
    },
    /// Wraps Layers and Objects that share a parent in a new Group (a Layer named `name`
    /// with the given id). The Group takes the place of the topmost member, and the members
    /// keep their relative order inside it.
    CreateGroup {
        id: Id,
        ids: Vec<Id>,
        name: String,
    },
    /// Dissolves Groups: their contents move up into the parent, in the Group's place and in
    /// order, and the empty Group is removed.
    Ungroup {
        ids: Vec<Id>,
    },
    /// Links Layers and Objects into the Link set `id`, so moving or transforming one moves
    /// or transforms the others. Any set a listed node already belongs to joins it whole.
    /// The nodes keep their own places in the Stacking order, in any Layer.
    Link {
        id: Id,
        ids: Vec<Id>,
    },
    /// Takes Layers and Objects out of their Link sets.
    Unlink {
        ids: Vec<Id>,
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
    /// Resizes the Canvas to `width` by `height` with its top-left corner at the workspace
    /// position `origin`. Objects keep their positions, size and membership, even when they
    /// end up entirely outside the Canvas. Locks don't matter: no Object is touched.
    ResizeCanvas {
        origin: Point,
        width: u32,
        height: u32,
    },
    /// Sets whether Objects belong to the Canvas or sit on the pasteboard. Only a drag
    /// changes membership; resizing the Canvas never does.
    SetMembership {
        ids: Vec<Id>,
        membership: Membership,
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
    /// Replaces the Canvas's background: the water colour and the texture over it. The
    /// template the Map was made from is left as it is.
    SetBackground {
        background: Background,
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
    /// Adds a Label style, last in the list. Its name must be new.
    CreateLabelStyle {
        style: LabelStyle,
    },
    /// Renames a Label style. The new name must not belong to another style.
    RenameLabelStyle {
        id: Id,
        name: String,
    },
    /// Replaces the settings of a Label style, and so restyles every Label that follows it
    /// and does not override them, as one edit. The style keeps its name.
    SetLabelStyle {
        style: LabelStyle,
    },
    /// Deletes a Label style other than the default. Labels that follow it get its values as
    /// overrides and move to the default style, so nothing changes on the Canvas.
    DeleteLabelStyle {
        id: Id,
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
    /// Copies Layers and Objects, each with everything inside it, to just above the original.
    /// Ids that sit inside another listed id come with it. `new_ids` names every copied node:
    /// the listed ids in order, each followed by its descendants in tree order. Copies are
    /// named "... copy", are unlocked, and keep every other setting (a Landmass keeps its
    /// Roughness, so its coast is identical). A Clip to a copied Landmass follows the copy.
    /// `offset` moves the copies by that many Canvas units, as an Option-drag does.
    Duplicate {
        ids: Vec<Id>,
        new_ids: Vec<Id>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<Id>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        subtrees: Vec<Subtree>,
        new_ids: Vec<Id>,
        #[serde(default, skip_serializing_if = "is_zero")]
        offset: [f64; 2],
    },
    /// Embeds Assets and font files in the Map (ADR 0005), as saving does for everything the
    /// Map uses. Files are stored once per content hash. An Asset or font file already
    /// embedded is left as it is: the embedded copy wins.
    EmbedAssets {
        assets: Vec<EmbedAsset>,
        fonts: Vec<EmbedFont>,
    },
    /// Replaces the embedded copies of Assets already embedded with fresh ones read from an
    /// installed pack ("Update from pack"). Files nothing refers to afterwards are dropped.
    UpdateFromPack {
        assets: Vec<EmbedAsset>,
    },
}

fn is_zero(offset: &[f64; 2]) -> bool {
    *offset == [0.0, 0.0]
}

/// One Stamp of a [`Command::PlaceStamps`], with the id it gets.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PlacedStamp {
    pub id: Id,
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
            Command::MoveNodes { .. } => "Move Nodes",
            Command::InsertNode { .. } => "Add Node",
            Command::DeleteNodes { .. } => "Delete Nodes",
            Command::Union { .. } => "Union",
            Command::Subtract { .. } => "Subtract",
            Command::SplitApart { .. } => "Split Apart",
            Command::ResizeCanvas { .. } => "Resize Canvas",
            Command::SetMembership { .. } => "Change Board",
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
    #[allow(
        clippy::too_many_lines,
        reason = "one arm per Command; split by family over time (#351)"
    )]
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
            } => {
                let landmass = Landmass {
                    fill: fill.unwrap_or(map.defaults.land_fill),
                    ..Landmass::new(geometry.clone())
                };
                object::create_object(map, *id, *parent, *index, ObjectKind::Landmass(landmass))
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
                object::create_object(map, *id, *parent, *index, ObjectKind::Lake(lake))
            }
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
            } => {
                if *width == 0 || *height == 0 {
                    return Err(CommandError::InvalidCanvasSize {
                        width: *width,
                        height: *height,
                    });
                }
                if !origin.iter().all(|c| c.is_finite()) {
                    return Err(CommandError::InvalidCanvasOrigin(*origin));
                }
                Ok(vec![Op::ResizeCanvas {
                    origin: *origin,
                    width: *width,
                    height: *height,
                }])
            }
            Command::SetMembership { ids, membership } => {
                let mut seen = HashSet::new();
                let mut ops = Vec::with_capacity(ids.len());
                for &id in ids {
                    if !matches!(existing(map, id)?, Node::Object(_)) {
                        return Err(CommandError::NotAnObject(id));
                    }
                    unlocked(map, id)?;
                    if seen.insert(id) {
                        ops.push(set(id, Property::Membership(*membership)));
                    }
                }
                Ok(ops)
            }
            Command::SetFill { ids, fill } => {
                let ids = land::outlined(map, ids)?;
                Ok(ids
                    .into_iter()
                    .map(|id| set(id, Property::Fill(*fill)))
                    .collect())
            }
            Command::SetRoughness { ids, roughness } => {
                if let Some(roughness) = roughness {
                    valid_roughness(roughness)?;
                }
                let ids = land::outlined(map, ids)?;
                Ok(ids
                    .into_iter()
                    .map(|id| set(id, Property::Roughness(roughness.clone())))
                    .collect())
            }
            Command::SetShore { ids, shore } => {
                if let Some(problem) = shore.as_ref().and_then(ShoreStyle::problem) {
                    return Err(CommandError::InvalidShore(problem));
                }
                let ids = land::outlined(map, ids)?;
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
            Command::SetBackground { background } => {
                let texture = &background.texture;
                if !(0.0..=1.0).contains(&texture.strength)
                    || !texture.scale.is_finite()
                    || texture.scale <= 0.0
                {
                    return Err(CommandError::InvalidBackground);
                }
                Ok(vec![Op::SetBackground {
                    background: background.clone(),
                }])
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
