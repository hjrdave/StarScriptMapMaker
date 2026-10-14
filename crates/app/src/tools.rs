//! The tool framework. The active tool is editor state; each tool turns Canvas input into
//! view changes or Commands. Object tools (land brush, stamps) arrive with their tickets and
//! register here.

use eframe::egui;
use ssmap_core::{Command, Id, Label};
use ssmap_render::View;

use crate::editor::Editor;
use crate::handles::{self, Bounds, Grip};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolId {
    Select,
    Pan,
    Label,
}

impl ToolId {
    pub const ALL: [ToolId; 3] = [ToolId::Select, ToolId::Pan, ToolId::Label];

    pub fn label(self) -> &'static str {
        match self {
            ToolId::Select => "Select",
            ToolId::Pan => "Pan",
            ToolId::Label => "Label",
        }
    }

    pub fn shortcut(self) -> egui::Key {
        match self {
            ToolId::Select => egui::Key::V,
            ToolId::Pan => egui::Key::H,
            ToolId::Label => egui::Key::T,
        }
    }
}

/// What a tool sees of one frame of Canvas input.
pub struct CanvasInput<'a> {
    pub response: &'a egui::Response,
    /// Physical pixels per egui point.
    pub pixels_per_point: f64,
}

/// Tool state that lasts across frames.
#[derive(Default)]
pub struct ToolState {
    drag: Option<Drag>,
    /// The Label whose text is being typed in the overlay box.
    pub editing: Option<TextEditing>,
}

/// Typing a Label's text in a box overlaid at the Label, inside an open gesture so the
/// whole edit is one Undo step.
pub struct TextEditing {
    pub id: Id,
    pub text: String,
    /// Set until the box has taken the keyboard and selected the text.
    pub just_opened: bool,
}

/// A Select-tool drag on the selection.
struct Drag {
    grip: Option<Grip>,
    /// Where the drag began, in Canvas units.
    start: [f64; 2],
    /// The selection's bounds when it began.
    bounds: Bounds,
    /// The selected Labels as they were, so every frame transforms from the start and
    /// rounding never builds up.
    labels: Vec<(Id, Label)>,
}

impl ToolState {
    /// Opens the text box on a Label, inside a gesture: the caller's, or a new "Edit Label".
    pub fn edit_text(&mut self, editor: &mut Editor, id: Id) {
        let Some(label) = editor.label(id) else {
            return;
        };
        let text = label.text.clone();
        editor.begin_gesture("Edit Label");
        self.editing = Some(TextEditing {
            id,
            text,
            just_opened: true,
        });
    }

    /// Whether a drag is moving, scaling or rotating the selection.
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }
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
    let ppp = input.pixels_per_point;
    let space_held = response.ctx.input(|i| i.key_down(egui::Key::Space));
    let temporary_pan = response.dragged_by(egui::PointerButton::Middle)
        || (space_held && response.dragged_by(egui::PointerButton::Primary));
    if temporary_pan || (editor.active_tool == ToolId::Pan && response.dragged()) {
        let delta = response.drag_delta();
        view.pan([f64::from(delta.x) * ppp, f64::from(delta.y) * ppp]);
        return;
    }
    let to_canvas = |pos: egui::Pos2| {
        let local = pos - response.rect.min;
        view.to_canvas([f64::from(local.x) * ppp, f64::from(local.y) * ppp])
    };
    let shift = response.ctx.input(|i| i.modifiers.shift);
    match editor.active_tool {
        ToolId::Select => {
            if response.drag_started_by(egui::PointerButton::Primary)
                && let Some(pos) = response.ctx.input(|i| i.pointer.press_origin())
            {
                start_drag(
                    editor,
                    view,
                    state,
                    response.rect.min,
                    ppp,
                    pos,
                    to_canvas(pos),
                );
            }
            if let Some(drag) = &state.drag {
                // Escape or undo ended the gesture under the drag.
                if !editor.gesture_open() {
                    state.drag = None;
                } else if response.dragged_by(egui::PointerButton::Primary)
                    && let Some(pos) = response.interact_pointer_pos()
                {
                    let commands = drag_commands(drag, to_canvas(pos), shift);
                    for command in commands {
                        editor.gesture(command);
                    }
                }
            }
            if response.drag_stopped() && state.drag.take().is_some() {
                editor.end_gesture();
            }
            if let Some(pos) = response.interact_pointer_pos() {
                let point = to_canvas(pos);
                if response.double_clicked() {
                    if let Some(id) = editor.pick(point)
                        && editor.label(id).is_some()
                    {
                        editor.select(vec![id]);
                        state.edit_text(editor, id);
                    }
                } else if response.clicked() {
                    let hit = editor.pick(point);
                    click_select(editor, hit, shift);
                }
            }
        }
        ToolId::Pan => {}
        ToolId::Label => {
            if response.clicked()
                && let Some(pos) = response.interact_pointer_pos()
                && let Some(id) = editor.place_label(to_canvas(pos))
            {
                // Typing joins the "Add Label" gesture, so placing and naming undo together.
                state.editing = Some(TextEditing {
                    id,
                    text: editor.label(id).map(|l| l.text.clone()).unwrap_or_default(),
                    just_opened: true,
                });
                editor.active_tool = ToolId::Select;
            }
        }
    }
}

/// A click: selects what is under the pointer, Shift adds or removes it, and empty Canvas
/// clears the Selection.
fn click_select(editor: &mut Editor, hit: Option<Id>, shift: bool) {
    match (hit, shift) {
        (Some(id), true) => {
            let mut selection = editor.selection().to_vec();
            match selection.iter().position(|&s| s == id) {
                Some(i) => {
                    selection.remove(i);
                }
                None => selection.push(id),
            }
            editor.select(selection);
        }
        (Some(id), false) => editor.select(vec![id]),
        (None, true) => {}
        (None, false) => editor.select(Vec::new()),
    }
}

/// Starts moving, scaling or rotating the selection, picking what is under the pointer
/// first if it isn't selected.
fn start_drag(
    editor: &mut Editor,
    view: &View,
    state: &mut ToolState,
    origin: egui::Pos2,
    ppp: f64,
    pos: egui::Pos2,
    point: [f64; 2],
) {
    let mut grip = None;
    let mut bounds = Bounds::of_selection(editor);
    match bounds.and_then(|b| handles::hit(&b, view, origin, ppp, pos)) {
        Some(hit) => grip = Some(hit),
        None => {
            let on_selection = bounds.is_some_and(|b| b.contains(point));
            if !on_selection {
                let Some(id) = editor.pick(point) else {
                    return;
                };
                editor.select(vec![id]);
                bounds = Bounds::of_selection(editor);
            }
        }
    }
    let labels: Vec<(Id, Label)> = editor
        .selection()
        .iter()
        .filter_map(|&id| editor.label(id).map(|label| (id, label.clone())))
        .collect();
    let Some(bounds) = bounds.filter(|_| !labels.is_empty()) else {
        return;
    };
    editor.begin_gesture(match grip {
        None => "Move",
        Some(Grip::Resize(_)) => "Scale",
        Some(Grip::Rotate) => "Rotate",
    });
    state.drag = Some(Drag {
        grip,
        start: point,
        bounds,
        labels,
    });
}

/// The Commands that put the dragged Labels where the pointer at `point` says.
fn drag_commands(drag: &Drag, point: [f64; 2], snap: bool) -> Vec<Command> {
    let delta = [point[0] - drag.start[0], point[1] - drag.start[1]];
    let transform: Box<dyn Fn(&Label) -> Label> = match drag.grip {
        None => Box::new(move |label| handles::moved(label, delta)),
        Some(Grip::Resize(handle)) => {
            let (scale, fixed) = drag.bounds.scaling(handle, delta);
            Box::new(move |label| handles::scaled(label, scale, fixed))
        }
        Some(Grip::Rotate) => {
            let centre = drag.bounds.centre();
            let angle = |p: [f64; 2]| (p[1] - centre[1]).atan2(p[0] - centre[0]).to_degrees();
            let mut degrees = angle(point) - angle(drag.start);
            if snap {
                degrees = (degrees / 15.0).round() * 15.0;
            }
            Box::new(move |label| handles::rotated(label, degrees, centre))
        }
    };
    drag.labels
        .iter()
        .map(|(id, label)| Command::SetLabel {
            id: *id,
            label: Box::new(transform(label)),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handles::Handle;

    fn drag(grip: Option<Grip>) -> Drag {
        Drag {
            grip,
            start: [100.0, 0.0],
            bounds: Bounds {
                min: [-100.0, -50.0],
                max: [100.0, 50.0],
            },
            labels: vec![(
                Id::new(),
                Label {
                    text: "x".into(),
                    size: 10.0,
                    ..Label::default()
                },
            )],
        }
    }

    fn dragged(drag: &Drag, point: [f64; 2], snap: bool) -> Label {
        match drag_commands(drag, point, snap).remove(0) {
            Command::SetLabel { label, .. } => *label,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn drags_transform_from_where_they_began() {
        let moved = dragged(&drag(None), [130.0, 20.0], false);
        assert_eq!(moved.position, [30.0, 20.0]);
        let scaled = dragged(
            &drag(Some(Grip::Resize(Handle::Right))),
            [300.0, 0.0],
            false,
        );
        assert_eq!(scaled.size, 20.0);
        assert_eq!(scaled.position, [100.0, 0.0], "scaled about the left edge");
        let rotated = dragged(&drag(Some(Grip::Rotate)), [0.0, 100.0], false);
        assert!((rotated.rotation - 90.0).abs() < 1e-9);
        let snapped = dragged(&drag(Some(Grip::Rotate)), [100.0, 20.0], true);
        assert!((snapped.rotation - 15.0).abs() < 1e-9);
    }
}
