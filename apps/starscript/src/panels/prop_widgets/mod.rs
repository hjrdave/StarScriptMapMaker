//! The building blocks of the Properties panel (#168): section headers, numeric scrub fields,
//! sliders, colour swatches, segmented controls and the inherited / overridden states.
//!
//! The widgets never touch the Editor. Each returns an [`Edit`] saying whether a drag or typed
//! entry began, changed the value, or finished, and [`LiveEdit`] turns that into one gesture,
//! so one scrub is one Undo step. A spec can be [`mixed`](ScrubSpec::mixed), for a setting that
//! differs across a multi-selection: the box says "Mixed" and an edit sets every item.

mod color;
mod expr;
mod number;
mod swatch;

use eframe::egui::{self, Color32, Sense, Stroke, StrokeKind, Vec2};
use ssmap_core::{Color, Command};

use self::color::hex_text;
pub use self::color::parse_hex;
use self::expr::eval_expression;
pub use self::number::{Handle, MIXED, ScrubSpec, number_field, scrub_field, slider};
pub use self::swatch::{SwatchChange, SwatchResponse, swatch, swatch_unlabelled};
use crate::editor::{Editor, GestureId};
use crate::icons::{self, Icon};
use crate::theme::{self, metrics};

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
    /// The gesture this edit opened; `None` when it opened none, or joins someone else's.
    owns_gesture: Option<GestureId>,
}

impl Drop for LiveEdit {
    fn drop(&mut self) {
        // The Editor goes with the panel state, taking the open gesture with it.
        if let Some(id) = self.owns_gesture.take() {
            std::mem::forget(id);
        }
    }
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
        self.apply_all(editor, edit, label, || vec![command()]);
    }

    /// [`Self::apply`] for an edit that is several Commands, one per selected item: all of them
    /// are one gesture, so one Undo step puts every item back.
    pub fn apply_all(
        &mut self,
        editor: &mut Editor,
        edit: Edit,
        label: &str,
        commands: impl FnOnce() -> Vec<Command>,
    ) {
        if (edit.began || edit.changed) && self.owns_gesture.is_none() && !editor.gesture_open() {
            let id = editor.begin_gesture(label);
            if editor.is_open(&id) {
                self.owns_gesture = Some(id);
            } else {
                editor.cancel_gesture(id);
            }
        }
        // Escape or undo closed the gesture: a later change is a step of its own.
        if let Some(id) = self.owns_gesture.take_if(|id| !editor.is_open(id)) {
            editor.end_gesture(id);
        }
        if edit.changed {
            for (i, command) in commands().into_iter().enumerate() {
                match &self.owns_gesture {
                    Some(id) if i == 0 => {
                        editor.gesture_replace(id, command);
                    }
                    Some(id) => {
                        editor.gesture(id, command);
                    }
                    // Joins a gesture someone else opened, or is a step of its own.
                    None => {
                        let _ = editor.run(command);
                    }
                }
            }
        }
        if edit.finished
            && let Some(id) = self.owns_gesture.take()
        {
            editor.end_gesture(id);
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
/// The label column: labels line up across rows and sections.
const LABEL_MIN: f32 = 80.0;

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
    // The action takes one id when it is there; the fields below must keep theirs when it
    // comes or goes in the middle of a drag.
    if header.action.is_none() {
        ui.skip_ahead_auto_ids(1);
    }
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
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(label_width(ui, label, provenance), metrics::FIELD[0]),
            Sense::hover(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), label)
        });
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
// Two-column grid, icon toggles and chips

/// Two fields side by side, each half the row (X / Y, Scale / Rotation).
pub fn pair<A, B>(
    ui: &mut egui::Ui,
    left: impl FnOnce(&mut egui::Ui) -> A,
    right: impl FnOnce(&mut egui::Ui) -> B,
) -> (A, B) {
    ui.columns(2, |columns| {
        let (first, second) = columns.split_at_mut(1);
        (left(&mut first[0]), right(&mut second[0]))
    })
}

/// The side of an icon toggle or icon action: 26 × 24.
const ICON_BUTTON: Vec2 = Vec2::new(26.0, 24.0);

/// An outlined icon button that stays pressed while `on` (Flip horizontal). Its accessible
/// name is `label` and it reads as checked or not. Returns whether it was clicked.
pub fn icon_toggle(ui: &mut egui::Ui, icon: Icon, label: &str, on: bool) -> bool {
    outlined_icon(ui, icon, label, Some(on), label).clicked()
}

/// An outlined icon button for a one-off action next to icon toggles (Flip vertical).
pub fn icon_action(ui: &mut egui::Ui, icon: Icon, label: &str) -> bool {
    outlined_icon(ui, icon, label, None, label).clicked()
}

/// [`icon_action`] that can be disabled and says what it does on hover. Returns whether it
/// was clicked.
pub fn icon_action_enabled(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    enabled: bool,
    hint: &str,
) -> bool {
    ui.add_enabled_ui(enabled, |ui| outlined_icon(ui, icon, label, None, hint))
        .inner
        .clicked()
}

fn outlined_icon(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    on: Option<bool>,
    hint: &str,
) -> egui::Response {
    let palette = theme::palette(ui.ctx());
    let enabled = ui.is_enabled();
    let (rect, response) = ui.allocate_exact_size(ICON_BUTTON, Sense::click());
    response.widget_info(|| match on {
        Some(on) => egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, on, label),
        None => egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label),
    });
    let pressed = on == Some(true);
    let (fill, stroke, glyph) = if !enabled {
        (
            palette.bg_field,
            palette.border_subtle,
            palette.text_tertiary,
        )
    } else if pressed {
        (palette.gold_subtle, palette.gold, palette.gold_text)
    } else if response.hovered() {
        (
            palette.hover_fill,
            palette.border_strong,
            palette.text_primary,
        )
    } else {
        (
            palette.bg_raised,
            palette.border_strong,
            palette.text_secondary,
        )
    };
    ui.painter().rect(
        rect,
        theme::RADIUS_FIELD,
        fill,
        Stroke::new(1.0, stroke),
        StrokeKind::Inside,
    );
    icons::paint(ui, rect, icon, 14.0, glyph);
    response.on_hover_text(hint)
}

/// A rounded chip that is on or off ("Randomise: Flip · Variant"). Flips `on` and returns
/// whether it was clicked.
pub fn chip_toggle(ui: &mut egui::Ui, label: &str, on: &mut bool) -> bool {
    let palette = theme::palette(ui.ctx());
    let enabled = ui.is_enabled();
    let font = egui::FontId::proportional(theme::BODY);
    let text_w = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), Color32::WHITE)
        .size()
        .x;
    let height = metrics::FIELD[0];
    let (rect, response) = ui.allocate_exact_size(Vec2::new(text_w + 20.0, height), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, *on, label)
    });
    let (fill, stroke, text) = if !enabled {
        (
            Color32::TRANSPARENT,
            palette.border_subtle,
            palette.text_tertiary,
        )
    } else if *on {
        (palette.gold_subtle, palette.gold, palette.gold_text)
    } else if response.hovered() {
        (
            palette.hover_fill,
            palette.border_strong,
            palette.text_primary,
        )
    } else {
        (
            Color32::TRANSPARENT,
            palette.border_strong,
            palette.text_secondary,
        )
    };
    ui.painter().rect(
        rect,
        height / 2.0,
        fill,
        Stroke::new(1.0, stroke),
        StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        font,
        text,
    );
    let clicked = enabled && response.clicked();
    if clicked {
        *on = !*on;
    }
    clicked
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
            let (x, scale) = values.split_at_mut(1);
            pair(
                ui,
                |ui| {
                    scrub_field(
                        ui,
                        &mut x[0],
                        &ScrubSpec::new("X").handle(Handle::Text("X")),
                    )
                },
                |ui| {
                    let spec = ScrubSpec::new("Scale")
                        .unit("%")
                        .handle(Handle::Text("S"))
                        .mixed(true);
                    scrub_field(ui, &mut scale[0], &spec)
                },
            );
            ui.horizontal(|ui| {
                icon_toggle(ui, Icon::FlipHorizontal, "Flip horizontal", self.check);
                icon_action(ui, Icon::FlipVertical, "Flip vertical");
                chip_toggle(ui, "Variant", &mut self.check);
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn icon_toggles_and_chips_read_as_checked_and_a_compact_field_keeps_its_name() {
        use egui::accesskit::Toggled;
        use egui_kittest::Harness;
        use egui_kittest::kittest::{NodeT as _, Queryable, by};

        let mut harness = Harness::new_ui(|ui| {
            icon_toggle(ui, Icon::FlipHorizontal, "Flip horizontal", true);
            icon_action(ui, Icon::FlipVertical, "Flip vertical");
            chip_toggle(ui, "Variant", &mut false);
            let mut scale = 150.0;
            let spec = ScrubSpec::new("Scale").unit("%").handle(Handle::Text("S"));
            scrub_field(ui, &mut scale, &spec);
        });
        harness.run();
        let toggled = |label| harness.get_by_label(label).accesskit_node().toggled();
        assert_eq!(toggled("Flip horizontal"), Some(Toggled::True));
        assert_eq!(toggled("Flip vertical"), None);
        assert_eq!(toggled("Variant"), Some(Toggled::False));
        // The handle inside the box keeps the field's name; the value carries the unit.
        harness.get_by_label("Scale");
        harness.get(by().value("150%"));
    }

    #[test]
    fn a_live_edit_makes_one_undo_step() {
        use ssmap_core::{Canvas, DistanceUnit, Map, Scale};

        let mut editor = Editor::new(Map::new(Canvas::new(
            100,
            100,
            Scale::new(1.0, DistanceUnit::Miles),
        )));
        let layer = editor.new_layer().expect("a Layer is made");
        let mut live = LiveEdit::default();
        let scrub = |began, changed, finished| Edit {
            began,
            changed,
            finished,
            reset: false,
        };
        for (i, opacity) in [0.8, 0.5, 0.2].into_iter().enumerate() {
            live.apply(
                &mut editor,
                scrub(i == 0, true, false),
                "Change Opacity",
                || Command::SetOpacity { id: layer, opacity },
            );
            assert!(editor.gesture_open());
        }
        live.apply(
            &mut editor,
            scrub(false, false, true),
            "Change Opacity",
            || unreachable!("nothing changed"),
        );
        assert!(!editor.gesture_open());
        assert_eq!(editor.undo_label(), Some("Change Opacity"));
        editor.undo();
        assert_ne!(editor.undo_label(), Some("Change Opacity"));
    }
}
