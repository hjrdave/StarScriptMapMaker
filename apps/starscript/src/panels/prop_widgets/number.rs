//! Numeric scrub fields, number boxes and sliders.

use super::*;

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
    /// Where a slider's track ends when that is short of `range.max`; a typed value may
    /// still go up to `range.max`.
    pub slider_max: Option<f64>,
    /// What double-clicking a slider resets to.
    pub default: Option<f64>,
    /// Whether the slider track is logarithmic, so small values are easy to set on a wide range.
    pub log: bool,
    /// The setting differs across the selection: the box says "Mixed" and the thumb is neutral.
    /// Any edit sets every item, even to the value shown.
    pub mixed: bool,
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
            slider_max: None,
            default: None,
            log: false,
            mixed: false,
        }
    }

    /// Marks the setting as differing across the selection.
    pub fn mixed(mut self, mixed: bool) -> Self {
        self.mixed = mixed;
        self
    }

    /// A logarithmic slider track, for settings that span a wide range.
    pub fn logarithmic(mut self) -> Self {
        self.log = true;
        self
    }

    /// The slider track covers `range.min..=max`; typing may go beyond, up to `range.max`.
    pub fn slider_max(mut self, max: f64) -> Self {
        self.slider_max = Some(max);
        self
    }

    pub fn default_value(mut self, default: f64) -> Self {
        self.default = Some(default);
        self
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
        if self.mixed {
            return MIXED.to_owned();
        }
        format!("{value:.*}{}", self.decimals, self.unit)
    }

    /// What typing starts from: nothing when mixed.
    fn typing_start(&self, value: f64) -> String {
        if self.mixed {
            String::new()
        } else {
            format!("{value:.*}", self.decimals)
        }
    }
}

/// What the box of a setting that differs across the selection says.
pub const MIXED: &str = "Mixed";

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

/// Where a logarithmic track bends, as a fraction of the span: the first part of the track is
/// spent on the smallest values without letting `ln(0)` in.
const LOG_KNEE: f64 = 0.01;

/// Where `value` sits along a track from `min` to `max`, 0..=1.
fn slider_t(min: f64, max: f64, value: f64, log: bool) -> f64 {
    let linear = ((value - min) / (max - min)).clamp(0.0, 1.0);
    if log {
        (linear / LOG_KNEE).ln_1p() / (1.0 / LOG_KNEE).ln_1p()
    } else {
        linear
    }
}

/// The value a slider holds at `t` (0..=1) along its track.
fn slider_value(min: f64, max: f64, t: f64, decimals: usize, log: bool) -> f64 {
    let t = t.clamp(0.0, 1.0);
    let linear = if log {
        LOG_KNEE * ((t * (1.0 / LOG_KNEE).ln_1p()).exp() - 1.0)
    } else {
        t
    };
    round_to(min + linear.clamp(0.0, 1.0) * (max - min), decimals)
}

/// Width of the number box, including its up / down arrows.
const BOX_WIDTH: f32 = 76.0;
/// Width of the up / down arrows at the right of the number box.
const STEPPER_WIDTH: f32 = 16.0;

/// The number box with up / down arrows: shows the value, types an exact one (Enter or blur
/// commits, Esc cancels, clamped to the spec's hard range) and nudges by the spec's step when an
/// arrow is clicked (⇧ ×10, ⌥ ×0.1). Returns the new value when it changed.
fn number_box(
    ui: &mut egui::Ui,
    value: f64,
    spec: &ScrubSpec<'_>,
    typing: &mut Option<String>,
    width: f32,
) -> Option<f64> {
    let palette = theme::palette(ui.ctx());
    let enabled = ui.is_enabled();
    let mut next = None;
    let (box_rect, box_response) = ui.allocate_exact_size(
        Vec2::new(width, spec.height),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    ui.painter().rect(
        box_rect,
        theme::RADIUS_FIELD,
        palette.bg_field,
        Stroke::new(1.0, palette.border_subtle),
        StrokeKind::Inside,
    );
    let split = box_rect.right() - STEPPER_WIDTH;
    let text_rect = egui::Rect::from_min_max(box_rect.min, egui::pos2(split, box_rect.max.y));
    let arrows = egui::Rect::from_min_max(egui::pos2(split, box_rect.min.y), box_rect.max);
    if let Some(text) = typing.as_mut() {
        let field = ui.put(
            text_rect,
            egui::TextEdit::singleline(text)
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::symmetric(6, 0))
                .horizontal_align(egui::Align::RIGHT)
                .font(egui::FontId::new(theme::MONO, theme::mono_family())),
        );
        if !field.has_focus() && !field.lost_focus() {
            field.request_focus();
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            *typing = None;
        } else if field.lost_focus() {
            // Enter or clicking away both commit.
            let typed = typing.take().unwrap_or_default();
            if let Some(result) = eval_expression(&typed, value) {
                let result = round_to(spec.range.clamp(result), spec.decimals.max(3));
                if result != value || spec.mixed {
                    next = Some(result);
                }
            }
        }
    } else {
        paint_value(
            ui,
            text_rect,
            &spec.format(value),
            spec.provenance,
            egui::Align2::RIGHT_CENTER,
        );
        box_response.widget_info(|| {
            egui::WidgetInfo::text_edit(enabled, spec.format(value), spec.format(value), spec.label)
        });
        if enabled && box_response.clicked() {
            *typing = Some(spec.typing_start(value));
        }
    }

    // The arrows: up on top, down below, each half the box high.
    let mid = arrows.center().y;
    let up = egui::Rect::from_min_max(arrows.min, egui::pos2(arrows.max.x, mid));
    let down = egui::Rect::from_min_max(egui::pos2(arrows.min.x, mid), arrows.max);
    for (half, dir) in [(up, 1.0_f64), (down, -1.0_f64)] {
        let response = ui.interact(
            half,
            ui.id().with(("stepper", spec.label, dir as i32)),
            if enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        let name = if dir > 0.0 {
            format!("Increase {}", spec.label)
        } else {
            format!("Decrease {}", spec.label)
        };
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &name));
        if enabled && response.clicked() {
            let speed = Speed::from_modifiers(ui.input(|i| i.modifiers));
            let stepped = round_to(
                spec.range.clamp(value + dir * spec.step * speed.factor()),
                spec.decimals.max(if speed == Speed::Fine { 2 } else { 0 }),
            );
            if stepped != value {
                next = Some(stepped);
            }
        }
        let colour = if !enabled {
            palette.text_tertiary
        } else if response.hovered() {
            palette.gold_text
        } else {
            palette.text_secondary
        };
        let c = half.center();
        let (w, h) = (3.5, 2.0);
        let points = if dir > 0.0 {
            vec![
                egui::pos2(c.x - w, c.y + h),
                egui::pos2(c.x + w, c.y + h),
                egui::pos2(c.x, c.y - h),
            ]
        } else {
            vec![
                egui::pos2(c.x - w, c.y - h),
                egui::pos2(c.x + w, c.y - h),
                egui::pos2(c.x, c.y + h),
            ]
        };
        ui.painter()
            .add(egui::Shape::convex_polygon(points, colour, Stroke::NONE));
    }
    next
}

/// A whole-number setting (like the ripple Count): a number box with up / down arrows and no
/// slider. Typing and the arrows each commit as one instant step.
pub fn number_field(ui: &mut egui::Ui, value: &mut f64, spec: &ScrubSpec<'_>) -> Edit {
    let id = ui.make_persistent_id(("number", spec.label));
    let mut typing: Option<String> = ui.data(|d| d.get_temp(id));
    let mut edit = Edit::default();
    ui.horizontal(|ui| {
        let (label_rect, _) = ui.allocate_exact_size(
            Vec2::new(label_width(ui, spec.label, spec.provenance), spec.height),
            Sense::hover(),
        );
        paint_label(ui, label_rect, spec.label, spec.provenance);
        if let Some(next) = number_box(ui, *value, spec, &mut typing, BOX_WIDTH) {
            *value = next;
            edit = Edit::instant();
        }
        if trailing(ui, spec.label, spec.provenance) {
            edit.reset = true;
        }
    });
    ui.data_mut(|d| match typing {
        Some(text) => {
            d.insert_temp(id, text);
        }
        None => {
            d.remove::<String>(id);
        }
    });
    edit
}

/// A slider with a small number box. Dragging is one gesture; clicking the track is one step.
/// Arrow keys nudge (⇧ ×10, ⌥ ×0.1) and a double-click resets to the spec's default. The box
/// types an exact value (Enter or blur commits, Esc cancels), clamped to the spec's range.
#[allow(
    clippy::too_many_lines,
    reason = "existing long function; split over time (#351)"
)]
pub fn slider(ui: &mut egui::Ui, value: &mut f64, spec: &ScrubSpec<'_>) -> Edit {
    let palette = theme::palette(ui.ctx());
    let enabled = ui.is_enabled();
    let id = ui.make_persistent_id(("slider", spec.label));
    let mut typing: Option<String> = ui.data(|d| d.get_temp(id));
    let mut edit = Edit::default();
    ui.horizontal(|ui| {
        let (label_rect, _) = ui.allocate_exact_size(
            Vec2::new(label_width(ui, spec.label, spec.provenance), spec.height),
            Sense::hover(),
        );
        paint_label(ui, label_rect, spec.label, spec.provenance);

        let readout_w = BOX_WIDTH;
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
        let min = spec.range.min;
        let max = spec
            .slider_max
            .unwrap_or(spec.range.max)
            .min(spec.range.max);
        let span = max - min;
        let usable = span.is_finite() && span > 0.0;
        if enabled && usable {
            if response.double_clicked()
                && let Some(default) = spec.default
            {
                let default = spec.range.clamp(default);
                if default != *value || spec.mixed {
                    *value = default;
                    edit = Edit::instant();
                }
            } else {
                if response.drag_started() || response.clicked() {
                    edit.began = true;
                }
                if (response.dragged() || response.clicked())
                    && let Some(pos) = response.interact_pointer_pos()
                {
                    let t = f64::from((pos.x - rect.left() - 6.0) / (rect.width() - 12.0));
                    let next = slider_value(min, max, t, spec.decimals, spec.log);
                    if next != *value || spec.mixed {
                        *value = next;
                        edit.changed = true;
                    }
                }
                if response.drag_stopped() || response.clicked() {
                    edit.finished = true;
                }
            }
            if response.has_focus() {
                let (dir, speed) = ui.input(|i| {
                    let dir = f64::from(
                        i32::from(i.key_pressed(egui::Key::ArrowRight))
                            + i32::from(i.key_pressed(egui::Key::ArrowUp))
                            - i32::from(i.key_pressed(egui::Key::ArrowLeft))
                            - i32::from(i.key_pressed(egui::Key::ArrowDown)),
                    );
                    (dir, Speed::from_modifiers(i.modifiers))
                });
                if dir != 0.0 {
                    let base = if spec.log {
                        spec.step
                    } else {
                        (span / 100.0).max(spec.step)
                    };
                    let nudge = base * speed.factor();
                    let next = round_to(
                        spec.range.clamp(*value + dir * nudge),
                        spec.decimals.max(if speed == Speed::Fine { 2 } else { 0 }),
                    );
                    if next != *value {
                        *value = next;
                        edit = Edit::instant();
                    }
                }
            }
        }
        let t = if usable {
            slider_t(min, max, *value, spec.log) as f32
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
        } else if spec.mixed && !response.dragged() {
            (
                palette.border_strong,
                palette.gold,
                palette.text_tertiary,
                6.0,
            )
        } else if response.dragged() {
            (palette.border_strong, palette.gold, palette.gold_text, 7.0)
        } else if response.hovered() || response.has_focus() {
            (palette.border_strong, palette.gold, palette.gold_text, 6.5)
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
        // A mixed setting has no place on the track: the thumb waits in the middle, unfilled.
        let x = if spec.mixed {
            (x0 + x1) / 2.0
        } else {
            x0 + t * (x1 - x0)
        };
        ui.painter().line_segment(
            [egui::pos2(x0, y), egui::pos2(x1, y)],
            Stroke::new(3.0, line),
        );
        if !spec.mixed {
            ui.painter().line_segment(
                [egui::pos2(x0, y), egui::pos2(x, y)],
                Stroke::new(3.0, fill),
            );
        }
        ui.painter().circle(
            egui::pos2(x, y),
            radius,
            thumb,
            Stroke::new(1.0, palette.bg_field),
        );
        if let Some(next) = number_box(ui, *value, spec, &mut typing, readout_w) {
            *value = next;
            edit = Edit::instant();
        }
        if trailing(ui, spec.label, spec.provenance) {
            edit.reset = true;
        }
    });
    ui.data_mut(|d| match typing {
        Some(text) => {
            d.insert_temp(id, text);
        }
        None => {
            d.remove::<String>(id);
        }
    });
    edit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slider_value_maps_and_clamps_the_track() {
        assert_eq!(slider_value(0.0, 50.0, 0.5, 1, false), 25.0);
        assert_eq!(slider_value(0.0, 50.0, 2.0, 1, false), 50.0);
        assert_eq!(slider_value(10.0, 20.0, -1.0, 0, false), 10.0);
        assert_eq!(slider_value(10.0, 20.0, 2.0, 0, true), 20.0);
    }

    #[test]
    fn logarithmic_track_favours_small_values_and_round_trips() {
        // Half way along a log track is far below half the span.
        assert!(slider_value(0.0, 100.0, 0.5, 3, true) < 10.0);
        assert_eq!(slider_value(0.0, 100.0, 0.0, 3, true), 0.0);
        assert_eq!(slider_value(0.0, 100.0, 1.0, 3, true), 100.0);
        for v in [0.0, 1.0, 5.0, 40.0, 100.0] {
            let t = slider_t(0.0, 100.0, v, true);
            assert!((slider_value(0.0, 100.0, t, 3, true) - v).abs() < 0.01);
        }
        assert_eq!(slider_t(0.0, 100.0, 50.0, false), 0.5);
        assert_eq!(slider_t(0.0, 100.0, 500.0, true), 1.0);
    }
}
