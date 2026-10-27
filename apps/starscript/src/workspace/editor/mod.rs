//! Editor state around one Map: history, Selection, the active Layer and tool, and the
//! message bar. Every change to the Map goes through a Command here.

mod caches;
mod clipboard;
mod gesture;
mod history;
mod paste;
mod placement;
mod shapes;
mod strokes;
mod structure;
#[cfg(test)]
mod tests;
mod toasts;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ssmap_core::asset::{AssetRef, credit_lines};
use ssmap_core::file::Recovered;
use ssmap_core::land::{self, BrushStroke};
use ssmap_core::scatter::{self, ScatterBrush, ScatterStroke, Terrain};
use ssmap_core::{
    Align, Blob, Command, CommandError, DefaultLayers, EmbedAsset, EmbedFont, ExecuteError,
    ExportSettings, History, Id, Label, Map, MultiPolygon, Node, NodeRef, ObjectKind, Op, OpError,
    PasteGaps, Point, Rect, ResolvedLabel, Selection, Stamp, Subtree,
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
use crate::toast::Toasts;
use crate::tools::ToolId;
use caches::Caches;
use clipboard::ClipboardSlot;
use gesture::OpenGesture;
pub use gesture::{Gesture, GestureId};

/// The land brush's starting diameter, in Canvas units.
pub const DEFAULT_BRUSH_SIZE: f64 = 48.0;

/// The name of the Layer the water brush paints into.
pub const WATER_LAYER: &str = "Water";

pub struct Editor {
    /// Read through [`Editor::map`]. Only Commands change it, through History (ADR 0004).
    map: Map,
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
    /// Whether the Canvas is what's selected, rather than layers or Objects.
    canvas_selected: bool,
    /// Set by a tool while it holds something Escape should drop (a lasso half drawn).
    pub tool_busy: bool,
    /// Set by Escape for the busy tool to act on.
    pub cancel_tool: bool,
    toasts: Toasts,
    /// The open gesture. Set and cleared only with History's open transaction, in
    /// `open_gesture` and `close_gesture`.
    gesture: Option<OpenGesture>,
    /// The pick indexes, Roughness cache and Stamp art.
    caches: Caches,
    /// The Asset the Stamp tool places, chosen in the Asset browser.
    pub selected_asset: Option<AssetRef>,
    /// The scatter brush's settings. Its radius is half the shared `brush_size`.
    pub scatter: ScatterBrush,
    /// What Copy or Cut last took. Paste reads it back.
    clipboard: ClipboardSlot,
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
            canvas_selected: false,
            active_layer: None,
            default_layers: DefaultLayers::default(),
            active_tool: ToolId::Select,
            scatter: ScatterBrush::default(),
            clipboard: ClipboardSlot::default(),
            brush_size: DEFAULT_BRUSH_SIZE,
            erase_default: false,
            node_edit: None,
            tool_busy: false,
            cancel_tool: false,
            toasts: Toasts::default(),
            gesture: None,
            caches: Caches::new(),
            selected_asset: None,
        }
    }

    /// The Map. Read-only: edits go through Commands.
    pub fn map(&self) -> &Map {
        &self.map
    }

    /// The Map for a test to build up with raw Ops. Not for production code (ADR 0004).
    #[cfg(test)]
    pub(crate) fn map_mut(&mut self) -> &mut Map {
        &mut self.map
    }

    /// The Map's export settings, which the export dialog and the export-region tool write
    /// outside Commands (ADR 0011).
    pub fn export_settings_mut(&mut self) -> &mut ExportSettings {
        &mut self.map.export_settings
    }

    /// Applies a Recovery journal's Ops to the Map, all or nothing.
    pub fn replay(&mut self, recovered: Recovered) -> Result<(), OpError> {
        recovered.replay(&mut self.map)
    }

    /// Where Stamp art comes from: it sizes Stamps for picking and handles.
    pub fn set_sprites(&mut self, sprites: SpriteStore) {
        self.caches.set_sprites(sprites);
        self.sync_embedded();
        self.prefetch_sprites();
    }

    /// Starts rasterising the art of the Map's Stamps in the background, so a Map that has
    /// just opened shows them as they finish rather than waiting on each at first draw.
    fn prefetch_sprites(&self) {
        let Some(sprites) = self.caches.sprites() else {
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
        self.caches.sync_embedded(&self.map);
    }

    /// The Stamps whose Asset is neither embedded nor installed, with the Asset's name: the
    /// Canvas draws a box for each and the app writes the name on it.
    pub fn missing_stamps(&self) -> Vec<(Point, f64, String)> {
        let Some(sprites) = self.caches.sprites() else {
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
        self.caches.sprites()
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
        self.canvas_selected = false;
        if let Some(&last) = self.selection.last() {
            self.active_layer = match self.map.node(last) {
                Some(Node::Layer(_)) => Some(last),
                _ => self.map.parent(last),
            };
        }
    }

    /// Whether the Canvas itself is selected (by the Canvas tool).
    pub fn canvas_selected(&self) -> bool {
        self.canvas_selected
    }

    /// Selects the Canvas, which empties the Selection; `false` deselects it.
    pub fn select_canvas(&mut self, selected: bool) {
        if selected {
            self.select(Vec::new());
        }
        self.canvas_selected = selected;
    }

    /// The topmost unlocked Landmass or Lake drawn at `point`, in Canvas units.
    fn pick_outline(&mut self, point: [f64; 2]) -> Option<Id> {
        self.sync_background_outlines();
        self.caches.pick_outline(&self.map, point)
    }

    /// The topmost unlocked Stamp whose Silhouette is under `point`, in Canvas units.
    fn pick_stamp(&mut self, point: [f64; 2]) -> Option<Id> {
        self.sync_embedded();
        self.caches.pick_stamp(&self.map, point)
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
        Some(stamp.bounds(&self.caches.sprites()?.shape(&stamp.asset)?))
    }

    /// Computes every roughened outline on a thread of its own, so a Map that has just opened
    /// shows at once with its stored outlines. `repaint` is called as each outline completes.
    pub fn start_background_roughness(&mut self, repaint: impl Fn() + Send + 'static) {
        self.caches.start_background_roughness(&self.map, repaint);
    }

    /// Computes the rings shore ripples and shading follow on a thread of their own, so
    /// dragging a Roughness slider over a rough coastline never waits for them. `repaint` is
    /// called as each Object's rings land.
    pub fn compute_shore_rings_in_background(
        &mut self,
        repaint: impl Fn() + Send + Sync + 'static,
    ) {
        self.scene.compute_rings_in_background(repaint);
    }

    /// Whether roughened outlines are still being computed in the background.
    pub fn outlines_rebuilding(&self) -> bool {
        self.caches.outlines_rebuilding()
    }

    /// Drops what was built from placeholder outlines once the background ones have landed.
    fn sync_background_outlines(&mut self) {
        if self.caches.sync_background_outlines() {
            self.scene.invalidate();
        }
    }

    /// What the Canvas draws for the Map, rebuilt only once it has changed. Drops roughened
    /// outlines of Objects that are gone.
    pub fn scene_content(&mut self) -> Content {
        self.sync_embedded();
        self.sync_background_outlines();
        self.caches.retain(&self.map);
        self.scene.content(&self.map, self.caches.roughness())
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
        self.caches.retain(&self.map);
        job.step(&mut ExportContext {
            device,
            queue,
            renderer,
            scene: &mut self.scene,
            map: &self.map,
            roughness: self.caches.roughness(),
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

    /// Runs one Command as its own Undo step. A refusal shows in the status bar and is
    /// returned too.
    pub fn run(&mut self, command: Command) -> Result<(), ExecuteError> {
        let selection = self.selection.clone();
        self.run_selecting(command, selection)
    }

    /// Runs one Command as its own Undo step and selects `selection` afterwards. A refusal
    /// shows in the status bar and is returned too.
    pub fn run_selecting(
        &mut self,
        command: Command,
        selection: Selection,
    ) -> Result<(), ExecuteError> {
        if self.gesture.is_some() {
            return self.execute_in_gesture(&command);
        }
        if let Err(err) = self.history.begin(command.label(), self.selection.clone()) {
            self.error(err.to_string());
            return Err(err.into());
        }
        match self.history.execute(&mut self.map, &command) {
            Ok(()) => {
                if let Err(err) = self.history.commit(selection.clone()) {
                    self.error(err.to_string());
                    return Err(err.into());
                }
                self.restore(selection);
                Ok(())
            }
            Err(err) => {
                // Rolling back fails only when no Undo step is open, and one was opened above.
                let _ = self.history.rollback(&mut self.map);
                self.refused(&err);
                Err(err)
            }
        }
    }

    /// Embeds every Asset and bundled font the Map uses that it doesn't embed yet, as one Undo
    /// step (ADR 0005). Assets no installed pack provides are skipped, and an Asset already
    /// embedded keeps its embedded copy. Returns whether anything was embedded.
    pub fn embed_used_assets(&mut self) -> bool {
        let mut assets = Vec::new();
        if let Some(library) = self
            .caches
            .sprites()
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
        self.run(Command::EmbedAssets { assets, fonts }).is_ok()
    }

    /// Re-pulls the art of every embedded Asset that an installed pack now provides differently,
    /// as one Undo step ("Update from pack"). Returns whether anything changed.
    pub fn update_from_packs(&mut self) -> bool {
        let Some(library) = self
            .caches
            .sprites()
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
        self.run(Command::UpdateFromPack { assets }).is_ok()
    }

    /// Takes the Ops that committed steps, undo and redo applied since the last call, for the
    /// Recovery journal.
    pub fn take_applied(&mut self) -> Vec<Op> {
        self.history.take_applied()
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

    /// Restores a recorded Selection, dropping anything that no longer exists.
    fn restore(&mut self, mut selection: Selection) {
        // Undo and redo can renumber nodes.
        self.select_nodes(Vec::new());
        selection.retain(|&id| self.map.contains(id));
        // A selected Canvas stays selected through its own edits, and undo and redo of them.
        let canvas = self.canvas_selected && selection.is_empty();
        self.select(selection);
        self.canvas_selected = canvas;
        if self.active_layer.is_some_and(|id| !self.map.contains(id)) {
            self.active_layer = None;
        }
    }
}
