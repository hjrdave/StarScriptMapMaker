//! The colour swatch and its picker.

use super::*;

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
    swatch_row(ui, label, true, colour, optional, provenance)
}

/// [`swatch`] under the row that chose it (Colour: Tint): the label column stays empty and
/// `label` is only the accessible name.
pub fn swatch_unlabelled(
    ui: &mut egui::Ui,
    label: &str,
    colour: &mut Option<Color>,
    optional: bool,
) -> SwatchResponse {
    swatch_row(ui, label, false, colour, optional, Provenance::Plain)
}

fn swatch_row(
    ui: &mut egui::Ui,
    label: &str,
    show_label: bool,
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
            let label_w = if show_label {
                label_width(ui, label, provenance)
            } else {
                LABEL_MIN
            };
            let (label_rect, _) =
                ui.allocate_exact_size(Vec2::new(label_w, metrics::FIELD[0]), Sense::hover());
            if show_label {
                paint_label(ui, label_rect, label, provenance);
            }
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
