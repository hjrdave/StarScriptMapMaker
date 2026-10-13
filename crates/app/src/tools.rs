//! The tool framework. The active tool is editor state; each tool turns Canvas input into
//! view changes or Commands. Object tools (land brush, stamps, labels) arrive with their
//! tickets and register here.

use eframe::egui;
use ssmap_render::View;

use crate::editor::Editor;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolId {
    Select,
    Pan,
}

impl ToolId {
    pub const ALL: [ToolId; 2] = [ToolId::Select, ToolId::Pan];

    pub fn label(self) -> &'static str {
        match self {
            ToolId::Select => "Select",
            ToolId::Pan => "Pan",
        }
    }

    pub fn shortcut(self) -> egui::Key {
        match self {
            ToolId::Select => egui::Key::V,
            ToolId::Pan => egui::Key::H,
        }
    }
}

/// What a tool sees of one frame of Canvas input.
pub struct CanvasInput<'a> {
    pub response: &'a egui::Response,
    /// Physical pixels per egui point.
    pub pixels_per_point: f64,
}

/// Lets the active tool handle Canvas input. Panning with the middle button, or with Space
/// held, works whatever the tool.
pub fn canvas_input(editor: &mut Editor, view: &mut View, input: &CanvasInput<'_>) {
    let response = input.response;
    let space_held = response.ctx.input(|i| i.key_down(egui::Key::Space));
    let temporary_pan = response.dragged_by(egui::PointerButton::Middle)
        || (space_held && response.dragged_by(egui::PointerButton::Primary));
    if temporary_pan || (editor.active_tool == ToolId::Pan && response.dragged()) {
        let delta = response.drag_delta();
        view.pan([
            f64::from(delta.x) * input.pixels_per_point,
            f64::from(delta.y) * input.pixels_per_point,
        ]);
        return;
    }
    match editor.active_tool {
        ToolId::Select => {
            // Nothing on the Canvas is pickable yet, so a click there clears the Selection.
            if response.clicked() {
                editor.select(Vec::new());
            }
        }
        ToolId::Pan => {}
    }
}
