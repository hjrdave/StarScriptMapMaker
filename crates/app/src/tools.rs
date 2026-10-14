//! The tool framework. The active tool is editor state; each tool turns Canvas input into
//! view changes or Commands. Object tools (land brush, stamps, labels) arrive with their
//! tickets and register here.

use eframe::egui;
use ssmap_core::{Affine, Command};
use ssmap_render::View;

use crate::editor::Editor;
use crate::handles::{self, Bounds, Handle};

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

impl CanvasInput<'_> {
    /// The Canvas point under a screen position.
    fn canvas_point(&self, view: &View, pos: egui::Pos2) -> [f64; 2] {
        let local = pos - self.response.rect.min;
        view.to_canvas([
            f64::from(local.x) * self.pixels_per_point,
            f64::from(local.y) * self.pixels_per_point,
        ])
    }
}

/// Tool state that lasts across frames, such as a drag in progress.
#[derive(Default)]
pub struct ToolState {
    drag: Option<Drag>,
}

/// A Select-tool drag. Each frame re-issues the whole move or resize from where it started.
struct Drag {
    /// The Canvas point where the drag began.
    start: [f64; 2],
    kind: DragKind,
}

enum DragKind {
    Move,
    Resize { handle: Handle, bounds: Bounds },
}

/// Lets the active tool handle Canvas input. Panning with the middle button, or with Space
/// held, works whatever the tool.
pub fn canvas_input(
    editor: &mut Editor,
    view: &mut View,
    state: &mut ToolState,
    input: &CanvasInput<'_>,
) {
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
        ToolId::Select => select_tool(editor, view, state, input),
        ToolId::Pan => {}
    }
}

/// Click to select (Shift toggles), drag a selected Object to move it, drag a handle to
/// resize. Each drag is one Undo step, and Escape cancels it.
fn select_tool(editor: &mut Editor, view: &View, state: &mut ToolState, input: &CanvasInput<'_>) {
    let response = input.response;
    let ppp = input.pixels_per_point;
    let shift = response.ctx.input(|i| i.modifiers.shift);
    let bounds = Bounds::of_selection(&editor.map, editor.selection());
    let handle_at =
        |pos: egui::Pos2| bounds.and_then(|b| handles::hit(&b, view, response.rect.min, ppp, pos));

    if let Some(pos) = response.hover_pos()
        && state.drag.is_none()
        && let Some(handle) = handle_at(pos)
    {
        response.ctx.set_cursor_icon(handle.cursor());
    }

    if response.clicked_by(egui::PointerButton::Primary)
        && let Some(pos) = response.interact_pointer_pos()
    {
        let hit = editor.pick(input.canvas_point(view, pos));
        let selection = match hit {
            Some(id) if shift && editor.is_selected(id) => editor
                .selection()
                .iter()
                .copied()
                .filter(|&s| s != id)
                .collect(),
            Some(id) if shift => {
                let mut selection = editor.selection().to_vec();
                selection.push(id);
                selection
            }
            Some(id) => vec![id],
            None if shift => editor.selection().to_vec(),
            None => Vec::new(),
        };
        editor.select(selection);
    }

    if response.drag_started_by(egui::PointerButton::Primary)
        && let Some(pos) = response.interact_pointer_pos()
        && !editor.gesture_open()
    {
        // `press_origin` is where the button went down, before the drag threshold.
        let origin = response
            .ctx
            .input(|i| i.pointer.press_origin())
            .unwrap_or(pos);
        let start_point = input.canvas_point(view, origin);
        state.drag = match (handle_at(origin), bounds) {
            (Some(handle), Some(bounds)) => Some(Drag {
                start: start_point,
                kind: DragKind::Resize { handle, bounds },
            }),
            _ => match editor.pick(start_point) {
                Some(id) => {
                    if !editor.is_selected(id) {
                        editor.select(vec![id]);
                    }
                    Some(Drag {
                        start: start_point,
                        kind: DragKind::Move,
                    })
                }
                None => None,
            },
        };
        if let Some(drag) = &state.drag {
            editor.begin_gesture(match drag.kind {
                DragKind::Move => "Move",
                DragKind::Resize { .. } => "Resize",
            });
        }
    }

    if response.dragged_by(egui::PointerButton::Primary)
        && let Some(drag) = &state.drag
        && let Some(pos) = response.interact_pointer_pos()
    {
        // Escape rolled the gesture back: ignore the rest of this drag.
        if !editor.gesture_open() {
            state.drag = None;
            return;
        }
        let point = input.canvas_point(view, pos);
        let delta = [point[0] - drag.start[0], point[1] - drag.start[1]];
        let ids = editor.selected_outlines();
        let command = match &drag.kind {
            DragKind::Move => Some(Command::Translate { ids, offset: delta }),
            DragKind::Resize { handle, bounds } => {
                let resized = bounds.resized(*handle, delta);
                let transform = Affine::rect_to_rect(&bounds.rect(), &resized.rect());
                // At the opposite edge the box has no size; keep the last frame's shape.
                transform
                    .is_invertible()
                    .then_some(Command::Transform { ids, transform })
            }
        };
        if let Some(command) = command {
            editor.gesture_replace(command);
        }
    }

    if response.drag_stopped() && state.drag.take().is_some() {
        editor.end_gesture();
    }
}
