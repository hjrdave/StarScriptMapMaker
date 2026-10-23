//! The tool framework. The active tool is editor state; each tool turns Canvas input into
//! view changes or Commands. Object tools (stamps) arrive with their tickets and register
//! here.

use eframe::egui;
use lyon::math::point;
use lyon::path::Path;
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, VertexBuffers,
};
use ssmap_core::land::{self, BrushStroke};
use ssmap_core::{
    Affine, Command, ExportRegion, Id, Label, LabelStyles, Membership, MultiPolygon, Node, NodeRef,
    Point, Stamp,
};
use ssmap_render::View;

use crate::art_board::{self, BoardDrag};
use crate::editor::{Editor, GestureId};
use crate::export_region::{self, RegionDrag};
use crate::handles::{self, Bounds, Grip};

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
    ArtBoard,
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
        ToolId::ArtBoard,
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
            ToolId::ArtBoard => "Art board",
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
            ToolId::ArtBoard => (none, egui::Key::A),
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

/// What a tool sees of one frame of Canvas input.
pub struct CanvasInput<'a> {
    pub response: &'a egui::Response,
    /// Physical pixels per egui point.
    pub pixels_per_point: f64,
}

/// One frame of primary-button pointer input as plain data, so a tool can be a function from
/// this to `Command`s and be tested without a running `egui::Context`. Positions are egui
/// points on the screen; `rect_min` is the top-left of the Canvas widget.
#[derive(Clone, Debug, Default)]
pub(crate) struct PointerInput {
    pub rect_min: egui::Pos2,
    /// Physical pixels per egui point.
    pub pixels_per_point: f64,
    /// The pointer is over the Canvas widget.
    pub hovered: bool,
    pub hover_pos: Option<egui::Pos2>,
    /// Where the button went down.
    pub press_origin: Option<egui::Pos2>,
    /// The pointer position while the button is held, or on release.
    pub pointer_pos: Option<egui::Pos2>,
    pub drag_started: bool,
    pub dragging: bool,
    /// A drag ended (any button).
    pub drag_stopped: bool,
    pub clicked: bool,
    pub double_clicked: bool,
    pub shift: bool,
    pub alt: bool,
    /// Enter went down this frame.
    pub enter: bool,
}

impl PointerInput {
    fn from_response(response: &egui::Response, pixels_per_point: f64) -> Self {
        let primary = egui::PointerButton::Primary;
        let (press_origin, shift, alt, enter) = response.ctx.input(|i| {
            (
                i.pointer.press_origin(),
                i.modifiers.shift,
                i.modifiers.alt,
                i.key_pressed(egui::Key::Enter),
            )
        });
        Self {
            rect_min: response.rect.min,
            pixels_per_point,
            hovered: response.hovered(),
            hover_pos: response.hover_pos(),
            press_origin,
            pointer_pos: response.interact_pointer_pos(),
            drag_started: response.drag_started_by(primary),
            dragging: response.dragged_by(primary),
            drag_stopped: response.drag_stopped(),
            clicked: response.clicked_by(primary),
            double_clicked: response.double_clicked_by(primary),
            shift,
            alt,
            enter,
        }
    }

    /// The Canvas point under a screen position.
    fn canvas_point(&self, view: &View, pos: egui::Pos2) -> [f64; 2] {
        let local = pos - self.rect_min;
        view.to_canvas([
            f64::from(local.x) * self.pixels_per_point,
            f64::from(local.y) * self.pixels_per_point,
        ])
    }
}

/// Tool state that lasts across frames.
#[derive(Default)]
pub struct ToolState {
    drag: Option<Drag>,
    stroke: Option<Stroke>,
    node_drag: Option<NodeDrag>,
    lasso: Option<Lasso>,
    /// An Export Region drag in progress, with the region as it was before the drag.
    region_drag: Option<(RegionDrag, ExportRegion)>,
    /// An Art board resize in progress.
    board_drag: Option<BoardDrag>,
    /// Where the board's label was last drawn, in screen points, for clicking it.
    board_label: Option<egui::Rect>,
    /// A box-select in node-edit mode: where it began and where the pointer is, in Canvas units.
    marquee: Option<(Point, Point)>,
    /// A box-select dragged from empty Canvas with the Select tool.
    select_box: Option<SelectBox>,
    /// The Label whose text is being typed in the overlay box.
    pub editing: Option<TextEditing>,
    /// The gesture a `drag`, `stroke`, `node_drag` or `board_drag` holds open, and the tool
    /// that opened it.
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

/// A box-select in progress, in Canvas units. Shift keeps what was selected when it began.
struct SelectBox {
    start: Point,
    end: Point,
    keep: Vec<Id>,
}

/// A lasso loop being drawn: freehand while the button is held, or corner by corner.
struct Lasso {
    points: Vec<Point>,
    freehand: bool,
    erase: bool,
}

/// A node drag in progress: where it began, and the outline's shape then.
struct NodeDrag {
    start: Point,
    shape: Vec<usize>,
}

/// The dark line under an in-progress stroke's gold dashes, so they show on any map colour.
const HALO: egui::Stroke = egui::Stroke {
    width: 3.0,
    color: egui::Color32::from_black_alpha(110),
};
/// The dash and gap lengths of an in-progress stroke's outline, in points.
const DASH: f32 = 6.0;
const GAP: f32 = 4.0;

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
            self.board_drag = None;
        }
    }
}

/// How close, in egui points, the pointer must be to grab a node or an edge.
const NODE_GRAB: f64 = 8.0;
const EDGE_GRAB: f64 = 6.0;
/// Past this many nodes in view, unselected ones are thinned out so the Canvas stays legible
/// and responsive.
const MAX_DRAWN_NODES: usize = 3000;

/// A land brush stroke in progress. Nothing changes in the Map until it is released; until
/// then an overlay shows the area it covers.
struct Stroke {
    /// The pointer's path in Canvas units, from where the button went down.
    points: Vec<Point>,
    erase: bool,
    /// The overlay as last computed, and how many points it covers.
    preview: Option<(usize, MultiPolygon)>,
}

/// Typing a Label's text in a box overlaid at the Label, inside an open gesture so the
/// whole edit is one Undo step.
pub struct TextEditing {
    pub id: Id,
    pub text: String,
    /// Set until the box has taken the keyboard and selected the text.
    pub just_opened: bool,
    /// The gesture the text is typed in: "Edit Label", or "Add Label" when the Label was just
    /// placed.
    gesture: GestureId,
}

impl TextEditing {
    /// Runs a Command in the gesture the text is typed in.
    pub fn run(&self, editor: &mut Editor, command: Command) -> bool {
        editor.gesture(&self.gesture, command)
    }

    /// Commits the gesture the text was typed in.
    pub fn finish(self, editor: &mut Editor) {
        editor.end_gesture(self.gesture);
    }
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
    /// The Label styles the Labels follow, which resizing needs to scale what they inherit.
    styles: LabelStyles,
    /// The selected Stamps as they were, for the same reason.
    stamps: Vec<(Id, Stamp)>,
    /// An Option-drag: the originals that stay put, and the ids their copies took.
    copies: Option<(Vec<Id>, Vec<Id>)>,
}

impl ToolState {
    /// Opens the text box on a Label, inside a new "Edit Label" gesture.
    pub fn edit_text(&mut self, editor: &mut Editor, id: Id) {
        if editor.label(id).is_none() {
            return;
        }
        let gesture = editor.begin_gesture("Edit Label");
        self.edit_placed_text(editor, id, gesture);
    }

    /// Opens the text box on the Label `id` inside `gesture`, which already holds its
    /// placement ("Add Label").
    fn edit_placed_text(&mut self, editor: &mut Editor, id: Id, gesture: GestureId) {
        let Some(label) = editor.label(id) else {
            editor.cancel_gesture(gesture);
            return;
        };
        let text = label.text.clone();
        self.editing = Some(TextEditing {
            id,
            text,
            just_opened: true,
            gesture,
        });
    }

    /// Whether a drag is moving, scaling or rotating the selection.
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// The cursor HUD while the Art board is being resized, such as "Board · 4096 × 2560".
    pub fn board_hud(&self, editor: &Editor) -> Option<String> {
        self.board_drag?;
        let canvas = &editor.map.canvas;
        Some(art_board::hud_text(canvas.width, canvas.height))
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
    if editor.active_tool != ToolId::Select {
        editor.leave_node_edit();
    }
    state.drop_held_for_other_tool(editor);
    if editor.active_tool != ToolId::ArtBoard {
        state.board_label = None;
        editor.select_board(false);
    }
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
        ToolId::Select if editor.node_edit().is_some() => {
            let pointer = PointerInput::from_response(response, ppp);
            if let Some(icon) = node_edit_tool(editor, view, state, &pointer) {
                response.ctx.set_cursor_icon(icon);
            }
        }
        ToolId::LandBrush | ToolId::WaterBrush => {
            let pointer = PointerInput::from_response(response, ppp);
            let water = editor.active_tool == ToolId::WaterBrush;
            if let Some(icon) = land_brush(editor, view, state, &pointer, water) {
                response.ctx.set_cursor_icon(icon);
            }
        }
        ToolId::Lasso => {
            let pointer = PointerInput::from_response(response, ppp);
            if pointer.hovered {
                response.ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
            }
            lasso_tool(editor, view, state, &pointer);
        }
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
                let duplicate = response.ctx.input(|i| i.modifiers.alt);
                let point = to_canvas(pos);
                let grip = grip_at(pos);
                let on_selection = bounds.is_some_and(|b| b.contains(point));
                if grip.is_none() && !on_selection && editor.pick(point).is_none() {
                    // Empty Canvas or pasteboard: the drag draws a selection box.
                    let keep = if shift {
                        editor.selection().to_vec()
                    } else {
                        Vec::new()
                    };
                    state.select_box = Some(SelectBox {
                        start: point,
                        end: point,
                        keep,
                    });
                } else {
                    start_drag(editor, state, bounds, grip, point, duplicate);
                }
            }
            select_box_tool(
                editor,
                view,
                state,
                &PointerInput::from_response(response, ppp),
            );
            let over_selection = || {
                let pos = response.hover_pos()?;
                let inside =
                    grip_at(pos).is_none() && bounds.is_some_and(|b| b.contains(to_canvas(pos)));
                inside.then_some(())
            };
            let option_held = response.ctx.input(|i| i.modifiers.alt);
            if state
                .drag
                .as_ref()
                .is_some_and(|drag| drag.copies.is_some())
                || (state.drag.is_none() && option_held && over_selection().is_some())
            {
                response.ctx.set_cursor_icon(egui::CursorIcon::Copy);
            }
            // Escape or undo ended the gesture under the drag.
            if state.drag.is_some() && !state.holds_open(editor) {
                state.drag = None;
            }
            if let Some(drag) = &state.drag
                && response.dragged_by(egui::PointerButton::Primary)
                && let Some(pos) = response.interact_pointer_pos()
                && let Some(commands) = drag_commands(drag, to_canvas(pos), shift)
            {
                let mut commands = commands.into_iter();
                if let Some(first) = commands.next() {
                    state.replace_held(editor, first);
                }
                for command in commands {
                    state.run_held(editor, command);
                }
                if let Some(command) = membership_command(editor, drag, to_canvas(pos)) {
                    state.run_held(editor, command);
                }
            }
            if response.drag_stopped() && state.drag.take().is_some() {
                state.end_held(editor);
            }
            if let Some(pos) = response.interact_pointer_pos() {
                let point = to_canvas(pos);
                if response.double_clicked() {
                    if let Some(id) = editor.pick(point) {
                        if editor.label(id).is_some() {
                            editor.select(vec![id]);
                            state.edit_text(editor, id);
                        } else {
                            editor.enter_node_edit(id);
                        }
                    }
                } else if response.clicked() {
                    let hit = editor.pick(point);
                    click_select(editor, hit, shift);
                }
            }
        }
        ToolId::Pan => {}
        ToolId::Stamp | ToolId::StampEraser => {
            let pointer = PointerInput::from_response(response, ppp);
            let erase = editor.active_tool == ToolId::StampEraser;
            if let Some(icon) = scatter_tool(editor, view, state, &pointer, erase) {
                response.ctx.set_cursor_icon(icon);
            }
        }
        ToolId::ExportRegion => {
            let pointer = PointerInput::from_response(response, ppp);
            if let Some(icon) = export_region_tool(editor, view, state, &pointer) {
                response.ctx.set_cursor_icon(icon);
            }
        }
        ToolId::ArtBoard => {
            let pointer = PointerInput::from_response(response, ppp);
            if let Some(icon) = art_board_tool(editor, view, state, &pointer) {
                response.ctx.set_cursor_icon(icon);
            }
        }
        ToolId::Label => {
            if response.clicked()
                && let Some(pos) = response.interact_pointer_pos()
                && let Some((id, gesture)) = editor.place_label(to_canvas(pos))
            {
                // Typing joins the "Add Label" gesture, so placing and naming undo together.
                state.edit_placed_text(editor, id, gesture);
                editor.active_tool = ToolId::Select;
            }
        }
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

/// The Canvas's box in screen points.
fn board_screen_rect(editor: &Editor, view: &View, origin: egui::Pos2, ppp: f64) -> egui::Rect {
    let rect = editor.map.canvas.rect();
    egui::Rect::from_two_pos(
        handles::to_screen(view, origin, ppp, rect.min),
        handles::to_screen(view, origin, ppp, rect.max),
    )
}

/// Select the Canvas by clicking its edge or its label (not its inside, so Objects on it are
/// not selected by accident), then resize it from its handles. Shift keeps the aspect ratio
/// and Alt resizes from the centre. Each resize is one `ResizeCanvas` Undo step and never
/// touches an Object.
fn art_board_tool(
    editor: &mut Editor,
    view: &View,
    state: &mut ToolState,
    input: &PointerInput,
) -> Option<egui::CursorIcon> {
    let ppp = input.pixels_per_point;
    let canvas = editor.map.canvas.clone();
    let bounds = Bounds {
        min: canvas.origin,
        max: canvas.rect().max,
    };
    let selected = editor.board_selected();
    let grip_at =
        |pos: egui::Pos2| match handles::hit(&bounds, view, input.rect_min, ppp, pos, false) {
            Some(Grip::Resize(handle)) if selected => Some(handle),
            _ => None,
        };
    let screen = board_screen_rect(editor, view, input.rect_min, ppp);
    let on_edge = |pos: egui::Pos2| {
        screen.expand(EDGE_GRAB as f32).contains(pos)
            && !screen.shrink(EDGE_GRAB as f32).contains(pos)
    };
    let board_label = state.board_label;
    let on_label = |pos: egui::Pos2| board_label.is_some_and(|label| label.contains(pos));
    let mut cursor = None;
    if state.board_drag.is_none()
        && let Some(pos) = input.hover_pos
    {
        if let Some(handle) = grip_at(pos) {
            cursor = Some(handle.cursor());
        } else if on_edge(pos) || on_label(pos) {
            cursor = Some(egui::CursorIcon::PointingHand);
        }
    }
    if input.drag_started
        && !editor.gesture_open()
        && let Some(pos) = input.press_origin
        && let Some(handle) = grip_at(pos)
        && state.begin_held(editor, ToolId::ArtBoard, "Resize Canvas")
    {
        state.board_drag = Some(BoardDrag {
            handle,
            start: input.canvas_point(view, pos),
            origin: canvas.origin,
            size: [canvas.width, canvas.height],
        });
    }
    if let Some(drag) = state.board_drag {
        // Escape or undo ended the gesture under the drag.
        if !state.holds_open(editor) {
            state.board_drag = None;
        } else if input.dragging
            && let Some(pos) = input.pointer_pos
        {
            let (origin, width, height) =
                drag.resized(input.canvas_point(view, pos), input.shift, input.alt);
            state.replace_held(
                editor,
                Command::ResizeCanvas {
                    origin,
                    width,
                    height,
                },
            );
        }
    }
    if input.drag_stopped && state.board_drag.take().is_some() {
        state.end_held(editor);
    }
    if input.clicked
        && let Some(pos) = input.pointer_pos
        && grip_at(pos).is_none()
    {
        editor.select_board(on_edge(pos) || on_label(pos));
    }
    cursor
}

/// Draws the board's label above its top-left corner, and its handles when it is selected.
fn paint_board(
    painter: &egui::Painter,
    editor: &Editor,
    view: &View,
    state: &mut ToolState,
    origin: egui::Pos2,
    ppp: f64,
) {
    let palette = crate::theme::palette(painter.ctx());
    let screen = board_screen_rect(editor, view, origin, ppp);
    let selected = editor.board_selected();
    let color = if selected {
        palette.select
    } else {
        palette.text_secondary
    };
    let text = painter.text(
        screen.left_top() - egui::vec2(0.0, 4.0),
        egui::Align2::LEFT_BOTTOM,
        "Board",
        egui::FontId::proportional(crate::theme::CAPTION),
        color,
    );
    state.board_label = Some(text.expand(3.0));
    if selected {
        let bounds = Bounds {
            min: editor.map.canvas.origin,
            max: editor.map.canvas.rect().max,
        };
        handles::paint(painter, &bounds, view, origin, ppp, false, palette.select);
    }
}

/// The custom export region, if the Map has one.
pub fn custom_region(editor: &Editor) -> Option<Bounds> {
    match &editor.map.export_settings.region {
        ExportRegion::Custom { rect } => Some(Bounds {
            min: rect.min,
            max: rect.max,
        }),
        ExportRegion::Canvas => None,
    }
}

/// Draw the custom export region by dragging on empty Canvas, move it by dragging inside,
/// and resize it from its handles. The region is stored in the Map's export settings as
/// `ExportRegion::Custom` while dragging; Escape puts it back as it was.
fn export_region_tool(
    editor: &mut Editor,
    view: &View,
    state: &mut ToolState,
    input: &PointerInput,
) -> Option<egui::CursorIcon> {
    let ppp = input.pixels_per_point;
    let canvas = [
        f64::from(editor.map.canvas.width),
        f64::from(editor.map.canvas.height),
    ];
    if std::mem::take(&mut editor.cancel_tool)
        && let Some((_, before)) = state.region_drag.take()
    {
        editor.map.export_settings.region = before;
        return None;
    }
    let existing = custom_region(editor);
    let grip_at = |pos: egui::Pos2| {
        let bounds = existing?;
        handles::hit(&bounds, view, input.rect_min, ppp, pos, false)
    };
    let cursor = input.hovered.then(|| match input.hover_pos {
        Some(pos) if state.region_drag.is_none() => match grip_at(pos) {
            Some(grip) => grip.cursor(),
            None if existing.is_some_and(|b| b.contains(input.canvas_point(view, pos))) => {
                egui::CursorIcon::Move
            }
            None => egui::CursorIcon::Crosshair,
        },
        _ => egui::CursorIcon::Crosshair,
    });
    if input.drag_started
        && state.region_drag.is_none()
        && let Some(pos) = input.press_origin
    {
        let start = input.canvas_point(view, pos);
        let drag = match (existing, grip_at(pos)) {
            (Some(bounds), Some(Grip::Resize(handle))) => RegionDrag::Resize {
                handle,
                start,
                original: bounds.rect(),
            },
            (Some(bounds), _) if bounds.contains(start) => RegionDrag::Move {
                start,
                original: bounds.rect(),
            },
            _ => RegionDrag::Draw { anchor: start },
        };
        state.region_drag = Some((drag, editor.map.export_settings.region.clone()));
    }
    let Some((drag, before)) = state.region_drag.clone() else {
        return cursor;
    };
    if input.dragging
        && let Some(pos) = input.pointer_pos
    {
        let rect = drag.rect(input.canvas_point(view, pos), canvas);
        editor.map.export_settings.region = ExportRegion::Custom { rect };
    }
    if input.drag_stopped {
        state.region_drag = None;
        if let ExportRegion::Custom { rect } = &editor.map.export_settings.region
            && !export_region::is_usable(rect)
        {
            editor.map.export_settings.region = before;
        }
    }
    cursor
}

/// Paint land by dragging, or erase with the erase modifier held. Each stroke is one Undo
/// step, made on release; Escape drops a stroke in progress. The brush rules (what merges,
/// what is created, what is erased) live in [`land::brush_command`], or with `water`, in
/// [`land::water_command`].
fn land_brush(
    editor: &mut Editor,
    view: &View,
    state: &mut ToolState,
    input: &PointerInput,
    water: bool,
) -> Option<egui::CursorIcon> {
    let radius = editor.brush_size / 2.0;
    let erase_held = input.alt != editor.erase_default;
    let cursor = input.hovered.then_some(egui::CursorIcon::Crosshair);

    // A click is a stroke of one point: a single dab.
    let pressed = input.drag_started || input.clicked;
    if pressed
        && state.stroke.is_none()
        && !editor.gesture_open()
        && let Some(origin) = input.press_origin.or(input.pointer_pos)
    {
        let label = if water {
            Editor::water_label(erase_held)
        } else {
            editor.brush_label(erase_held)
        };
        let tool = editor.active_tool;
        if state.begin_held(editor, tool, label) {
            state.stroke = Some(Stroke {
                points: vec![input.canvas_point(view, origin)],
                erase: erase_held,
                preview: None,
            });
        }
    }

    // Escape rolled the gesture back: drop the stroke.
    if state.stroke.is_some() && !state.holds_open(editor) {
        state.stroke = None;
    }
    let Some(stroke) = &mut state.stroke else {
        return cursor;
    };
    if let Some(pos) = input.pointer_pos {
        let point = input.canvas_point(view, pos);
        // A tenth of the radius, but at least a screen pixel, keeps the path light.
        let spacing = (radius * 0.1).max(1.0 / view.zoom);
        let last = *stroke.points.last().expect("a stroke starts with a point");
        if (point[0] - last[0]).hypot(point[1] - last[1]) >= spacing {
            stroke.points.push(point);
        }
    }

    let tolerance = land::simplify_tolerance(radius, 1.0 / view.zoom);
    let finished = |stroke: &Stroke| BrushStroke {
        region: land::brush_region(&stroke.points, radius, tolerance),
        start: stroke.points.first().copied().unwrap_or_default(),
        erase: stroke.erase,
    };
    let released = input.drag_stopped || input.clicked;
    if !released
        && stroke
            .preview
            .as_ref()
            .is_none_or(|(n, _)| *n != stroke.points.len())
    {
        // The shore the release would commit, rough as it will be, once per change.
        let area = editor
            .brush_preview(finished(stroke), water)
            .unwrap_or_default();
        stroke.preview = Some((stroke.points.len(), area));
    }
    if released && let Some(stroke) = state.stroke.take() {
        let finished = finished(&stroke);
        if let Some(id) = state.take_held() {
            if water {
                editor.finish_water_stroke(id, finished);
            } else {
                editor.finish_brush_stroke(id, finished);
            }
        }
    }
    cursor
}

/// The Stamp tool and the Stamp eraser. Stamp: a click places one Stamp, and a drag scatters
/// Stamps along the path. Eraser: a click or drag removes Stamps. On release one Command applies
/// ([`Editor::place_stamp`], [`Editor::scatter_stamps`], [`Editor::erase_stamps`]). Nothing
/// changes in the Map while dragging; Escape drops the stroke. egui's drag threshold separates
/// a slightly shaky click from a drag.
fn scatter_tool(
    editor: &mut Editor,
    view: &View,
    state: &mut ToolState,
    input: &PointerInput,
    erase: bool,
) -> Option<egui::CursorIcon> {
    let cursor = input.hovered.then_some(egui::CursorIcon::Crosshair);
    if std::mem::take(&mut editor.cancel_tool) {
        state.stroke = None;
        return cursor;
    }
    if !erase
        && input.clicked
        && let Some(pos) = input.pointer_pos
    {
        if editor.selected_asset.is_none() {
            editor.error("Choose an Asset in the Asset browser first");
        } else {
            editor.place_stamp(input.canvas_point(view, pos));
        }
        return cursor;
    }
    let pressed = input.drag_started || (erase && input.clicked);
    if pressed
        && state.stroke.is_none()
        && let Some(origin) = input.press_origin.or(input.pointer_pos)
    {
        state.stroke = Some(Stroke {
            points: vec![input.canvas_point(view, origin)],
            erase,
            preview: None,
        });
    }
    let Some(stroke) = &mut state.stroke else {
        return cursor;
    };
    if let Some(pos) = input.pointer_pos {
        let point = input.canvas_point(view, pos);
        let spacing = (editor.brush_size * 0.05).max(1.0 / view.zoom);
        let last = *stroke.points.last().expect("a stroke starts with a point");
        if (point[0] - last[0]).hypot(point[1] - last[1]) >= spacing {
            stroke.points.push(point);
        }
    }
    if (input.drag_stopped || input.clicked)
        && let Some(stroke) = state.stroke.take()
    {
        if erase {
            editor.erase_stamps(&stroke.points);
        } else {
            editor.scatter_stamps(&stroke.points);
        }
    }
    cursor
}

/// Draw a region to add to land, or with the erase modifier to cut out: drag a freehand loop, or
/// click corners and close the loop by clicking the first one, double-clicking or pressing
/// Enter. Escape drops a loop in progress. The region follows the land brush's merge rules.
fn lasso_tool(editor: &mut Editor, view: &View, state: &mut ToolState, input: &PointerInput) {
    let (alt, enter) = (input.alt, input.enter);
    if std::mem::take(&mut editor.cancel_tool) {
        state.lasso = None;
        return;
    }
    let pointer = input.pointer_pos.map(|pos| input.canvas_point(view, pos));
    let near_first = |lasso: &Lasso, point: Point| {
        let reach = NODE_GRAB * input.pixels_per_point / view.zoom;
        match lasso.points.first() {
            Some(first) if lasso.points.len() >= 3 => {
                (point[0] - first[0]).hypot(point[1] - first[1]) <= reach
            }
            _ => false,
        }
    };

    if input.drag_started
        && state.lasso.is_none()
        && let Some(origin) = input.press_origin
    {
        state.lasso = Some(Lasso {
            points: vec![input.canvas_point(view, origin)],
            freehand: true,
            erase: alt,
        });
    }
    if input.dragging
        && let Some(lasso) = &mut state.lasso
        && lasso.freehand
        && let Some(point) = pointer
    {
        let spacing = 2.0 * input.pixels_per_point / view.zoom;
        let last = *lasso.points.last().expect("a loop starts with a point");
        if (point[0] - last[0]).hypot(point[1] - last[1]) >= spacing {
            lasso.points.push(point);
        }
    }

    let mut close = false;
    if input.drag_stopped && state.lasso.as_ref().is_some_and(|l| l.freehand) {
        close = true;
    } else if input.clicked
        && let Some(point) = pointer
    {
        match &mut state.lasso {
            None => {
                state.lasso = Some(Lasso {
                    points: vec![point],
                    freehand: false,
                    erase: alt,
                });
            }
            Some(lasso) if near_first(lasso, point) => close = true,
            Some(lasso) => lasso.points.push(point),
        }
    }
    if enter || (input.double_clicked && state.lasso.is_some()) {
        close = true;
    }

    if close && let Some(lasso) = state.lasso.take() {
        let tolerance = if lasso.freehand {
            land::simplify_tolerance(4.0 / view.zoom, 1.0 / view.zoom)
        } else {
            0.0
        };
        let region = land::lasso_region(&lasso.points, tolerance);
        if region.is_empty() {
            editor.info("The loop doesn't enclose any area");
            return;
        }
        let id = editor.begin_gesture(editor.brush_label(lasso.erase));
        editor.finish_brush_stroke(
            id,
            BrushStroke {
                region,
                start: lasso.points.first().copied().unwrap_or_default(),
                erase: lasso.erase,
            },
        );
    }
}

/// Grows the box-select started with the Select tool while the pointer drags, and selects the
/// Objects inside it on release.
fn select_box_tool(editor: &mut Editor, view: &View, state: &mut ToolState, input: &PointerInput) {
    let Some(select_box) = &mut state.select_box else {
        return;
    };
    if input.dragging
        && let Some(pos) = input.pointer_pos
    {
        select_box.end = input.canvas_point(view, pos);
    }
    if input.drag_stopped
        && let Some(done) = state.select_box.take()
    {
        let mut selection = done.keep;
        for id in objects_in_box(editor, done.start, done.end) {
            if !selection.contains(&id) {
                selection.push(id);
            }
        }
        editor.select(selection);
    }
}

/// Draws a lasso loop in progress: the corners so far, and a line to the pointer.
fn paint_lasso(
    painter: &egui::Painter,
    view: &View,
    state: &ToolState,
    origin: egui::Pos2,
    hover: Option<egui::Pos2>,
    ppp: f64,
) {
    let Some(lasso) = &state.lasso else {
        return;
    };
    let palette = crate::theme::palette(painter.ctx());
    let color = if lasso.erase {
        palette.danger
    } else {
        palette.gold
    };
    let mut points: Vec<egui::Pos2> = lasso
        .points
        .iter()
        .map(|&p| crate::handles::to_screen(view, origin, ppp, p))
        .collect();
    if !lasso.freehand
        && let Some(hover) = hover
    {
        points.push(hover);
    }
    painter.add(egui::Shape::line(points.clone(), HALO));
    painter.extend(egui::Shape::dashed_line(
        &points,
        egui::Stroke::new(1.5, color),
        DASH,
        GAP,
    ));
    if !lasso.freehand {
        for &p in &lasso.points {
            let pos = crate::handles::to_screen(view, origin, ppp, p);
            painter.circle_filled(pos, 3.5, color);
        }
    }
}

/// Where the pointer is in an outline being edited.
enum NodeHit {
    Node(NodeRef),
    /// On an edge: the edge's first node, and the nearest point on it.
    Edge(NodeRef, Point),
    Inside,
    Outside,
}

fn hit_outline(outline: &MultiPolygon, point: Point, node_reach: f64, edge_reach: f64) -> NodeHit {
    let mut best_node: Option<(f64, NodeRef)> = None;
    let mut best_edge: Option<(f64, NodeRef, Point)> = None;
    for (part, polygon) in outline.0.iter().enumerate() {
        for (ring_number, ring) in polygon.rings().enumerate() {
            for (index, &a) in ring.iter().enumerate() {
                let node = NodeRef {
                    part,
                    ring: ring_number,
                    index,
                };
                let d = (a[0] - point[0]).hypot(a[1] - point[1]);
                if d <= node_reach && best_node.is_none_or(|(best, _)| d < best) {
                    best_node = Some((d, node));
                }
                let b = ring[(index + 1) % ring.len()];
                let (d, nearest) = nearest_on_segment(a, b, point);
                if d <= edge_reach && best_edge.is_none_or(|(best, ..)| d < best) {
                    best_edge = Some((d, node, nearest));
                }
            }
        }
    }
    if let Some((_, node)) = best_node {
        NodeHit::Node(node)
    } else if let Some((_, edge, nearest)) = best_edge {
        NodeHit::Edge(edge, nearest)
    } else if outline.contains(point) {
        NodeHit::Inside
    } else {
        NodeHit::Outside
    }
}

fn nearest_on_segment(a: Point, b: Point, p: Point) -> (f64, Point) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length_squared = dx * dx + dy * dy;
    let t = if length_squared == 0.0 {
        0.0
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length_squared).clamp(0.0, 1.0)
    };
    let nearest = [a[0] + t * dx, a[1] + t * dy];
    ((p[0] - nearest[0]).hypot(p[1] - nearest[1]), nearest)
}

/// Node-edit mode (double-click a Landmass or Lake): click a node to select it (Shift adds),
/// drag nodes to move them, drag empty space to box-select, double-click an edge to add a node,
/// Delete removes the selected nodes, and Escape or a click outside the shape leaves. Each
/// drag is one Undo step.
fn node_edit_tool(
    editor: &mut Editor,
    view: &View,
    state: &mut ToolState,
    input: &PointerInput,
) -> Option<egui::CursorIcon> {
    let shift = input.shift;
    let mut cursor = None;
    let per_point = input.pixels_per_point / view.zoom;
    let hit_at = |editor: &Editor, pos: egui::Pos2| {
        let point = input.canvas_point(view, pos);
        editor.edited_outline().map(|outline| {
            hit_outline(outline, point, NODE_GRAB * per_point, EDGE_GRAB * per_point)
        })
    };

    if let Some(pos) = input.hover_pos
        && state.node_drag.is_none()
        && let Some(NodeHit::Node(_)) = hit_at(editor, pos)
    {
        cursor = Some(egui::CursorIcon::PointingHand);
    }

    // Drag: move nodes, or box-select.
    if input.drag_started && !editor.gesture_open() {
        let origin = input.press_origin.or(input.pointer_pos);
        if let Some(origin) = origin {
            let start = input.canvas_point(view, origin);
            match hit_at(editor, origin) {
                Some(NodeHit::Node(node)) => {
                    let mut nodes = editor
                        .node_edit()
                        .map(|e| e.nodes.clone())
                        .unwrap_or_default();
                    if !nodes.contains(&node) {
                        if shift {
                            nodes.push(node);
                        } else {
                            nodes = vec![node];
                        }
                        editor.select_nodes(nodes);
                    }
                    let shape = editor.edited_shape();
                    if state.begin_held(editor, ToolId::Select, "Move Nodes") {
                        state.node_drag = Some(NodeDrag { start, shape });
                    }
                }
                _ => state.marquee = Some((start, start)),
            }
        }
    }

    if input.dragging
        && let Some(pos) = input.pointer_pos
    {
        let point = input.canvas_point(view, pos);
        // Escape rolled the gesture back.
        if state.node_drag.is_some() && !state.holds_open(editor) {
            state.node_drag = None;
        }
        if let Some(drag) = &state.node_drag {
            if let Some(edit) = editor.node_edit().cloned() {
                let offset = [point[0] - drag.start[0], point[1] - drag.start[1]];
                state.replace_held(
                    editor,
                    Command::MoveNodes {
                        id: edit.id,
                        nodes: edit.nodes,
                        offset,
                    },
                );
            }
        } else if let Some((_, end)) = &mut state.marquee {
            *end = point;
        }
    }

    if input.drag_stopped {
        if let Some(drag) = state.node_drag.take() {
            state.end_held(editor);
            editor.keep_nodes_if_shape(&drag.shape);
        } else if let Some((a, b)) = state.marquee.take() {
            let (min, max) = (
                [a[0].min(b[0]), a[1].min(b[1])],
                [a[0].max(b[0]), a[1].max(b[1])],
            );
            let mut nodes = if shift {
                editor
                    .node_edit()
                    .map(|e| e.nodes.clone())
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            if let Some(outline) = editor.edited_outline() {
                for (part, polygon) in outline.0.iter().enumerate() {
                    for (ring_number, ring) in polygon.rings().enumerate() {
                        for (index, p) in ring.iter().enumerate() {
                            let node = NodeRef {
                                part,
                                ring: ring_number,
                                index,
                            };
                            if p[0] >= min[0]
                                && p[0] <= max[0]
                                && p[1] >= min[1]
                                && p[1] <= max[1]
                                && !nodes.contains(&node)
                            {
                                nodes.push(node);
                            }
                        }
                    }
                }
            }
            editor.select_nodes(nodes);
        }
    }

    if input.double_clicked
        && let Some(pos) = input.pointer_pos
        && let Some(NodeHit::Edge(edge, point)) = hit_at(editor, pos)
    {
        editor.insert_node(edge, point);
        return cursor;
    }

    if input.clicked
        && let Some(pos) = input.pointer_pos
    {
        let current = editor
            .node_edit()
            .map(|e| e.nodes.clone())
            .unwrap_or_default();
        match hit_at(editor, pos) {
            Some(NodeHit::Node(node)) if shift && current.contains(&node) => {
                editor.select_nodes(current.into_iter().filter(|&n| n != node).collect());
            }
            Some(NodeHit::Node(node)) if shift => {
                let mut nodes = current;
                nodes.push(node);
                editor.select_nodes(nodes);
            }
            Some(NodeHit::Node(node)) => editor.select_nodes(vec![node]),
            Some(NodeHit::Edge(..) | NodeHit::Inside) => {
                if !shift {
                    editor.select_nodes(Vec::new());
                }
            }
            Some(NodeHit::Outside) | None => editor.leave_node_edit(),
        }
    }
    cursor
}

/// Draws the outline being edited, its nodes (those in view, thinned when crowded) and a
/// box-select in progress.
fn paint_nodes(
    painter: &egui::Painter,
    editor: &Editor,
    view: &View,
    state: &ToolState,
    origin: egui::Pos2,
    hover: Option<egui::Pos2>,
    ppp: f64,
) {
    let (Some(edit), Some(outline)) = (editor.node_edit(), editor.edited_outline()) else {
        return;
    };
    let to_screen = |p: Point| crate::handles::to_screen(view, origin, ppp, p);
    let area = painter.clip_rect().expand(24.0);
    let select = crate::theme::palette(painter.ctx()).select;
    let line = egui::Stroke::new(1.5, select);

    let mut visible: Vec<(NodeRef, egui::Pos2)> = Vec::new();
    for (part, polygon) in outline.0.iter().enumerate() {
        for (ring_number, ring) in polygon.rings().enumerate() {
            let mut run: Vec<egui::Pos2> = Vec::new();
            let screen: Vec<egui::Pos2> = ring.iter().map(|&p| to_screen(p)).collect();
            for (index, &a) in screen.iter().enumerate() {
                let b = screen[(index + 1) % screen.len()];
                if area.contains(a) {
                    visible.push((
                        NodeRef {
                            part,
                            ring: ring_number,
                            index,
                        },
                        a,
                    ));
                }
                if area.intersects(egui::Rect::from_two_pos(a, b)) {
                    if run.is_empty() {
                        run.push(a);
                    }
                    run.push(b);
                } else if !run.is_empty() {
                    painter.add(egui::Shape::line(std::mem::take(&mut run), line));
                }
            }
            if !run.is_empty() {
                painter.add(egui::Shape::line(run, line));
            }
        }
    }
    let stride = visible.len().div_ceil(MAX_DRAWN_NODES).max(1);
    let hovered = hover.and_then(|pos| {
        let reach = NODE_GRAB as f32;
        visible
            .iter()
            .find(|(_, p)| p.distance(pos) <= reach)
            .map(|(node, _)| *node)
    });
    for (n, (node, pos)) in visible.iter().enumerate() {
        let selected = edit.nodes.contains(node);
        if !selected && hovered != Some(*node) && n % stride != 0 {
            continue;
        }
        let half = if selected || hovered == Some(*node) {
            4.5
        } else {
            3.0
        };
        let rect = egui::Rect::from_center_size(*pos, egui::vec2(half * 2.0, half * 2.0));
        let (fill, edge) = if selected {
            (select, egui::Color32::WHITE)
        } else {
            (egui::Color32::WHITE, select)
        };
        painter.rect_filled(rect, 1.0, fill);
        painter.rect_stroke(
            rect,
            1.0,
            egui::Stroke::new(1.0, edge),
            egui::StrokeKind::Middle,
        );
    }
    // Double-clicking an edge adds a node there: show where, as a ghost + under the pointer.
    let per_point = ppp / view.zoom;
    if hovered.is_none()
        && state.node_drag.is_none()
        && state.marquee.is_none()
        && let Some(pos) = hover
        && let Some(on_edge) = {
            let local = pos - origin;
            let point = view.to_canvas([f64::from(local.x) * ppp, f64::from(local.y) * ppp]);
            match hit_outline(outline, point, NODE_GRAB * per_point, EDGE_GRAB * per_point) {
                NodeHit::Edge(_, on_edge) => Some(on_edge),
                _ => None,
            }
        }
    {
        let at = to_screen(on_edge);
        painter.circle(
            at,
            5.0,
            select.gamma_multiply(0.35),
            egui::Stroke::new(1.0, select),
        );
        let arm = 2.5;
        let plus = egui::Stroke::new(1.25, egui::Color32::WHITE);
        painter.line_segment([at - egui::vec2(arm, 0.0), at + egui::vec2(arm, 0.0)], plus);
        painter.line_segment([at - egui::vec2(0.0, arm), at + egui::vec2(0.0, arm)], plus);
    }
    if let Some((a, b)) = state.marquee {
        let rect = egui::Rect::from_two_pos(to_screen(a), to_screen(b));
        painter.rect_filled(rect, 0.0, select.gamma_multiply(0.16));
        painter.rect_stroke(rect, 0.0, line, egui::StrokeKind::Middle);
    }
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
    if editor.active_tool == ToolId::ArtBoard {
        paint_board(painter, editor, view, state, origin, ppp);
    }
    if editor.active_tool == ToolId::Select {
        paint_nodes(painter, editor, view, state, origin, hover, ppp);
        if let Some(select_box) = &state.select_box {
            let select = crate::theme::palette(painter.ctx()).select;
            let to_screen = |p: Point| crate::handles::to_screen(view, origin, ppp, p);
            let rect =
                egui::Rect::from_two_pos(to_screen(select_box.start), to_screen(select_box.end));
            painter.rect_filled(rect, 0.0, select.gamma_multiply(0.16));
            painter.rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.5, select),
                egui::StrokeKind::Middle,
            );
        }
    }
    if let Some(bounds) = custom_region(editor) {
        // Editor-only: drawn with egui over the Canvas, so it never reaches an export.
        let accent = crate::theme::palette(painter.ctx()).gold;
        if editor.active_tool == ToolId::ExportRegion {
            handles::paint(painter, &bounds, view, origin, ppp, false, accent);
        } else {
            handles::paint_outline(painter, &bounds, view, origin, ppp, accent);
        }
    }
    if editor.active_tool == ToolId::Lasso {
        paint_lasso(painter, view, state, origin, hover, ppp);
    }
    if matches!(editor.active_tool, ToolId::Stamp | ToolId::StampEraser) {
        paint_scatter_cursor(painter, editor, view, state, origin, hover, ppp);
        return;
    }
    if !matches!(editor.active_tool, ToolId::LandBrush | ToolId::WaterBrush) {
        return;
    }
    let palette = crate::theme::palette(painter.ctx());
    let erase_held = painter.ctx().input(|i| i.modifiers.alt) != editor.erase_default;
    let erasing = state.stroke.as_ref().map_or(erase_held, |s| s.erase);
    let (fill, line) = if erasing {
        (palette.danger.gamma_multiply(0.3), palette.danger)
    } else {
        (palette.gold.gamma_multiply(0.25), palette.gold)
    };
    let radius = editor.brush_size / 2.0;
    if let Some(stroke) = &state.stroke
        && let Some((_, preview)) = &stroke.preview
    {
        let to_screen = |p: Point| crate::handles::to_screen(view, origin, ppp, p);
        painter.add(egui::Shape::mesh(region_mesh(preview, fill, to_screen)));
        // A thin guide that follows the rough edge without hiding it.
        let guide = egui::Stroke::new(1.0, line.gamma_multiply(0.7));
        for ring in preview.0.iter().flat_map(|polygon| polygon.rings()) {
            let mut points: Vec<egui::Pos2> = ring.iter().map(|&p| to_screen(p)).collect();
            if let Some(&first) = points.first() {
                points.push(first);
            }
            painter.add(egui::Shape::line(points, guide));
        }
    }
    if let Some(pos) = hover {
        brush_ring(painter, pos, (radius * view.zoom / ppp) as f32);
    }
}

/// The brush cursor: a white ring over a dark one, so it shows on any map colour.
fn brush_ring(painter: &egui::Painter, pos: egui::Pos2, radius: f32) {
    painter.circle_stroke(
        pos,
        radius,
        egui::Stroke::new(3.0, egui::Color32::from_black_alpha(140)),
    );
    painter.circle_stroke(pos, radius, egui::Stroke::new(1.25, egui::Color32::WHITE));
}

/// The scatter brush's and Stamp eraser's cursor: the brush circle, and the path of a stroke
/// in progress.
fn paint_scatter_cursor(
    painter: &egui::Painter,
    editor: &Editor,
    view: &View,
    state: &ToolState,
    origin: egui::Pos2,
    hover: Option<egui::Pos2>,
    ppp: f64,
) {
    let line = if editor.active_tool == ToolId::StampEraser {
        egui::Color32::from_rgb(200, 60, 50)
    } else {
        egui::Color32::from_rgb(60, 110, 70)
    };
    let screen_radius = (editor.brush_size / 2.0 * view.zoom / ppp) as f32;
    if let Some(stroke) = &state.stroke {
        let points: Vec<egui::Pos2> = stroke
            .points
            .iter()
            .map(|&p| crate::handles::to_screen(view, origin, ppp, p))
            .collect();
        painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(2.0, line.gamma_multiply(0.5)),
        ));
    }
    if let Some(pos) = hover {
        brush_ring(painter, pos, screen_radius);
    }
}

/// A filled region as an egui mesh in screen points. Holes stay open (even-odd).
fn region_mesh(
    region: &MultiPolygon,
    color: egui::Color32,
    to_screen: impl Fn(Point) -> egui::Pos2,
) -> egui::Mesh {
    let mut builder = Path::builder();
    for ring in region.0.iter().flat_map(|polygon| polygon.rings()) {
        let mut points = ring.iter().map(|&p| {
            let screen = to_screen(p);
            point(screen.x, screen.y)
        });
        let Some(first) = points.next() else {
            continue;
        };
        builder.begin(first);
        for p in points {
            builder.line_to(p);
        }
        builder.close();
    }
    let mut buffers: VertexBuffers<egui::Pos2, u32> = VertexBuffers::new();
    let options = FillOptions::tolerance(0.25).with_fill_rule(FillRule::EvenOdd);
    let tessellated = FillTessellator::new().tessellate_path(
        &builder.build(),
        &options,
        &mut BuffersBuilder::new(&mut buffers, |vertex: FillVertex| {
            let p = vertex.position();
            egui::pos2(p.x, p.y)
        }),
    );
    let mut mesh = egui::Mesh::default();
    if tessellated.is_ok() {
        mesh.indices = buffers.indices;
        mesh.vertices = buffers
            .vertices
            .into_iter()
            .map(|pos| egui::epaint::Vertex {
                pos,
                uv: egui::epaint::WHITE_UV,
                color,
            })
            .collect();
    }
    mesh
}

/// The unlocked Objects whose bounds meet the box between `a` and `b`, in draw order. Reaches
/// the pasteboard as well as the Canvas.
fn objects_in_box(editor: &mut Editor, a: Point, b: Point) -> Vec<Id> {
    let (min, max) = (
        [a[0].min(b[0]), a[1].min(b[1])],
        [a[0].max(b[0]), a[1].max(b[1])],
    );
    let candidates: Vec<Id> = editor
        .map
        .draw_order()
        .into_iter()
        .filter(|&id| !editor.map.is_locked(id))
        .collect();
    candidates
        .into_iter()
        .filter(|&id| {
            Bounds::of_ids(editor, &[id]).is_some_and(|bounds| {
                bounds.min[0] <= max[0]
                    && bounds.max[0] >= min[0]
                    && bounds.min[1] <= max[1]
                    && bounds.max[1] >= min[1]
            })
        })
        .collect()
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

/// Whether the selection holds a Label or a Stamp, the only selected things that rotate.
pub(crate) fn selection_has_label(editor: &Editor) -> bool {
    editor
        .selection()
        .iter()
        .any(|&id| editor.label(id).is_some() || editor.stamp(id).is_some())
}

/// Starts moving, scaling or rotating the selection from the Canvas point `point`, picking
/// what is under the pointer first if it isn't selected. `grip` is the handle under it.
fn start_drag(
    editor: &mut Editor,
    state: &mut ToolState,
    mut bounds: Option<Bounds>,
    grip: Option<Grip>,
    point: [f64; 2],
    duplicate: bool,
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
    // Whatever is linked to the selection moves or transforms with it, as one step. A locked
    // linked node blocks it all. An Option-drag copies only the selection, unlinked.
    let along = if duplicate && grip.is_none() {
        Vec::new()
    } else {
        let Some(along) = editor.linked_movers() else {
            return;
        };
        along
    };
    let moving: Vec<Id> = editor.selection().iter().copied().chain(along).collect();
    let outlines: Vec<Id> = moving
        .iter()
        .copied()
        .filter(|&id| {
            matches!(editor.map.node(id), Some(ssmap_core::Node::Object(o)) if o.kind.geometry().is_some())
        })
        .collect();
    let labels: Vec<(Id, Label)> = moving
        .iter()
        .filter_map(|&id| editor.label(id).map(|label| (id, label.clone())))
        .collect();
    let stamps: Vec<(Id, Stamp)> = moving
        .iter()
        .filter_map(|&id| editor.stamp(id).map(|stamp| (id, stamp.clone())))
        .collect();
    let Some(bounds) =
        bounds.filter(|_| !(outlines.is_empty() && labels.is_empty() && stamps.is_empty()))
    else {
        return;
    };
    // Option held as a move begins: leave the originals and drag copies of them. Locked
    // ones can't move, so they aren't copied.
    let mut copies = None;
    if duplicate && grip.is_none() {
        let sources: Vec<Id> = editor
            .selection()
            .iter()
            .copied()
            .filter(|&id| {
                (outlines.contains(&id) || editor.label(id).is_some() || editor.stamp(id).is_some())
                    && !editor.map.node(id).is_some_and(ssmap_core::Node::locked)
            })
            .collect();
        if sources.is_empty() {
            return;
        }
        if !state.begin_held(editor, ToolId::Select, "Duplicate Objects") {
            return;
        }
        let (command, new_ids) = editor.duplicate_command(&sources, [0.0, 0.0]);
        if !state.run_held(editor, command) {
            state.cancel_held(editor);
            return;
        }
        editor.select(new_ids.clone());
        copies = Some((sources, new_ids));
    } else {
        let label = match grip {
            None => "Move",
            Some(Grip::Resize(_)) => "Resize",
            Some(Grip::Rotate) => "Rotate",
        };
        if !state.begin_held(editor, ToolId::Select, label) {
            return;
        }
    }
    state.drag = Some(Drag {
        grip,
        start: point,
        bounds,
        outlines,
        labels,
        styles: editor.map.label_styles().clone(),
        stamps,
        copies,
    });
}

/// The `SetMembership` Command a move drag ends each frame with: the pointer at `point`
/// decides the board the dragged Objects belong to, inside the Canvas or off it, as in Figma.
/// `None` when nothing changes board, or for a resize or rotation.
fn membership_command(editor: &Editor, drag: &Drag, point: [f64; 2]) -> Option<Command> {
    if drag.grip.is_some() {
        return None;
    }
    let canvas = editor.map.canvas.rect();
    let inside = point[0] >= canvas.min[0]
        && point[0] <= canvas.max[0]
        && point[1] >= canvas.min[1]
        && point[1] <= canvas.max[1];
    let target = if inside {
        Membership::Canvas
    } else {
        Membership::Pasteboard
    };
    let moved: Vec<Id> = match &drag.copies {
        Some((_, new_ids)) => new_ids.clone(),
        None => drag
            .outlines
            .iter()
            .copied()
            .chain(drag.labels.iter().map(|(id, _)| *id))
            .chain(drag.stamps.iter().map(|(id, _)| *id))
            .collect(),
    };
    let ids: Vec<Id> = moved
        .into_iter()
        .filter(|&id| {
            matches!(editor.map.node(id), Some(Node::Object(o)) if o.membership != target)
                && !editor.map.is_locked(id)
        })
        .collect();
    (!ids.is_empty()).then_some(Command::SetMembership {
        ids,
        membership: target,
    })
}

/// What a drag does to each Label, from how it was when the drag began.
type LabelTransform = Box<dyn Fn(&Label) -> Label>;

/// What a drag does to each Stamp.
type StampTransform = Box<dyn Fn(&Stamp) -> Stamp>;

/// The Commands that put the dragged selection where the pointer at `point` says: one for
/// the Landmasses and Lakes, then one per Label and per Stamp. `None` keeps the last frame's,
/// as when a resize reaches the opposite edge and the box has no size.
fn drag_commands(drag: &Drag, point: [f64; 2], snap: bool) -> Option<Vec<Command>> {
    let delta = [point[0] - drag.start[0], point[1] - drag.start[1]];
    if let Some((originals, new_ids)) = &drag.copies {
        return Some(vec![Command::Duplicate {
            ids: originals.clone(),
            new_ids: new_ids.clone(),
            offset: delta,
        }]);
    }
    let ids = drag.outlines.clone();
    let (outlines, transform, stamp_transform): (Option<Command>, LabelTransform, StampTransform) =
        match drag.grip {
            None => (
                Some(Command::Translate { ids, offset: delta }),
                Box::new(move |label| handles::moved(label, delta)),
                Box::new(move |stamp| handles::moved_stamp(stamp, delta)),
            ),
            Some(Grip::Resize(handle)) => {
                let resized = drag.bounds.resized(handle, delta);
                let transform = Affine::rect_to_rect(&drag.bounds.rect(), &resized.rect());
                if !transform.is_invertible() {
                    return None;
                }
                let (scale, fixed) = drag.bounds.scaling(handle, delta);
                let styles = drag.styles.clone();
                (
                    Some(Command::Transform { ids, transform }),
                    Box::new(move |label| handles::scaled(label, &styles, scale, fixed)),
                    Box::new(move |stamp| handles::scaled_stamp(stamp, scale, fixed)),
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
                    Box::new(move |stamp| handles::rotated_stamp(stamp, degrees, centre)),
                )
            }
        };
    let outlines = outlines.filter(|_| !drag.outlines.is_empty());
    let labels = drag.labels.iter().map(|(id, label)| Command::SetLabel {
        id: *id,
        label: Box::new(transform(label)),
    });
    let stamps = drag.stamps.iter().map(|(id, stamp)| Command::SetStamp {
        id: *id,
        stamp: Box::new(stamp_transform(stamp)),
    });
    Some(outlines.into_iter().chain(labels).chain(stamps).collect())
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
                    size: Some(10.0),
                    ..Label::default()
                },
            )],
            styles: LabelStyles::default(),
            stamps: Vec::new(),
            copies: None,
        }
    }

    #[test]
    fn an_option_drag_re_issues_one_duplicate_with_the_pointer_offset() {
        let (original, copy) = (Id::new(), Id::new());
        let option_drag = Drag {
            copies: Some((vec![original], vec![copy])),
            ..drag(None)
        };
        let commands = drag_commands(&option_drag, [130.0, 20.0], false).unwrap();
        assert!(matches!(
            &commands[..],
            [Command::Duplicate { ids, new_ids, offset: [30.0, 20.0] }]
                if ids == &vec![original] && new_ids == &vec![copy]
        ));
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
        assert_eq!(scaled.size, Some(20.0));
        assert_eq!(scaled.position, [100.0, 0.0], "scaled about the left edge");
        let rotated = dragged(&drag(Some(Grip::Rotate)), [0.0, 100.0], false);
        assert!((rotated.rotation - 90.0).abs() < 1e-9);
        let snapped = dragged(&drag(Some(Grip::Rotate)), [100.0, 20.0], true);
        assert!((snapped.rotation - 15.0).abs() < 1e-9);
    }

    #[test]
    fn stamps_move_scale_and_rotate_about_the_drag() {
        let id = Id::new();
        let with_stamp = |grip| Drag {
            labels: Vec::new(),
            stamps: vec![(
                id,
                Stamp {
                    position: [10.0, 20.0],
                    ..Stamp::default()
                },
            )],
            ..drag(grip)
        };
        let stamp = |commands: Vec<Command>| match commands.into_iter().next() {
            Some(Command::SetStamp { stamp, .. }) => *stamp,
            other => panic!("expected SetStamp, got {other:?}"),
        };
        let moved = stamp(drag_commands(&with_stamp(None), [130.0, 20.0], false).unwrap());
        assert_eq!(moved.position, [40.0, 40.0]);
        // Doubling the box about the middle of its left edge doubles the Stamp and its
        // distance from that point (-100, 0).
        let grown = stamp(
            drag_commands(
                &with_stamp(Some(Grip::Resize(Handle::Right))),
                [300.0, 0.0],
                false,
            )
            .unwrap(),
        );
        assert_eq!(grown.scale, 2.0);
        assert_eq!(grown.position, [-100.0 + 110.0 * 2.0, 20.0 * 2.0]);
        let turned =
            stamp(drag_commands(&with_stamp(Some(Grip::Rotate)), [0.0, 100.0], false).unwrap());
        assert_eq!(turned.rotation, 90.0);
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

#[cfg(test)]
mod link_tests {
    use super::*;
    use ssmap_core::{
        Canvas, DistanceUnit, Map, MultiPolygon, Node, Polygon, Scale, Stamp as MapStamp,
    };

    fn square(x: f64) -> MultiPolygon {
        MultiPolygon(vec![Polygon {
            exterior: vec![[x, 0.0], [x + 10.0, 0.0], [x + 10.0, 10.0], [x, 10.0]],
            holes: Vec::new(),
        }])
    }

    /// Two Landmasses and a Stamp in three Layers, linked, with the first Landmass selected.
    fn linked() -> (Editor, [Id; 3]) {
        let mut editor = Editor::new(Map::new(Canvas::new(
            500,
            500,
            Scale::new(1.0, DistanceUnit::Miles),
        )));
        let mut ids = [Id::new(), Id::new(), Id::new()];
        for (n, id) in ids.iter_mut().enumerate() {
            let layer = Id::new();
            assert!(
                editor
                    .run(Command::CreateLayer {
                        id: layer,
                        parent: None,
                        index: None,
                        name: format!("Layer {n}"),
                    })
                    .is_ok()
            );
            let command = if n == 2 {
                Command::CreateStamp {
                    id: *id,
                    parent: layer,
                    index: None,
                    stamp: Box::new(MapStamp::new("pack/asset".parse().unwrap(), [200.0, 5.0])),
                }
            } else {
                Command::CreateLandmass {
                    id: *id,
                    parent: layer,
                    index: None,
                    geometry: square(n as f64 * 100.0),
                    fill: None,
                }
            };
            assert!(editor.run(command).is_ok());
        }
        editor.select(ids.to_vec());
        assert!(editor.link_selection());
        editor.select(vec![ids[0]]);
        (editor, ids)
    }

    fn left(editor: &Editor, id: Id) -> f64 {
        match editor.map.node(id) {
            Some(Node::Object(o)) => o.kind.geometry().unwrap().0[0].exterior[0][0],
            _ => panic!("not an Object"),
        }
    }

    #[test]
    fn dragging_one_linked_object_moves_the_others_in_one_undo_step() {
        let (mut editor, [a, b, s]) = linked();
        let undo_label = editor.undo_label().map(str::to_owned);
        let mut state = ToolState::default();
        let bounds = Bounds::of_selection(&mut editor);
        start_drag(&mut editor, &mut state, bounds, None, [5.0, 5.0], false);
        let drag = state.drag.as_ref().expect("the drag started");
        assert_eq!(drag.outlines, vec![a, b]);
        assert_eq!(drag.stamps.len(), 1);
        for command in drag_commands(drag, [25.0, 5.0], false).unwrap() {
            assert!(state.run_held(&mut editor, command));
        }
        state.drag = None;
        state.end_held(&mut editor);
        assert_eq!(left(&editor, a), 20.0);
        assert_eq!(left(&editor, b), 120.0);
        assert_eq!(editor.stamp(s).unwrap().position, [220.0, 5.0]);

        assert_eq!(editor.undo_label(), Some("Move"));
        editor.undo();
        assert_eq!(left(&editor, a), 0.0);
        assert_eq!(left(&editor, b), 100.0);
        assert_eq!(editor.stamp(s).unwrap().position, [200.0, 5.0]);
        assert_eq!(editor.undo_label().map(str::to_owned), undo_label);
    }

    #[test]
    fn a_locked_linked_object_blocks_the_drag_and_the_status_names_it() {
        let (mut editor, [a, b, _]) = linked();
        assert!(
            editor
                .run(Command::RenameObject {
                    id: b,
                    name: "Far Isle".into()
                })
                .is_ok()
        );
        assert!(
            editor
                .run(Command::SetLocked {
                    id: b,
                    locked: true
                })
                .is_ok()
        );
        let before = editor.map.view();
        let mut state = ToolState::default();
        let bounds = Bounds::of_selection(&mut editor);
        start_drag(&mut editor, &mut state, bounds, None, [5.0, 5.0], false);
        assert!(state.drag.is_none() && !editor.gesture_open());
        assert_eq!(editor.map.view(), before);
        let text = &editor.notice().expect("an error toast").text;
        assert!(
            text.contains("Far Isle") && text.contains("locked"),
            "{text}"
        );
        assert_eq!(editor.selection(), [a]);
    }

    #[test]
    fn an_option_drag_copies_only_the_selection() {
        let (mut editor, [a, b, _]) = linked();
        let mut state = ToolState::default();
        let bounds = Bounds::of_selection(&mut editor);
        start_drag(&mut editor, &mut state, bounds, None, [5.0, 5.0], true);
        let drag = state.drag.as_ref().expect("the drag started");
        assert_eq!(
            drag.copies.as_ref().map(|(from, _)| from.clone()),
            Some(vec![a])
        );
        state.drag = None;
        state.end_held(&mut editor);
        assert_eq!(left(&editor, b), 100.0);
        let copy = editor.selection()[0];
        assert_ne!(copy, a);
        assert!(!editor.map.is_linked(copy));
    }
}

#[cfg(test)]
mod pointer_tests {
    use super::*;
    use ssmap_core::{Canvas, DistanceUnit, Map, Polygon, Scale};

    fn editor() -> Editor {
        Editor::new(Map::new(Canvas::new(
            500,
            500,
            Scale::new(1.0, DistanceUnit::Miles),
        )))
    }

    /// An Editor whose Map has a Layer for the land brush to paint into.
    fn land_editor() -> Editor {
        let mut editor = editor();
        let layer = Id::new();
        editor
            .run(Command::CreateLayer {
                id: layer,
                parent: None,
                index: None,
                name: "Land".into(),
            })
            .unwrap();
        // Selecting a Layer makes it the active one; the Selection can then be emptied.
        editor.select(vec![layer]);
        editor.select(Vec::new());
        editor
    }

    /// One pixel per Canvas unit, and the widget at the screen origin.
    fn view() -> View {
        View::fit([500.0, 500.0], [500, 500], 0.0)
    }

    fn at(x: f32, y: f32) -> egui::Pos2 {
        egui::pos2(x, y)
    }

    fn input() -> PointerInput {
        PointerInput {
            pixels_per_point: 1.0,
            ..PointerInput::default()
        }
    }

    fn press(x: f32, y: f32) -> PointerInput {
        PointerInput {
            drag_started: true,
            press_origin: Some(at(x, y)),
            pointer_pos: Some(at(x, y)),
            ..input()
        }
    }

    fn drag_to(x: f32, y: f32) -> PointerInput {
        PointerInput {
            dragging: true,
            pointer_pos: Some(at(x, y)),
            ..input()
        }
    }

    fn release(x: f32, y: f32) -> PointerInput {
        PointerInput {
            drag_stopped: true,
            pointer_pos: Some(at(x, y)),
            ..input()
        }
    }

    fn bounds(min: [f64; 2], max: [f64; 2]) -> Option<Bounds> {
        Some(Bounds { min, max })
    }

    /// Draws a region from (100, 100) to (300, 300) and lets go.
    fn with_region() -> (Editor, View, ToolState) {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        export_region_tool(&mut editor, &view, &mut state, &press(100.0, 100.0));
        export_region_tool(&mut editor, &view, &mut state, &drag_to(300.0, 300.0));
        export_region_tool(&mut editor, &view, &mut state, &release(300.0, 300.0));
        (editor, view, state)
    }

    #[test]
    fn dragging_on_empty_canvas_draws_the_export_region() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        export_region_tool(&mut editor, &view, &mut state, &press(100.0, 100.0));
        export_region_tool(&mut editor, &view, &mut state, &drag_to(300.0, 250.0));
        assert_eq!(
            custom_region(&editor),
            bounds([100.0, 100.0], [300.0, 250.0])
        );
        export_region_tool(&mut editor, &view, &mut state, &release(300.0, 250.0));
        assert!(state.region_drag.is_none());
        assert!(custom_region(&editor).is_some());
    }

    #[test]
    fn a_region_drag_is_kept_inside_the_canvas() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        export_region_tool(&mut editor, &view, &mut state, &press(100.0, 100.0));
        export_region_tool(&mut editor, &view, &mut state, &drag_to(900.0, -50.0));
        assert_eq!(custom_region(&editor), bounds([100.0, 0.0], [500.0, 100.0]));
    }

    #[test]
    fn a_click_without_a_drag_leaves_no_region() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        export_region_tool(&mut editor, &view, &mut state, &press(100.0, 100.0));
        export_region_tool(&mut editor, &view, &mut state, &release(100.0, 100.0));
        assert_eq!(custom_region(&editor), None);
        assert!(state.region_drag.is_none());
    }

    #[test]
    fn dragging_inside_an_existing_region_moves_it() {
        let (mut editor, view, mut state) = with_region();
        export_region_tool(&mut editor, &view, &mut state, &press(200.0, 200.0));
        export_region_tool(&mut editor, &view, &mut state, &drag_to(230.0, 220.0));
        assert_eq!(
            custom_region(&editor),
            bounds([130.0, 120.0], [330.0, 320.0])
        );
    }

    #[test]
    fn dragging_a_handle_resizes_the_region() {
        let (mut editor, view, mut state) = with_region();
        // The bottom-right handle.
        export_region_tool(&mut editor, &view, &mut state, &press(300.0, 300.0));
        export_region_tool(&mut editor, &view, &mut state, &drag_to(350.0, 340.0));
        assert_eq!(
            custom_region(&editor),
            bounds([100.0, 100.0], [350.0, 340.0])
        );
    }

    #[test]
    fn escape_puts_the_previous_region_back() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        export_region_tool(&mut editor, &view, &mut state, &press(100.0, 100.0));
        export_region_tool(&mut editor, &view, &mut state, &drag_to(300.0, 300.0));
        editor.cancel_tool = true;
        export_region_tool(&mut editor, &view, &mut state, &input());
        assert_eq!(custom_region(&editor), None);
        assert!(state.region_drag.is_none());
    }

    #[test]
    fn the_export_region_cursor_follows_the_pointer() {
        let (mut editor, view, mut state) = with_region();
        let hover = |x, y| PointerInput {
            hovered: true,
            hover_pos: Some(at(x, y)),
            ..input()
        };
        let mut cursor = |input| export_region_tool(&mut editor, &view, &mut state, &input);
        assert_eq!(cursor(input()), None);
        assert_eq!(cursor(hover(10.0, 10.0)), Some(egui::CursorIcon::Crosshair));
        assert_eq!(cursor(hover(200.0, 200.0)), Some(egui::CursorIcon::Move));
    }

    #[test]
    fn clicking_the_canvas_edge_selects_the_art_board_but_its_inside_does_not() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        let click = |x, y| PointerInput {
            clicked: true,
            pointer_pos: Some(at(x, y)),
            ..input()
        };
        art_board_tool(&mut editor, &view, &mut state, &click(250.0, 250.0));
        assert!(!editor.board_selected());
        art_board_tool(&mut editor, &view, &mut state, &click(2.0, 250.0));
        assert!(editor.board_selected());
        art_board_tool(&mut editor, &view, &mut state, &click(250.0, 250.0));
        assert!(!editor.board_selected());
    }

    #[test]
    fn dragging_a_handle_of_the_selected_art_board_resizes_the_canvas() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        editor.select_board(true);
        art_board_tool(&mut editor, &view, &mut state, &press(500.0, 500.0));
        assert!(state.board_drag.is_some());
        art_board_tool(&mut editor, &view, &mut state, &drag_to(600.0, 560.0));
        assert_eq!(
            (editor.map.canvas.width, editor.map.canvas.height),
            (600, 560)
        );
        art_board_tool(&mut editor, &view, &mut state, &release(600.0, 560.0));
        assert!(state.board_drag.is_none());
        assert!(!editor.gesture_open());
    }

    #[test]
    fn escape_mid_drag_rolls_the_art_board_resize_back() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        editor.select_board(true);
        art_board_tool(&mut editor, &view, &mut state, &press(500.0, 500.0));
        art_board_tool(&mut editor, &view, &mut state, &drag_to(600.0, 560.0));
        assert_eq!(editor.map.canvas.width, 600);
        assert!(editor.cancel_open_gesture());
        art_board_tool(&mut editor, &view, &mut state, &drag_to(620.0, 580.0));
        assert!(state.board_drag.is_none() && state.held.is_none());
        assert_eq!(
            (editor.map.canvas.width, editor.map.canvas.height),
            (500, 500)
        );
        assert!(!editor.gesture_open());
    }

    #[test]
    fn switching_tools_mid_drag_rolls_the_gesture_back() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        editor.select_board(true);
        editor.active_tool = ToolId::ArtBoard;
        art_board_tool(&mut editor, &view, &mut state, &press(500.0, 500.0));
        art_board_tool(&mut editor, &view, &mut state, &drag_to(600.0, 560.0));
        assert_eq!(editor.map.canvas.width, 600);
        editor.active_tool = ToolId::Select;
        state.drop_held_for_other_tool(&mut editor);
        assert!(state.board_drag.is_none() && state.held.is_none());
        assert!(!editor.gesture_open());
        assert_eq!(
            (editor.map.canvas.width, editor.map.canvas.height),
            (500, 500)
        );
        assert_eq!(editor.undo_label(), None);
    }

    #[test]
    fn a_handle_of_an_unselected_art_board_is_not_grabbed() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        art_board_tool(&mut editor, &view, &mut state, &press(500.0, 500.0));
        assert!(state.board_drag.is_none());
    }

    fn outline() -> MultiPolygon {
        MultiPolygon(vec![Polygon {
            exterior: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
            holes: Vec::new(),
        }])
    }

    #[test]
    fn hit_outline_finds_a_node_within_reach() {
        let hit = hit_outline(&outline(), [103.0, 2.0], 5.0, 5.0);
        let node = NodeRef {
            part: 0,
            ring: 0,
            index: 1,
        };
        assert!(matches!(hit, NodeHit::Node(found) if found == node));
    }

    #[test]
    fn hit_outline_prefers_a_node_over_an_edge() {
        // Within reach of both the corner and the two edges meeting there.
        let hit = hit_outline(&outline(), [1.0, 1.0], 5.0, 5.0);
        assert!(matches!(hit, NodeHit::Node(NodeRef { index: 0, .. })));
    }

    #[test]
    fn hit_outline_finds_the_nearest_point_on_an_edge() {
        match hit_outline(&outline(), [50.0, 3.0], 5.0, 5.0) {
            NodeHit::Edge(edge, nearest) => {
                assert_eq!(edge.index, 0);
                assert_eq!(nearest, [50.0, 0.0]);
            }
            _ => panic!("expected an edge hit"),
        }
    }

    #[test]
    fn hit_outline_checks_the_closing_edge() {
        match hit_outline(&outline(), [2.0, 50.0], 5.0, 5.0) {
            NodeHit::Edge(edge, nearest) => {
                assert_eq!(edge.index, 3);
                assert_eq!(nearest, [0.0, 50.0]);
            }
            _ => panic!("expected an edge hit"),
        }
    }

    #[test]
    fn hit_outline_tells_inside_from_outside() {
        let inside = hit_outline(&outline(), [50.0, 50.0], 5.0, 5.0);
        assert!(matches!(inside, NodeHit::Inside));
        let outside = hit_outline(&outline(), [150.0, 50.0], 5.0, 5.0);
        assert!(matches!(outside, NodeHit::Outside));
    }

    /// An Editor in node-edit mode on a Landmass shaped like `outline()`.
    fn node_editor() -> Editor {
        let mut editor = land_editor();
        let layer = editor.map.children(None).first().copied();
        let id = Id::new();
        assert!(
            editor
                .run(Command::CreateLandmass {
                    id,
                    parent: layer.unwrap_or_else(Id::new),
                    index: None,
                    geometry: outline(),
                    fill: None,
                })
                .is_ok()
        );
        editor.enter_node_edit(id);
        editor
    }

    fn selected_nodes(editor: &Editor) -> Vec<NodeRef> {
        editor
            .node_edit()
            .map(|e| e.nodes.clone())
            .unwrap_or_default()
    }

    #[test]
    fn node_edit_click_selects_a_node_and_shift_adds_another() {
        let mut editor = node_editor();
        let (view, mut state) = (view(), ToolState::default());
        node_edit_tool(&mut editor, &view, &mut state, &click_at(100.0, 0.0));
        let first = NodeRef {
            part: 0,
            ring: 0,
            index: 1,
        };
        assert_eq!(selected_nodes(&editor), vec![first]);
        let shift_click = PointerInput {
            shift: true,
            ..click_at(100.0, 100.0)
        };
        node_edit_tool(&mut editor, &view, &mut state, &shift_click);
        assert_eq!(selected_nodes(&editor).len(), 2);
    }

    #[test]
    fn node_edit_click_outside_leaves_node_edit() {
        let mut editor = node_editor();
        let (view, mut state) = (view(), ToolState::default());
        node_edit_tool(&mut editor, &view, &mut state, &click_at(300.0, 300.0));
        assert!(editor.node_edit().is_none());
    }

    #[test]
    fn node_edit_double_click_on_an_edge_adds_a_node() {
        let mut editor = node_editor();
        let (view, mut state) = (view(), ToolState::default());
        let double = PointerInput {
            double_clicked: true,
            ..click_at(50.0, 1.0)
        };
        node_edit_tool(&mut editor, &view, &mut state, &double);
        assert_eq!(editor.edited_shape(), vec![5]);
    }

    #[test]
    fn node_edit_drag_moves_the_selected_node_in_one_step() {
        let mut editor = node_editor();
        let (view, mut state) = (view(), ToolState::default());
        node_edit_tool(&mut editor, &view, &mut state, &press(100.0, 0.0));
        node_edit_tool(&mut editor, &view, &mut state, &drag_to(120.0, 10.0));
        node_edit_tool(&mut editor, &view, &mut state, &release(120.0, 10.0));
        let moved = editor.edited_outline().map(|o| o.0[0].exterior[1]);
        assert_eq!(moved, Some([120.0, 10.0]));
    }

    #[test]
    fn node_edit_drag_on_empty_space_box_selects() {
        let mut editor = node_editor();
        let (view, mut state) = (view(), ToolState::default());
        node_edit_tool(&mut editor, &view, &mut state, &press(50.0, 50.0));
        node_edit_tool(&mut editor, &view, &mut state, &drag_to(150.0, 150.0));
        node_edit_tool(&mut editor, &view, &mut state, &release(150.0, 150.0));
        assert_eq!(selected_nodes(&editor).len(), 1);
    }

    #[test]
    fn node_edit_hover_over_a_node_asks_for_a_pointing_hand() {
        let mut editor = node_editor();
        let (view, mut state) = (view(), ToolState::default());
        let hover = |x, y| PointerInput {
            hovered: true,
            hover_pos: Some(at(x, y)),
            ..input()
        };
        let icon = node_edit_tool(&mut editor, &view, &mut state, &hover(2.0, 2.0));
        assert_eq!(icon, Some(egui::CursorIcon::PointingHand));
        let icon = node_edit_tool(&mut editor, &view, &mut state, &hover(50.0, 50.0));
        assert_eq!(icon, None);
    }

    #[test]
    fn nearest_on_segment_projects_onto_the_segment() {
        let (d, nearest) = nearest_on_segment([0.0, 0.0], [10.0, 0.0], [4.0, 3.0]);
        assert_eq!((d, nearest), (3.0, [4.0, 0.0]));
    }

    #[test]
    fn nearest_on_segment_clamps_to_the_ends() {
        let (d, nearest) = nearest_on_segment([0.0, 0.0], [10.0, 0.0], [-3.0, 4.0]);
        assert_eq!((d, nearest), (5.0, [0.0, 0.0]));
        let (d, nearest) = nearest_on_segment([0.0, 0.0], [10.0, 0.0], [13.0, 4.0]);
        assert_eq!((d, nearest), (5.0, [10.0, 0.0]));
    }

    #[test]
    fn nearest_on_segment_handles_a_zero_length_segment() {
        let (d, nearest) = nearest_on_segment([2.0, 2.0], [2.0, 2.0], [5.0, 6.0]);
        assert_eq!((d, nearest), (5.0, [2.0, 2.0]));
    }

    fn click_at(x: f32, y: f32) -> PointerInput {
        PointerInput {
            clicked: true,
            press_origin: Some(at(x, y)),
            pointer_pos: Some(at(x, y)),
            ..input()
        }
    }

    fn hovering() -> PointerInput {
        PointerInput {
            hovered: true,
            hover_pos: Some(at(10.0, 10.0)),
            ..input()
        }
    }

    /// A land brush stroke from (100, 100) to (200, 150), up to and including the release.
    fn paint_stroke(editor: &mut Editor, view: &View, state: &mut ToolState, water: bool) {
        land_brush(editor, view, state, &press(100.0, 100.0), water);
        land_brush(editor, view, state, &drag_to(150.0, 120.0), water);
        land_brush(editor, view, state, &drag_to(200.0, 150.0), water);
        land_brush(editor, view, state, &release(200.0, 150.0), water);
    }

    #[test]
    fn a_land_brush_drag_is_one_undo_step_made_on_release() {
        let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
        let before = editor.history_steps().1;
        land_brush(&mut editor, &view, &mut state, &press(100.0, 100.0), false);
        assert!(editor.gesture_open() && state.stroke.is_some());
        land_brush(
            &mut editor,
            &view,
            &mut state,
            &drag_to(200.0, 150.0),
            false,
        );
        assert!(editor.gesture_open());
        land_brush(
            &mut editor,
            &view,
            &mut state,
            &release(200.0, 150.0),
            false,
        );
        assert!(!editor.gesture_open() && state.stroke.is_none());
        assert_eq!(editor.undo_label(), Some("Paint Land"));
        assert_eq!(editor.history_steps().1, before + 1);
    }

    #[test]
    fn a_land_brush_click_starts_and_ends_a_stroke_in_one_frame() {
        let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
        let before = editor.history_steps().1;
        land_brush(
            &mut editor,
            &view,
            &mut state,
            &click_at(250.0, 250.0),
            false,
        );
        // A lone point has no outline to paint, so no Undo step is left, and nothing is held.
        assert!(!editor.gesture_open() && state.stroke.is_none());
        assert_eq!(editor.history_steps().1, before);
    }

    #[test]
    fn the_water_brush_paints_water() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        paint_stroke(&mut editor, &view, &mut state, true);
        assert_eq!(editor.undo_label(), Some("Paint Water"));
    }

    #[test]
    fn the_erase_modifier_flips_the_land_brush_to_erasing() {
        let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
        paint_stroke(&mut editor, &view, &mut state, false);
        let erase = |input: PointerInput| PointerInput { alt: true, ..input };
        land_brush(
            &mut editor,
            &view,
            &mut state,
            &erase(press(120.0, 110.0)),
            false,
        );
        land_brush(
            &mut editor,
            &view,
            &mut state,
            &erase(drag_to(180.0, 140.0)),
            false,
        );
        land_brush(
            &mut editor,
            &view,
            &mut state,
            &erase(release(180.0, 140.0)),
            false,
        );
        assert_eq!(editor.undo_label(), Some("Erase Land"));
    }

    #[test]
    fn escape_mid_stroke_drops_the_land_brush_stroke() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        land_brush(&mut editor, &view, &mut state, &press(100.0, 100.0), false);
        assert!(editor.cancel_open_gesture());
        land_brush(
            &mut editor,
            &view,
            &mut state,
            &drag_to(200.0, 150.0),
            false,
        );
        assert!(state.stroke.is_none());
        land_brush(
            &mut editor,
            &view,
            &mut state,
            &release(200.0, 150.0),
            false,
        );
        assert!(!editor.gesture_open());
        assert_eq!(editor.history_steps().1, 0);
    }

    #[test]
    fn switching_tools_mid_stroke_rolls_the_land_brush_back() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        editor.active_tool = ToolId::LandBrush;
        land_brush(&mut editor, &view, &mut state, &press(100.0, 100.0), false);
        assert!(editor.gesture_open());
        editor.active_tool = ToolId::Select;
        state.drop_held_for_other_tool(&mut editor);
        assert!(!editor.gesture_open());
        assert_eq!(editor.history_steps().1, 0);
    }

    #[test]
    fn the_brush_and_scatter_cursors_are_a_crosshair_over_the_canvas() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        let over = Some(egui::CursorIcon::Crosshair);
        assert_eq!(
            land_brush(&mut editor, &view, &mut state, &hovering(), false),
            over
        );
        assert_eq!(
            scatter_tool(&mut editor, &view, &mut state, &hovering(), false),
            over
        );
        assert_eq!(
            land_brush(&mut editor, &view, &mut state, &input(), false),
            None
        );
        assert_eq!(
            scatter_tool(&mut editor, &view, &mut state, &input(), true),
            None
        );
    }

    #[test]
    fn a_scatter_drag_collects_points_and_ends_on_release() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        scatter_tool(&mut editor, &view, &mut state, &press(100.0, 100.0), false);
        scatter_tool(
            &mut editor,
            &view,
            &mut state,
            &drag_to(160.0, 100.0),
            false,
        );
        scatter_tool(
            &mut editor,
            &view,
            &mut state,
            &drag_to(220.0, 100.0),
            false,
        );
        let points = state.stroke.as_ref().map(|s| s.points.len());
        assert!(points.is_some_and(|n| n >= 3), "points: {points:?}");
        scatter_tool(
            &mut editor,
            &view,
            &mut state,
            &release(220.0, 100.0),
            false,
        );
        assert!(state.stroke.is_none());
    }

    #[test]
    fn a_scatter_click_without_an_asset_asks_for_one_and_changes_nothing() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        scatter_tool(
            &mut editor,
            &view,
            &mut state,
            &click_at(100.0, 100.0),
            false,
        );
        assert!(state.stroke.is_none());
        assert_eq!(editor.history_steps().1, 0);
    }

    #[test]
    fn the_stamp_eraser_starts_a_stroke_on_a_click() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        scatter_tool(&mut editor, &view, &mut state, &press(100.0, 100.0), true);
        assert!(state.stroke.as_ref().is_some_and(|s| s.erase));
        scatter_tool(&mut editor, &view, &mut state, &release(100.0, 100.0), true);
        assert!(state.stroke.is_none());
    }

    #[test]
    fn escape_drops_a_scatter_stroke() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        scatter_tool(&mut editor, &view, &mut state, &press(100.0, 100.0), false);
        assert!(state.stroke.is_some());
        editor.cancel_tool = true;
        scatter_tool(
            &mut editor,
            &view,
            &mut state,
            &drag_to(200.0, 100.0),
            false,
        );
        assert!(state.stroke.is_none());
        assert!(!editor.cancel_tool);
    }

    fn click(x: f32, y: f32) -> PointerInput {
        PointerInput {
            clicked: true,
            pointer_pos: Some(at(x, y)),
            ..input()
        }
    }

    fn enter_key() -> PointerInput {
        PointerInput {
            enter: true,
            ..input()
        }
    }

    fn lasso_points(state: &ToolState) -> usize {
        state.lasso.as_ref().map_or(0, |l| l.points.len())
    }

    #[test]
    fn a_freehand_lasso_closes_on_release_into_one_undo_step() {
        let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
        let before = editor.history_steps().1;
        lasso_tool(&mut editor, &view, &mut state, &press(100.0, 100.0));
        for (x, y) in [(200.0, 100.0), (200.0, 200.0), (100.0, 200.0)] {
            lasso_tool(&mut editor, &view, &mut state, &drag_to(x, y));
        }
        assert!(state.lasso.as_ref().is_some_and(|l| l.freehand));
        lasso_tool(&mut editor, &view, &mut state, &release(100.0, 200.0));
        assert!(state.lasso.is_none());
        assert_eq!(editor.undo_label(), Some("Paint Land"));
        assert_eq!(editor.history_steps().1, before + 1);
    }

    #[test]
    fn clicking_the_first_corner_closes_a_polygon_lasso() {
        let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
        for (x, y) in [(100.0, 100.0), (200.0, 100.0), (200.0, 200.0)] {
            lasso_tool(&mut editor, &view, &mut state, &click(x, y));
        }
        assert_eq!(lasso_points(&state), 3);
        lasso_tool(&mut editor, &view, &mut state, &click(100.0, 100.0));
        assert!(state.lasso.is_none());
        assert_eq!(editor.undo_label(), Some("Paint Land"));
    }

    #[test]
    fn enter_closes_a_polygon_lasso_and_alt_makes_it_an_erase() {
        let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
        for (x, y) in [(100.0, 100.0), (200.0, 100.0), (200.0, 200.0)] {
            let alt_click = PointerInput {
                alt: true,
                ..click(x, y)
            };
            lasso_tool(&mut editor, &view, &mut state, &alt_click);
        }
        assert!(state.lasso.as_ref().is_some_and(|l| l.erase));
        lasso_tool(&mut editor, &view, &mut state, &enter_key());
        assert!(state.lasso.is_none());
    }

    #[test]
    fn a_lasso_that_encloses_nothing_changes_nothing() {
        let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
        let before = editor.history_steps().1;
        lasso_tool(&mut editor, &view, &mut state, &click(100.0, 100.0));
        lasso_tool(&mut editor, &view, &mut state, &enter_key());
        assert!(state.lasso.is_none());
        assert_eq!(editor.history_steps().1, before);
    }

    #[test]
    fn escape_drops_a_lasso_in_progress() {
        let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
        lasso_tool(&mut editor, &view, &mut state, &click(100.0, 100.0));
        assert_eq!(lasso_points(&state), 1);
        editor.cancel_tool = true;
        lasso_tool(&mut editor, &view, &mut state, &input());
        assert!(state.lasso.is_none());
        assert!(!editor.cancel_tool);
    }

    #[test]
    fn a_select_box_selects_the_objects_it_meets_on_release() {
        let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
        paint_stroke(&mut editor, &view, &mut state, false);
        editor.select(Vec::new());
        state.select_box = Some(SelectBox {
            start: [0.0, 0.0],
            end: [0.0, 0.0],
            keep: Vec::new(),
        });
        select_box_tool(&mut editor, &view, &mut state, &drag_to(300.0, 300.0));
        assert_eq!(
            state.select_box.as_ref().map(|b| b.end),
            Some([300.0, 300.0])
        );
        assert!(editor.selection().is_empty());
        select_box_tool(&mut editor, &view, &mut state, &release(300.0, 300.0));
        assert!(state.select_box.is_none());
        assert_eq!(editor.selection().len(), 1);
    }

    #[test]
    fn a_select_box_keeps_the_selection_it_was_given() {
        let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
        let kept = Id::new();
        state.select_box = Some(SelectBox {
            start: [400.0, 400.0],
            end: [400.0, 400.0],
            keep: vec![kept],
        });
        select_box_tool(&mut editor, &view, &mut state, &release(450.0, 450.0));
        assert_eq!(editor.selection(), [kept]);
    }
}
