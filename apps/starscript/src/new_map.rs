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

pub struct NewMapDialog {
    pub open: bool,
    width: u32,
    height: u32,
    distance: f64,
    unit: DistanceUnit,
}

impl Default for NewMapDialog {
    fn default() -> Self {
        Self {
            open: false,
            width: DEFAULT_SIDES.0,
            height: DEFAULT_SIDES.1,
            distance: DEFAULT_DISTANCE,
            unit: DistanceUnit::Miles,
        }
    }
}

impl NewMapDialog {
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
        let shown = dialog::show(ctx, "new_map", "New Map", 380.0, |ui| {
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
