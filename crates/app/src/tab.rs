//! One open Map and everything about it that is per Map: its editor, file, view and UI state.
//!
//! [`MapMakerApp`](crate::MapMakerApp) holds a list of `MapTab`s and routes through the active
//! one. A `MapTab` needs no window: it can be built, opened from a file and driven with
//! Commands in a plain test.
//!
//! # Canvas renderer with several tabs
//!
//! `canvas_view::install` creates one `CanvasRenderer` for the whole run. The renderer owns GPU
//! pipelines and the sprite atlas, which do not depend on the Map and are expensive, while a
//! `TessellationCache` holds tessellated geometry keyed by Object, which is only valid for one
//! Map. So **there is one renderer and each tab has its own `TessellationCache`**, created with
//! the tab and dropped with it. Each frame the active tab's cache is handed to the renderer
//! (`CanvasRenderer::use_tessellation_cache`), so switching back to a fully built Map only
//! merges its cached meshes again and tessellates nothing.
//!
//! Background work **keeps going** when a tab is not active: the tessellation and roughness
//! workers own their threads and finish into the tab's own caches. What pauses is only the
//! starting of further work, which the active tab's frame does; it resumes on return.

use std::path::{Path, PathBuf};

use ssmap_core::Map;
use ssmap_core::file::FileError;
use ssmap_render::TessellationCache;

use crate::canvas_view::CanvasView;
use crate::document::Document;
use crate::editor::Editor;
use crate::label_styles::LabelStylesPanel;
use crate::layer_tree::LayerTree;
use crate::properties::PropertiesPanel;
use crate::recovery::Recovery;
use crate::{Pending, RecoveryPrompt};

pub struct MapTab {
    pub(crate) editor: Editor,
    pub(crate) document: Document,
    pub(crate) layer_tree: LayerTree,
    pub(crate) properties: PropertiesPanel,
    pub(crate) label_styles: LabelStylesPanel,
    pub(crate) canvas_view: CanvasView,
    pub(crate) recovery: Recovery,
    /// A Recovery journal left by a crash, awaiting the user's decision.
    pub(crate) recovery_prompt: Option<RecoveryPrompt>,
    /// What to do once the user decides what to do with this Map's unsaved changes.
    pub(crate) pending: Option<Pending>,
    /// The folder of the last export, for Re-export when the Map has no file yet.
    pub(crate) export_directory: Option<PathBuf>,
    /// While quitting: the user has already decided about this tab's unsaved changes.
    pub(crate) quit_decided: bool,
}

impl MapTab {
    /// A tab showing `map` as a Map never saved. `cache` is this tab's tessellation cache.
    pub fn new(map: Map, cache: Option<TessellationCache>, recovery: Recovery) -> Self {
        Self {
            editor: Editor::new(map),
            document: Document::default(),
            layer_tree: LayerTree::default(),
            properties: PropertiesPanel::default(),
            label_styles: LabelStylesPanel::default(),
            canvas_view: CanvasView::new(cache),
            recovery,
            recovery_prompt: None,
            pending: None,
            export_directory: None,
            quit_decided: false,
        }
    }

    /// A tab showing the Map file at `path`.
    pub fn open(
        path: &Path,
        cache: Option<TessellationCache>,
        recovery: Recovery,
    ) -> Result<Self, FileError> {
        let (document, map) = Document::open(path)?;
        let mut tab = Self::new(map, cache, recovery);
        tab.document = document;
        Ok(tab)
    }

    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.document.has_unsaved_changes(&self.editor.map)
    }

    /// The Recovery journal being written for this tab's Map, once there is one.
    pub fn recovery_journal(&self) -> Option<&Path> {
        self.recovery.journal_path()
    }

    /// This tab's tessellation cache; `None` without a wgpu device.
    pub fn tessellation_cache(&self) -> Option<&TessellationCache> {
        self.canvas_view.cache()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssmap_core::{Canvas, Command, DistanceUnit, Id, Scale};

    #[test]
    fn a_tab_opens_a_map_and_tracks_unsaved_changes_without_a_window() {
        let dir = std::env::temp_dir().join(format!("ssmap-tab-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Isle.ssmap");
        let canvas = Canvas::new(400, 300, Scale::new(1.0, DistanceUnit::Miles));
        let mut tab = MapTab::new(Map::new(canvas), None, Recovery::new(None));
        tab.document.save_as(&path, &tab.editor.map, None).unwrap();

        let mut tab = MapTab::open(&path, None, Recovery::new(None)).unwrap();
        assert_eq!(tab.document().name(), "Isle.ssmap");
        assert!(!tab.has_unsaved_changes());
        let layers = tab.editor().map.children(None).len();
        assert!(tab.editor.run(Command::CreateLayer {
            id: Id::new(),
            parent: None,
            index: None,
            name: "Roads".into(),
        }));
        assert_eq!(tab.editor().map.children(None).len(), layers + 1);
        assert!(tab.has_unsaved_changes());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
