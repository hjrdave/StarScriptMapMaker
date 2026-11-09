//! The colour picker popover: Before and Now, a colour field, hue and opacity bars, and labelled
//! hex, R, G, B and opacity inputs, all in one 252 px column. Values are always 0 to 255.

use egui::ecolor::Hsva;
use egui::{Id, Pos2, Rect, Shape};

use super::*;

/// The width of the popover's column.
const WIDTH: f32 = 252.0;
const FIELD_HEIGHT: f32 = 150.0;
const BAR_HEIGHT: f32 = 14.0;
const INPUT_WIDTH: f32 = 46.0;
const HEX_WIDTH: f32 = 84.0;
const GAP: f32 = 6.0;

/// The colour a new optional colour starts from when it was "None".
const START: Color = Color::rgb(128, 128, 128);

/// What typing in the opacity box means: a percentage from 0 to 100, with or without "%".
pub(super) fn parse_percent(text: &str) -> Option<u8> {
    let value: f32 = text.trim().trim_end_matches('%').trim().parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    Some((value.clamp(0.0, 100.0) / 100.0 * 255.0).round() as u8)
}

/// What typing in an R, G or B box means: a whole number, kept to 0 to 255.
pub(super) fn parse_channel(text: &str) -> Option<u8> {
    let value: u32 = text.trim().parse().ok()?;
    Some(value.min(255) as u8)
}

/// The percentage the opacity box shows for an alpha byte.
pub(super) fn percent_text(alpha: u8) -> String {
    format!("{}%", (f32::from(alpha) / 255.0 * 100.0).round())
}

/// The picker. `colour` is the current value, `before` what it was when the popover opened.
/// Returns what the value became this frame, if the user changed it.
pub(super) fn picker(
    ui: &mut egui::Ui,
    id: Id,
    colour: Option<Color>,
    before: Option<Color>,
    optional: bool,
) -> Option<SwatchChange> {
    ui.set_width(WIDTH);
    ui.spacing_mut().item_spacing = Vec2::splat(GAP);
    let base = colour.unwrap_or(START);
    let mut pending = None;

    before_and_now(ui, before, colour);

    let mut hsva = current_hsva(ui, id, base);
    let mut moved = colour_field(ui, id.with("field"), &mut hsva);
    moved |= hue_bar(ui, id.with("hue"), &mut hsva);
    if moved {
        ui.data_mut(|d| d.insert_temp(id.with("hsv"), hsva));
        let [r, g, b] = hsva.to_srgb();
        pending = Some(SwatchChange::Colour(Color::rgba(r, g, b, base.a)));
    }
    let shown = pending.map_or(base, |change| match change {
        SwatchChange::Colour(c) => c,
        SwatchChange::None => base,
    });
    if let Some(alpha) = opacity_row(ui, id, shown) {
        pending = Some(SwatchChange::Colour(Color::rgba(
            shown.r, shown.g, shown.b, alpha,
        )));
    }
    if let Some(c) = value_inputs(ui, id, shown) {
        pending = Some(SwatchChange::Colour(c));
    }
    if optional && ui.button("None").clicked() {
        pending = Some(SwatchChange::None);
    }
    pending
}

/// The colour in the field's hue state: what was last dragged, unless the colour has since been
/// set another way, so the hue of a grey survives.
fn current_hsva(ui: &egui::Ui, id: Id, base: Color) -> Hsva {
    let stored: Option<Hsva> = ui.data(|d| d.get_temp(id.with("hsv")));
    stored
        .filter(|h| h.to_srgb() == [base.r, base.g, base.b])
        .unwrap_or_else(|| Hsva::from_srgb([base.r, base.g, base.b]))
}

fn color32(colour: Color) -> Color32 {
    Color32::from_rgba_unmultiplied(colour.r, colour.g, colour.b, colour.a)
}

/// Before and Now side by side, each half the column.
fn before_and_now(ui: &mut egui::Ui, before: Option<Color>, now: Option<Color>) {
    let palette = theme::palette(ui.ctx());
    ui.horizontal(|ui| {
        let half = (WIDTH - GAP) / 2.0;
        for (name, colour) in [("Before", before), ("Now", now)] {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(
                    egui::RichText::new(name)
                        .size(theme::CAPTION)
                        .color(palette.text_secondary),
                );
                let (rect, _) = ui.allocate_exact_size(Vec2::new(half, 28.0), Sense::hover());
                paint_chip(ui, rect, colour, palette);
            });
        }
    });
}

/// A colour over a chequerboard, or an empty box with a slash for "None".
pub(super) fn paint_chip(
    ui: &egui::Ui,
    rect: Rect,
    colour: Option<Color>,
    palette: &theme::Palette,
) {
    let painter = ui.painter();
    match colour {
        Some(c) => {
            checker(painter, rect);
            painter.rect_filled(rect, theme::RADIUS_CHIP, color32(c));
        }
        None => {
            painter.rect_filled(rect, theme::RADIUS_CHIP, palette.bg_field);
            painter.line_segment(
                [rect.left_bottom(), rect.right_top()],
                Stroke::new(1.0, palette.danger),
            );
        }
    }
    painter.rect_stroke(
        rect,
        theme::RADIUS_CHIP,
        Stroke::new(1.0, palette.border_strong),
        StrokeKind::Inside,
    );
}

/// A grey chequerboard to show translucency against.
fn checker(painter: &egui::Painter, rect: Rect) {
    let painter = painter.with_clip_rect(rect);
    let cell = 6.0;
    let mut y = 0;
    while rect.top() + y as f32 * cell < rect.bottom() {
        let mut x = 0;
        while rect.left() + x as f32 * cell < rect.right() {
            let shade = if (x + y) % 2 == 0 {
                Color32::from_gray(200)
            } else {
                Color32::from_gray(150)
            };
            let origin = Pos2::new(rect.left() + x as f32 * cell, rect.top() + y as f32 * cell);
            painter.rect_filled(Rect::from_min_size(origin, Vec2::splat(cell)), 0.0, shade);
            x += 1;
        }
        y += 1;
    }
}

/// Where in `rect` (0 to 1 each way) a click or drag on it is, if it is one.
fn drag_in(ui: &egui::Ui, rect: Rect, id: Id, name: &str) -> Option<Vec2> {
    let response = ui.interact(rect, id, Sense::click_and_drag());
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, ui.is_enabled(), name));
    if !(response.dragged() || response.clicked() || response.drag_started()) {
        return None;
    }
    let at = response.interact_pointer_pos()?;
    Some(Vec2::new(
        ((at.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
        ((at.y - rect.top()) / rect.height()).clamp(0.0, 1.0),
    ))
}

/// A knob: a white ring with a dark one outside it, readable on any colour.
fn thumb(painter: &egui::Painter, at: Pos2) {
    painter.circle_stroke(at, 6.0, Stroke::new(1.0, Color32::from_black_alpha(160)));
    painter.circle_stroke(at, 5.0, Stroke::new(2.0, Color32::WHITE));
}

/// A rectangle of vertex colours: `corners` are top-left, top-right, bottom-left, bottom-right.
fn gradient_quad(painter: &egui::Painter, rect: Rect, corners: [Color32; 4]) {
    let mut mesh = egui::Mesh::default();
    let points = [
        rect.left_top(),
        rect.right_top(),
        rect.left_bottom(),
        rect.right_bottom(),
    ];
    for (point, colour) in points.into_iter().zip(corners) {
        mesh.colored_vertex(point, colour);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 2, 3);
    painter.add(Shape::mesh(mesh));
}

/// The saturation and value field of the current hue. Returns whether it was dragged.
fn colour_field(ui: &mut egui::Ui, id: Id, hsva: &mut Hsva) -> bool {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(WIDTH, FIELD_HEIGHT), Sense::hover());
    let hue: Color32 = Hsva::new(hsva.h, 1.0, 1.0, 1.0).into();
    gradient_quad(
        ui.painter(),
        rect,
        [Color32::WHITE, hue, Color32::BLACK, Color32::BLACK],
    );
    let moved = drag_in(ui, rect, id, "Colour field").inspect(|at| {
        hsva.s = at.x;
        hsva.v = 1.0 - at.y;
    });
    let knob = Pos2::new(
        rect.left() + hsva.s * rect.width(),
        rect.top() + (1.0 - hsva.v) * rect.height(),
    );
    thumb(ui.painter(), knob);
    moved.is_some()
}

/// The hue bar. Returns whether it was dragged.
fn hue_bar(ui: &mut egui::Ui, id: Id, hsva: &mut Hsva) -> bool {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(WIDTH, BAR_HEIGHT), Sense::hover());
    let steps = 6;
    for i in 0..steps {
        let step = |n: i32| -> Color32 { Hsva::new(n as f32 / steps as f32, 1.0, 1.0, 1.0).into() };
        let width = rect.width() / steps as f32;
        let part = Rect::from_min_size(
            Pos2::new(rect.left() + width * i as f32, rect.top()),
            Vec2::new(width, rect.height()),
        );
        gradient_quad(
            ui.painter(),
            part,
            [step(i), step(i + 1), step(i), step(i + 1)],
        );
    }
    let moved = drag_in(ui, rect, id, "Hue").inspect(|at| hsva.h = at.x);
    let knob = Pos2::new(rect.left() + hsva.h * rect.width(), rect.center().y);
    thumb(ui.painter(), knob);
    moved.is_some()
}

/// The opacity bar with its percentage box. Returns the new alpha, if it changed.
fn opacity_row(ui: &mut egui::Ui, id: Id, colour: Color) -> Option<u8> {
    let mut alpha = None;
    ui.horizontal(|ui| {
        let bar = WIDTH - GAP - INPUT_WIDTH;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(bar, BAR_HEIGHT), Sense::hover());
        checker(ui.painter(), rect);
        let clear = Color32::from_rgba_unmultiplied(colour.r, colour.g, colour.b, 0);
        let solid = Color32::from_rgb(colour.r, colour.g, colour.b);
        gradient_quad(ui.painter(), rect, [clear, solid, clear, solid]);
        if let Some(at) = drag_in(ui, rect, id.with("opacity"), "Opacity") {
            alpha = Some((at.x * 255.0).round() as u8);
        }
        let knob = Pos2::new(
            rect.left() + f32::from(colour.a) / 255.0 * rect.width(),
            rect.center().y,
        );
        thumb(ui.painter(), knob);
        if let Some(text) = input(
            ui,
            id.with("opacity text"),
            "Opacity percentage",
            percent_text(colour.a),
            INPUT_WIDTH,
        ) && let Some(a) = parse_percent(&text)
        {
            alpha = Some(a);
        }
    });
    alpha
}

/// The hex, R, G and B inputs, each with its label, in one row. Returns the colour typed.
fn value_inputs(ui: &mut egui::Ui, id: Id, colour: Color) -> Option<Color> {
    let mut typed = None;
    let palette = theme::palette(ui.ctx());
    ui.horizontal(|ui| {
        let boxes = [
            ("Hex", "Hex colour", hex_text(colour), HEX_WIDTH),
            ("R", "Red", colour.r.to_string(), INPUT_WIDTH),
            ("G", "Green", colour.g.to_string(), INPUT_WIDTH),
            ("B", "Blue", colour.b.to_string(), INPUT_WIDTH),
        ];
        for (index, (name, accessible, text, width)) in boxes.into_iter().enumerate() {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(
                    egui::RichText::new(name)
                        .size(theme::CAPTION)
                        .color(palette.text_secondary),
                );
                let Some(entered) = input(ui, id.with(("input", name)), accessible, text, width)
                else {
                    return;
                };
                let channel = |value: u8, at: usize| {
                    let mut c = [colour.r, colour.g, colour.b];
                    c[at] = value;
                    Color::rgba(c[0], c[1], c[2], colour.a)
                };
                typed = match index {
                    0 => parse_hex(&entered, colour.a),
                    at => parse_channel(&entered).map(|v| channel(v, at - 1)),
                }
                .or(typed);
            });
        }
    });
    typed
}

/// A one-line monospace input. While it has focus it keeps what is typed, so a half-typed value
/// is not rewritten under the cursor. Returns the text when it changed.
fn input(ui: &mut egui::Ui, id: Id, name: &str, current: String, width: f32) -> Option<String> {
    let mut text: String = ui.data(|d| d.get_temp(id)).unwrap_or(current);
    let field = ui.add(
        egui::TextEdit::singleline(&mut text)
            .id(id)
            .desired_width(width)
            .font(egui::TextStyle::Monospace),
    );
    field.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, name));
    let changed = field.changed();
    if field.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text.clone()));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    changed.then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opacity_is_a_percentage_with_or_without_the_sign() {
        assert_eq!(parse_percent("100"), Some(255));
        assert_eq!(parse_percent(" 50 % "), Some(128));
        assert_eq!(parse_percent("0%"), Some(0));
        assert_eq!(parse_percent("250"), Some(255));
        assert_eq!(parse_percent("abc"), None);
        assert_eq!(percent_text(255), "100%");
        assert_eq!(percent_text(0), "0%");
    }

    #[test]
    fn channels_are_whole_numbers_up_to_255() {
        assert_eq!(parse_channel("200"), Some(200));
        assert_eq!(parse_channel("999"), Some(255));
        assert_eq!(parse_channel("-1"), None);
        assert_eq!(parse_channel(""), None);
    }
}
