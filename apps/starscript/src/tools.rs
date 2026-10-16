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

use crate::editor::Editor;
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
        ToolId::ExportRegion,
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
            ToolId::ExportRegion => "Export Region",
        }
    }

    /// The key and modifiers that select the tool.
    pub fn shortcut(self) -> (egui::Modifiers, egui::Key) {
        let none = egui::Modifiers::NONE;
        match self {
            ToolId::Select => (none, egui::Key::V),
            ToolId::Pan => (none, egui::Key::H),
            ToolId::LandBrush => (none, egui::Key::B),
            ToolId::WaterBrush => (none, egui::Key::W),
            ToolId::Lasso => (none, egui::Key::L),
            ToolId::Label => (none, egui::Key::T),
            ToolId::Stamp => (none, egui::Key::S),
            ToolId::StampEraser => (none, egui::Key::E),
            ToolId::ExportRegion => (none, egui::Key::X),
        }
    }

    /// The shortcut as shown in the UI, such as "V" or "⇧S".
    pub fn shortcut_text(self) -> String {
        let (modifiers, key) = self.shortcut();
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

/// Tool state that lasts across frames.
#[derive(Default)]
pub struct ToolState {
    drag: Option<Drag>,
    stroke: Option<Stroke>,
    node_drag: Option<NodeDrag>,
    lasso: Option<Lasso>,
    /// An Export Region drag in progress, with the region as it was before the drag.
    region_drag: Option<(RegionDrag, ExportRegion)>,
    /// A box-select in node-edit mode: where it began and where the pointer is, in Canvas units.
    marquee: Option<(Point, Point)>,
    /// The Label whose text is being typed in the overlay box.
    pub editing: Option<TextEditing>,
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
    /// Drops a node drag or box-select in progress, as when node editing ends.
    pub fn clear_node_gestures(&mut self) {
        self.node_drag = None;
        self.marquee = None;
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
    if editor.active_tool != ToolId::Select {
        editor.leave_node_edit();
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
            node_edit_tool(editor, view, state, input);
        }
        ToolId::LandBrush => land_brush(editor, view, state, input, false),
        ToolId::WaterBrush => land_brush(editor, view, state, input, true),
        ToolId::Lasso => lasso_tool(editor, view, state, input),
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
                start_drag(
                    editor,
                    state,
                    bounds,
                    grip_at(pos),
                    to_canvas(pos),
                    duplicate,
                );
            }
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
                    if let Some(command) = membership_command(editor, drag, to_canvas(pos)) {
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
        ToolId::Stamp => scatter_tool(editor, view, state, input, false),
        ToolId::ExportRegion => export_region_tool(editor, view, state, input),
        ToolId::StampEraser => scatter_tool(editor, view, state, input, true),
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
    input: &CanvasInput<'_>,
) {
    let response = input.response;
    let ppp = input.pixels_per_point;
    let canvas = [
        f64::from(editor.map.canvas.width),
        f64::from(editor.map.canvas.height),
    ];
    if std::mem::take(&mut editor.cancel_tool)
        && let Some((_, before)) = state.region_drag.take()
    {
        editor.map.export_settings.region = before;
        return;
    }
    let existing = custom_region(editor);
    let grip_at = |pos: egui::Pos2| {
        let bounds = existing?;
        handles::hit(&bounds, view, response.rect.min, ppp, pos, false)
    };
    if response.hovered() {
        let icon = match response.hover_pos() {
            Some(pos) if state.region_drag.is_none() => match grip_at(pos) {
                Some(grip) => grip.cursor(),
                None if existing.is_some_and(|b| b.contains(input.canvas_point(view, pos))) => {
                    egui::CursorIcon::Move
                }
                None => egui::CursorIcon::Crosshair,
            },
            _ => egui::CursorIcon::Crosshair,
        };
        response.ctx.set_cursor_icon(icon);
    }
    if response.drag_started_by(egui::PointerButton::Primary)
        && state.region_drag.is_none()
        && let Some(pos) = response.ctx.input(|i| i.pointer.press_origin())
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
        return;
    };
    if response.dragged_by(egui::PointerButton::Primary)
        && let Some(pos) = response.interact_pointer_pos()
    {
        let rect = drag.rect(input.canvas_point(view, pos), canvas);
        editor.map.export_settings.region = ExportRegion::Custom { rect };
    }
    if response.drag_stopped() {
        state.region_drag = None;
        if let ExportRegion::Custom { rect } = &editor.map.export_settings.region
            && !export_region::is_usable(rect)
        {
            editor.map.export_settings.region = before;
        }
    }
}

/// Paint land by dragging, or erase with the erase modifier held. Each stroke is one Undo
/// step, made on release; Escape drops a stroke in progress. The brush rules (what merges,
/// what is created, what is erased) live in [`land::brush_command`], or with `water`, in
/// [`land::water_command`].
fn land_brush(
    editor: &mut Editor,
    view: &View,
    state: &mut ToolState,
    input: &CanvasInput<'_>,
    water: bool,
) {
    let response = input.response;
    let radius = editor.brush_size / 2.0;
    let erase_held = response.ctx.input(|i| i.modifiers.alt) != editor.erase_default;
    if response.hovered() {
        response.ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
    }

    // A click is a stroke of one point: a single dab.
    let pressed = response.drag_started_by(egui::PointerButton::Primary)
        || response.clicked_by(egui::PointerButton::Primary);
    if pressed && state.stroke.is_none() && !editor.gesture_open() {
        let origin = response.ctx.input(|i| i.pointer.press_origin());
        if let Some(origin) = origin.or(response.interact_pointer_pos()) {
            let label = if water {
                Editor::water_label(erase_held)
            } else {
                editor.brush_label(erase_held)
            };
            editor.begin_gesture(label);
            state.stroke = Some(Stroke {
                points: vec![input.canvas_point(view, origin)],
                erase: erase_held,
                preview: None,
            });
        }
    }

    let Some(stroke) = &mut state.stroke else {
        return;
    };
    // Escape rolled the gesture back: drop the stroke.
    if !editor.gesture_open() {
        state.stroke = None;
        return;
    }
    if let Some(pos) = response.interact_pointer_pos() {
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
        start: stroke.points[0],
        erase: stroke.erase,
    };
    let released = response.drag_stopped() || response.clicked();
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
        if water {
            editor.finish_water_stroke(finished);
        } else {
            editor.finish_brush_stroke(finished);
        }
    }
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
    input: &CanvasInput<'_>,
    erase: bool,
) {
    let response = input.response;
    if response.hovered() {
        response.ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    if std::mem::take(&mut editor.cancel_tool) {
        state.stroke = None;
        return;
    }
    if !erase
        && response.clicked_by(egui::PointerButton::Primary)
        && let Some(pos) = response.interact_pointer_pos()
    {
        if editor.selected_asset.is_none() {
            editor.error("Choose an Asset in the Asset browser first");
        } else {
            editor.place_stamp(input.canvas_point(view, pos));
        }
        return;
    }
    let pressed = response.drag_started_by(egui::PointerButton::Primary)
        || (erase && response.clicked_by(egui::PointerButton::Primary));
    if pressed && state.stroke.is_none() {
        let origin = response.ctx.input(|i| i.pointer.press_origin());
        if let Some(origin) = origin.or(response.interact_pointer_pos()) {
            state.stroke = Some(Stroke {
                points: vec![input.canvas_point(view, origin)],
                erase,
                preview: None,
            });
        }
    }
    let Some(stroke) = &mut state.stroke else {
        return;
    };
    if let Some(pos) = response.interact_pointer_pos() {
        let point = input.canvas_point(view, pos);
        let spacing = (editor.brush_size * 0.05).max(1.0 / view.zoom);
        let last = *stroke.points.last().expect("a stroke starts with a point");
        if (point[0] - last[0]).hypot(point[1] - last[1]) >= spacing {
            stroke.points.push(point);
        }
    }
    if (response.drag_stopped() || response.clicked())
        && let Some(stroke) = state.stroke.take()
    {
        if erase {
            editor.erase_stamps(&stroke.points);
        } else {
            editor.scatter_stamps(&stroke.points);
        }
    }
}

/// Draw a region to add to land, or with the erase modifier to cut out: drag a freehand loop, or
/// click corners and close the loop by clicking the first one, double-clicking or pressing
/// Enter. Escape drops a loop in progress. The region follows the land brush's merge rules.
fn lasso_tool(editor: &mut Editor, view: &View, state: &mut ToolState, input: &CanvasInput<'_>) {
    let response = input.response;
    let (alt, enter) = response
        .ctx
        .input(|i| (i.modifiers.alt, i.key_pressed(egui::Key::Enter)));
    if response.hovered() {
        response.ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    if std::mem::take(&mut editor.cancel_tool) {
        state.lasso = None;
        return;
    }
    let pointer = response
        .interact_pointer_pos()
        .map(|pos| input.canvas_point(view, pos));
    let near_first = |lasso: &Lasso, point: Point| {
        let first = lasso.points[0];
        let reach = NODE_GRAB * input.pixels_per_point / view.zoom;
        lasso.points.len() >= 3 && (point[0] - first[0]).hypot(point[1] - first[1]) <= reach
    };

    if response.drag_started_by(egui::PointerButton::Primary)
        && state.lasso.is_none()
        && let Some(origin) = response.ctx.input(|i| i.pointer.press_origin())
    {
        state.lasso = Some(Lasso {
            points: vec![input.canvas_point(view, origin)],
            freehand: true,
            erase: alt,
        });
    }
    if response.dragged_by(egui::PointerButton::Primary)
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
    if response.drag_stopped() && state.lasso.as_ref().is_some_and(|l| l.freehand) {
        close = true;
    } else if response.clicked_by(egui::PointerButton::Primary)
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
    if enter || (response.double_clicked_by(egui::PointerButton::Primary) && state.lasso.is_some())
    {
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
        editor.begin_gesture(editor.brush_label(lasso.erase));
        editor.finish_brush_stroke(BrushStroke {
            region,
            start: lasso.points[0],
            erase: lasso.erase,
        });
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
    input: &CanvasInput<'_>,
) {
    let response = input.response;
    let shift = response.ctx.input(|i| i.modifiers.shift);
    let per_point = input.pixels_per_point / view.zoom;
    let hit_at = |editor: &Editor, pos: egui::Pos2| {
        let point = input.canvas_point(view, pos);
        editor.edited_outline().map(|outline| {
            hit_outline(outline, point, NODE_GRAB * per_point, EDGE_GRAB * per_point)
        })
    };

    if let Some(pos) = response.hover_pos()
        && state.node_drag.is_none()
        && let Some(NodeHit::Node(_)) = hit_at(editor, pos)
    {
        response.ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    // Drag: move nodes, or box-select.
    if response.drag_started_by(egui::PointerButton::Primary) && !editor.gesture_open() {
        let origin = response
            .ctx
            .input(|i| i.pointer.press_origin())
            .or(response.interact_pointer_pos());
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
                    state.node_drag = Some(NodeDrag {
                        start,
                        shape: editor.edited_shape(),
                    });
                    editor.begin_gesture("Move Nodes");
                }
                _ => state.marquee = Some((start, start)),
            }
        }
    }

    if response.dragged_by(egui::PointerButton::Primary)
        && let Some(pos) = response.interact_pointer_pos()
    {
        let point = input.canvas_point(view, pos);
        if let Some(drag) = &state.node_drag {
            if !editor.gesture_open() {
                // Escape rolled the gesture back.
                state.node_drag = None;
            } else if let Some(edit) = editor.node_edit().cloned() {
                let offset = [point[0] - drag.start[0], point[1] - drag.start[1]];
                editor.gesture_replace(Command::MoveNodes {
                    id: edit.id,
                    nodes: edit.nodes,
                    offset,
                });
            }
        } else if let Some((_, end)) = &mut state.marquee {
            *end = point;
        }
    }

    if response.drag_stopped() {
        if let Some(drag) = state.node_drag.take() {
            editor.end_gesture();
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

    if response.double_clicked_by(egui::PointerButton::Primary)
        && let Some(pos) = response.interact_pointer_pos()
        && let Some(NodeHit::Edge(edge, point)) = hit_at(editor, pos)
    {
        editor.insert_node(edge, point);
        return;
    }

    if response.clicked_by(egui::PointerButton::Primary)
        && let Some(pos) = response.interact_pointer_pos()
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
    if editor.active_tool == ToolId::Select {
        paint_nodes(painter, editor, view, state, origin, hover, ppp);
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
    let outlines = editor.selected_outlines();
    let labels: Vec<(Id, Label)> = editor
        .selection()
        .iter()
        .filter_map(|&id| editor.label(id).map(|label| (id, label.clone())))
        .collect();
    let stamps: Vec<(Id, Stamp)> = editor
        .selection()
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
        editor.begin_gesture("Duplicate Objects");
        let (command, new_ids) = editor.duplicate_command(&sources, [0.0, 0.0]);
        if !editor.gesture(command) {
            editor.cancel_gesture();
            return;
        }
        editor.select(new_ids.clone());
        copies = Some((sources, new_ids));
    } else {
        editor.begin_gesture(match grip {
            None => "Move",
            Some(Grip::Resize(_)) => "Resize",
            Some(Grip::Rotate) => "Rotate",
        });
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
