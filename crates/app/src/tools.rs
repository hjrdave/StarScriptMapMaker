//! The tool framework. The active tool is editor state; each tool turns Canvas input into
//! view changes or Commands. Object tools (land brush, stamps) arrive with their tickets and
//! register here.

use eframe::egui;
use ssmap_core::{Affine, Command, Id, Label};
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

/// A Select-tool drag on the selection. Each frame re-issues the whole move, scale or
/// rotation from where it started, so nothing drifts.
struct Drag {
    grip: Option<Grip>,
    /// Where the drag began, in Canvas units.
    start: [f64; 2],
    /// The selection's bounds when it began.
    bounds: Bounds,
    /// The selected Landmasses and Lakes, which move and resize but don't rotate.
    outlines: Vec<Id>,
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
            let bounds = Bounds::of_selection(editor);
            let rotatable = selection_has_label(editor);
            let grip_at = |pos: egui::Pos2| {
                let bounds = bounds?;
                handles::hit(&bounds, view, response.rect.min, ppp, pos, rotatable)
            };
            if state.drag.is_none()
                && let Some(pos) = response.hover_pos()
                && let Some(grip) = grip_at(pos)
            {
                response.ctx.set_cursor_icon(grip.cursor());
            }
            if response.drag_started_by(egui::PointerButton::Primary)
                && !editor.gesture_open()
                && let Some(pos) = response.ctx.input(|i| i.pointer.press_origin())
            {
                start_drag(editor, state, bounds, grip_at(pos), to_canvas(pos));
            }
            if let Some(drag) = &state.drag {
                // Escape or undo ended the gesture under the drag.
                if !editor.gesture_open() {
                    state.drag = None;
                } else if response.dragged_by(egui::PointerButton::Primary)
                    && let Some(pos) = response.interact_pointer_pos()
                    && let Some(commands) = drag_commands(drag, to_canvas(pos), shift)
                {
                    let mut commands = commands.into_iter();
                    if let Some(first) = commands.next() {
                        editor.gesture_replace(first);
                    }
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

/// Whether the selection holds a Label, the only selected things that rotate.
pub(crate) fn selection_has_label(editor: &Editor) -> bool {
    editor
        .selection()
        .iter()
        .any(|&id| editor.label(id).is_some())
}

/// Starts moving, scaling or rotating the selection from the Canvas point `point`, picking
/// what is under the pointer first if it isn't selected. `grip` is the handle under it.
fn start_drag(
    editor: &mut Editor,
    state: &mut ToolState,
    mut bounds: Option<Bounds>,
    grip: Option<Grip>,
    point: [f64; 2],
) {
    if grip.is_none() && !bounds.is_some_and(|b| b.contains(point)) {
        let Some(id) = editor.pick(point) else {
            return;
        };
        if !editor.is_selected(id) {
            editor.select(vec![id]);
        }
        bounds = Bounds::of_selection(editor);
    }
    let outlines = editor.selected_outlines();
    let labels: Vec<(Id, Label)> = editor
        .selection()
        .iter()
        .filter_map(|&id| editor.label(id).map(|label| (id, label.clone())))
        .collect();
    let Some(bounds) = bounds.filter(|_| !(outlines.is_empty() && labels.is_empty())) else {
        return;
    };
    editor.begin_gesture(match grip {
        None => "Move",
        Some(Grip::Resize(_)) => "Resize",
        Some(Grip::Rotate) => "Rotate",
    });
    state.drag = Some(Drag {
        grip,
        start: point,
        bounds,
        outlines,
        labels,
    });
}

/// What a drag does to each Label, from how it was when the drag began.
type LabelTransform = Box<dyn Fn(&Label) -> Label>;

/// The Commands that put the dragged selection where the pointer at `point` says: one for
/// the Landmasses and Lakes, then one per Label. `None` keeps the last frame's, as when a
/// resize reaches the opposite edge and the box has no size.
fn drag_commands(drag: &Drag, point: [f64; 2], snap: bool) -> Option<Vec<Command>> {
    let delta = [point[0] - drag.start[0], point[1] - drag.start[1]];
    let ids = drag.outlines.clone();
    let (outlines, transform): (Option<Command>, LabelTransform) = match drag.grip {
        None => (
            Some(Command::Translate { ids, offset: delta }),
            Box::new(move |label| handles::moved(label, delta)),
        ),
        Some(Grip::Resize(handle)) => {
            let resized = drag.bounds.resized(handle, delta);
            let transform = Affine::rect_to_rect(&drag.bounds.rect(), &resized.rect());
            if !transform.is_invertible() {
                return None;
            }
            let (scale, fixed) = drag.bounds.scaling(handle, delta);
            (
                Some(Command::Transform { ids, transform }),
                Box::new(move |label| handles::scaled(label, scale, fixed)),
            )
        }
        Some(Grip::Rotate) => {
            let centre = drag.bounds.centre();
            let angle = |p: [f64; 2]| (p[1] - centre[1]).atan2(p[0] - centre[0]).to_degrees();
            let mut degrees = angle(point) - angle(drag.start);
            if snap {
                degrees = (degrees / 15.0).round() * 15.0;
            }
            (
                None,
                Box::new(move |label| handles::rotated(label, degrees, centre)),
            )
        }
    };
    let outlines = outlines.filter(|_| !drag.outlines.is_empty());
    let labels = drag.labels.iter().map(|(id, label)| Command::SetLabel {
        id: *id,
        label: Box::new(transform(label)),
    });
    Some(outlines.into_iter().chain(labels).collect())
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
            outlines: Vec::new(),
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
        match drag_commands(drag, point, snap).unwrap().remove(0) {
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

    #[test]
    fn outlines_move_and_resize_with_labels_but_do_not_rotate() {
        let outline = Id::new();
        let mixed = |grip| Drag {
            outlines: vec![outline],
            ..drag(grip)
        };
        let commands = drag_commands(&mixed(None), [130.0, 20.0], false).unwrap();
        assert!(matches!(
            &commands[..],
            [Command::Translate { ids, offset: [30.0, 20.0] }, Command::SetLabel { .. }]
                if ids == &vec![outline]
        ));
        let commands = drag_commands(
            &mixed(Some(Grip::Resize(Handle::Right))),
            [300.0, 0.0],
            false,
        )
        .unwrap();
        assert!(matches!(
            &commands[..],
            [Command::Transform { transform, .. }, Command::SetLabel { .. }]
                if transform.0 == [2.0, 0.0, 0.0, 1.0, 100.0, 0.0]
        ));
        let commands = drag_commands(&mixed(Some(Grip::Rotate)), [0.0, 100.0], false).unwrap();
        assert!(matches!(&commands[..], [Command::SetLabel { .. }]));
        // Dragging the right edge onto the left leaves no box to map onto.
        let collapsed = drag_commands(
            &mixed(Some(Grip::Resize(Handle::Right))),
            [-100.0, 0.0],
            false,
        );
        assert!(collapsed.is_none());
    }
}
