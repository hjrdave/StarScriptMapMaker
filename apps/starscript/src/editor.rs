//! Editor state around one Map: history, Selection, the active Layer and tool, and the
//! message bar. Every change to the Map goes through a Command here.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use ssmap_core::asset::{AssetRef, credit_lines};
use ssmap_core::land::{self, BrushStroke};
use ssmap_core::scatter::{self, ScatterBrush, ScatterStroke, Terrain};
use ssmap_core::{
    Align, Blob, Command, CommandError, DefaultLayers, EmbedAsset, EmbedFont, ExecuteError,
    History, Id, Label, Map, MultiPolygon, Node, NodeRef, ObjectKind, Op, PasteGaps, PickIndex,
    Point, Rect, ResolvedLabel, RoughnessCache, Selection, Stamp, StampIndex, Subtree,
};
use ssmap_fonts::FALLBACK_FAMILY;
use ssmap_render::wgpu;
use ssmap_render::{
    CanvasRenderer, Content, ExportContext, ExportError, ExportJob, ExportStep, Fonts, LabelLayout,
    MapScene, SpriteState, SpriteStore,
};

use crate::clipboard::Clipboard;
use crate::handles::Bounds;
use crate::layer_tree::node_name;
use crate::toast::{Toast, ToastAction, ToastKind, Toasts};
use crate::tools::ToolId;

/// The land brush's starting diameter, in Canvas units.
pub const DEFAULT_BRUSH_SIZE: f64 = 48.0;

/// The name of the Layer the water brush paints into.
pub const WATER_LAYER: &str = "Water";

pub struct Editor {
    pub map: Map,
    /// What the Canvas draws, derived from the Map, and the Label layouts that picking and
    /// handles share with it.
    pub scene: MapScene,
    history: History,
    selection: Selection,
    /// The Layer new Objects go into.
    active_layer: Option<Id>,
    /// Where new Objects go while no Layer is active, as the Map's template names them.
    pub default_layers: DefaultLayers,
    pub active_tool: ToolId,
    /// The land brush's diameter, in Canvas units.
    pub brush_size: f64,
    /// Whether the land brush erases by default; holding the erase modifier gives the other.
    pub erase_default: bool,
    node_edit: Option<NodeEdit>,
    /// Whether the Canvas (the board) is what's selected, rather than layers or Objects.
    board_selected: bool,
    /// Set by a tool while it holds something Escape should drop (a lasso half drawn).
    pub tool_busy: bool,
    /// Set by Escape for the busy tool to act on.
    pub cancel_tool: bool,
    toasts: Toasts,
    /// Set while a gesture's transaction is open.
    gesture_open: bool,
    /// The Selection when the open gesture began, restored if it is cancelled.
    gesture_selection: Selection,
    /// Rebuilt lazily once the Map has changed.
    pick_index: Option<PickIndex>,
    /// Roughened outlines, shared by the screen and picking so they agree.
    roughness: RoughnessCache,
    /// The cache's generation when the Canvas content and picking were last built.
    outlines_seen: u64,
    /// Stamp art, shared with the Canvas renderer. Absent in headless tests.
    sprites: Option<SpriteStore>,
    stamp_index: Option<StampIndex>,
    /// The store's generation when `stamp_index` was last reset.
    sprites_generation: u64,
    /// The Asset the Stamp tool places, chosen in the Asset browser.
    pub selected_asset: Option<AssetRef>,
    /// The scatter brush's settings. Its radius is half the shared `brush_size`.
    pub scatter: ScatterBrush,
    /// What Copy or Cut last took. Paste reads it back.
    clipboard: Option<Clipboard>,
    /// The clipboard as text, until the app mirrors it to the system clipboard.
    clipboard_text: Option<String>,
}

/// What a new Object is, for choosing the Layer it goes into.
#[derive(Clone, Copy)]
enum Placing {
    Land,
    Stamp,
    Label,
}

/// Whether the app bundles a font family.
fn bundled_family(family: &str) -> bool {
    ssmap_fonts::FAMILIES.iter().any(|f| f.name == family)
}

/// A pseudo-random number below `n` (which must be above 0), unseeded: Stamp variants are
/// chosen when placed and then stored, so nothing needs to replay them.
fn random_below(n: usize) -> usize {
    use std::hash::{BuildHasher as _, Hasher as _};
    (std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish() as usize)
        % n
}

/// A pseudo-random seed for a scatter stroke. The Command it produces names every Stamp, so
/// the seed itself never needs to be stored.
fn random_seed() -> u64 {
    use std::hash::{BuildHasher as _, Hasher as _};
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
}

/// The Assets in the Variant group `asset` belongs to; empty when it has none.
fn group_of(library: &ssmap_core::asset::PackLibrary, asset: &AssetRef) -> Vec<AssetRef> {
    let Some((pack, found)) = library.asset(asset) else {
        return Vec::new();
    };
    let Some(group) = &found.variant_group else {
        return Vec::new();
    };
    pack.variant_group(group)
        .filter_map(|member| AssetRef::new(pack.id.clone(), member.id.clone()).ok())
        .collect()
}

/// Node-edit mode: one Landmass or Lake whose stored vertices show as nodes, and the nodes
/// selected among them.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeEdit {
    pub id: Id,
    pub nodes: Vec<NodeRef>,
}

impl Editor {
    pub fn new(map: Map) -> Self {
        let mut history = History::default();
        history.set_recording(true);
        Self {
            map,
            scene: MapScene::new(Fonts::bundled()),
            history,
            selection: Vec::new(),
            board_selected: false,
            active_layer: None,
            default_layers: DefaultLayers::default(),
            active_tool: ToolId::Select,
            scatter: ScatterBrush::default(),
            clipboard: None,
            clipboard_text: None,
            brush_size: DEFAULT_BRUSH_SIZE,
            erase_default: false,
            node_edit: None,
            tool_busy: false,
            cancel_tool: false,
            toasts: Toasts::default(),
            gesture_open: false,
            gesture_selection: Vec::new(),
            pick_index: None,
            roughness: RoughnessCache::new(),
            outlines_seen: 0,
            sprites: None,
            stamp_index: None,
            sprites_generation: 0,
            selected_asset: None,
        }
    }

    /// Where Stamp art comes from: it sizes Stamps for picking and handles.
    pub fn set_sprites(&mut self, sprites: SpriteStore) {
        self.sprites = Some(sprites);
        self.stamp_index = None;
        self.sync_embedded();
        self.prefetch_sprites();
    }

    /// Starts rasterising the art of the Map's Stamps in the background, so a Map that has
    /// just opened shows them as they finish rather than waiting on each at first draw.
    fn prefetch_sprites(&self) {
        let Some(sprites) = &self.sprites else {
            return;
        };
        let map = &self.map;
        let assets: Vec<AssetRef> = map
            .draw_order()
            .into_iter()
            .filter_map(|id| match map.node(id) {
                Some(Node::Object(object)) => match &object.kind {
                    ObjectKind::Stamp(stamp) => Some(stamp.asset.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        sprites.prefetch(&assets);
    }

    /// Points the sprite store at the Assets the Map embeds, which win over the installed
    /// packs'. Run before anything reads sprites, since an Op, Undo or Redo can change them.
    fn sync_embedded(&mut self) {
        if let Some(sprites) = &self.sprites {
            sprites.set_embedded(self.map.embedded());
            // The library can also change under us, when a pack is installed or deleted.
            let generation = sprites.generation();
            if generation != self.sprites_generation {
                self.sprites_generation = generation;
                self.stamp_index = None;
            }
        }
    }

    /// The Stamps whose Asset is neither embedded nor installed, with the Asset's name: the
    /// Canvas draws a box for each and the app writes the name on it.
    pub fn missing_stamps(&self) -> Vec<(Point, f64, String)> {
        let Some(sprites) = &self.sprites else {
            return Vec::new();
        };
        let library = sprites.library();
        let map = &self.map;
        map.draw_order()
            .into_iter()
            .filter_map(|id| match map.node(id) {
                Some(Node::Object(object)) => match &object.kind {
                    ObjectKind::Stamp(stamp) => Some(stamp),
                    _ => None,
                },
                _ => None,
            })
            .filter(|stamp| matches!(sprites.poll(&stamp.asset), SpriteState::Missing))
            .map(|stamp| {
                let name = library
                    .asset(&stamp.asset)
                    .map_or_else(|| stamp.asset.to_string(), |(_, found)| found.name.clone());
                (stamp.position, stamp.scale, name)
            })
            .collect()
    }

    pub fn sprites(&self) -> Option<&SpriteStore> {
        self.sprites.as_ref()
    }

    pub fn selection(&self) -> &[Id] {
        &self.selection
    }

    pub fn active_layer(&self) -> Option<Id> {
        self.active_layer
    }

    /// Replaces the Selection. The active Layer follows it: a selected Layer, or the Layer
    /// holding a selected Object.
    pub fn select(&mut self, ids: Vec<Id>) {
        if self.node_edit.as_ref().is_some_and(|edit| ids != [edit.id]) {
            self.node_edit = None;
        }
        self.selection = ids;
        self.board_selected = false;
        if let Some(&last) = self.selection.last() {
            self.active_layer = match self.map.node(last) {
                Some(Node::Layer(_)) => Some(last),
                _ => self.map.parent(last),
            };
        }
    }

    /// Whether the Canvas itself is selected (by the Art board tool).
    pub fn board_selected(&self) -> bool {
        self.board_selected
    }

    /// Selects the Canvas, which empties the Selection; `false` deselects it.
    pub fn select_board(&mut self, selected: bool) {
        if selected {
            self.select(Vec::new());
        }
        self.board_selected = selected;
    }

    pub fn node_edit(&self) -> Option<&NodeEdit> {
        self.node_edit.as_ref()
    }

    /// The stored outline (not the roughened one) of the Object in node-edit mode.
    pub fn edited_outline(&self) -> Option<&MultiPolygon> {
        match self.map.node(self.node_edit.as_ref()?.id)? {
            Node::Object(object) => object.kind.geometry(),
            Node::Layer(_) => None,
        }
    }

    /// Enters node-edit mode on a Landmass or Lake, selecting it.
    pub fn enter_node_edit(&mut self, id: Id) {
        self.node_edit = None;
        self.select(vec![id]);
        self.node_edit = Some(NodeEdit {
            id,
            nodes: Vec::new(),
        });
    }

    pub fn leave_node_edit(&mut self) {
        self.node_edit = None;
    }

    /// Selects nodes of the Object being edited.
    pub fn select_nodes(&mut self, nodes: Vec<NodeRef>) {
        if let Some(edit) = &mut self.node_edit {
            edit.nodes = nodes;
        }
    }

    /// The shape of the outline being edited: how many points each ring has. Node numbers
    /// stay meaningful while it does not change.
    pub fn edited_shape(&self) -> Vec<usize> {
        self.edited_outline()
            .map(|outline| {
                outline
                    .0
                    .iter()
                    .flat_map(|polygon| polygon.rings().map(<[Point]>::len))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Forgets the node selection unless the outline still has `shape`: a repair may have
    /// renumbered its nodes.
    pub fn keep_nodes_if_shape(&mut self, shape: &[usize]) {
        if self.edited_shape() != shape {
            self.select_nodes(Vec::new());
        }
    }

    /// Adds a node on an edge and selects it. One Undo step.
    pub fn insert_node(&mut self, edge: NodeRef, point: Point) {
        let Some(id) = self.node_edit.as_ref().map(|edit| edit.id) else {
            return;
        };
        if self.run(Command::InsertNode { id, edge, point }) {
            let added = NodeRef {
                index: edge.index + 1,
                ..edge
            };
            let landed = self
                .edited_outline()
                .is_some_and(|outline| outline.node(added) == Some(point));
            self.select_nodes(if landed { vec![added] } else { Vec::new() });
        }
    }

    /// Deletes the selected nodes. One Undo step; refused if a ring would be left with fewer
    /// than three points.
    pub fn delete_selected_nodes(&mut self) {
        let Some(edit) = self.node_edit.clone() else {
            return;
        };
        if edit.nodes.is_empty() {
            return;
        }
        let count = edit.nodes.len();
        let command = Command::DeleteNodes {
            id: edit.id,
            nodes: edit.nodes,
        };
        if self.run(command) {
            self.select_nodes(Vec::new());
            self.info_with_undo(match count {
                1 => "Deleted 1 node".to_string(),
                n => format!("Deleted {n} nodes"),
            });
        }
    }

    /// The topmost unlocked Landmass or Lake drawn at `point`, in Canvas units.
    fn pick_outline(&mut self, point: [f64; 2]) -> Option<Id> {
        self.sync_background_outlines();
        let map = &self.map;
        let index = match &mut self.pick_index {
            Some(index) if index.is_current(map) => index,
            slot => slot.insert(PickIndex::build(map, &mut self.roughness)),
        };
        index.pick(point)
    }

    /// The topmost unlocked Stamp whose Silhouette is under `point`, in Canvas units.
    fn pick_stamp(&mut self, point: [f64; 2]) -> Option<Id> {
        self.sync_embedded();
        let sprites = self.sprites.as_ref()?;
        let map = &self.map;
        let index = match &mut self.stamp_index {
            Some(index) if index.is_current(map) => index,
            slot => slot.insert(StampIndex::build(map, |asset| sprites.shape(asset))),
        };
        index.pick(point, |asset, uv| sprites.contains(asset, uv))
    }

    /// The Stamp with id `id`, if it is one.
    pub fn stamp(&self, id: Id) -> Option<&Stamp> {
        match self.map.node(id) {
            Some(Node::Object(object)) => match &object.kind {
                ObjectKind::Stamp(stamp) => Some(stamp),
                _ => None,
            },
            _ => None,
        }
    }

    /// The box around a Stamp's placed art, if its Asset is installed.
    pub fn stamp_bounds(&self, id: Id) -> Option<Rect> {
        let stamp = self.stamp(id)?;
        Some(stamp.bounds(&self.sprites.as_ref()?.shape(&stamp.asset)?))
    }

    /// Computes every roughened outline on a thread of its own, so a Map that has just opened
    /// shows at once with its stored outlines. `repaint` is called as each outline completes.
    pub fn start_background_roughness(&mut self, repaint: impl Fn() + Send + 'static) {
        let rebuild = self.roughness.rebuild(&self.map);
        if rebuild.is_empty() {
            return;
        }
        std::thread::spawn(move || rebuild.run(repaint));
    }

    /// Whether roughened outlines are still being computed in the background.
    pub fn outlines_rebuilding(&self) -> bool {
        self.roughness.is_rebuilding()
    }

    /// Drops what was built from placeholder outlines once the background ones have landed.
    fn sync_background_outlines(&mut self) {
        let generation = self.roughness.generation();
        if generation != self.outlines_seen {
            self.outlines_seen = generation;
            self.scene.invalidate();
            self.pick_index = None;
        }
    }

    /// What the Canvas draws for the Map, rebuilt only once it has changed. Drops roughened
    /// outlines of Objects that are gone.
    pub fn scene_content(&mut self) -> Content {
        self.sync_embedded();
        self.sync_background_outlines();
        self.roughness.retain(&self.map);
        self.scene.content(&self.map, &mut self.roughness)
    }

    /// Advances an export by one step, drawing from this Editor's Map and caches.
    pub fn step_export(
        &mut self,
        job: &mut ExportJob,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut CanvasRenderer,
    ) -> Result<ExportStep, ExportError> {
        self.sync_embedded();
        self.sync_background_outlines();
        self.roughness.retain(&self.map);
        job.step(&mut ExportContext {
            device,
            queue,
            renderer,
            scene: &mut self.scene,
            map: &self.map,
            roughness: &mut self.roughness,
        })
    }

    /// The selected Landmasses and Lakes, which move and resize with the handles.
    pub fn selected_outlines(&self) -> Vec<Id> {
        self.selection
            .iter()
            .copied()
            .filter(|&id| {
                matches!(self.map.node(id), Some(Node::Object(o)) if o.kind.geometry().is_some())
            })
            .collect()
    }

    pub fn is_selected(&self, id: Id) -> bool {
        self.selection.contains(&id)
    }

    /// Runs one Command as its own Undo step. A refusal shows in the status bar.
    pub fn run(&mut self, command: Command) -> bool {
        let selection = self.selection.clone();
        self.run_selecting(command, selection)
    }

    /// Runs one Command as its own Undo step and selects `selection` afterwards.
    pub fn run_selecting(&mut self, command: Command, selection: Selection) -> bool {
        if self.gesture_open {
            return self.gesture(command);
        }
        let begun = self.history.begin(command.label(), self.selection.clone());
        debug_assert!(begun.is_ok(), "no transaction is open outside gestures");
        match self.history.execute(&mut self.map, &command) {
            Ok(()) => {
                self.history
                    .commit(selection.clone())
                    .expect("opened above");
                self.restore(selection);
                true
            }
            Err(err) => {
                self.history.rollback(&mut self.map).expect("opened above");
                self.refused(&err);
                false
            }
        }
    }

    /// Opens a gesture: Commands until `end_gesture` make one Undo step, applied live.
    pub fn begin_gesture(&mut self, label: &str) {
        if self.gesture_open {
            return;
        }
        self.history
            .begin(label, self.selection.clone())
            .expect("no other transaction is open");
        self.gesture_open = true;
        self.gesture_selection = self.selection.clone();
    }

    /// Runs a Command inside the open gesture, or as its own step if none is open.
    pub fn gesture(&mut self, command: Command) -> bool {
        if !self.gesture_open {
            return self.run(command);
        }
        match self.history.execute(&mut self.map, &command) {
            Ok(()) => true,
            Err(err) => {
                self.refused(&err);
                false
            }
        }
    }

    pub fn end_gesture(&mut self) {
        if std::mem::take(&mut self.gesture_open) {
            self.history
                .commit(self.selection.clone())
                .expect("the gesture's transaction is open");
        }
    }

    /// What a land brush stroke started now would be called in the undo history: it paints or
    /// erases a selected Lake, and otherwise land.
    pub fn brush_label(&self, erase: bool) -> &'static str {
        let lake = self.selection.iter().any(|&id| {
            matches!(self.map.node(id), Some(Node::Object(o)) if matches!(o.kind, ObjectKind::Lake(_)))
        });
        match (erase, lake) {
            (false, false) => "Paint Land",
            (false, true) => "Paint Lake",
            (true, false) => "Erase Land",
            (true, true) => "Erase Lake",
        }
    }

    /// Finishes the land brush stroke whose gesture is open: runs the one Command the brush
    /// rules choose ([`land::brush_command`]) and commits it as the gesture's Undo step. A new
    /// Landmass becomes the Selection, so the next stroke merges into it. A stroke that changes
    /// nothing, or is refused, leaves no Undo step.
    pub fn finish_brush_stroke(&mut self, stroke: BrushStroke) {
        let new_id = Id::new();
        let layer = self.target_layer(Placing::Land);
        let planned = land::brush_command(
            &self.map,
            &mut self.roughness,
            &self.selection,
            layer,
            stroke,
            new_id,
        );
        match planned {
            Ok(Some(command)) => {
                let creates = matches!(command, Command::CreateLandmass { .. });
                if self.gesture(command) {
                    if creates {
                        self.select(vec![new_id]);
                    }
                    self.end_gesture();
                } else {
                    self.cancel_gesture();
                }
            }
            Ok(None) => self.cancel_gesture(),
            Err(err) => {
                self.cancel_gesture();
                let text = err.to_string();
                let mut chars = text.chars();
                let first = chars.next().map(|c| c.to_uppercase().to_string());
                self.error(first.unwrap_or_default() + chars.as_str());
            }
        }
    }

    /// The area, with its rough shoreline, that finishing `stroke` now would add or cut away,
    /// for the live preview. It plans the same Command a release would run
    /// ([`land::brush_command`], or [`land::water_command`]), so the two can't disagree. `None`
    /// when the stroke changes nothing.
    pub fn brush_preview(&mut self, stroke: BrushStroke, water: bool) -> Option<MultiPolygon> {
        let new_id = Id::new();
        let (add, region) = (!stroke.erase, stroke.region.clone());
        let planned = if water {
            let layer = self.water_layer();
            land::water_command(
                &self.map,
                &mut self.roughness,
                &self.selection,
                layer,
                stroke,
                new_id,
            )
        } else {
            let layer = self.target_layer(Placing::Land);
            land::brush_command(
                &self.map,
                &mut self.roughness,
                &self.selection,
                layer,
                stroke,
                new_id,
            )
        };
        match planned {
            Ok(Some(command)) => land::preview_area(&self.map, &mut self.roughness, &command),
            // A first water stroke has no Layer yet; it will create one and a Lake in it.
            Err(_) if add => land::preview_area(
                &self.map,
                &mut self.roughness,
                &Command::CreateLake {
                    id: new_id,
                    parent: new_id,
                    index: None,
                    geometry: region,
                    fill: None,
                },
            ),
            _ => None,
        }
    }

    /// What a water brush stroke would be called in the undo history.
    pub fn water_label(erase: bool) -> &'static str {
        if erase { "Erase Water" } else { "Paint Water" }
    }

    /// The top-level Layer the water brush paints into: the Map's "Water" Layer, if it has one.
    fn water_layer(&self) -> Option<Id> {
        self.map.children(None).iter().copied().find(|&id| {
            matches!(self.map.node(id), Some(Node::Layer(layer)) if layer.name == WATER_LAYER)
        })
    }

    /// Finishes the water brush stroke whose gesture is open, as [`Self::finish_brush_stroke`]
    /// does for land: one Command ([`land::water_command`]) as the gesture's Undo step. The
    /// first stroke that needs it also creates the "Water" Layer, in the same Undo step, so
    /// water has its own Layer to rename, lock, hide and reorder. A new Lake becomes the
    /// Selection, so the next stroke extends it.
    pub fn finish_water_stroke(&mut self, stroke: BrushStroke) {
        let new_id = Id::new();
        let existing = self.water_layer();
        let mut planned = land::water_command(
            &self.map,
            &mut self.roughness,
            &self.selection,
            existing,
            stroke.clone(),
            new_id,
        );
        if planned == Err(land::BrushError::NoLayer) {
            let id = Id::new();
            let created = self.gesture(Command::CreateLayer {
                id,
                parent: None,
                index: None,
                name: WATER_LAYER.to_owned(),
            });
            if !created {
                self.cancel_gesture();
                return;
            }
            planned = land::water_command(
                &self.map,
                &mut self.roughness,
                &self.selection,
                Some(id),
                stroke,
                new_id,
            );
        }
        match planned {
            Ok(Some(command)) => {
                let creates = matches!(command, Command::CreateLake { .. });
                if self.gesture(command) {
                    if creates {
                        self.select(vec![new_id]);
                    }
                    self.end_gesture();
                } else {
                    self.cancel_gesture();
                }
            }
            Ok(None) => self.cancel_gesture(),
            Err(err) => {
                self.cancel_gesture();
                self.error(err.to_string());
            }
        }
    }

    /// The selected Landmasses, topmost first.
    fn selected_landmasses(&self) -> Vec<Id> {
        let order = self.map.draw_order();
        let mut ids: Vec<Id> = self
            .selection
            .iter()
            .copied()
            .filter(|&id| {
                matches!(self.map.node(id), Some(Node::Object(o)) if matches!(o.kind, ObjectKind::Landmass(_)))
            })
            .collect();
        ids.sort_by_key(|&id| std::cmp::Reverse(order.iter().position(|&o| o == id)));
        ids
    }

    /// Whether Union and Subtract have enough to work on: two or more selected Landmasses.
    pub fn can_combine(&self) -> bool {
        self.selected_landmasses().len() >= 2
    }

    /// Whether a selected Landmass has more than one part.
    pub fn can_split(&self) -> bool {
        self.selected_landmasses()
            .iter()
            .any(|&id| self.part_count(id) > 1)
    }

    fn part_count(&self, id: Id) -> usize {
        match self.map.node(id) {
            Some(Node::Object(o)) => o.kind.geometry().map_or(0, |g| g.0.len()),
            _ => 0,
        }
    }

    /// Merges the selected Landmasses into the topmost one, which stays selected.
    pub fn union_selection(&mut self) {
        let ids = self.selected_landmasses();
        if let Some(&keeper) = ids.first() {
            self.run_selecting(Command::Union { ids }, vec![keeper]);
        }
    }

    /// Cuts the topmost selected Landmass out of the others.
    pub fn subtract_selection(&mut self) {
        let ids = self.selected_landmasses();
        let rest: Vec<Id> = ids.iter().skip(1).copied().collect();
        if !ids.is_empty() {
            self.run_selecting(Command::Subtract { ids }, rest);
        }
    }

    /// Makes each part of every selected Landmass a Landmass of its own, as one Undo step.
    pub fn split_selection(&mut self) {
        let targets: Vec<Id> = self
            .selected_landmasses()
            .into_iter()
            .filter(|&id| self.part_count(id) > 1)
            .collect();
        if targets.is_empty() {
            return;
        }
        let mut selection = Vec::new();
        self.begin_gesture("Split Apart");
        for id in targets {
            let new_ids: Vec<Id> = (1..self.part_count(id)).map(|_| Id::new()).collect();
            selection.push(id);
            selection.extend(&new_ids);
            if !self.gesture(Command::SplitApart { id, new_ids }) {
                self.cancel_gesture();
                return;
            }
        }
        self.select(selection);
        self.end_gesture();
    }

    /// Escape: undoes everything the open gesture did, Selection included.
    pub fn cancel_gesture(&mut self) {
        if std::mem::take(&mut self.gesture_open) {
            self.history
                .rollback(&mut self.map)
                .expect("the gesture's transaction is open");
            let selection = std::mem::take(&mut self.gesture_selection);
            self.restore(selection);
        }
    }

    /// Replaces everything the open gesture has done so far with `command`. Drags use it to
    /// re-issue their whole effect each frame, so the Undo step holds one Command and nothing
    /// drifts. Without an open gesture it runs `command` as its own step.
    pub fn gesture_replace(&mut self, command: Command) -> bool {
        if !self.gesture_open {
            return self.run(command);
        }
        self.history
            .rewind(&mut self.map)
            .expect("the gesture's transaction is open");
        self.gesture(command)
    }

    pub fn gesture_open(&self) -> bool {
        self.gesture_open
    }

    pub fn can_undo(&self) -> bool {
        !self.gesture_open && self.history.undo_len() > 0
    }

    pub fn can_redo(&self) -> bool {
        !self.gesture_open && self.history.redo_len() > 0
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.history.undo_label()
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.history.redo_label()
    }

    /// Every step in the undo history, oldest first, and how many of them are applied. The
    /// steps past that count are the ones undo moved over and redo can bring back.
    pub fn history_steps(&self) -> (Vec<String>, usize) {
        let applied = self.history.undo_len();
        let steps = self
            .history
            .undo_labels()
            .chain(self.history.redo_labels())
            .map(str::to_owned)
            .collect();
        (steps, applied)
    }

    /// Moves the Map to the point where `applied` steps are applied, by undoing or redoing
    /// the steps between. Later steps stay available to redo.
    pub fn jump_to_history(&mut self, applied: usize) {
        self.end_gesture();
        let total = self.history.undo_len() + self.history.redo_len();
        let applied = applied.min(total);
        // Each pass must move the history, or something is wrong and the jump stops.
        while self.history.undo_len() > applied {
            let before = self.history.undo_len();
            self.undo();
            if self.history.undo_len() == before {
                break;
            }
        }
        while self.history.undo_len() < applied {
            let before = self.history.undo_len();
            self.redo();
            if self.history.undo_len() == before {
                break;
            }
        }
    }

    pub fn undo(&mut self) {
        self.end_gesture();
        if let Ok(Some(selection)) = self.history.undo(&mut self.map) {
            self.restore(selection);
        }
    }

    pub fn redo(&mut self) {
        self.end_gesture();
        if let Ok(Some(selection)) = self.history.redo(&mut self.map) {
            self.restore(selection);
        }
    }

    /// Embeds every Asset and bundled font the Map uses that it doesn't embed yet, as one Undo
    /// step (ADR 0005). Assets no installed pack provides are skipped, and an Asset already
    /// embedded keeps its embedded copy. Returns whether anything was embedded.
    pub fn embed_used_assets(&mut self) -> bool {
        let mut assets = Vec::new();
        if let Some(library) = self
            .sprites
            .as_ref()
            .map(|sprites| sprites.library().clone())
        {
            for reference in self.map.used_assets() {
                if self.map.embedded().assets.contains_key(&reference) {
                    continue;
                }
                let Some((pack, asset)) = library.asset(&reference) else {
                    continue;
                };
                match EmbedAsset::read(pack, asset) {
                    Ok(embed) => assets.push(embed),
                    Err(err) => self.error(format!("Couldn’t embed {reference}: {err}")),
                }
            }
        }
        let mut families = self.map.used_font_families();
        if !families.is_empty() {
            families.insert(FALLBACK_FAMILY.to_owned());
        }
        let fonts: Vec<EmbedFont> = ssmap_fonts::FONTS
            .iter()
            .filter(|font| families.contains(font.family))
            .filter(|font| !self.map.embedded().fonts.contains_key(font.file))
            .filter_map(|font| {
                let family = ssmap_fonts::FAMILIES
                    .iter()
                    .find(|f| f.name == font.family)?;
                Some(EmbedFont {
                    family: font.family.to_owned(),
                    file: font.file.to_owned(),
                    licence: family.licence.to_owned(),
                    data: Blob::new(font.data),
                })
            })
            .collect();
        if assets.is_empty() && fonts.is_empty() {
            return false;
        }
        self.run(Command::EmbedAssets { assets, fonts })
    }

    /// Re-pulls the art of every embedded Asset that an installed pack now provides differently,
    /// as one Undo step ("Update from pack"). Returns whether anything changed.
    pub fn update_from_packs(&mut self) -> bool {
        let Some(library) = self
            .sprites
            .as_ref()
            .map(|sprites| sprites.library().clone())
        else {
            return false;
        };
        let mut assets = Vec::new();
        let mut errors = Vec::new();
        for (reference, current) in &self.map.embedded().assets {
            let Some((pack, asset)) = library.asset(reference) else {
                continue;
            };
            match EmbedAsset::read(pack, asset) {
                Ok(embed) if embed.record != *current => assets.push(embed),
                Ok(_) => {}
                Err(err) => errors.push(format!("Couldn’t update {reference}: {err}")),
            }
        }
        for message in errors {
            self.error(message);
        }
        if assets.is_empty() {
            return false;
        }
        self.run(Command::UpdateFromPack { assets })
    }

    /// Takes the Ops that committed steps, undo and redo applied since the last call, for the
    /// Recovery journal.
    pub fn take_applied(&mut self) -> Vec<Op> {
        self.history.take_applied()
    }

    /// Creates a Layer above the active one (or on top), selects it and returns its id.
    pub fn new_layer(&mut self) -> Option<Id> {
        self.new_container("Layer")
    }

    /// Creates an empty Group (a Layer named "Group N") where `new_layer` would put a Layer.
    pub fn new_group(&mut self) -> Option<Id> {
        self.new_container("Group")
    }

    fn new_container(&mut self, kind: &str) -> Option<Id> {
        let (parent, index) = match self.active_layer.filter(|&id| self.map.contains(id)) {
            Some(active) => {
                let parent = self.map.parent(active);
                let position = self
                    .map
                    .children(parent)
                    .iter()
                    .position(|&id| id == active)
                    .expect("a node is listed by its parent");
                (parent, Some(position + 1))
            }
            None => (None, None),
        };
        let id = Id::new();
        let name = format!("{kind} {}", self.layer_count() + 1);
        let command = Command::CreateLayer {
            id,
            parent,
            index,
            name,
        };
        self.run_selecting(command, vec![id]).then_some(id)
    }

    /// Wraps the selection in a new Group and selects it. One Undo step; refused with a
    /// toast when the selection doesn't share a parent.
    pub fn group_selection(&mut self) -> Option<Id> {
        let ids = self.selected_roots();
        if ids.is_empty() {
            return None;
        }
        let id = Id::new();
        let name = format!("Group {}", self.layer_count() + 1);
        self.run_selecting(Command::CreateGroup { id, ids, name }, vec![id])
            .then_some(id)
    }

    /// Dissolves the selected Layers and Groups, selecting what was inside. One Undo step.
    pub fn ungroup_selection(&mut self) -> bool {
        let ids: Vec<Id> = self
            .selected_roots()
            .into_iter()
            .filter(|&id| self.map.node(id).is_some_and(Node::is_layer))
            .collect();
        if ids.is_empty() {
            return false;
        }
        let freed: Vec<Id> = ids
            .iter()
            .flat_map(|&id| self.map.children(Some(id)).to_vec())
            .collect();
        self.run_selecting(Command::Ungroup { ids }, freed)
    }

    /// The Objects that move or transform with the selection because of Links. When a linked
    /// Layer or Object is locked nothing may move, so this says so in the status bar and
    /// returns `None`.
    pub fn linked_movers(&mut self) -> Option<Vec<Id>> {
        match self.map.linked_movers(&self.selection) {
            Ok(movers) => Some(movers),
            Err(err) => {
                self.refused(&err.into());
                None
            }
        }
    }

    /// Whether the selection holds two or more Layers or Objects to link.
    pub fn can_link(&self) -> bool {
        self.selection
            .iter()
            .filter(|&&id| self.map.contains(id))
            .count()
            > 1
    }

    /// Links the selected Layers and Objects so they move and transform together, wherever
    /// they sit in the tree. One Undo step.
    pub fn link_selection(&mut self) -> bool {
        if !self.can_link() {
            return false;
        }
        let ids = self.selection.clone();
        self.run(Command::Link { id: Id::new(), ids })
    }

    /// Whether any selected Layer or Object is linked.
    pub fn can_unlink(&self) -> bool {
        self.selection.iter().any(|&id| self.map.is_linked(id))
    }

    /// Takes the selected Layers and Objects out of their Link sets. One Undo step.
    pub fn unlink_selection(&mut self) -> bool {
        let ids: Vec<Id> = self
            .selection
            .iter()
            .copied()
            .filter(|&id| self.map.is_linked(id))
            .collect();
        !ids.is_empty() && self.run(Command::Unlink { ids })
    }

    pub fn delete_selection(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        let ids = self.selection.clone();
        let count = ids.len();
        if self.run_selecting(Command::Delete { ids }, Vec::new()) {
            self.info_with_undo(match count {
                1 => "Deleted 1 item".to_string(),
                n => format!("Deleted {n} items"),
            });
        }
    }

    /// A Duplicate of `ids` with fresh ids, and the ids of the copies at the top of each
    /// duplicated item. Ids inside another listed id come with it.
    pub fn duplicate_command(&self, ids: &[Id], offset: [f64; 2]) -> (Command, Vec<Id>) {
        let listed: HashSet<Id> = ids.iter().copied().collect();
        let mut seen = HashSet::new();
        let mut new_ids = Vec::new();
        let mut tops = Vec::new();
        for &id in ids {
            let mut inside = false;
            let mut above = self.map.parent(id);
            while let Some(ancestor) = above {
                inside |= listed.contains(&ancestor);
                above = self.map.parent(ancestor);
            }
            if inside || !seen.insert(id) {
                continue;
            }
            tops.push(Id::new());
            new_ids.push(tops[tops.len() - 1]);
            let mut pending = self.map.children(Some(id)).to_vec();
            while let Some(child) = pending.pop() {
                new_ids.push(Id::new());
                pending.extend(self.map.children(Some(child)));
            }
        }
        let command = Command::Duplicate {
            ids: ids.to_vec(),
            new_ids,
            offset,
        };
        (command, tops)
    }

    /// Duplicates the selection in place, just above the originals, and selects the copies.
    /// One Undo step. Returns the copies' ids.
    pub fn duplicate_selection(&mut self) -> Option<Vec<Id>> {
        if self.selection.is_empty() || self.gesture_open {
            return None;
        }
        let ids = self.selection.clone();
        let only_layers = ids
            .iter()
            .all(|&id| self.map.node(id).is_some_and(Node::is_layer));
        let (command, copies) = self.duplicate_command(&ids, [0.0, 0.0]);
        self.begin_gesture(if only_layers {
            "Duplicate Layer"
        } else {
            "Duplicate Objects"
        });
        if !self.gesture(command) {
            self.cancel_gesture();
            return None;
        }
        self.select(copies.clone());
        self.end_gesture();
        Some(copies)
    }

    /// Whether Paste has anything to paste.
    pub fn has_clipboard(&self) -> bool {
        self.clipboard.is_some()
    }

    /// Makes `clipboard` what Paste pastes: a Copy or Cut in another tab or Map.
    pub fn set_clipboard(&mut self, clipboard: Clipboard) {
        self.clipboard = Some(clipboard);
    }

    /// What Paste would need that this Map lacks. `None` when nothing is copied.
    pub fn paste_gaps(&mut self) -> Option<PasteGaps> {
        self.sync_embedded();
        let clipboard = self.clipboard.as_ref()?;
        Some(self.map.paste_gaps(
            &clipboard.subtrees,
            |asset| {
                self.sprites
                    .as_ref()
                    .is_none_or(|sprites| sprites.library().asset(asset).is_some())
            },
            bundled_family,
        ))
    }

    /// The text to mirror to the system clipboard after a Copy or Cut, once.
    pub fn take_clipboard_text(&mut self) -> Option<String> {
        self.clipboard_text.take()
    }

    /// The selected branches, in stacking order, without any that sit inside another selected
    /// branch.
    fn selected_roots(&self) -> Vec<Id> {
        let listed: HashSet<Id> = self.selection.iter().copied().collect();
        let mut roots: Vec<(Vec<usize>, Id)> = Vec::new();
        for &id in &self.selection {
            if !self.map.contains(id) || roots.iter().any(|(_, r)| *r == id) {
                continue;
            }
            let inside = std::iter::successors(self.map.parent(id), |&p| self.map.parent(p))
                .any(|ancestor| listed.contains(&ancestor));
            if !inside {
                roots.push((self.tree_path(id), id));
            }
        }
        roots.sort();
        roots.into_iter().map(|(_, id)| id).collect()
    }

    /// The sibling positions from the root down to `id`: sorting by it gives stacking order.
    fn tree_path(&self, id: Id) -> Vec<usize> {
        let mut path = Vec::new();
        let mut at = id;
        loop {
            let parent = self.map.parent(at);
            path.push(
                self.map
                    .children(parent)
                    .iter()
                    .position(|&c| c == at)
                    .unwrap_or(0),
            );
            match parent {
                Some(parent) => at = parent,
                None => break,
            }
        }
        path.reverse();
        path
    }

    fn subtree(&self, id: Id) -> Subtree {
        Subtree {
            id,
            node: self.map.node(id).expect("listed ids exist").clone(),
            children: self
                .map
                .children(Some(id))
                .iter()
                .map(|&child| self.subtree(child))
                .collect(),
        }
    }

    /// Copies the selected Layers and Objects, with everything inside them, locked ones too.
    /// Returns false when nothing is selected.
    pub fn copy_selection(&mut self) -> bool {
        let roots = self.selected_roots();
        if roots.is_empty() {
            return false;
        }
        let mut objects = Vec::new();
        let mut pending = roots.clone();
        while let Some(id) = pending.pop() {
            match self.map.node(id) {
                Some(Node::Object(_)) => objects.push(id),
                _ => pending.extend(self.map.children(Some(id))),
            }
        }
        // A Stamp whose art is not installed has no box, so it counts as its position.
        let mut points: Vec<[f64; 2]> = objects
            .iter()
            .filter(|&&id| self.stamp(id).is_some() && self.stamp_bounds(id).is_none())
            .filter_map(|&id| self.stamp(id).map(|stamp| stamp.position))
            .collect();
        if let Some(bounds) = Bounds::of_ids(self, &objects) {
            points.extend([bounds.min, bounds.max]);
        }
        let centre = Bounds::around(&points).map(|bounds| bounds.centre());
        let subtrees = roots.iter().map(|&id| self.subtree(id)).collect();
        let clipboard = Clipboard::new(subtrees, centre);
        self.clipboard_text = Some(clipboard.to_text());
        self.clipboard = Some(clipboard);
        true
    }

    /// Copies the selection, then removes the originals as one Undo step. Locked Layers and
    /// Objects are copied but stay.
    pub fn cut_selection(&mut self) {
        if self.gesture_open || !self.copy_selection() {
            return;
        }
        let roots = self.selected_roots();
        let removable: Vec<Id> = roots
            .iter()
            .copied()
            .filter(|&id| !self.branch_locked(id))
            .collect();
        if removable.len() < roots.len() {
            self.info("Locked layers were copied but not cut");
        }
        if !removable.is_empty() {
            self.run_selecting(Command::Delete { ids: removable }, Vec::new());
        }
    }

    /// Whether `id`, a Layer above it or anything inside it is locked.
    fn branch_locked(&self, id: Id) -> bool {
        let mut stack = vec![id];
        self.map.is_locked(id) || {
            let mut found = false;
            while let Some(at) = stack.pop() {
                found |= self.map.node(at).is_some_and(Node::locked);
                stack.extend(self.map.children(Some(at)));
            }
            found
        }
    }

    /// Pastes what Copy or Cut took, as one Undo step, with the middle of the pasted Objects
    /// at `at` (Canvas units). It goes above the selected Layer, or on top when nothing is
    /// selected, and the pasted items become the Selection. Returns their ids.
    pub fn paste(&mut self, at: [f64; 2]) -> Option<Vec<Id>> {
        if self.gesture_open {
            return None;
        }
        let mut clipboard = self.clipboard.clone()?;
        let (parent, index) = self.paste_destination(clipboard.has_objects)?;
        self.map
            .fit_for_paste(&mut clipboard.subtrees, bundled_family, FALLBACK_FAMILY);
        let mut new_ids = Vec::new();
        let mut tops = Vec::new();
        for root in &clipboard.subtrees {
            tops.push(Id::new());
            new_ids.push(tops[tops.len() - 1]);
            let mut pending: Vec<&Subtree> = root.children.iter().rev().collect();
            while let Some(tree) = pending.pop() {
                new_ids.push(Id::new());
                pending.extend(tree.children.iter().rev());
            }
        }
        let offset = clipboard
            .centre
            .map_or([0.0, 0.0], |c| [at[0] - c[0], at[1] - c[1]]);
        let command = Command::Paste {
            parent,
            index,
            subtrees: clipboard.subtrees,
            new_ids,
            offset,
        };
        self.run_selecting(command, tops.clone()).then_some(tops)
    }

    /// The Layer (`None` for the root) and position pasted items go in. Layers go just above
    /// the selected item; Objects need a Layer, so they go on top of the selected Layer, or
    /// of the Layer holding the selected Object, or of the active Layer.
    fn paste_destination(&mut self, has_objects: bool) -> Option<(Option<Id>, Option<usize>)> {
        let selected = self.selected_roots().last().copied();
        if !has_objects {
            return Some(match selected {
                Some(id) => {
                    let parent = self.map.parent(id);
                    let index = self.map.children(parent).iter().position(|&c| c == id)? + 1;
                    (parent, Some(index))
                }
                None => (None, None),
            });
        }
        let layer = match selected.and_then(|id| self.map.node(id).map(|n| (id, n.is_layer()))) {
            Some((id, true)) => Some(id),
            Some((id, false)) => self.map.parent(id),
            None => self.active_layer.filter(|&id| self.map.contains(id)),
        };
        match layer {
            Some(layer) => Some((Some(layer), None)),
            None => {
                self.info("Select a layer to paste into");
                None
            }
        }
    }

    /// The Objects directly on the active Layer.
    pub fn active_layer_objects(&self) -> Vec<Id> {
        let Some(layer) = self.active_layer.filter(|&id| self.map.contains(id)) else {
            return Vec::new();
        };
        self.map
            .children(Some(layer))
            .iter()
            .copied()
            .filter(|&id| matches!(self.map.node(id), Some(Node::Object(_))))
            .collect()
    }

    /// Select All: selects every Object on the active Layer.
    pub fn select_all(&mut self) {
        let objects = self.active_layer_objects();
        if !objects.is_empty() {
            self.select(objects);
        }
    }

    /// Deselect: empties the Selection; the active Layer stays.
    pub fn deselect(&mut self) {
        self.select(Vec::new());
    }

    /// Deletes the active Layer with what it holds. One Undo step.
    pub fn delete_active_layer(&mut self) {
        if let Some(layer) = self.active_layer.filter(|&id| self.map.contains(id)) {
            self.select(vec![layer]);
            self.delete_selection();
        }
    }

    /// The newest toast still showing.
    pub fn notice(&self) -> Option<&Toast> {
        self.toasts.items().last()
    }

    /// This tab's toasts, for drawing and expiring. Actions the history has moved past are
    /// dropped first.
    pub fn toasts_mut(&mut self) -> &mut Toasts {
        let (open, depth) = (self.gesture_open, self.history.undo_len());
        self.toasts.retain_actions(|action| match action {
            ToastAction::Undo { steps } => !open && depth == steps,
        });
        &mut self.toasts
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.toasts
            .push(text, ToastKind::Info, None, Instant::now());
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.toasts
            .push(text, ToastKind::Error, None, Instant::now());
    }

    /// Tells the user about a Command they might regret, with an Undo on the toast.
    fn info_with_undo(&mut self, text: impl Into<String>) {
        let action = ToastAction::Undo {
            steps: self.history.undo_len(),
        };
        self.toasts
            .push(text, ToastKind::Info, Some(action), Instant::now());
    }

    /// Runs the action a toast carried, if it still applies.
    pub fn run_toast_action(&mut self, action: ToastAction) {
        match action {
            ToastAction::Undo { steps } => {
                if self.history.undo_len() == steps {
                    self.undo();
                }
            }
        }
    }

    /// Shows why a Command was refused, naming Layers and Objects the way the tree does.
    fn refused(&mut self, err: &ExecuteError) {
        let name = |id: Id| format!("“{}”", node_name(&self.map, id));
        let text = match err {
            ExecuteError::Command(CommandError::Locked(id)) => format!("{} is locked", name(*id)),
            ExecuteError::Command(CommandError::InLockedLayer { target, layer }) => {
                format!("{} is inside locked Layer {}", name(*target), name(*layer))
            }
            ExecuteError::Command(CommandError::ContainsLocked { target, locked }) => {
                format!("{} contains locked {}", name(*target), name(*locked))
            }
            ExecuteError::Command(CommandError::LinkedLocked { target, locked }) => format!(
                "{} is linked to locked {}, so nothing linked to it can move. Unlock it or unlink it first",
                name(*target),
                name(*locked)
            ),
            other => other.to_string(),
        };
        self.error(text);
    }

    /// The Label with id `id`, if it is one.
    pub fn label(&self, id: Id) -> Option<&Label> {
        match self.map.node(id) {
            Some(Node::Object(object)) => match &object.kind {
                ObjectKind::Label(label) => Some(label),
                _ => None,
            },
            _ => None,
        }
    }

    /// A Label with its style applied, and its shaped text.
    pub fn label_layout(&mut self, id: Id) -> Option<(ResolvedLabel, Arc<LabelLayout>)> {
        let label = self.map.resolved_label(id)?;
        let layout = self.scene.label_layout(id, &label);
        Some((label, layout))
    }

    /// The topmost Object under a Canvas point that can be selected there: visible, and
    /// neither locked nor in a locked Layer.
    pub fn pick(&mut self, point: [f64; 2]) -> Option<Id> {
        // The index finds the topmost Landmass or Lake whose roughened fill is under the
        // point; Labels are weighed against it in draw order, so what shows on top is
        // what's picked.
        let outline = self.pick_outline(point);
        let stamp = self.pick_stamp(point);
        for id in self.map.draw_order().into_iter().rev() {
            if Some(id) == outline || Some(id) == stamp {
                return Some(id);
            }
            if !self.map.is_locked(id)
                && let Some((label, layout)) = self.label_layout(id)
                && layout.contains(&label, point)
            {
                return Some(id);
            }
        }
        None
    }

    /// Opens an "Add Label" gesture and places a Label at `position` in the active Layer,
    /// creating a Layer if the Map has none. The gesture stays open for typing the text.
    pub fn place_label(&mut self, position: [f64; 2]) -> Option<Id> {
        self.end_gesture();
        self.begin_gesture("Add Label");
        let Some(parent) = self.placement_layer(Placing::Label) else {
            self.cancel_gesture();
            return None;
        };
        let id = Id::new();
        let label = Label {
            text: "Label".into(),
            position,
            ..Label::default()
        };
        if !self.gesture(Command::CreateLabel {
            id,
            parent,
            index: None,
            label: Box::new(label),
        }) {
            self.cancel_gesture();
            return None;
        }
        self.select(vec![id]);
        Some(id)
    }

    /// The credit lines for the attribution-required Assets the Map uses; empty when none, or
    /// when no packs are loaded.
    pub fn credit_lines(&self) -> Vec<String> {
        let Some(sprites) = self.sprites.as_ref() else {
            return Vec::new();
        };
        let assets = sprites.library().attribution_required_assets(&self.map);
        credit_lines(&assets)
    }

    /// Adds the credit lines to the Map as one left-aligned Label near the bottom-left corner
    /// of the Canvas, as one Undo step ("Add Credits Label"), and selects it. Returns `None`
    /// when the Map has nothing to credit.
    pub fn add_credits_label(&mut self) -> Option<Id> {
        let lines = self.credit_lines();
        if lines.is_empty() {
            return None;
        }
        self.end_gesture();
        self.begin_gesture("Add Credits Label");
        let Some(parent) = self.placement_layer(Placing::Label) else {
            self.cancel_gesture();
            return None;
        };
        let (width, height) = (
            f64::from(self.map.canvas.width),
            f64::from(self.map.canvas.height),
        );
        let size = (width.min(height) / 80.0).max(8.0);
        let line_height = 1.3;
        let margin = size * 2.0;
        let block = size * line_height * lines.len() as f64;
        let id = Id::new();
        let label = Label {
            text: lines.join("\n"),
            position: [margin, (height - margin - block / 2.0).max(block / 2.0)],
            align: Align::Left,
            size: Some(size),
            line_height: Some(line_height),
            ..Label::default()
        };
        if !self.gesture(Command::CreateLabel {
            id,
            parent,
            index: None,
            label: Box::new(label),
        }) {
            self.cancel_gesture();
            return None;
        }
        self.select(vec![id]);
        self.end_gesture();
        Some(id)
    }

    /// Places the selected Asset (or a random member of its Variant group) as a Stamp at
    /// `position` in the active Layer, as one Undo step, and selects it. Does nothing when no
    /// Asset is selected.
    pub fn place_stamp(&mut self, position: [f64; 2]) -> Option<Id> {
        let asset = self.placement_asset()?;
        self.end_gesture();
        self.begin_gesture("Add Stamp");
        let Some(parent) = self.placement_layer(Placing::Stamp) else {
            self.cancel_gesture();
            return None;
        };
        let id = Id::new();
        if !self.gesture(Command::CreateStamp {
            id,
            parent,
            index: None,
            stamp: Box::new(Stamp::new(asset, position)),
        }) {
            self.cancel_gesture();
            return None;
        }
        self.select(vec![id]);
        self.end_gesture();
        Some(id)
    }

    /// Finishes a scatter brush stroke: scatters Stamps of the selected Asset along `points` in
    /// the active Layer as one Undo step ([`scatter::scatter_command`]). Does nothing, with a
    /// message, when no Asset is chosen, and leaves no Undo step when nothing could be placed.
    pub fn scatter_stamps(&mut self, points: &[Point]) -> bool {
        let Some(selected) = self.selected_asset.clone() else {
            self.error("Choose an Asset in the Asset browser first");
            return false;
        };
        let variants = if self.scatter.random_variant {
            self.variant_group(&selected)
        } else {
            Vec::new()
        };
        let terrain = Terrain::of(&self.map, &mut self.roughness);
        self.end_gesture();
        self.begin_gesture("Scatter Stamps");
        let Some(parent) = self.placement_layer(Placing::Stamp) else {
            self.cancel_gesture();
            return false;
        };
        let base = Stamp::new(selected, [0.0, 0.0]);
        let command = scatter::scatter_command(
            &self.scatter,
            ScatterStroke {
                points,
                radius: self.brush_size / 2.0,
                parent,
                base: &base,
                variants: &variants,
            },
            &terrain,
            random_seed(),
            Id::new,
        );
        if let Some(command) = command
            && self.gesture(command)
        {
            self.end_gesture();
            true
        } else {
            self.cancel_gesture();
            false
        }
    }

    /// Finishes a Stamp eraser stroke: removes the Stamps in the active Layer whose Anchor is
    /// inside the brush, as one Undo step. Other Layers are never touched.
    pub fn erase_stamps(&mut self, points: &[Point]) -> bool {
        let Some(layer) = self.active_layer.filter(|&id| self.map.contains(id)) else {
            self.info("Select a Layer to erase Stamps from");
            return false;
        };
        match scatter::erase_command(&self.map, layer, points, self.brush_size / 2.0) {
            Some(command) => self.run(command),
            None => false,
        }
    }

    /// Whether the Selection holds a Stamp, so Shuffle variant has something to work on.
    pub fn can_shuffle(&self) -> bool {
        self.selection.iter().any(|&id| self.stamp(id).is_some())
    }

    /// Shuffle variant: each selected Stamp takes a random Asset from its own Variant group,
    /// all as one Undo step.
    pub fn shuffle_variants(&mut self) -> bool {
        let Some(library) = self.sprites.as_ref().map(SpriteStore::library) else {
            return false;
        };
        let commands = scatter::shuffle_commands(
            &self.map,
            &self.selection,
            |asset| group_of(&library, asset),
            random_seed(),
        );
        if commands.is_empty() {
            self.info("No selected Stamp has another variant to take");
            return false;
        }
        self.end_gesture();
        self.begin_gesture("Shuffle Variant");
        for command in commands {
            if !self.gesture(command) {
                self.cancel_gesture();
                return false;
            }
        }
        self.end_gesture();
        true
    }

    /// "Pine cluster · v2": the Asset's name, and its place in its Variant group when it has
    /// one. Falls back to the reference when the Asset is not in the library.
    pub fn asset_caption(&self, asset: &AssetRef) -> String {
        let library = self.sprites.as_ref().map(SpriteStore::library);
        let Some((_, found)) = library.as_ref().and_then(|library| library.asset(asset)) else {
            return asset.to_string();
        };
        let group = self.variant_group(asset);
        match group.iter().position(|member| member == asset) {
            Some(index) if group.len() > 1 => format!("{} · v{}", found.name, index + 1),
            _ => found.name.clone(),
        }
    }

    /// The Assets in the Variant group `asset` belongs to; empty when it has none.
    fn variant_group(&self, asset: &AssetRef) -> Vec<AssetRef> {
        self.sprites
            .as_ref()
            .map(|sprites| group_of(&sprites.library(), asset))
            .unwrap_or_default()
    }

    /// The concrete Asset the next Stamp gets: the selected one, or a random sibling from its
    /// Variant group.
    fn placement_asset(&self) -> Option<AssetRef> {
        let selected = self.selected_asset.as_ref()?;
        let library = self.sprites.as_ref()?.library();
        let (pack, asset) = library.asset(selected)?;
        let Some(group) = &asset.variant_group else {
            return Some(selected.clone());
        };
        let members: Vec<&str> = pack
            .variant_group(group)
            .map(|member| member.id.as_str())
            .collect();
        let pick = members[random_below(members.len())];
        AssetRef::new(pack.id.clone(), pick).ok()
    }

    /// The template's Layer for what is being placed, if the Map still has it.
    fn default_layer(&self, placing: Placing) -> Option<Id> {
        let layers = &self.default_layers;
        let layer = match placing {
            Placing::Land => layers.land,
            Placing::Stamp => layers.assets,
            Placing::Label => layers.labels,
        };
        layer.filter(|&id| matches!(self.map.node(id), Some(Node::Layer(_))))
    }

    /// The template's Layer for `placing` if no Layer is active, or if the active one is the
    /// template's Layer for something else (placing a Stamp makes its Layer active, and the next
    /// Label should still go to the Labels Layer); else the active Layer.
    fn target_layer(&self, placing: Placing) -> Option<Id> {
        let active = self.active_layer.filter(|&id| self.map.contains(id));
        let layers = &self.default_layers;
        let taken_by_other = |id: Id| {
            let others = match placing {
                Placing::Land => [layers.assets, layers.labels],
                Placing::Stamp => [layers.land, layers.labels],
                Placing::Label => [layers.land, layers.assets],
            };
            others.contains(&Some(id))
        };
        match active {
            Some(id) if !taken_by_other(id) => Some(id),
            _ => self.default_layer(placing).or(active),
        }
    }

    /// The Layer new Objects go into: see [`Self::target_layer`], else the top Layer, creating
    /// a Layer if the Map has none. Runs inside the open gesture.
    fn placement_layer(&mut self, placing: Placing) -> Option<Id> {
        match self.target_layer(placing) {
            Some(layer) => Some(layer),
            None => match self.map.children(None).last() {
                Some(&top) => Some(top),
                None => {
                    let id = Id::new();
                    let name = format!("Layer {}", self.layer_count() + 1);
                    let command = Command::CreateLayer {
                        id,
                        parent: None,
                        index: None,
                        name,
                    };
                    self.gesture(command).then_some(id)
                }
            },
        }
    }

    fn layer_count(&self) -> usize {
        self.map
            .ids()
            .filter(|&id| self.map.node(id).is_some_and(Node::is_layer))
            .count()
    }

    /// Restores a recorded Selection, dropping anything that no longer exists.
    fn restore(&mut self, mut selection: Selection) {
        // Undo and redo can renumber nodes.
        self.select_nodes(Vec::new());
        selection.retain(|&id| self.map.contains(id));
        // A selected board stays selected through its own edits, and undo and redo of them.
        let board = self.board_selected && selection.is_empty();
        self.select(selection);
        self.board_selected = board;
        if self.active_layer.is_some_and(|id| !self.map.contains(id)) {
            self.active_layer = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssmap_core::{Canvas, DistanceUnit, Scale};

    fn editor() -> Editor {
        Editor::new(Map::new(Canvas::new(
            100,
            100,
            Scale::new(1.0, DistanceUnit::Miles),
        )))
    }

    #[test]
    fn new_layers_go_above_the_active_one_and_get_selected() {
        let mut editor = editor();
        let first = editor.new_layer().unwrap();
        let second = editor.new_layer().unwrap();
        editor.select(vec![first]);
        let third = editor.new_layer().unwrap();
        assert_eq!(editor.map.children(None), &[first, third, second]);
        assert_eq!(editor.selection(), &[third]);
        assert_eq!(editor.active_layer(), Some(third));
    }

    #[test]
    fn duplicating_selects_the_copy_and_is_one_named_undo_step() {
        let mut editor = editor();
        let first = editor.new_layer().unwrap();
        let second = editor.new_layer().unwrap();
        editor.select(vec![first]);
        let copy = editor.duplicate_selection().unwrap();
        assert_eq!(editor.map.children(None), &[first, copy[0], second]);
        assert_eq!(editor.selection(), &copy[..]);
        assert_eq!(editor.undo_label(), Some("Duplicate Layer"));
        editor.undo();
        assert_eq!(editor.map.children(None), &[first, second]);
    }

    fn add_stamp(editor: &mut Editor, layer: Id, x: f64, y: f64) -> Id {
        let id = Id::new();
        assert!(editor.run(Command::CreateStamp {
            id,
            parent: layer,
            index: None,
            stamp: Box::new(Stamp::new("pack/asset".parse().unwrap(), [x, y])),
        }));
        id
    }

    fn stamp_position(editor: &Editor, id: Id) -> [f64; 2] {
        editor.stamp(id).unwrap().position
    }

    #[test]
    fn a_paste_puts_the_middle_of_the_copied_objects_under_the_point() {
        let mut editor = editor();
        let layer = editor.new_layer().unwrap();
        let a = add_stamp(&mut editor, layer, 10.0, 10.0);
        let b = add_stamp(&mut editor, layer, 30.0, 20.0);
        editor.select(vec![a, b]);
        assert!(editor.copy_selection());
        // The middle of the two Stamps is (20, 15).
        let pasted = editor.paste([100.0, 200.0]).unwrap();
        assert_eq!(pasted.len(), 2);
        assert_eq!(stamp_position(&editor, pasted[0]), [90.0, 195.0]);
        assert_eq!(stamp_position(&editor, pasted[1]), [110.0, 205.0]);
        assert_eq!(editor.selection(), &pasted[..]);
        // The originals stay where they were, and one undo removes the paste.
        assert_eq!(stamp_position(&editor, a), [10.0, 10.0]);
        assert_eq!(editor.undo_label(), Some("Paste"));
        editor.undo();
        assert_eq!(editor.map.children(Some(layer)), &[a, b]);
    }

    #[test]
    fn pasted_layers_go_above_the_selected_layer_and_keep_their_structure() {
        let mut editor = editor();
        let first = editor.new_layer().unwrap();
        let stamp = add_stamp(&mut editor, first, 5.0, 5.0);
        let second = editor.new_layer().unwrap();
        editor.select(vec![first]);
        editor.copy_selection();
        let pasted = editor.paste([0.0, 0.0]).unwrap();
        assert_eq!(editor.map.children(None), &[first, pasted[0], second]);
        assert_eq!(editor.map.children(Some(pasted[0])).len(), 1);
        assert_ne!(editor.map.children(Some(pasted[0])), &[stamp]);
        assert_eq!(node_name(&editor.map, pasted[0]), "Layer 1 copy");
        // With nothing selected a paste lands on top.
        editor.deselect();
        let top = editor.paste([0.0, 0.0]).unwrap();
        assert_eq!(editor.map.children(None).last(), Some(&top[0]));
    }

    #[test]
    fn cut_removes_the_originals_in_one_step_and_skips_locked_layers() {
        let mut editor = editor();
        let open = editor.new_layer().unwrap();
        let locked = editor.new_layer().unwrap();
        assert!(editor.run(Command::SetLocked {
            id: locked,
            locked: true
        }));
        editor.select(vec![open, locked]);
        editor.cut_selection();
        assert_eq!(editor.map.children(None), &[locked]);
        assert_eq!(editor.undo_label(), Some("Delete"));
        // Both were copied, the locked one with its lock.
        let pasted = editor.paste([0.0, 0.0]).unwrap();
        assert_eq!(pasted.len(), 2);
        editor.undo();
        editor.undo();
        assert_eq!(editor.map.children(None), &[open, locked]);
    }

    #[test]
    fn pasting_objects_needs_a_layer() {
        let mut editor = editor();
        let layer = editor.new_layer().unwrap();
        let a = add_stamp(&mut editor, layer, 1.0, 1.0);
        editor.select(vec![a]);
        editor.copy_selection();
        let pasted = editor.paste([1.0, 1.0]).unwrap();
        assert_eq!(editor.map.parent(pasted[0]), Some(layer));
        assert_eq!(editor.map.children(Some(layer)).len(), 2);
    }

    fn add_label(editor: &mut Editor, layer: Id, mut label: Label) -> Id {
        label.text = "Hello".into();
        let id = Id::new();
        assert!(editor.run(Command::CreateLabel {
            id,
            parent: layer,
            index: None,
            label: Box::new(label),
        }));
        id
    }

    fn label_of(editor: &Editor, id: Id) -> Label {
        match editor.map.node(id) {
            Some(Node::Object(object)) => match &object.kind {
                ObjectKind::Label(label) => (**label).clone(),
                _ => panic!("not a Label"),
            },
            _ => panic!("not an Object"),
        }
    }

    /// Copies `ids` in one Map and hands the copy to a second, empty one.
    fn copy_into_another_map(from: &mut Editor, ids: Vec<Id>) -> (Editor, Id) {
        from.select(ids);
        assert!(from.copy_selection());
        let text = from.take_clipboard_text().unwrap();
        let mut to = editor();
        let layer = to.new_layer().unwrap();
        to.set_clipboard(Clipboard::from_text(&text).unwrap());
        (to, layer)
    }

    #[test]
    fn pasting_into_another_map_needs_nothing_when_it_has_it_all() {
        let mut from = editor();
        let layer = from.new_layer().unwrap();
        let label = add_label(&mut from, layer, Label::default());
        let (mut to, target) = copy_into_another_map(&mut from, vec![label]);
        assert_eq!(to.paste_gaps(), Some(PasteGaps::default()));
        let pasted = to.paste([5.0, 5.0]).unwrap();
        assert_eq!(to.map.parent(pasted[0]), Some(target));
        // Cancelling the popup is not pasting: the Map is as it was.
        assert_eq!(to.undo_label(), Some("Paste"));
        to.undo();
        assert!(to.map.children(Some(target)).is_empty());
    }

    #[test]
    fn a_label_style_the_other_map_lacks_falls_back_to_its_default() {
        let mut from = editor();
        let layer = from.new_layer().unwrap();
        let style = Id::new();
        assert!(from.run(Command::CreateLabelStyle {
            style: ssmap_core::LabelStyle::new(style, "Realm"),
        }));
        let label = add_label(
            &mut from,
            layer,
            Label {
                style,
                size: Some(33.0),
                ..Label::default()
            },
        );
        let (mut to, _) = copy_into_another_map(&mut from, vec![label]);
        assert_eq!(to.paste_gaps().unwrap().label_styles, 1);
        let pasted = to.paste([0.0, 0.0]).unwrap();
        let label = label_of(&to, pasted[0]);
        assert_eq!(label.style, ssmap_core::LabelStyle::DEFAULT_ID);
        assert_eq!(label.size, Some(33.0));
        assert_eq!(to.map.label_styles().iter().count(), 1);
    }

    #[test]
    fn a_font_the_other_map_cannot_draw_gets_the_fallback_font() {
        let mut from = editor();
        let layer = from.new_layer().unwrap();
        let label = add_label(
            &mut from,
            layer,
            Label {
                font: Some(ssmap_core::FontFace {
                    family: "Papyrus".into(),
                    ..Default::default()
                }),
                ..Label::default()
            },
        );
        let (mut to, _) = copy_into_another_map(&mut from, vec![label]);
        assert_eq!(to.paste_gaps().unwrap().fonts, vec!["Papyrus".to_owned()]);
        let pasted = to.paste([0.0, 0.0]).unwrap();
        let font = label_of(&to, pasted[0]).font.unwrap();
        assert_eq!(font.family, FALLBACK_FAMILY);
    }

    #[test]
    fn an_asset_the_other_map_lacks_pastes_as_a_placeholder_and_cancel_pastes_nothing() {
        let mut from = editor();
        let layer = from.new_layer().unwrap();
        let stamp = add_stamp(&mut from, layer, 1.0, 1.0);
        let (mut to, target) = copy_into_another_map(&mut from, vec![stamp]);
        // No pack is installed here, and nothing is embedded.
        to.set_sprites(SpriteStore::new(Arc::new(
            ssmap_core::asset::PackLibrary::default(),
        )));
        let gaps = to.paste_gaps().unwrap();
        assert_eq!(gaps.assets, vec!["pack/asset".parse::<AssetRef>().unwrap()]);
        // Cancelling the popup never reaches paste: the Map is untouched.
        assert!(to.map.children(Some(target)).is_empty());
        assert_ne!(to.undo_label(), Some("Paste"));
        // Pasting anyway keeps the Stamp, which draws as a placeholder.
        let pasted = to.paste([0.0, 0.0]).unwrap();
        assert_eq!(
            to.stamp(pasted[0]).unwrap().asset,
            "pack/asset".parse::<AssetRef>().unwrap()
        );
        assert_eq!(to.undo_label(), Some("Paste"));
    }

    #[test]
    fn a_paste_with_nothing_copied_does_nothing() {
        let mut editor = editor();
        editor.new_layer().unwrap();
        assert!(editor.paste([0.0, 0.0]).is_none());
        editor.deselect();
        assert!(!editor.copy_selection());
    }

    #[test]
    fn a_duplicate_with_nothing_selected_does_nothing() {
        let mut editor = editor();
        editor.new_layer().unwrap();
        editor.deselect();
        assert!(editor.duplicate_selection().is_none());
    }

    #[test]
    fn a_water_stroke_makes_a_lake_in_a_water_layer_in_one_undo_step() {
        let mut editor = editor();
        let stroke = |erase| BrushStroke {
            region: land::stroke_region(&[[10.0, 10.0], [60.0, 10.0]], 5.0),
            start: [10.0, 10.0],
            erase,
        };
        editor.begin_gesture(Editor::water_label(false));
        editor.finish_water_stroke(stroke(false));
        assert_eq!(editor.undo_label(), Some("Paint Water"));
        let layer = editor.water_layer().expect("a Water layer");
        let lakes = editor.map.children(Some(layer)).to_vec();
        assert_eq!(lakes.len(), 1);
        assert_eq!(editor.selection(), lakes);

        // A second stroke extends the Lake and reuses the Layer.
        editor.begin_gesture(Editor::water_label(false));
        editor.finish_water_stroke(stroke(false));
        assert_eq!(editor.map.children(Some(layer)), lakes);
        assert_eq!(editor.map.children(None), &[layer]);

        // Undoing both steps removes the Lake and the Layer it created.
        editor.undo();
        assert!(editor.map.contains(lakes[0]));
        editor.undo();
        assert!(!editor.map.contains(layer));
    }

    #[test]
    fn undo_restores_the_selection_and_redo_reapplies() {
        let mut editor = editor();
        let layer = editor.new_layer().unwrap();
        editor.undo();
        assert!(!editor.map.contains(layer));
        assert!(editor.selection().is_empty());
        assert_eq!(editor.active_layer(), None);
        editor.redo();
        assert_eq!(editor.selection(), &[layer]);
    }

    #[test]
    fn refusals_show_a_message_and_change_nothing() {
        let mut editor = editor();
        let layer = editor.new_layer().unwrap();
        editor.run(Command::SetLocked {
            id: layer,
            locked: true,
        });
        let before = editor.map.view();
        assert!(!editor.run(Command::RenameLayer {
            id: layer,
            name: "Nope".into(),
        }));
        assert_eq!(editor.map.view(), before);
        let notice = editor.notice().expect("a visible message");
        assert!(notice.is_error());
        assert_eq!(notice.text, "“Layer 1” is locked");
    }

    #[test]
    fn a_gesture_is_one_undo_step_and_cancel_restores() {
        let mut editor = editor();
        let layer = editor.new_layer().unwrap();
        editor.begin_gesture("Change Opacity");
        for opacity in [0.8, 0.5, 0.2] {
            editor.gesture(Command::SetOpacity { id: layer, opacity });
        }
        editor.end_gesture();
        assert_eq!(editor.undo_label(), Some("Change Opacity"));
        editor.undo();
        assert_eq!(opacity(&editor, layer), 1.0);

        editor.begin_gesture("Change Opacity");
        editor.gesture(Command::SetOpacity {
            id: layer,
            opacity: 0.3,
        });
        editor.cancel_gesture();
        assert_eq!(opacity(&editor, layer), 1.0);
        assert_eq!(editor.redo_label(), Some("Change Opacity"));
    }

    #[test]
    fn placing_a_label_creates_a_layer_and_is_one_undo_step() {
        let mut editor = editor();
        let id = editor.place_label([50.0, 50.0]).unwrap();
        let mut label = editor.label(id).unwrap().clone();
        label.text = "Thule".into();
        editor.gesture(Command::SetLabel {
            id,
            label: Box::new(label),
        });
        editor.end_gesture();
        assert_eq!(editor.selection(), &[id]);
        assert_eq!(editor.label(id).unwrap().text, "Thule");
        assert_eq!(editor.undo_label(), Some("Add Label"));
        editor.undo();
        assert!(editor.map.children(None).is_empty());
        assert!(editor.selection().is_empty());
    }

    #[test]
    fn cancelling_a_placement_restores_the_selection() {
        let mut editor = editor();
        let layer = editor.new_layer().unwrap();
        editor.place_label([50.0, 50.0]).unwrap();
        editor.cancel_gesture();
        assert_eq!(editor.map.children(Some(layer)).len(), 0);
        assert_eq!(editor.selection(), &[layer]);
    }

    #[test]
    fn picking_finds_the_topmost_unlocked_visible_label() {
        let mut editor = editor();
        let below = editor.place_label([50.0, 50.0]).unwrap();
        editor.end_gesture();
        let above = editor.place_label([52.0, 50.0]).unwrap();
        editor.end_gesture();
        assert_eq!(editor.pick([50.0, 50.0]), Some(above));
        assert_eq!(editor.pick([-500.0, 50.0]), None);
        editor.run(Command::SetLocked {
            id: above,
            locked: true,
        });
        assert_eq!(editor.pick([50.0, 50.0]), Some(below));
        let layer = editor.map.parent(below).unwrap();
        editor.run(Command::SetVisible {
            id: layer,
            visible: false,
        });
        assert_eq!(editor.pick([50.0, 50.0]), None);
    }

    fn opacity(editor: &Editor, id: Id) -> f32 {
        match editor.map.node(id) {
            Some(Node::Layer(layer)) => layer.opacity,
            _ => panic!("a Layer"),
        }
    }

    fn editor_with_square() -> (Editor, Id) {
        let mut editor = editor();
        let layer = editor.new_layer().unwrap();
        let id = Id::new();
        editor.run(Command::CreateLandmass {
            id,
            parent: layer,
            index: None,
            geometry: MultiPolygon(vec![ssmap_core::Polygon {
                exterior: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                holes: Vec::new(),
            }]),
            fill: None,
        });
        (editor, id)
    }

    fn node(index: usize) -> NodeRef {
        NodeRef {
            part: 0,
            ring: 0,
            index,
        }
    }

    #[test]
    fn node_edit_mode_follows_the_selection_and_edits_undo_cleanly() {
        let (mut editor, id) = editor_with_square();
        editor.enter_node_edit(id);
        assert_eq!(editor.selection(), &[id]);

        // A drag is one gesture and one Undo step, however many frames it takes.
        editor.select_nodes(vec![node(2)]);
        editor.begin_gesture("Move Nodes");
        for step in 1..=5 {
            editor.gesture_replace(Command::MoveNodes {
                id,
                nodes: vec![node(2)],
                offset: [f64::from(step), 0.0],
            });
        }
        editor.end_gesture();
        assert_eq!(
            editor.edited_outline().unwrap().node(node(2)),
            Some([15.0, 10.0])
        );
        editor.undo();
        assert_eq!(
            editor.edited_outline().unwrap().node(node(2)),
            Some([10.0, 10.0])
        );
        assert!(editor.node_edit().is_some_and(|edit| edit.nodes.is_empty()));

        editor.insert_node(node(0), [5.0, -2.0]);
        assert_eq!(editor.node_edit().unwrap().nodes, vec![node(1)]);
        assert_eq!(editor.edited_shape(), vec![5]);
        editor.delete_selected_nodes();
        assert_eq!(editor.edited_shape(), vec![4]);

        // Leaving, or selecting something else, ends the mode.
        editor.select(Vec::new());
        assert!(editor.node_edit().is_none());
    }

    #[test]
    fn deleting_offers_an_undo_that_undoes_it() {
        let (mut editor, id) = editor_with_square();
        editor.select(vec![id]);
        let before = editor.map.view();
        editor.delete_selection();
        assert_ne!(editor.map.view(), before);
        let toast = editor.notice().expect("a toast").clone();
        assert!(!toast.is_error());
        let action = toast.action.expect("an Undo action");
        let live = editor.toasts_mut().items()[0].action;
        assert_eq!(live, Some(action));
        editor.run_toast_action(action);
        assert_eq!(editor.map.view(), before);
    }

    #[test]
    fn the_undo_action_goes_once_the_history_moves_on() {
        let (mut editor, id) = editor_with_square();
        editor.select(vec![id]);
        editor.delete_selection();
        let action = editor.notice().unwrap().action.unwrap();
        editor.new_layer();
        assert!(editor.toasts_mut().items()[0].action.is_none());
        let after = editor.map.view();
        editor.run_toast_action(action);
        assert_eq!(editor.map.view(), after);
    }

    #[test]
    fn deleting_nodes_offers_an_undo() {
        let (mut editor, id) = editor_with_square();
        editor.enter_node_edit(id);
        editor.select_nodes(vec![node(0)]);
        editor.delete_selected_nodes();
        let toast = editor.notice().expect("a toast");
        assert_eq!(toast.text, "Deleted 1 node");
        assert!(toast.action.is_some());
    }

    #[test]
    fn deleting_nodes_that_would_leave_fewer_than_three_is_refused() {
        let (mut editor, id) = editor_with_square();
        editor.enter_node_edit(id);
        editor.select_nodes(vec![node(0), node(1)]);
        editor.delete_selected_nodes();
        assert_eq!(editor.edited_shape(), vec![4]);
        assert!(editor.notice().is_some_and(|notice| notice.is_error()));
    }

    #[test]
    fn union_and_split_apart_work_on_the_selection_in_one_undo_step() {
        let (mut editor, a) = editor_with_square();
        let layer = editor.map.parent(a).unwrap();
        let b = Id::new();
        editor.run(Command::CreateLandmass {
            id: b,
            parent: layer,
            index: None,
            geometry: MultiPolygon(vec![ssmap_core::Polygon {
                exterior: vec![[5.0, 5.0], [20.0, 5.0], [20.0, 20.0], [5.0, 20.0]],
                holes: Vec::new(),
            }]),
            fill: None,
        });
        editor.select(vec![a, b]);
        assert!(editor.can_combine() && !editor.can_split());
        editor.union_selection();
        assert_eq!(editor.selection(), &[b]);
        assert!(!editor.map.contains(a));

        editor.undo();
        assert!(editor.map.contains(a) && editor.map.contains(b));

        // Two islands split into two Landmasses with one Undo step.
        editor.select(vec![a]);
        editor.run(Command::AddRegion {
            id: a,
            region: MultiPolygon(vec![ssmap_core::Polygon {
                exterior: vec![[50.0, 50.0], [60.0, 50.0], [60.0, 60.0], [50.0, 60.0]],
                holes: Vec::new(),
            }]),
        });
        assert!(editor.can_split());
        let before = editor.map.children(Some(layer)).len();
        editor.split_selection();
        assert_eq!(editor.map.children(Some(layer)).len(), before + 1);
        assert_eq!(editor.selection().len(), 2);
        editor.undo();
        assert_eq!(editor.map.children(Some(layer)).len(), before);
    }
    /// An Editor whose sprites come from the core crate's test pack.
    fn editor_with_assets() -> Editor {
        use ssmap_core::asset::{PackLibrary, PackRoots};
        let roots = PackRoots {
            built_in: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../crates/core/tests/packs"),
            user: None,
        };
        let (library, problems) = PackLibrary::load(&roots);
        assert!(problems.is_empty(), "{problems:?}");
        let mut editor = editor();
        editor.set_sprites(SpriteStore::new(Arc::new(library)));
        editor
    }

    fn asset(reference: &str) -> AssetRef {
        reference.parse().unwrap()
    }

    /// An Editor whose only pack asks for attribution, with one Stamp of it placed.
    fn editor_using_credited_asset() -> Editor {
        use ssmap_core::asset::{PackLibrary, PackSource, parse_manifest};
        let manifest = "[pack]\nid = \"icons\"\nname = \"Icons\"\nlicence = \"CC-BY-3.0\"\n\
                        credits = \"Icons made by Lorc.\"\nattribution_required = true\n\
                        modified = \"2026-01-01\"\n\n[[asset]]\nid = \"sword\"\nname = \"Sword\"\n\
                        category = \"c\"\nfile = \"sword.svg\"\nanchor = [0.5, 0.5]\nwidth = 8\n\
                        author = \"Lorc\"\n";
        let pack = parse_manifest(
            std::path::Path::new("icons/pack.toml"),
            manifest,
            PackSource::BuiltIn,
        )
        .unwrap();
        let mut library = PackLibrary::default();
        library.add(pack, &mut Vec::new());
        let mut editor = editor();
        editor.set_sprites(SpriteStore::new(Arc::new(library)));
        editor.selected_asset = Some(asset("icons/sword"));
        editor.place_stamp([40.0, 50.0]).unwrap();
        editor
    }

    #[test]
    fn a_map_without_credited_assets_has_no_credit_lines() {
        let mut editor = editor_with_assets();
        editor.selected_asset = Some(asset("test-pack/castle"));
        editor.place_stamp([40.0, 50.0]).unwrap();
        assert!(editor.credit_lines().is_empty());
        assert_eq!(editor.add_credits_label(), None);
    }

    #[test]
    fn adding_the_credits_as_a_label_is_one_undo_step() {
        let mut editor = editor_using_credited_asset();
        let lines = editor.credit_lines();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].starts_with("Sword by Lorc, Icons (CC-BY-3.0)"));

        let id = editor.add_credits_label().unwrap();
        assert_eq!(editor.label(id).unwrap().text, lines.join("\n"));
        assert_eq!(editor.undo_label(), Some("Add Credits Label"));
        editor.undo();
        assert!(!editor.map.contains(id));
        editor.redo();
        assert!(editor.map.contains(id));
    }

    #[test]
    fn a_new_map_places_stamps_and_labels_in_the_template_layers_without_setup() {
        use ssmap_core::{Canvas, DistanceUnit, Scale};
        let canvas = Canvas::new(1000, 800, Scale::new(1.0, DistanceUnit::Miles));
        let (map, layers, problem) = crate::new_map::start_map(canvas);
        assert_eq!(problem, None);
        let mut editor = editor_with_assets();
        editor.map = map;
        editor.default_layers = layers;
        assert_eq!(editor.active_layer(), None);

        editor.selected_asset = Some(asset("test-pack/castle"));
        let stamp = editor.place_stamp([40.0, 50.0]).unwrap();
        assert_eq!(editor.map.parent(stamp), layers.assets);
        let label = editor.place_label([60.0, 50.0]).unwrap();
        assert_eq!(editor.map.parent(label), layers.labels);
        assert_eq!(editor.default_layer(Placing::Land), layers.land);
        assert_eq!(editor.map.children(layers.assets), [stamp]);
    }

    #[test]
    fn the_stamp_tool_places_the_selected_asset_in_one_undo_step() {
        let mut editor = editor_with_assets();
        assert_eq!(editor.place_stamp([40.0, 50.0]), None, "no Asset chosen");
        assert!(editor.map.children(None).is_empty(), "and nothing was made");

        editor.selected_asset = Some(asset("test-pack/castle"));
        let id = editor.place_stamp([40.0, 50.0]).unwrap();
        let stamp = editor.stamp(id).unwrap();
        assert_eq!(stamp.asset, asset("test-pack/castle"));
        assert_eq!(stamp.position, [40.0, 50.0]);
        assert_eq!(editor.selection(), &[id]);
        // The Layer the Map lacked was made in the same step.
        assert_eq!(editor.map.children(None).len(), 1);
        assert_eq!(editor.undo_label(), Some("Add Stamp"));
        editor.undo();
        assert!(editor.map.children(None).is_empty());
    }

    #[test]
    fn a_variant_group_gives_its_members_and_stamps_keep_what_they_got() {
        let mut editor = editor_with_assets();
        editor.selected_asset = Some(asset("test-pack/mountain-1"));
        let mut seen = std::collections::HashSet::new();
        for i in 0..60 {
            let id = editor.place_stamp([f64::from(i), 0.0]).unwrap();
            let got = editor.stamp(id).unwrap().asset.to_string();
            assert!(
                got == "test-pack/mountain-1" || got == "test-pack/mountain-2",
                "{got}"
            );
            seen.insert(got);
        }
        assert_eq!(seen.len(), 2, "both variants turn up in 60 tries");
    }

    #[test]
    fn picking_goes_by_silhouette_and_the_topmost_stamp_wins() {
        let mut editor = editor_with_assets();
        editor.selected_asset = Some(asset("test-pack/castle"));
        let below = editor.place_stamp([200.0, 200.0]).unwrap();
        let above = editor.place_stamp([210.0, 200.0]).unwrap();
        // The Anchor is on the art, so a point just above it is inside the Silhouette.
        assert_eq!(editor.pick([205.0, 190.0]), Some(above));
        assert_eq!(editor.pick([-50.0, -50.0]), None);
        // The box's corner, outside the outline, finds nothing.
        let bounds = editor.stamp_bounds(below).unwrap();
        assert_eq!(
            editor.pick([bounds.min[0] + 0.1, bounds.min[1] + 0.1]),
            None
        );

        // A locked Stamp can't be picked there, so what's beneath is.
        editor.run(Command::SetLocked {
            id: above,
            locked: true,
        });
        assert_eq!(editor.pick([205.0, 190.0]), Some(below));
    }

    fn square(min: f64, max: f64) -> MultiPolygon {
        MultiPolygon(vec![ssmap_core::Polygon {
            exterior: vec![[min, min], [max, min], [max, max], [min, max]],
            holes: Vec::new(),
        }])
    }

    /// An Editor with a Landmass over (0,0)-(200,200) holding a Lake over (80,80)-(120,120).
    fn editor_with_island() -> (Editor, Id) {
        let mut editor = editor_with_assets();
        let layer = editor.new_layer().unwrap();
        editor.run(Command::CreateLandmass {
            id: Id::new(),
            parent: layer,
            index: None,
            geometry: square(0.0, 200.0),
            fill: None,
        });
        editor.run(Command::CreateLake {
            id: Id::new(),
            parent: layer,
            index: None,
            geometry: square(80.0, 120.0),
            fill: None,
        });
        editor.selected_asset = Some(asset("test-pack/mountain-1"));
        editor.brush_size = 400.0;
        editor.scatter.density = 40.0;
        editor.scatter.spacing = 0.0;
        (editor, layer)
    }

    fn stamps_in(editor: &Editor, layer: Id) -> Vec<Stamp> {
        editor
            .map
            .children(Some(layer))
            .iter()
            .filter_map(|&id| editor.stamp(id).cloned())
            .collect()
    }

    #[test]
    fn a_scatter_stroke_is_one_undo_step_and_land_only_keeps_off_water_and_lakes() {
        let (mut editor, layer) = editor_with_island();
        let before = editor.map.children(Some(layer)).len();
        assert!(editor.scatter_stamps(&[[20.0, 100.0], [180.0, 100.0]]));
        assert_eq!(editor.undo_label(), Some("Scatter Stamps"));
        let placed = stamps_in(&editor, layer);
        assert!(placed.len() > 10, "{}", placed.len());
        let lake = square(80.0, 120.0);
        for stamp in &placed {
            let [x, y] = stamp.position;
            assert!((0.0..=200.0).contains(&x) && (0.0..=200.0).contains(&y));
            assert!(!lake.contains(stamp.position), "{:?}", stamp.position);
        }
        editor.undo();
        assert_eq!(editor.map.children(Some(layer)).len(), before);
    }

    #[test]
    fn water_only_scatters_off_the_land() {
        let (mut editor, layer) = editor_with_island();
        editor.scatter.placement = ssmap_core::scatter::Placement::Water;
        assert!(editor.scatter_stamps(&[[100.0, 100.0], [300.0, 300.0]]));
        let land = square(0.0, 200.0);
        let lake = square(80.0, 120.0);
        for stamp in stamps_in(&editor, layer) {
            assert!(!land.contains(stamp.position) || lake.contains(stamp.position));
        }
    }

    #[test]
    fn the_stamp_eraser_only_touches_the_active_layer() {
        let (mut editor, layer) = editor_with_island();
        editor.scatter.placement = ssmap_core::scatter::Placement::Anywhere;
        assert!(editor.scatter_stamps(&[[100.0, 100.0]]));
        let other = editor.new_layer().unwrap();
        assert!(editor.scatter_stamps(&[[100.0, 100.0]]));
        assert_eq!(editor.active_layer(), Some(other));
        let (here, there) = (
            stamps_in(&editor, other).len(),
            stamps_in(&editor, layer).len(),
        );
        assert!(here > 0 && there > 0);

        assert!(editor.erase_stamps(&[[100.0, 100.0]]));
        assert_eq!(editor.undo_label(), Some("Delete"));
        assert_eq!(stamps_in(&editor, other).len(), 0);
        assert_eq!(
            stamps_in(&editor, layer).len(),
            there,
            "the other Layer is intact"
        );
        editor.undo();
        assert_eq!(stamps_in(&editor, other).len(), here);
    }

    #[test]
    fn shuffle_variant_is_one_undo_step() {
        let mut editor = editor_with_assets();
        editor.selected_asset = Some(asset("test-pack/mountain-1"));
        let ids: Vec<Id> = (0..20)
            .map(|i| editor.place_stamp([f64::from(i), 0.0]).unwrap())
            .collect();
        editor.select(ids.clone());
        let before: Vec<AssetRef> = ids
            .iter()
            .map(|&id| editor.stamp(id).unwrap().asset.clone())
            .collect();
        assert!(editor.can_shuffle());
        assert!(editor.shuffle_variants());
        assert_eq!(editor.undo_label(), Some("Shuffle Variant"));
        let after: Vec<AssetRef> = ids
            .iter()
            .map(|&id| editor.stamp(id).unwrap().asset.clone())
            .collect();
        assert_ne!(before, after);
        editor.undo();
        let restored: Vec<AssetRef> = ids
            .iter()
            .map(|&id| editor.stamp(id).unwrap().asset.clone())
            .collect();
        assert_eq!(before, restored);
        assert_ne!(editor.undo_label(), Some("Shuffle Variant"));
    }
}
