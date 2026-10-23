//! The input a tool reads each frame.

use eframe::egui;
use ssmap_render::View;

/// One frame of Canvas input as plain data: the primary-button pointer, the keys and modifiers
/// a tool reads, and double-click. A tool can be a function from this to `Command`s and be
/// tested without a running `egui::Context`. Positions are egui points on the screen;
/// `rect_min` is the top-left of the Canvas widget.
#[derive(Clone, Debug, Default)]
pub(crate) struct CanvasFrame {
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
    /// The middle button is dragging.
    pub middle_dragging: bool,
    /// Any button is dragging.
    pub any_dragging: bool,
    /// How far the pointer moved this frame, in egui points.
    pub drag_delta: egui::Vec2,
    pub space_held: bool,
    pub shift: bool,
    pub alt: bool,
    /// Enter went down this frame.
    pub enter: bool,
}

impl CanvasFrame {
    pub(super) fn from_response(response: &egui::Response, pixels_per_point: f64) -> Self {
        let primary = egui::PointerButton::Primary;
        let (press_origin, shift, alt, enter, space_held) = response.ctx.input(|i| {
            (
                i.pointer.press_origin(),
                i.modifiers.shift,
                i.modifiers.alt,
                i.key_pressed(egui::Key::Enter),
                i.key_down(egui::Key::Space),
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
            middle_dragging: response.dragged_by(egui::PointerButton::Middle),
            any_dragging: response.dragged(),
            drag_delta: response.drag_delta(),
            space_held,
            shift,
            alt,
            enter,
        }
    }

    /// The Canvas point under a screen position.
    pub(super) fn canvas_point(&self, view: &View, pos: egui::Pos2) -> [f64; 2] {
        let local = pos - self.rect_min;
        view.to_canvas([
            f64::from(local.x) * self.pixels_per_point,
            f64::from(local.y) * self.pixels_per_point,
        ])
    }
}
