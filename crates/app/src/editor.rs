//! Editor state around one Map: history, Selection, the active Layer and tool, and the
//! message bar. Every change to the Map goes through a Command here.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ssmap_core::asset::PackLibrary;
use ssmap_core::{
    AssetRef, Command, CommandError, ExecuteError, History, Id, Label, Map, Node, ObjectKind, Op,
    PickIndex, RoughnessCache, Selection, Stamp,
};
use ssmap_render::wgpu;
use ssmap_render::{
    Content, Fonts, LabelLayout, MapScene, StampAtlas, StampIndex, StampShape, StampSource,
};

use crate::layer_tree::node_name;
use crate::tools::ToolId;

/// How long a message stays in the status bar.
pub const NOTICE_DURATION: Duration = Duration::from_secs(5);

pub struct Editor {
    pub map: Map,
    /// What the Canvas draws, derived from the Map, and the Label layouts that picking and
    /// handles share with it.
    pub scene: MapScene,
    history: History,
    selection: Selection,
    /// The Layer new Objects go into.
    active_layer: Option<Id>,
    pub active_tool: ToolId,
    notice: Option<Notice>,
    /// Set while a gesture's transaction is open.
    gesture_open: bool,
    /// The Selection when the open gesture began, restored if it is cancelled.
    gesture_selection: Selection,
    /// Rebuilt lazily once the Map has changed.
    pick_index: Option<PickIndex>,
    /// Roughened outlines, shared by the screen and picking so they agree.
    roughness: RoughnessCache,
    /// Every loaded Asset pack.
    pub library: Arc<PackLibrary>,
    /// The Asset the Stamp tool places, chosen in the Asset browser.
    pub selected_asset: Option<AssetRef>,
    /// The shapes of the Assets drawn so far, for picking and selection bounds.
    stamp_shapes: HashMap<AssetRef, StampShape>,
    /// The Map revision `stamp_shapes` was last gathered for.
    shapes_revision: Option<u64>,
    /// Rebuilt lazily once the Map has changed.
    stamp_index: Option<StampIndex>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub text: String,
    pub is_error: bool,
    pub shown_at: Instant,
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
            active_layer: None,
            active_tool: ToolId::Select,
            notice: None,
            gesture_open: false,
            gesture_selection: Vec::new(),
            pick_index: None,
            roughness: RoughnessCache::new(),
            library: Arc::default(),
            selected_asset: None,
            stamp_shapes: HashMap::new(),
            shapes_revision: None,
            stamp_index: None,
        }
    }

    pub fn with_library(map: Map, library: Arc<PackLibrary>) -> Self {
        let selected_asset = library.packs().iter().find_map(|pack| {
            let asset = pack.assets.first()?;
            AssetRef::new(&pack.id, &asset.id).ok()
        });
        Self {
            library,
            selected_asset,
            ..Self::new(map)
        }
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
        self.selection = ids;
        if let Some(&last) = self.selection.last() {
            self.active_layer = match self.map.node(last) {
                Some(Node::Layer(_)) => Some(last),
                _ => self.map.parent(last),
            };
        }
    }

    /// The topmost unlocked Landmass or Lake drawn at `point`, in Canvas units.
    fn pick_outline(&mut self, point: [f64; 2]) -> Option<Id> {
        let map = &self.map;
        let index = match &mut self.pick_index {
            Some(index) if index.is_current(map) => index,
            slot => slot.insert(PickIndex::build(map, &mut self.roughness)),
        };
        index.pick(point)
    }

    /// What the Canvas draws for the Map, rebuilt only once it has changed. Drops roughened
    /// outlines of Objects that are gone.
    ///
    /// `gpu` is the Stamp atlas and device it lives on; without one (UI tests) Stamps draw
    /// nothing. Building the content also loads every Asset the Stamps use, so afterwards
    /// picking and selection bounds know their shapes.
    pub fn scene_content(
        &mut self,
        gpu: Option<(&mut StampAtlas, &wgpu::Device, &wgpu::Queue)>,
    ) -> Content {
        self.roughness.retain(&self.map);
        let mut source = gpu.map(|(atlas, device, queue)| StampSource {
            library: &self.library,
            atlas,
            device,
            queue,
        });
        let content = self
            .scene
            .content(&self.map, &mut self.roughness, source.as_mut());
        if self.shapes_revision != Some(self.map.revision())
            && let Some(source) = &source
        {
            self.shapes_revision = Some(self.map.revision());
            for id in self.map.draw_order() {
                if let Some(stamp) = self.stamp(id)
                    && let Some(sprite) = source.atlas.loaded(&stamp.asset)
                {
                    self.stamp_shapes
                        .insert(stamp.asset.clone(), sprite.shape.clone());
                }
            }
        }
        content
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

    /// What a Stamp's Asset looks like, once it has been drawn.
    pub fn stamp_shape(&self, asset: &AssetRef) -> Option<&StampShape> {
        self.stamp_shapes.get(asset)
    }

    /// The topmost unlocked Stamp whose Silhouette is under `point`.
    fn pick_stamp(&mut self, point: [f64; 2]) -> Option<Id> {
        let map = &self.map;
        let shapes = &self.stamp_shapes;
        let index = match &mut self.stamp_index {
            Some(index) if index.is_current(map) => index,
            slot => slot.insert(StampIndex::build(map, |asset| shapes.get(asset))),
        };
        index.pick(point).map(|(id, _)| id)
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

    /// Takes the Ops that committed steps, undo and redo applied since the last call, for the
    /// Recovery journal.
    pub fn take_applied(&mut self) -> Vec<Op> {
        self.history.take_applied()
    }

    /// Creates a Layer above the active one (or on top), selects it and returns its id.
    pub fn new_layer(&mut self) -> Option<Id> {
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
        let name = format!("Layer {}", self.layer_count() + 1);
        let command = Command::CreateLayer {
            id,
            parent,
            index,
            name,
        };
        self.run_selecting(command, vec![id]).then_some(id)
    }

    pub fn delete_selection(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        let ids = self.selection.clone();
        self.run_selecting(Command::Delete { ids }, Vec::new());
    }

    pub fn notice(&self) -> Option<&Notice> {
        self.notice
            .as_ref()
            .filter(|notice| notice.shown_at.elapsed() < NOTICE_DURATION)
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice {
            text: text.into(),
            is_error: false,
            shown_at: Instant::now(),
        });
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice {
            text: text.into(),
            is_error: true,
            shown_at: Instant::now(),
        });
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

    /// A Label and its shaped text.
    pub fn label_layout(&mut self, id: Id) -> Option<(Label, Arc<LabelLayout>)> {
        let label = self.label(id)?.clone();
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
        let Some(parent) = self.target_layer() else {
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

    /// The Layer new Objects go into: the active Layer, else the topmost, creating one if the
    /// Map has none. Runs inside an open gesture.
    fn target_layer(&mut self) -> Option<Id> {
        if let Some(layer) = self.active_layer.filter(|&id| self.map.contains(id)) {
            return Some(layer);
        }
        if let Some(&top) = self.map.children(None).last() {
            return Some(top);
        }
        let id = Id::new();
        let name = format!("Layer {}", self.layer_count() + 1);
        self.gesture(Command::CreateLayer {
            id,
            parent: None,
            index: None,
            name,
        })
        .then_some(id)
    }

    /// The Asset the Stamp tool places next: the selected one, or a random member of its
    /// Variant group. The Stamp records the concrete Asset, so the Map never depends on the
    /// pack's grouping afterwards.
    pub fn next_stamp_asset(&self) -> Option<AssetRef> {
        let selected = self.selected_asset.clone()?;
        let (pack, asset) = self.library.asset(&selected)?;
        let Some(group) = &asset.variant_group else {
            return Some(selected);
        };
        let members: Vec<&ssmap_core::asset::Asset> = pack.variant_group(group).collect();
        let pick = members[random_below(members.len())];
        AssetRef::new(&pack.id, &pick.id).ok()
    }

    /// Places the selected Asset as a Stamp at `position` in the active Layer, creating a
    /// Layer if the Map has none, as one Undo step.
    pub fn place_stamp(&mut self, position: [f64; 2]) -> Option<Id> {
        let asset = self.next_stamp_asset()?;
        self.end_gesture();
        self.begin_gesture("Add Stamp");
        let Some(parent) = self.target_layer() else {
            self.cancel_gesture();
            return None;
        };
        let id = Id::new();
        let stamp = Stamp {
            position,
            ..Stamp::new(asset)
        };
        if !self.gesture(Command::CreateStamp {
            id,
            parent,
            index: None,
            stamp,
        }) {
            self.cancel_gesture();
            return None;
        }
        self.select(vec![id]);
        self.end_gesture();
        Some(id)
    }

    fn layer_count(&self) -> usize {
        self.map
            .ids()
            .filter(|&id| self.map.node(id).is_some_and(Node::is_layer))
            .count()
    }

    /// Restores a recorded Selection, dropping anything that no longer exists.
    fn restore(&mut self, mut selection: Selection) {
        selection.retain(|&id| self.map.contains(id));
        self.select(selection);
        if self.active_layer.is_some_and(|id| !self.map.contains(id)) {
            self.active_layer = None;
        }
    }
}

/// A pseudo-random number below `bound` (at least 1), seeded from the clock. Used only to
/// choose among variants; the choice is recorded in the Command, so replay is exact.
fn random_below(bound: usize) -> usize {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    (hasher.finish() % bound as u64) as usize
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
        assert!(notice.is_error);
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

    /// An editor holding the core test pack, with its first Asset selected.
    fn stamping_editor() -> Editor {
        let roots = ssmap_core::asset::PackRoots {
            built_in: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../core/tests/packs"),
            user: None,
        };
        let (library, problems) = PackLibrary::load(&roots);
        assert!(problems.is_empty(), "{problems:?}");
        Editor::with_library(
            Map::new(Canvas::new(500, 500, Scale::new(1.0, DistanceUnit::Miles))),
            Arc::new(library),
        )
    }

    #[test]
    fn placing_a_stamp_creates_a_layer_and_is_one_undo_step() {
        let mut editor = stamping_editor();
        assert_eq!(
            editor.selected_asset,
            Some("test-pack/mountain-1".parse().unwrap())
        );
        let id = editor.place_stamp([200.0, 150.0]).unwrap();
        assert_eq!(editor.stamp(id).unwrap().position, [200.0, 150.0]);
        assert_eq!(editor.selection(), &[id]);
        assert_eq!(editor.undo_label(), Some("Add Stamp"));
        editor.undo();
        assert!(
            editor.map.children(None).is_empty(),
            "the Layer is undone too"
        );
    }

    #[test]
    fn placing_picks_a_variant_but_records_a_concrete_asset() {
        let mut editor = stamping_editor();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..64 {
            let asset = editor.next_stamp_asset().unwrap();
            assert_eq!(asset.pack, "test-pack");
            seen.insert(asset.asset);
        }
        let expected: std::collections::HashSet<String> =
            ["mountain-1".to_owned(), "mountain-2".to_owned()].into();
        assert_eq!(seen, expected, "only the Variant group, and both of them");
        // A lone Asset is always itself.
        editor.selected_asset = Some("test-pack/castle".parse().unwrap());
        assert_eq!(
            editor.next_stamp_asset(),
            Some("test-pack/castle".parse().unwrap())
        );
        editor.selected_asset = Some("test-pack/ghost".parse().unwrap());
        assert_eq!(editor.next_stamp_asset(), None, "not in any pack");
    }

    #[test]
    fn picking_finds_the_stamp_under_its_silhouette() {
        let mut editor = stamping_editor();
        editor.selected_asset = Some("test-pack/castle".parse().unwrap());
        let id = editor.place_stamp([200.0, 200.0]).unwrap();
        let mut mask = ssmap_render::raster::Raster::new(2, 2);
        mask.pixels = vec![255; 16];
        // A 100x100 box with its Anchor at the bottom centre: x 150..250, y 100..200.
        editor.stamp_shapes.insert(
            "test-pack/castle".parse().unwrap(),
            StampShape {
                size: [100.0, 100.0],
                anchor: [0.5, 1.0],
                mask: Arc::new(mask),
            },
        );
        assert_eq!(editor.pick([200.0, 150.0]), Some(id));
        assert_eq!(editor.pick([200.0, 250.0]), None, "below the Anchor");
        editor.run(Command::SetLocked { id, locked: true });
        assert_eq!(
            editor.pick([200.0, 150.0]),
            None,
            "locked Stamps aren't picked"
        );
    }

    fn opacity(editor: &Editor, id: Id) -> f32 {
        match editor.map.node(id) {
            Some(Node::Layer(layer)) => layer.opacity,
            _ => panic!("a Layer"),
        }
    }
}
