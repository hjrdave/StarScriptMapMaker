//! The New Map dialog: the Canvas size and real-world scale a new Map starts with. The Map
//! itself comes from the starter template (see [`start_map`]).

use eframe::egui;
use ssmap_core::{
    Canvas, DefaultLayers, DistanceUnit, LabelStyles, Map, MapTemplate, Scale, TemplateError,
};

use crate::{dialog, theme};

/// The smallest and largest side of a Canvas, in nominal pixels.
pub const SIDE_RANGE: std::ops::RangeInclusive<u32> = 256..=16384;

/// What the dialog starts with.
const DEFAULT_SIDES: (u32, u32) = (4096, 3072);
const DEFAULT_DISTANCE: f64 = 0.25;

/// A new Map from the starter template, with `canvas`, and the Layers new Objects go into at
/// first. If the template can't be read, the Map is blank apart from the starter Label styles,
/// and the problem is returned beside it.
pub fn start_map(canvas: Canvas) -> (Map, DefaultLayers, Option<String>) {
    start_map_from(MapTemplate::starter(), canvas)
}

/// Like [`start_map`], from any template that was loaded.
pub fn start_map_from(
    template: Result<MapTemplate, TemplateError>,
    canvas: Canvas,
) -> (Map, DefaultLayers, Option<String>) {
    match template {
        Ok(template) => (template.new_map(canvas), template.default_layers(), None),
        Err(err) => (
            Map::new(canvas).with_label_styles(LabelStyles::starter()),
            DefaultLayers::default(),
            Some(format!("Couldn’t read the starter template: {err}")),
        ),
    }
}

/// The width of a template card and of the preview above the cards, in points.
const CARD_WIDTH: f32 = 128.0;
const CARD_GAP: f32 = 8.0;
const CARDS_PER_ROW: usize = 3;
const CARD_THUMB_HEIGHT: f32 = 80.0;
const PREVIEW_HEIGHT: f32 = 170.0;

pub struct NewMapDialog {
    pub open: bool,
    width: u32,
    height: u32,
    distance: f64,
    unit: DistanceUnit,
    /// Every template that could be loaded, the starter first.
    templates: Vec<MapTemplate>,
    /// The index in `templates` of the one the Map will start from.
    selected: usize,
}

impl Default for NewMapDialog {
    fn default() -> Self {
        Self {
            open: false,
            width: DEFAULT_SIDES.0,
            height: DEFAULT_SIDES.1,
            distance: DEFAULT_DISTANCE,
            unit: DistanceUnit::Miles,
            templates: MapTemplate::load_all().0,
            selected: 0,
        }
    }
}

impl NewMapDialog {
    /// The templates on offer, the starter first.
    pub fn templates(&self) -> &[MapTemplate] {
        &self.templates
    }

    /// The name of the template the Map will start from.
    pub fn selected_name(&self) -> Option<&str> {
        self.templates.get(self.selected).map(MapTemplate::name)
    }

    /// Selects the template with the given file-name id. Returns whether there is one.
    pub fn select(&mut self, id: &str) -> bool {
        match self.templates.iter().position(|t| t.id() == id) {
            Some(index) => {
                self.selected = index;
                true
            }
            None => false,
        }
    }

    /// The template to start the Map from, or the starter's own error if none could be loaded.
    pub fn chosen_template(&self) -> Result<MapTemplate, TemplateError> {
        match self.templates.get(self.selected) {
            Some(template) => Ok(template.clone()),
            None => MapTemplate::starter(),
        }
    }

    /// Why the settings can't make a Canvas, if they can't.
    pub fn problem(&self) -> Option<String> {
        if !SIDE_RANGE.contains(&self.width) || !SIDE_RANGE.contains(&self.height) {
            return Some(format!(
                "Each side must be from {} to {} pixels.",
                SIDE_RANGE.start(),
                SIDE_RANGE.end()
            ));
        }
        if !(self.distance.is_finite() && self.distance > 0.0) {
            return Some("The scale must be greater than 0.".into());
        }
        None
    }

    /// The Canvas the settings describe, unless they have a [`problem`](Self::problem).
    pub fn canvas(&self) -> Option<Canvas> {
        self.problem().is_none().then(|| {
            Canvas::new(
                self.width,
                self.height,
                Scale::new(self.distance, self.unit),
            )
        })
    }

    /// The real-world area the Canvas covers, like "1,024 × 768 mi".
    pub fn coverage(&self) -> String {
        coverage_text(self.width, self.height, self.distance, self.unit)
    }

    /// Shows the dialog while it is open. Returns the Canvas once the user presses Create.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<Canvas> {
        if !self.open {
            return None;
        }
        let mut created = None;
        let mut cancelled = false;
        let width = CARD_WIDTH * CARDS_PER_ROW as f32 + CARD_GAP * (CARDS_PER_ROW - 1) as f32;
        let shown = dialog::show(ctx, "new_map", "New Map", width, |ui| {
            if !self.templates.is_empty() {
                self.template_picker(ui, width);
                ui.add_space(12.0);
            }
            egui::Grid::new("new map settings")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Width");
                    ui.add(
                        egui::DragValue::new(&mut self.width)
                            .range(SIDE_RANGE)
                            .suffix(" px"),
                    );
                    ui.end_row();
                    ui.label("Height");
                    ui.add(
                        egui::DragValue::new(&mut self.height)
                            .range(SIDE_RANGE)
                            .suffix(" px"),
                    );
                    ui.end_row();
                    ui.label("One pixel is");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut self.distance)
                                .speed(0.01)
                                .range(0.0001..=100_000.0),
                        );
                        egui::ComboBox::from_id_salt("new map unit")
                            .selected_text(unit_name(self.unit))
                            .show_ui(ui, |ui| {
                                for unit in [DistanceUnit::Miles, DistanceUnit::Kilometres] {
                                    ui.selectable_value(&mut self.unit, unit, unit_name(unit));
                                }
                            });
                    });
                    ui.end_row();
                });
            ui.add_space(8.0);
            let problem = self.problem();
            let p = theme::palette(ui.ctx());
            match &problem {
                Some(problem) => {
                    ui.colored_label(p.danger, problem);
                }
                None => {
                    ui.label(egui::RichText::new(self.coverage()).color(p.text_secondary));
                }
            }
            dialog::buttons(ui, |ui| {
                if dialog::primary_button(ui, problem.is_none(), "Create").clicked() {
                    created = self.canvas();
                }
                if ui.button("Cancel").clicked() {
                    cancelled = true;
                }
            });
        });
        if created.is_none() && dialog::enter_confirms(ctx) {
            created = self.canvas();
        }
        if created.is_some() || cancelled || shown.cancelled {
            self.open = false;
        }
        created
    }
}

impl NewMapDialog {
    /// The live preview of the selected template and a card for each template.
    fn template_picker(&mut self, ui: &mut egui::Ui, width: f32) {
        let p = theme::palette(ui.ctx());
        let aspect = self.width as f32 / self.height as f32;
        let (preview, _) =
            ui.allocate_exact_size(egui::vec2(width, PREVIEW_HEIGHT), egui::Sense::hover());
        if let Some(template) = self.templates.get(self.selected) {
            let fitted = fit(preview, aspect);
            paint_template(ui.painter(), fitted, template);
            ui.painter().rect_stroke(
                fitted,
                egui::CornerRadius::same(theme::RADIUS_FIELD),
                egui::Stroke::new(1.0, p.border_strong),
                egui::StrokeKind::Outside,
            );
        }
        ui.add_space(4.0);
        let caption = match self.selected_name() {
            Some(name) => format!("Preview: {name}"),
            None => String::new(),
        };
        ui.label(egui::RichText::new(caption).color(p.text_secondary));
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing = egui::vec2(CARD_GAP, CARD_GAP);
        let mut chosen = self.selected;
        for (row, templates) in self.templates.chunks(CARDS_PER_ROW).enumerate() {
            ui.horizontal(|ui| {
                for (i, template) in templates.iter().enumerate() {
                    let index = row * CARDS_PER_ROW + i;
                    if template_card(ui, template, index == self.selected).clicked() {
                        chosen = index;
                    }
                }
            });
        }
        self.selected = chosen;
    }
}

/// The largest rectangle with the given width ÷ height ratio centred in `outer`.
fn fit(outer: egui::Rect, aspect: f32) -> egui::Rect {
    let mut size = egui::vec2(outer.width(), outer.width() / aspect);
    if size.y > outer.height() {
        size = egui::vec2(outer.height() * aspect, outer.height());
    }
    egui::Rect::from_center_size(outer.center(), size)
}

/// One selectable card: the template's look and its name.
fn template_card(ui: &mut egui::Ui, template: &MapTemplate, selected: bool) -> egui::Response {
    let p = theme::palette(ui.ctx());
    let size = egui::vec2(CARD_WIDTH, CARD_THUMB_HEIGHT + 26.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::RadioButton,
            true,
            selected,
            template.name(),
        )
    });
    let thumb = egui::Rect::from_min_size(rect.min, egui::vec2(CARD_WIDTH, CARD_THUMB_HEIGHT));
    let radius = egui::CornerRadius::same(theme::RADIUS_FIELD);
    ui.painter().rect_filled(rect, radius, p.bg_field);
    paint_template(ui.painter(), thumb.shrink(2.0), template);
    let stroke = if selected {
        egui::Stroke::new(2.0, p.gold)
    } else if response.hovered() {
        egui::Stroke::new(1.0, p.gold_ring)
    } else {
        egui::Stroke::new(1.0, p.border_strong)
    };
    ui.painter()
        .rect_stroke(rect, radius, stroke, egui::StrokeKind::Inside);
    ui.painter().text(
        egui::pos2(rect.center().x, thumb.max.y + 13.0),
        egui::Align2::CENTER_CENTER,
        template.name(),
        egui::FontId::new(theme::BODY, theme::ui_family()),
        if selected {
            p.text_primary
        } else {
            p.text_secondary
        },
    );
    response
}

fn colour(c: ssmap_core::Color) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

/// Paints a small stand-in for a Map of `template` into `rect`: its water with the texture's
/// light and dark mottling, and an island in the template's land fill with its shore.
fn paint_template(painter: &egui::Painter, rect: egui::Rect, template: &MapTemplate) {
    let painter = painter.with_clip_rect(rect);
    let background = template.background();
    painter.rect_filled(rect, 0.0, colour(background.water));
    // Soft blotches stand in for the noise; a fixed pattern, so the preview doesn't flicker.
    let texture = &background.texture;
    let strength = texture.strength.clamp(0.0, 1.0);
    let spread = (rect.width() / 5.0) * texture.scale.clamp(0.25, 4.0);
    for i in 0..14 {
        let t = i as f32;
        let at = egui::pos2(
            rect.min.x + rect.width() * ((t * 0.37 + 0.11).fract()),
            rect.min.y + rect.height() * ((t * 0.61 + 0.29).fract()),
        );
        let tint = if i % 2 == 0 {
            texture.light
        } else {
            texture.dark
        };
        let alpha = (strength * 70.0) as u8;
        let tint = egui::Color32::from_rgba_unmultiplied(tint.r, tint.g, tint.b, alpha);
        painter.circle_filled(at, spread * (0.6 + 0.4 * ((t * 0.53).fract())), tint);
    }
    let defaults = template.defaults();
    let shore = &defaults.shore;
    let centre = rect.center();
    let radius = egui::vec2(rect.width() * 0.3, rect.height() * 0.32);
    let blob = |grow: f32| -> Vec<egui::Pos2> {
        (0..48)
            .map(|k| {
                let a = k as f32 / 48.0 * std::f32::consts::TAU;
                let wobble = 1.0 + 0.08 * (3.0 * a).sin() + 0.05 * (5.0 * a + 1.0).sin();
                centre + egui::vec2(a.cos() * radius.x, a.sin() * radius.y) * wobble * grow
            })
            .collect()
    };
    painter.add(egui::Shape::convex_polygon(
        blob(1.18),
        colour(shore.shading.color),
        egui::Stroke::NONE,
    ));
    let width = (shore.outline.width as f32).clamp(0.5, 2.0);
    painter.add(egui::Shape::convex_polygon(
        blob(1.0),
        colour(defaults.land_fill),
        egui::Stroke::new(width, colour(shore.outline.color)),
    ));
}

/// The real-world size of a `width` × `height` px Canvas at `distance` per pixel.
pub fn coverage_text(width: u32, height: u32, distance: f64, unit: DistanceUnit) -> String {
    format!(
        "{} × {} {}",
        group_digits(f64::from(width) * distance),
        group_digits(f64::from(height) * distance),
        unit_abbreviation(unit)
    )
}

/// `value` to at most two decimals, with thousands separators.
fn group_digits(value: f64) -> String {
    let text = format!("{value:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let mut grouped = String::new();
    for (i, digit) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    if fraction.is_empty() {
        grouped
    } else {
        format!("{grouped}.{fraction}")
    }
}

fn unit_abbreviation(unit: DistanceUnit) -> &'static str {
    match unit {
        DistanceUnit::Miles => "mi",
        DistanceUnit::Kilometres => "km",
    }
}

fn unit_name(unit: DistanceUnit) -> &'static str {
    match unit {
        DistanceUnit::Miles => "miles",
        DistanceUnit::Kilometres => "kilometres",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_make_a_canvas() {
        let canvas = NewMapDialog::default().canvas().unwrap();
        assert_eq!((canvas.width, canvas.height), (4096, 3072));
        assert_eq!(canvas.scale, Scale::new(0.25, DistanceUnit::Miles));
    }

    #[test]
    fn the_coverage_line_is_the_real_world_size() {
        assert_eq!(NewMapDialog::default().coverage(), "1,024 × 768 mi");
        assert_eq!(
            coverage_text(1000, 800, 2.0, DistanceUnit::Kilometres),
            "2,000 × 1,600 km"
        );
        assert_eq!(
            coverage_text(300, 256, 0.125, DistanceUnit::Miles),
            "37.5 × 32 mi"
        );
    }

    #[test]
    fn sizes_and_scales_out_of_range_are_refused() {
        let dialog = |width, height, distance| NewMapDialog {
            width,
            height,
            distance,
            ..NewMapDialog::default()
        };
        assert!(dialog(100, 3072, 1.0).canvas().is_none());
        assert!(dialog(4096, 20_000, 1.0).canvas().is_none());
        assert!(dialog(4096, 3072, 0.0).problem().is_some());
        assert!(dialog(4096, 3072, f64::NAN).problem().is_some());
        assert!(dialog(256, 16384, 3.0).canvas().is_some());
    }

    #[test]
    fn a_new_map_has_the_starter_layers_and_the_chosen_canvas() {
        let canvas = Canvas::new(1000, 800, Scale::new(2.0, DistanceUnit::Kilometres));
        let (map, layers, problem) = start_map(canvas.clone());
        assert_eq!(problem, None);
        for layer in [layers.land, layers.assets, layers.labels] {
            assert!(map.contains(layer.expect("the template names its default Layers")));
        }
        assert_eq!(map.canvas, canvas);
        assert_eq!(map.children(None).len(), 4);
        assert_eq!(map.label_styles().len(), 5);
    }
}
