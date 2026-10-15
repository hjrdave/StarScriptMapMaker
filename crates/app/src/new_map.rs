//! The New Map dialog: the Canvas size and real-world scale a new Map starts with. The Map
//! itself comes from the starter template (see [`start_map`]).

use eframe::egui;
use ssmap_core::{Canvas, DefaultLayers, DistanceUnit, LabelStyles, Map, MapTemplate, Scale};

/// The smallest and largest side of a Canvas, in nominal pixels.
pub const SIDE_RANGE: std::ops::RangeInclusive<u32> = 256..=16384;

/// What the dialog starts with.
const DEFAULT_SIDES: (u32, u32) = (4096, 3072);
const DEFAULT_DISTANCE: f64 = 0.25;

/// A new Map from the starter template, with `canvas`, and the Layers new Objects go into at
/// first. If the template can't be read, the Map is blank apart from the starter Label styles,
/// and the problem is returned beside it.
pub fn start_map(canvas: Canvas) -> (Map, DefaultLayers, Option<String>) {
    match MapTemplate::starter() {
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

    /// Shows the dialog while it is open. Returns the Canvas once the user presses Create.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<Canvas> {
        if !self.open {
            return None;
        }
        let mut created = None;
        let mut cancelled = false;
        let modal = egui::Modal::new(egui::Id::new("new_map")).show(ctx, |ui| {
            ui.heading("New Map");
            egui::Grid::new("new map settings")
                .num_columns(2)
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
            let problem = self.problem();
            if let Some(problem) = &problem {
                ui.colored_label(ui.visuals().error_fg_color, problem);
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(problem.is_none(), egui::Button::new("Create"))
                    .clicked()
                {
                    created = self.canvas();
                }
                if ui.button("Cancel").clicked() {
                    cancelled = true;
                }
            });
        });
        if created.is_some() || cancelled || modal.should_close() {
            self.open = false;
        }
        created
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
