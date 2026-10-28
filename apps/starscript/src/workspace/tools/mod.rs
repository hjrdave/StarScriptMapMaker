//! The tool framework. The active tool is editor state; each tool turns Canvas input into
//! view changes or Commands. Object tools (stamps) arrive with their tickets and register
//! here.

mod brush;
mod canvas_tool;
mod drag;
mod export_region;
mod input;
mod label;
mod lasso;
mod node_edit;
mod pan;
mod scatter;
mod select;
#[cfg(test)]
mod test_util;

use eframe::egui;
use ssmap_core::{Command, ExportRegion, Id, Point};
use ssmap_render::View;

use crate::editor::{Editor, GestureId};
use crate::export_region::RegionDrag;

use self::brush::Stroke;
use self::drag::Drag;
use self::input::CanvasFrame;
use self::lasso::Lasso;
use self::node_edit::NodeDrag;
use self::select::SelectBox;

pub use self::export_region::custom_region;
pub use self::label::TextEditing;
pub(crate) use self::select::selection_has_label;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolId {
    Select,
    Pan,
    LandBrush,
    WaterBrush,
    Lasso,
    Label,
    Stamp,
    StampEraser,
    Canvas,
    /// Drawing the custom export region; started from the Export dialog, not the dock.
    ExportRegion,
}

impl ToolId {
    pub const ALL: [ToolId; 9] = [
        ToolId::Select,
        ToolId::Pan,
        ToolId::LandBrush,
        ToolId::WaterBrush,
        ToolId::Lasso,
        ToolId::Label,
        ToolId::Stamp,
        ToolId::StampEraser,
        ToolId::Canvas,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ToolId::Select => "Select",
            ToolId::Pan => "Pan",
            ToolId::LandBrush => "Land Brush",
            ToolId::WaterBrush => "Water Brush",
            ToolId::Lasso => "Lasso",
            ToolId::Label => "Label",
            ToolId::Stamp => "Stamp",
            ToolId::StampEraser => "Stamp Eraser",
            ToolId::Canvas => "Canvas",
            ToolId::ExportRegion => "Export Region",
        }
    }

    /// The key and modifiers that select the tool, if it has a shortcut.
    pub fn shortcut(self) -> Option<(egui::Modifiers, egui::Key)> {
        let none = egui::Modifiers::NONE;
        Some(match self {
            ToolId::Select => (none, egui::Key::V),
            ToolId::Pan => (none, egui::Key::H),
            ToolId::LandBrush => (none, egui::Key::B),
            ToolId::WaterBrush => (none, egui::Key::W),
            ToolId::Lasso => (none, egui::Key::L),
            ToolId::Label => (none, egui::Key::T),
            ToolId::Stamp => (none, egui::Key::S),
            ToolId::StampEraser => (none, egui::Key::E),
            ToolId::Canvas => (none, egui::Key::A),
            ToolId::ExportRegion => return None,
        })
    }

    /// The shortcut as shown in the UI, such as "V" or "⇧S".
    pub fn shortcut_text(self) -> String {
        let Some((modifiers, key)) = self.shortcut() else {
            return String::new();
        };
        let shift = if modifiers.shift { "⇧" } else { "" };
        format!("{shift}{}", key.name())
    }
}

/// The modifier that turns the land brush into an eraser (Option on macOS).
pub const ERASE_MODIFIER: &str = if cfg!(target_os = "macos") {
    "Option"
} else {
    "Alt"
};

/// The land brush size range, as a diameter in Canvas units.
pub const BRUSH_SIZES: std::ops::RangeInclusive<f64> = 2.0..=2000.0;

/// Where the Size slider's track ends; the number box types on up to the hard maximum.
pub const BRUSH_SIZE_SLIDER_MAX: f64 = 200.0;

/// Whether the tool paints with the shared brush size.
pub fn uses_brush_size(tool: ToolId) -> bool {
    matches!(
        tool,
        ToolId::LandBrush | ToolId::WaterBrush | ToolId::Stamp | ToolId::StampEraser
    )
}

/// How close, in egui points, the pointer must be to grab a node or an edge.
const NODE_GRAB: f64 = 8.0;
const EDGE_GRAB: f64 = 6.0;

/// Tool state that lasts across frames.
#[derive(Default)]
pub struct ToolState {
    drag: Option<Drag>,
    stroke: Option<Stroke>,
    node_drag: Option<NodeDrag>,
    lasso: Option<Lasso>,
    /// An Export Region drag in progress, with the region as it was before the drag.
    region_drag: Option<(RegionDrag, ExportRegion)>,
    /// A board resize in progress.
    canvas_drag: Option<crate::canvas_resize::CanvasDrag>,
    /// A board being moved by its label.
    board_move: Option<crate::canvas_resize::BoardMove>,
    /// A board being drawn on empty pasteboard.
    board_create: Option<crate::canvas_resize::BoardCreate>,
    /// Where each board's label was last drawn, in screen points, for clicking it.
    board_labels: Vec<(Id, egui::Rect)>,
    /// A box-select in node-edit mode: where it began and where the pointer is, in Canvas units.
    marquee: Option<(Point, Point)>,
    /// A box-select dragged from empty Canvas with the Select tool.
    select_box: Option<SelectBox>,
    /// The Label whose text is being typed in the overlay box.
    pub editing: Option<TextEditing>,
    /// The gesture a `drag`, `stroke`, `node_drag`, `canvas_drag`, `board_move` or
    /// `board_create` holds open, and the tool that opened it.
    held: Option<(ToolId, GestureId)>,
}

impl Drop for ToolState {
    fn drop(&mut self) {
        // The Editor goes with the tool state, taking the open gesture with it.
        if let Some((_, id)) = self.held.take() {
            std::mem::forget(id);
        }
    }
}

impl ToolState {
    /// Drops a node drag or box-select in progress, as when node editing ends. The gesture a
    /// node drag held is rolled back.
    pub fn clear_node_gestures(&mut self, editor: &mut Editor) {
        if self.node_drag.take().is_some() {
            self.cancel_held(editor);
        }
        self.marquee = None;
        self.select_box = None;
    }

    /// Opens a gesture for `tool` to hold until [`Self::end_held`] or [`Self::cancel_held`].
    /// `false` if it could not be opened (the status bar says why).
    fn begin_held(&mut self, editor: &mut Editor, tool: ToolId, label: &str) -> bool {
        let id = editor.begin_gesture(label);
        if editor.is_open(&id) {
            self.held = Some((tool, id));
            true
        } else {
            editor.cancel_gesture(id);
            false
        }
    }

    /// Whether the held gesture is still open. Escape or undo can close it under a drag; a
    /// stale token is dropped.
    fn holds_open(&mut self, editor: &mut Editor) -> bool {
        let live = self.held.as_ref().is_some_and(|(_, id)| editor.is_open(id));
        if !live {
            self.cancel_held(editor);
        }
        live
    }

    /// Runs a Command in the held gesture.
    fn run_held(&self, editor: &mut Editor, command: Command) -> bool {
        self.held
            .as_ref()
            .is_some_and(|(_, id)| editor.gesture(id, command))
    }

    /// Replaces what the held gesture has done so far with `command`.
    fn replace_held(&self, editor: &mut Editor, command: Command) -> bool {
        self.held
            .as_ref()
            .is_some_and(|(_, id)| editor.gesture_replace(id, command))
    }

    /// Commits the held gesture as one Undo step.
    fn end_held(&mut self, editor: &mut Editor) {
        if let Some((_, id)) = self.held.take() {
            editor.end_gesture(id);
        }
    }

    /// Hands over the held gesture's token, for a call that ends it.
    fn take_held(&mut self) -> Option<GestureId> {
        self.held.take().map(|(_, id)| id)
    }

    /// Rolls the held gesture back.
    fn cancel_held(&mut self, editor: &mut Editor) {
        if let Some((_, id)) = self.held.take() {
            editor.cancel_gesture(id);
        }
    }

    /// Rolls back a gesture held for a tool that is no longer active, and drops its drag.
    fn drop_held_for_other_tool(&mut self, editor: &mut Editor) {
        if self
            .held
            .as_ref()
            .is_some_and(|(tool, _)| *tool != editor.active_tool)
        {
            self.cancel_held(editor);
            self.drag = None;
            self.stroke = None;
            self.node_drag = None;
            self.canvas_drag = None;
            self.board_move = None;
            self.board_create = None;
        }
    }

    /// Whether a drag is moving, scaling or rotating the selection.
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// The cursor HUD while a board is being resized or drawn, such as "Canvas · 4096 × 2560".
    pub fn canvas_hud(&self, editor: &Editor) -> Option<String> {
        let board = self.canvas_drag.map(|drag| drag.board).or_else(|| {
            self.board_create
                .filter(|create| create.created)
                .map(|create| create.board)
        })?;
        let map = editor.map();
        let canvas = &map.board(board)?.canvas;
        Some(crate::canvas_resize::hud_text(
            &map.board_label(board)?,
            canvas.width,
            canvas.height,
        ))
    }
}

/// Lets the active tool handle Canvas input. Panning with the middle button, or with Space
/// held, works whatever the tool. This is the only place that reads the `egui::Response`; the
/// tools see a `CanvasFrame`.
pub fn canvas_input(
    editor: &mut Editor,
    view: &mut View,
    state: &mut ToolState,
    response: &egui::Response,
    pixels_per_point: f64,
) {
    let frame = CanvasFrame::from_response(response, pixels_per_point);
    if editor.active_tool != ToolId::Select {
        editor.leave_node_edit();
    }
    state.drop_held_for_other_tool(editor);
    if editor.active_tool != ToolId::Canvas {
        state.board_labels.clear();
        editor.select_board(None);
    }
    if pan::pan_tool(editor, view, &frame) {
        return;
    }
    let cursor = match editor.active_tool {
        ToolId::Select if editor.node_edit().is_some() => {
            node_edit::node_edit_tool(editor, view, state, &frame)
        }
        ToolId::Select => select::select_tool(editor, view, state, &frame),
        ToolId::LandBrush | ToolId::WaterBrush => {
            let water = editor.active_tool == ToolId::WaterBrush;
            brush::land_brush(editor, view, state, &frame, water)
        }
        ToolId::Lasso => {
            lasso::lasso_tool(editor, view, state, &frame);
            frame.hovered.then_some(egui::CursorIcon::Crosshair)
        }
        ToolId::Pan => None,
        ToolId::Stamp | ToolId::StampEraser => {
            let erase = editor.active_tool == ToolId::StampEraser;
            scatter::scatter_tool(editor, view, state, &frame, erase)
        }
        ToolId::ExportRegion => export_region::export_region_tool(editor, view, state, &frame),
        ToolId::Canvas => canvas_tool::canvas_tool(editor, view, state, &frame),
        ToolId::Label => {
            label::label_tool(editor, view, state, &frame);
            None
        }
    };
    if let Some(icon) = cursor {
        response.ctx.set_cursor_icon(icon);
    }
    if editor.active_tool != ToolId::ExportRegion {
        state.region_drag = None;
    }
    if editor.active_tool != ToolId::Lasso {
        state.lasso = None;
    }
    let scattering = matches!(editor.active_tool, ToolId::Stamp | ToolId::StampEraser);
    if !scattering && !matches!(editor.active_tool, ToolId::LandBrush | ToolId::WaterBrush) {
        state.stroke = None;
    }
    editor.tool_busy = state.lasso.is_some()
        || state.region_drag.is_some()
        || (scattering && state.stroke.is_some());
}

/// Draws what the active tool shows over the Canvas: the land brush's outline under the
/// pointer, and the area a stroke in progress covers.
pub fn paint_overlay(
    painter: &egui::Painter,
    editor: &Editor,
    view: &View,
    state: &mut ToolState,
    origin: egui::Pos2,
    hover: Option<egui::Pos2>,
    ppp: f64,
) {
    if editor.active_tool == ToolId::Canvas {
        canvas_tool::paint_canvas(painter, editor, view, state, origin, ppp);
    }
    if editor.active_tool == ToolId::Select {
        node_edit::paint_nodes(painter, editor, view, state, origin, hover, ppp);
        select::paint_select_box(painter, view, state, origin, ppp);
    }
    export_region::paint_region(painter, editor, view, origin, ppp);
    if editor.active_tool == ToolId::Lasso {
        lasso::paint_lasso(painter, view, state, origin, hover, ppp);
    }
    if matches!(editor.active_tool, ToolId::Stamp | ToolId::StampEraser) {
        scatter::paint_scatter_cursor(painter, editor, view, state, origin, hover, ppp);
        return;
    }
    brush::paint_brush(painter, editor, view, state, origin, hover, ppp);
}
