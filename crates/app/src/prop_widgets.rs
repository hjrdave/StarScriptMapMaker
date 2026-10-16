//! The building blocks of the Properties panel (#168): section headers, numeric scrub fields,
//! sliders, colour swatches, segmented controls and the inherited / overridden states.
//!
//! The widgets never touch the Editor. Each returns an [`Edit`] saying whether a drag or typed
//! entry began, changed the value, or finished, and [`LiveEdit`] turns that into one gesture,
//! so one scrub is one Undo step. The Mixed state (multi-edit) is out of scope.

use eframe::egui::{self, Color32, Sense, Stroke, StrokeKind, Vec2};
use ssmap_core::{Color, Command};

use crate::editor::Editor;
use crate::icons::{self, Icon};
use crate::theme::{self, metrics};

// ---------------------------------------------------------------------------------------------
// Arithmetic

/// Evaluates what was typed into a numeric field: a number, or simple arithmetic with
/// `+ - * /` and parentheses (`2716+88`). A leading `+`, `*` or `/` applies to `current`
/// (`*2`, `/3`). Returns `None` for anything that does not give a finite number.
pub fn eval_expression(text: &str, current: f64) -> Option<f64> {
    let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let text = text.replace('×', "*").replace('÷', "/").replace('−', "-");
    let relative = matches!(text.chars().next()?, '*' | '/' | '+');
    let source = if relative {
        format!("({current}){text}")
    } else {
        text
    };
    let mut parser = Parser {
        chars: source.chars().collect(),
        at: 0,
    };
    let value = parser.sum()?;
    (parser.at == parser.chars.len() && value.is_finite()).then_some(value)
}

struct Parser {
    chars: Vec<char>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn sum(&mut self) -> Option<f64> {
        let mut value = self.product()?;
        while let Some(op @ ('+' | '-')) = self.peek() {
            self.at += 1;
            let rhs = self.product()?;
            value = if op == '+' { value + rhs } else { value - rhs };
        }
        Some(value)
    }

    fn product(&mut self) -> Option<f64> {
        let mut value = self.unary()?;
        while let Some(op @ ('*' | '/')) = self.peek() {
            self.at += 1;
            let rhs = self.unary()?;
            if op == '/' && rhs == 0.0 {
                return None;
            }
            value = if op == '*' { value * rhs } else { value / rhs };
        }
        Some(value)
    }

    fn unary(&mut self) -> Option<f64> {
        match self.peek()? {
            '-' => {
                self.at += 1;
                self.unary().map(|v| -v)
            }
            '(' => {
                self.at += 1;
                let value = self.sum()?;
                if self.peek()? != ')' {
                    return None;
                }
                self.at += 1;
                Some(value)
            }
            _ => self.number(),
        }
    }

    fn number(&mut self) -> Option<f64> {
        let start = self.at;
        while let Some(c) = self.peek() {
            // A sign is part of the number only as an exponent's, as in the `1e-3` that
            // formatting a small value gives.
            let exponent_sign = matches!(c, '-' | '+')
                && self.at > start
                && matches!(self.chars[self.at - 1], 'e' | 'E');
            if c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E') || exponent_sign {
                self.at += 1;
            } else {
                break;
            }
        }
        self.chars[start..self.at]
            .iter()
            .collect::<String>()
            .parse()
            .ok()
    }
}

// ---------------------------------------------------------------------------------------------
// Scrub maths

/// The speed modifier held during a drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speed {
    Normal,
    /// ⇧: ×10.
    Coarse,
    /// ⌥: ×0.1.
    Fine,
}

impl Speed {
    pub fn from_modifiers(modifiers: egui::Modifiers) -> Self {
        if modifiers.shift {
            Speed::Coarse
        } else if modifiers.alt {
            Speed::Fine
        } else {
            Speed::Normal
        }
    }

    pub fn factor(self) -> f64 {
        match self {
            Speed::Normal => 1.0,
            Speed::Coarse => 10.0,
            Speed::Fine => 0.1,
        }
    }
}

/// The range a field accepts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub min: f64,
    pub max: f64,
}

impl Range {
    pub const ANY: Range = Range {
        min: f64::NEG_INFINITY,
        max: f64::INFINITY,
    };

    pub fn new(min: f64, max: f64) -> Self {
        Range { min, max }
    }

    pub fn clamp(self, value: f64) -> f64 {
        value.clamp(self.min, self.max)
    }
}

/// The value after dragging `pixels` from `start`: `step` units per pixel, scaled by the
/// speed modifier, clamped to `range`.
pub fn scrub_value(start: f64, pixels: f64, step: f64, speed: Speed, range: Range) -> f64 {
    range.clamp(start + pixels * step * speed.factor())
}

fn round_to(value: f64, decimals: usize) -> f64 {
    let scale = 10f64.powi(decimals as i32);
    (value * scale).round() / scale
}

// ---------------------------------------------------------------------------------------------
// Gestures

/// What a widget did this frame. A scrub drag begins, changes many times, then finishes;
/// typed entry and a click on a segment do all three in one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Edit {
    pub began: bool,
    pub changed: bool,
    pub finished: bool,
    /// The reset arrow of an overridden property was clicked.
    pub reset: bool,
}

impl Edit {
    pub fn instant() -> Self {
        Edit {
            began: true,
            changed: true,
            finished: true,
            reset: false,
        }
    }
}

/// Turns [`Edit`]s into one gesture on the Editor: one Undo step per scrub.
#[derive(Default)]
pub struct LiveEdit {
    owns_gesture: bool,
}

impl LiveEdit {
    /// Applies `command` (built only when the value changed) to the open gesture, opening
    /// it with `label` first, and closes it when the edit finishes. A gesture someone else
    /// opened is joined and left open.
    pub fn apply(
        &mut self,
        editor: &mut Editor,
        edit: Edit,
        label: &str,
        command: impl FnOnce() -> Command,
    ) {
        if (edit.began || edit.changed) && !self.owns_gesture && !editor.gesture_open() {
            editor.begin_gesture(label);
            self.owns_gesture = true;
        }
        if edit.changed {
            if self.owns_gesture {
                editor.gesture_replace(command());
            } else {
                editor.gesture(command());
            }
        }
        if self.owns_gesture && (edit.finished || !editor.gesture_open()) {
            self.owns_gesture = false;
            editor.end_gesture();
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Inherited and overridden

/// Where a property's value comes from. Never shown by colour alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Provenance {
    /// Set here, with nothing to inherit.
    #[default]
    Plain,
    /// Taken from a default: italic-style value with a link glyph.
    Inherited,
    /// Set here over a default: gold dot, gold label and a reset arrow.
    Overridden,
}

const DOT_WIDTH: f32 = 10.0;
const LABEL_MIN: f32 = 56.0;

/// The label of a field, with the gold dot when overridden.
fn paint_label(ui: &egui::Ui, rect: egui::Rect, label: &str, provenance: Provenance) {
    let palette = theme::palette(ui.ctx());
    let colour = match (ui.is_enabled(), provenance) {
        (false, _) => palette.text_tertiary,
        (_, Provenance::Overridden) => palette.gold_text,
        _ => palette.text_secondary,
    };
    let mut left = rect.left();
    if provenance == Provenance::Overridden {
        ui.painter()
            .circle_filled(egui::pos2(left + 3.0, rect.center().y), 3.0, palette.gold);
        left += DOT_WIDTH;
    }
    ui.painter().text(
        egui::pos2(left, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(theme::BODY),
        colour,
    );
}

fn label_width(ui: &egui::Ui, label: &str, provenance: Provenance) -> f32 {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::FontId::proportional(theme::BODY),
        Color32::WHITE,
    );
    let dot = if provenance == Provenance::Overridden {
        DOT_WIDTH
    } else {
        0.0
    };
    (galley.size().x + dot + 8.0).max(LABEL_MIN)
}

/// The width the trailing glyph or reset arrow of a provenance takes.
fn trailing_width(provenance: Provenance) -> f32 {
    match provenance {
        Provenance::Plain => 0.0,
        Provenance::Inherited => 20.0,
        Provenance::Overridden => 28.0,
    }
}

/// What the trailing glyph or reset arrow takes from the row, with the gap before it.
fn gap_before_trailing(ui: &egui::Ui, provenance: Provenance) -> f32 {
    match trailing_width(provenance) {
        0.0 => 0.0,
        width => width + ui.spacing().item_spacing.x,
    }
}

/// The trailing part of a row: the link glyph when inherited, the reset arrow when
/// overridden (its accessible name says what it resets). Returns whether reset was clicked.
fn trailing(ui: &mut egui::Ui, label: &str, provenance: Provenance) -> bool {
    match provenance {
        Provenance::Plain => false,
        Provenance::Inherited => {
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
            let palette = theme::palette(ui.ctx());
            icons::paint(ui, rect, Icon::Inherited, 14.0, palette.text_tertiary);
            response.on_hover_text("Inherited from the default");
            false
        }
        Provenance::Overridden => {
            icons::icon_button(ui, Icon::Reset, &format!("Reset {label}")).clicked()
        }
    }
}

fn paint_value(
    ui: &egui::Ui,
    rect: egui::Rect,
    text: &str,
    provenance: Provenance,
    align: egui::Align2,
) {
    let palette = theme::palette(ui.ctx());
    let colour = match (ui.is_enabled(), provenance) {
        (false, _) => palette.text_tertiary,
        (_, Provenance::Inherited) => palette.text_secondary,
        _ => palette.text_primary,
    };
    let pos = if align == egui::Align2::RIGHT_CENTER {
        egui::pos2(rect.right() - 6.0, rect.center().y)
    } else {
        egui::pos2(rect.left() + 6.0, rect.center().y)
    };
    let font = egui::FontId::new(theme::MONO, theme::mono_family());
    let text_rect = ui.painter().text(pos, align, text, font, colour);
    if provenance == Provenance::Inherited {
        // The bundled fonts have no italic face, so inherited values get a dotted underline
        // as well as the link glyph.
        let y = text_rect.bottom() + 1.0;
        let mut x = text_rect.left();
        while x < text_rect.right() {
            ui.painter().hline(
                x..=(x + 1.5).min(text_rect.right()),
                y,
                Stroke::new(1.0, colour),
            );
            x += 3.0;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Section header

fn section_id(kind: &str) -> egui::Id {
    egui::Id::new(("section", kind))
}

/// Whether the section of this kind is open. Remembered per kind, not per Map.
pub fn section_open(ctx: &egui::Context, kind: &str) -> bool {
    ctx.data_mut(|d| *d.get_persisted_mut_or(section_id(kind), true))
}

/// Makes the section of this kind start collapsed, until the user opens it.
pub fn default_closed(ctx: &egui::Context, kind: &str) {
    ctx.data_mut(|d| {
        d.get_persisted_mut_or(section_id(kind), false);
    });
}

/// What a section header offers.
pub struct SectionHeader<'a> {
    /// Names the section kind, and remembers whether it is collapsed.
    pub kind: &'a str,
    pub title: &'a str,
    /// For example "1 override".
    pub badge: Option<&'a str>,
    /// An action at the right edge, as an icon button with its accessible name.
    pub action: Option<(Icon, &'a str)>,
}

/// A collapsible section: heading, optional badge and trailing action, then `body` while open.
/// Returns whether the trailing action was clicked.
pub fn section(
    ui: &mut egui::Ui,
    header: SectionHeader<'_>,
    body: impl FnOnce(&mut egui::Ui),
) -> bool {
    let open = section_open(ui.ctx(), header.kind);
    let palette = theme::palette(ui.ctx());
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 28.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::CollapsingHeader,
            ui.is_enabled(),
            open,
            header.title,
        )
    });
    let mut clicked_action = false;
    let mut right = rect.right();
    if let Some((icon, label)) = header.action {
        let button = egui::Rect::from_min_size(
            egui::pos2(right - 24.0, rect.center().y - 12.0),
            Vec2::splat(24.0),
        );
        clicked_action = ui
            .put(button, |ui: &mut egui::Ui| {
                icons::icon_button(ui, icon, label)
            })
            .clicked();
        right -= 28.0;
    }
    let chevron = if open {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };
    let chevron_rect = egui::Rect::from_min_size(rect.left_top(), Vec2::new(18.0, rect.height()));
    icons::paint(
        ui,
        chevron_rect,
        chevron,
        icons::GRID,
        palette.text_secondary,
    );
    let title = ui.painter().text(
        egui::pos2(rect.left() + 20.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        header.title,
        egui::FontId::proportional(theme::HEADING),
        palette.text_primary,
    );
    if let Some(badge) = header.badge {
        let galley = ui.painter().layout_no_wrap(
            badge.to_owned(),
            egui::FontId::proportional(theme::CAPTION),
            palette.gold_text,
        );
        let size = galley.size() + Vec2::new(8.0, 2.0);
        let chip = egui::Rect::from_min_size(
            egui::pos2(title.right() + 8.0, rect.center().y - size.y / 2.0),
            size,
        );
        if chip.right() <= right {
            ui.painter()
                .rect_filled(chip, theme::RADIUS_CHIP, palette.gold_subtle);
            ui.painter()
                .galley(chip.min + Vec2::new(4.0, 1.0), galley, palette.gold_text);
        }
    }
    if response.clicked() && !clicked_action {
        ui.ctx()
            .data_mut(|d| d.insert_persisted(section_id(header.kind), !open));
    }
    if open {
        body(ui);
    }
    clicked_action
}

// ---------------------------------------------------------------------------------------------
// Numeric scrub field

/// The look and limits of a numeric field.
#[derive(Clone, Copy)]
pub struct ScrubSpec<'a> {
    pub label: &'a str,
    /// "px", "%", "°" or "pt".
    pub unit: &'a str,
    pub range: Range,
    /// Units per dragged pixel.
    pub step: f64,
    pub decimals: usize,
    pub provenance: Provenance,
    /// 22 or 26 px high.
    pub height: f32,
}

impl<'a> ScrubSpec<'a> {
    pub fn new(label: &'a str) -> Self {
        ScrubSpec {
            label,
            unit: "",
            range: Range::ANY,
            step: 1.0,
            decimals: 0,
            provenance: Provenance::Plain,
            height: metrics::FIELD[0],
        }
    }

    pub fn unit(mut self, unit: &'a str) -> Self {
        self.unit = unit;
        self
    }

    pub fn range(mut self, min: f64, max: f64) -> Self {
        self.range = Range::new(min, max);
        self
    }

    pub fn step(mut self, step: f64) -> Self {
        self.step = step;
        self
    }

    pub fn decimals(mut self, decimals: usize) -> Self {
        self.decimals = decimals;
        self
    }

    pub fn provenance(mut self, provenance: Provenance) -> Self {
        self.provenance = provenance;
        self
    }

    /// The 26 px field height.
    pub fn tall(mut self) -> Self {
        self.height = metrics::FIELD[1];
        self
    }

    fn format(&self, value: f64) -> String {
        format!("{value:.*}{}", self.decimals, self.unit)
    }
}

#[derive(Clone, Default)]
struct ScrubState {
    /// The value when the drag began.
    start: f64,
    /// The pointer travel since the drag began, so clamping at an end loses nothing.
    travel: f64,
    /// The text being typed, while the field is in typing mode.
    typing: Option<String>,
    dragging: bool,
}

/// A numeric field whose label is the scrub handle. Dragging the label changes the value
/// (⇧ ×10, ⌥ ×0.1); clicking the value types one, with arithmetic, on Enter; Esc cancels.
pub fn scrub_field(ui: &mut egui::Ui, value: &mut f64, spec: &ScrubSpec<'_>) -> Edit {
    let palette = theme::palette(ui.ctx());
    let id = ui.make_persistent_id(("scrub", spec.label));
    let mut state: ScrubState = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let mut edit = Edit::default();
    let enabled = ui.is_enabled();

    ui.horizontal(|ui| {
        let (label_rect, handle) = ui.allocate_exact_size(
            Vec2::new(label_width(ui, spec.label, spec.provenance), spec.height),
            Sense::drag(),
        );
        handle.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::DragValue, enabled, spec.label)
        });
        paint_label(ui, label_rect, spec.label, spec.provenance);
        if enabled {
            handle
                .clone()
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        }

        if handle.drag_started() {
            state.start = *value;
            state.travel = 0.0;
            state.dragging = true;
            edit.began = true;
        }
        if handle.dragged() && state.dragging {
            state.travel += f64::from(handle.drag_delta().x);
            let speed = Speed::from_modifiers(ui.input(|i| i.modifiers));
            let next = scrub_value(state.start, state.travel, spec.step, speed, spec.range);
            let next = round_to(next, spec.decimals);
            if next != *value {
                *value = next;
                edit.changed = true;
            }
        }
        if handle.drag_stopped() && state.dragging {
            state.dragging = false;
            edit.finished = true;
        }

        let field_w = (ui.available_width() - gap_before_trailing(ui, spec.provenance)).max(40.0);
        let (field_rect, field) =
            ui.allocate_exact_size(Vec2::new(field_w, spec.height), Sense::click());
        ui.painter().rect(
            field_rect,
            theme::RADIUS_FIELD,
            palette.bg_field,
            Stroke::new(1.0, palette.border_subtle),
            StrokeKind::Inside,
        );
        if let Some(text) = state.typing.as_mut() {
            let response = ui.put(
                field_rect,
                egui::TextEdit::singleline(text)
                    .frame(egui::Frame::NONE)
                    .margin(egui::Margin::symmetric(6, 0))
                    .font(egui::FontId::new(theme::MONO, theme::mono_family())),
            );
            if !response.has_focus() && !response.lost_focus() {
                response.request_focus();
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                state.typing = None;
            } else if response.lost_focus() {
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let typed = state.typing.take().unwrap_or_default();
                    if let Some(result) = eval_expression(&typed, *value) {
                        let result = round_to(spec.range.clamp(result), spec.decimals.max(3));
                        if result != *value {
                            *value = result;
                            edit = Edit::instant();
                        }
                    }
                }
                state.typing = None;
            }
        } else {
            paint_value(
                ui,
                field_rect,
                &spec.format(*value),
                spec.provenance,
                egui::Align2::LEFT_CENTER,
            );
            field.widget_info(|| {
                egui::WidgetInfo::text_edit(enabled, spec.format(*value), spec.format(*value), "")
            });
            if enabled && field.clicked() {
                state.typing = Some(format!("{value:.*}", spec.decimals));
            }
        }
        if trailing(ui, spec.label, spec.provenance) {
            edit.reset = true;
        }
    });

    ui.data_mut(|d| d.insert_temp(id, state));
    edit
}

// ---------------------------------------------------------------------------------------------
// Slider

/// A slider with a value readout. Dragging is one gesture; clicking the track is one step.
pub fn slider(ui: &mut egui::Ui, value: &mut f64, spec: &ScrubSpec<'_>) -> Edit {
    let palette = theme::palette(ui.ctx());
    let enabled = ui.is_enabled();
    let mut edit = Edit::default();
    ui.horizontal(|ui| {
        let (label_rect, _) = ui.allocate_exact_size(
            Vec2::new(label_width(ui, spec.label, spec.provenance), spec.height),
            Sense::hover(),
        );
        paint_label(ui, label_rect, spec.label, spec.provenance);

        let readout_w = 48.0;
        let track_w = (ui.available_width()
            - readout_w
            - ui.spacing().item_spacing.x
            - gap_before_trailing(ui, spec.provenance))
        .max(40.0);
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(track_w, spec.height),
            if enabled {
                Sense::click_and_drag()
            } else {
                Sense::hover()
            },
        );
        response.widget_info(|| egui::WidgetInfo::slider(enabled, *value, spec.label));
        let span = spec.range.max - spec.range.min;
        let usable = span.is_finite() && span > 0.0;
        if enabled && usable {
            if response.drag_started() || response.clicked() {
                edit.began = true;
            }
            if (response.dragged() || response.clicked())
                && let Some(pos) = response.interact_pointer_pos()
            {
                let t = f64::from(
                    ((pos.x - rect.left() - 6.0) / (rect.width() - 12.0)).clamp(0.0, 1.0),
                );
                let next = round_to(spec.range.min + t * span, spec.decimals);
                if next != *value {
                    *value = next;
                    edit.changed = true;
                }
            }
            if response.drag_stopped() || response.clicked() {
                edit.finished = true;
            }
        }
        let t = if usable {
            ((*value - spec.range.min) / span).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let (line, fill, thumb, radius) = if !enabled {
            (
                palette.border_subtle,
                palette.border_strong,
                palette.text_tertiary,
                6.0,
            )
        } else if response.dragged() {
            (palette.border_strong, palette.gold, palette.gold_text, 7.0)
        } else {
            (
                palette.border_strong,
                palette.gold,
                palette.text_primary,
                6.0,
            )
        };
        let y = rect.center().y;
        let (x0, x1) = (rect.left() + 6.0, rect.right() - 6.0);
        let x = x0 + t * (x1 - x0);
        ui.painter().line_segment(
            [egui::pos2(x0, y), egui::pos2(x1, y)],
            Stroke::new(3.0, line),
        );
        ui.painter().line_segment(
            [egui::pos2(x0, y), egui::pos2(x, y)],
            Stroke::new(3.0, fill),
        );
        ui.painter().circle(
            egui::pos2(x, y),
            radius,
            thumb,
            Stroke::new(1.0, palette.bg_field),
        );
        let (readout, _) =
            ui.allocate_exact_size(Vec2::new(readout_w, spec.height), Sense::hover());
        paint_value(
            ui,
            readout,
            &spec.format(*value),
            spec.provenance,
            egui::Align2::RIGHT_CENTER,
        );
        if trailing(ui, spec.label, spec.provenance) {
            edit.reset = true;
        }
    });
    edit
}

// ---------------------------------------------------------------------------------------------
// Colour swatch

/// "#RRGGBB" for the hex field.
fn hex_text(colour: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", colour.r, colour.g, colour.b)
}

/// Parses what was typed in the hex field. Accepts `#abc`, `#aabbcc` (keeping `alpha`) and
/// `#aabbccdd`, with or without the `#`.
pub fn parse_hex(text: &str, alpha: u8) -> Option<Color> {
    let digits = text.trim().trim_start_matches('#');
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |range: std::ops::Range<usize>| u8::from_str_radix(&digits[range], 16).ok();
    match digits.len() {
        3 => Some(Color::rgba(
            byte(0..1)? * 17,
            byte(1..2)? * 17,
            byte(2..3)? * 17,
            alpha,
        )),
        6 => Some(Color::rgba(byte(0..2)?, byte(2..4)?, byte(4..6)?, alpha)),
        8 => Some(Color::rgba(
            byte(0..2)?,
            byte(2..4)?,
            byte(4..6)?,
            byte(6..8)?,
        )),
        _ => None,
    }
}

/// What the swatch's value became.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SwatchChange {
    Colour(Color),
    /// "None" was chosen, for an optional colour.
    None,
}

/// The result of [`swatch`].
pub struct SwatchResponse {
    pub edit: Edit,
    pub change: Option<SwatchChange>,
}

/// A colour swatch that opens a popover with the picker, a hex field and opacity. With
/// `optional`, the popover also offers "None" and `colour` may be `None`. A picker drag is one
/// gesture, finished when the pointer is released.
pub fn swatch(
    ui: &mut egui::Ui,
    label: &str,
    colour: &mut Option<Color>,
    optional: bool,
    provenance: Provenance,
) -> SwatchResponse {
    let palette = theme::palette(ui.ctx());
    let id = ui.make_persistent_id(("swatch", label));
    let mut edit = Edit::default();
    let enabled = ui.is_enabled();
    let response = ui
        .horizontal(|ui| {
            let (label_rect, _) = ui.allocate_exact_size(
                Vec2::new(label_width(ui, label, provenance), metrics::FIELD[0]),
                Sense::hover(),
            );
            paint_label(ui, label_rect, label, provenance);
            let (rect, response) = ui.allocate_exact_size(
                Vec2::new(44.0, metrics::FIELD[0]),
                if enabled {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::ColorButton, enabled, label)
            });
            let inner = rect.shrink(2.0);
            match *colour {
                Some(c) => {
                    let c32 = Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a);
                    egui::color_picker::show_color_at(ui.painter(), c32, inner);
                }
                None => {
                    let stroke = Stroke::new(1.0, palette.border_strong);
                    ui.painter()
                        .rect_stroke(inner, theme::RADIUS_CHIP, stroke, StrokeKind::Inside);
                    ui.painter().line_segment(
                        [inner.left_bottom(), inner.right_top()],
                        Stroke::new(1.0, palette.danger),
                    );
                }
            }
            let outline = if response.hovered() {
                palette.border_strong
            } else {
                palette.border_subtle
            };
            ui.painter().rect_stroke(
                rect,
                theme::RADIUS_CHIP,
                Stroke::new(1.0, outline),
                StrokeKind::Inside,
            );
            ui.label(
                egui::RichText::new(colour.map_or("None".to_owned(), hex_text))
                    .size(theme::CAPTION)
                    .color(palette.text_secondary),
            );
            if trailing(ui, label, provenance) {
                edit.reset = true;
            }
            response
        })
        .inner;

    let mut pending: Option<SwatchChange> = None;
    if enabled {
        egui::Popup::from_toggle_button_response(&response)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_min_width(220.0);
                let base = colour.unwrap_or(Color::rgb(128, 128, 128));
                let mut c32 = Color32::from_rgba_unmultiplied(base.r, base.g, base.b, base.a);
                if egui::color_picker::color_picker_color32(
                    ui,
                    &mut c32,
                    egui::color_picker::Alpha::OnlyBlend,
                ) {
                    let [r, g, b, a] = c32.to_srgba_unmultiplied();
                    pending = Some(SwatchChange::Colour(Color::rgba(r, g, b, a)));
                }
                ui.horizontal(|ui| {
                    ui.label("Hex");
                    let hex_id = id.with("hex");
                    let mut text: String = ui
                        .data(|d| d.get_temp(hex_id))
                        .unwrap_or_else(|| hex_text(base));
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .desired_width(80.0)
                            .font(egui::TextStyle::Monospace),
                    );
                    if field.changed()
                        && let Some(c) = parse_hex(&text, base.a)
                    {
                        pending = Some(SwatchChange::Colour(c));
                    }
                    if field.has_focus() {
                        ui.data_mut(|d| d.insert_temp(hex_id, text));
                    } else {
                        ui.data_mut(|d| d.remove::<String>(hex_id));
                    }
                });
                let mut opacity = f64::from(base.a) / 255.0 * 100.0;
                let spec = ScrubSpec::new("Opacity").unit("%").range(0.0, 100.0);
                if slider(ui, &mut opacity, &spec).changed {
                    let alpha = (opacity / 100.0 * 255.0).round() as u8;
                    pending = Some(SwatchChange::Colour(Color::rgba(
                        base.r, base.g, base.b, alpha,
                    )));
                }
                if optional && ui.button("None").clicked() {
                    pending = Some(SwatchChange::None);
                }
            });
    }

    let editing_id = id.with("editing");
    let editing = ui.data(|d| d.get_temp::<bool>(editing_id).unwrap_or(false));
    if let Some(next) = pending {
        edit.began = !editing;
        edit.changed = true;
        ui.data_mut(|d| d.insert_temp(editing_id, true));
        *colour = match next {
            SwatchChange::Colour(c) => Some(c),
            SwatchChange::None => None,
        };
    }
    // A picker edit ends when the pointer is released.
    if (editing || pending.is_some()) && !ui.input(|i| i.pointer.any_down()) {
        edit.finished = true;
        ui.data_mut(|d| d.insert_temp(editing_id, false));
    }
    SwatchResponse {
        edit,
        change: pending,
    }
}

// ---------------------------------------------------------------------------------------------
// Segmented control and checkbox row

/// A row of mutually exclusive options (Paint / Erase, Land / Water / Any). Returns whether
/// the selection changed.
pub fn segmented(ui: &mut egui::Ui, options: &[&str], selected: &mut usize) -> bool {
    let palette = theme::palette(ui.ctx());
    let enabled = ui.is_enabled();
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let width = (ui.available_width() / options.len() as f32).max(48.0);
        for (i, option) in options.iter().enumerate() {
            let (rect, response) = ui.allocate_exact_size(
                Vec2::new(width, metrics::FIELD[0]),
                if enabled {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            let is_selected = *selected == i;
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::RadioButton,
                    enabled,
                    is_selected,
                    *option,
                )
            });
            let (fill, text) = if !enabled {
                (palette.bg_field, palette.text_tertiary)
            } else if is_selected {
                (palette.bg_raised, palette.text_primary)
            } else if response.hovered() {
                (palette.hover_fill, palette.text_primary)
            } else {
                (palette.bg_field, palette.text_secondary)
            };
            let outline = if is_selected && enabled {
                palette.gold
            } else {
                palette.border_subtle
            };
            ui.painter().rect(
                rect,
                theme::RADIUS_FIELD,
                fill,
                Stroke::new(1.0, outline),
                StrokeKind::Inside,
            );
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                *option,
                egui::FontId::proportional(theme::BODY),
                text,
            );
            if response.clicked() && !is_selected {
                *selected = i;
                changed = true;
            }
        }
    });
    changed
}

/// A label with the provenance dot, a control after it (a dropdown, say) and the trailing
/// glyph or reset arrow. Returns whether reset was clicked.
pub fn field_row(
    ui: &mut egui::Ui,
    label: &str,
    provenance: Provenance,
    body: impl FnOnce(&mut egui::Ui),
) -> bool {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(label_width(ui, label, provenance), metrics::FIELD[0]),
            Sense::hover(),
        );
        paint_label(ui, rect, label, provenance);
        ui.scope(|ui| {
            let room = (ui.available_width() - gap_before_trailing(ui, provenance)).max(40.0);
            ui.set_max_width(room);
            body(ui);
        });
        trailing(ui, label, provenance)
    })
    .inner
}

/// A checkbox with its label, at the field height. Returns whether it changed.
pub fn checkbox_row(ui: &mut egui::Ui, label: &str, checked: &mut bool) -> bool {
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = metrics::FIELD[0];
        ui.checkbox(checked, label).changed()
    })
    .inner
}

// ---------------------------------------------------------------------------------------------
// Gallery

/// Every widget in every state, to compare with `Components.html`. The gallery test renders it.
pub struct Gallery {
    values: [f64; 4],
    colour: Option<Color>,
    segment: usize,
    check: bool,
}

impl Default for Gallery {
    fn default() -> Self {
        Gallery {
            values: [2716.0, 50.0, 12.0, 30.0],
            colour: Some(Color::rgb(200, 160, 60)),
            segment: 0,
            check: false,
        }
    }
}

impl Gallery {
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let header = SectionHeader {
            kind: "gallery",
            title: "Gallery section",
            badge: Some("1 override"),
            action: Some((Icon::Add, "Add gallery item")),
        };
        section(ui, header, |ui| {
            let values = &mut self.values;
            scrub_field(
                ui,
                &mut values[0],
                &ScrubSpec::new("Width").unit("px").range(0.0, 10000.0),
            );
            scrub_field(
                ui,
                &mut values[1],
                &ScrubSpec::new("Tall field")
                    .unit("%")
                    .range(0.0, 100.0)
                    .tall(),
            );
            scrub_field(
                ui,
                &mut values[2],
                &ScrubSpec::new("Inherited angle")
                    .unit("°")
                    .provenance(Provenance::Inherited),
            );
            scrub_field(
                ui,
                &mut values[3],
                &ScrubSpec::new("Overridden size")
                    .unit("pt")
                    .provenance(Provenance::Overridden),
            );
            ui.add_enabled_ui(false, |ui| {
                let mut v = 5.0;
                scrub_field(ui, &mut v, &ScrubSpec::new("Disabled field").unit("px"));
                let mut s = 40.0;
                let spec = ScrubSpec::new("Disabled slider")
                    .unit("%")
                    .range(0.0, 100.0);
                slider(ui, &mut s, &spec);
            });
            let mut opacity = values[1];
            let spec = ScrubSpec::new("Opacity").unit("%").range(0.0, 100.0);
            slider(ui, &mut opacity, &spec);
            swatch(ui, "Fill", &mut self.colour, true, Provenance::Plain);
            segmented(ui, &["Paint", "Erase"], &mut self.segment);
            checkbox_row(ui, "Snap to grid", &mut self.check);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_evaluates_sums_and_products() {
        assert_eq!(eval_expression("2716+88", 0.0), Some(2804.0));
        assert_eq!(eval_expression("2 + 3 * 4", 0.0), Some(14.0));
        assert_eq!(eval_expression("(2+3)*4", 0.0), Some(20.0));
        assert_eq!(eval_expression("-5", 0.0), Some(-5.0));
        assert_eq!(eval_expression("10--2", 0.0), Some(12.0));
        assert_eq!(eval_expression("1.5", 0.0), Some(1.5));
        assert_eq!(eval_expression("1e3", 0.0), Some(1000.0));
    }

    #[test]
    fn leading_operators_apply_to_the_current_value() {
        assert_eq!(eval_expression("*2", 21.0), Some(42.0));
        assert_eq!(eval_expression("/3", 90.0), Some(30.0));
        assert_eq!(eval_expression("+8", 2.0), Some(10.0));
        assert_eq!(eval_expression("*2+1", 5.0), Some(11.0));
        assert_eq!(eval_expression("*2", -4.0), Some(-8.0));
    }

    #[test]
    fn bad_input_is_rejected() {
        for bad in [
            "", "  ", "abc", "1+", "*", "(1+2", "1+2)", "1/0", "/0", "1..2", "2,3", "--", "1e999",
        ] {
            assert_eq!(eval_expression(bad, 4.0), None, "{bad:?}");
        }
    }

    #[test]
    fn scrub_applies_the_modifiers() {
        let any = Range::ANY;
        assert_eq!(scrub_value(10.0, 5.0, 1.0, Speed::Normal, any), 15.0);
        assert_eq!(scrub_value(10.0, 5.0, 1.0, Speed::Coarse, any), 60.0);
        assert!((scrub_value(10.0, 5.0, 1.0, Speed::Fine, any) - 10.5).abs() < 1e-9);
        assert_eq!(scrub_value(10.0, -5.0, 2.0, Speed::Normal, any), 0.0);
    }

    #[test]
    fn scrub_clamps_to_the_range() {
        let range = Range::new(0.0, 100.0);
        assert_eq!(scrub_value(90.0, 50.0, 1.0, Speed::Normal, range), 100.0);
        assert_eq!(scrub_value(10.0, -50.0, 1.0, Speed::Coarse, range), 0.0);
        assert_eq!(range.clamp(-1.0), 0.0);
        assert_eq!(range.clamp(101.0), 100.0);
    }

    #[test]
    fn modifiers_choose_the_speed() {
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        let alt = egui::Modifiers {
            alt: true,
            ..Default::default()
        };
        assert_eq!(Speed::from_modifiers(shift), Speed::Coarse);
        assert_eq!(Speed::from_modifiers(alt), Speed::Fine);
        assert_eq!(Speed::from_modifiers(egui::Modifiers::NONE), Speed::Normal);
    }

    #[test]
    fn hex_field_parses_short_and_long_forms() {
        assert_eq!(parse_hex("#fff", 255), Some(Color::rgb(255, 255, 255)));
        assert_eq!(parse_hex("c8a03c", 10), Some(Color::rgba(200, 160, 60, 10)));
        assert_eq!(
            parse_hex("#c8a03c80", 255),
            Some(Color::rgba(200, 160, 60, 128))
        );
        assert_eq!(parse_hex("#12", 255), None);
        assert_eq!(parse_hex("#ggg", 255), None);
    }
}
