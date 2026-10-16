//! The Export PNG dialog: size, region, background, credits, and file name. It only collects
//! the settings and stores them in the Map; running the export is a separate step.

use std::path::PathBuf;

use eframe::egui;
use ssmap_core::{
    Canvas, ExportRegion, ExportSettings, MAX_EXPORT_SIDE, SizeMode, WARN_EXPORT_SIDE,
};

use crate::{dialog, prop_widgets, theme};

/// What the dialog tells the user about the chosen size.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SizeMessage {
    /// The size is fine.
    Size(String),
    /// The export works but a side is above [`WARN_EXPORT_SIDE`].
    Warning(String),
    /// The export can't be made.
    Error(String),
}

impl SizeMessage {
    /// The text to show.
    pub fn text(&self) -> &str {
        match self {
            Self::Size(text) | Self::Warning(text) | Self::Error(text) => text,
        }
    }
}

/// The size message for `settings` on `canvas`.
pub fn size_message(settings: &ExportSettings, canvas: &Canvas) -> SizeMessage {
    match settings.output_size(canvas) {
        Ok(size) => {
            let text = format!("{} × {} px", size.width, size.height);
            if size.warn_large {
                SizeMessage::Warning(format!(
                    "{text}: a side is over {WARN_EXPORT_SIDE} px. This export is large and may be slow or run out of memory."
                ))
            } else {
                SizeMessage::Size(text)
            }
        }
        Err(err) => SizeMessage::Error(err.to_string()),
    }
}

/// The size mode `to` as it starts after switching, keeping the current pixel width where the
/// new mode has one.
pub fn switch_size_mode(to: &SizeMode, settings: &ExportSettings, canvas: &Canvas) -> SizeMode {
    let width = settings
        .output_size(canvas)
        .map_or(canvas.width, |size| size.width);
    match to {
        SizeMode::Scale { .. } => SizeMode::Scale { factor: 1.0 },
        SizeMode::Width { .. } => SizeMode::Width { pixels: width },
        SizeMode::Print { .. } => SizeMode::Print {
            width_inches: 12.0,
            dpi: 300,
        },
    }
}

/// The Scale buttons beside the field.
const SCALE_PRESETS: [f64; 4] = [1.0, 2.0, 4.0, 8.0];

/// What the rendered image takes in memory, as text: 4 bytes a pixel. `None` while the size
/// can't be made.
pub fn memory_estimate(settings: &ExportSettings, canvas: &Canvas) -> Option<String> {
    let size = settings.output_size(canvas).ok()?;
    let bytes = u64::from(size.width) * u64::from(size.height) * 4;
    let mib = bytes as f64 / (1024.0 * 1024.0);
    Some(if mib >= 1024.0 {
        format!("{:.1} GB", mib / 1024.0)
    } else {
        format!("{mib:.0} MB")
    })
}

/// Whether the Scale `factor` makes an image the export can write, with `settings` otherwise.
pub fn scale_allowed(factor: f64, settings: &ExportSettings, canvas: &Canvas) -> bool {
    let mut trial = settings.clone();
    trial.size = SizeMode::Scale { factor };
    trial.output_size(canvas).is_ok()
}

fn mode_name(mode: &SizeMode) -> &'static str {
    match mode {
        SizeMode::Scale { .. } => "Scale",
        SizeMode::Width { .. } => "Width in pixels",
        SizeMode::Print { .. } => "Inches × DPI",
    }
}

/// The whole Canvas, or a custom region that starts as the current one.
fn region_switch(custom: bool, settings: &ExportSettings, canvas: &Canvas) -> ExportRegion {
    if custom {
        ExportRegion::Custom {
            rect: settings.region_rect(canvas),
        }
    } else {
        ExportRegion::Canvas
    }
}

/// What the user chose in the dialog.
#[derive(Clone, PartialEq, Debug)]
pub enum DialogAction {
    /// Store the settings in the Map.
    Save(ExportSettings),
    /// Store the settings, then export with them.
    Export(ExportSettings),
    /// Store the settings and switch to the Export Region tool to draw the region.
    DrawRegion(ExportSettings),
}

#[derive(Default)]
pub struct ExportDialog {
    pub open: bool,
    /// The settings being edited; copied from and back to the Map.
    draft: ExportSettings,
    /// The folder to export into. Not stored in the Map, which keeps the file name only.
    pub directory: String,
}

impl ExportDialog {
    /// Opens the dialog showing the Map's stored `settings`. `directory` replaces the folder
    /// when given; otherwise the last one stays.
    pub fn open_with(&mut self, settings: &ExportSettings, directory: Option<PathBuf>) {
        self.draft = settings.clone();
        if let Some(directory) = directory {
            self.directory = directory.to_string_lossy().into_owned();
        }
        self.open = true;
    }

    pub fn draft(&self) -> &ExportSettings {
        &self.draft
    }

    /// Shows the dialog while it is open. Returns what the user chose once they press Save
    /// or Export.
    pub fn show(&mut self, ctx: &egui::Context, canvas: &Canvas) -> Option<DialogAction> {
        if !self.open {
            return None;
        }
        let mut action = None;
        let mut cancelled = false;
        let shown = dialog::show(ctx, "export_png", "Export PNG", 460.0, |ui| {
            self.region_ui(ui, canvas, &mut action);
            ui.separator();
            self.size_ui(ui, canvas);
            ui.separator();
            prop_widgets::checkbox_row(
                ui,
                "Transparent background",
                &mut self.draft.transparent_background,
            );
            prop_widgets::checkbox_row(
                ui,
                "Write a credits.txt file beside the PNG",
                &mut self.draft.credits.text_file,
            );
            prop_widgets::checkbox_row(
                ui,
                "Write the credits into the PNG",
                &mut self.draft.credits.png_chunk,
            );
            ui.separator();
            egui::Grid::new("export file")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("File name");
                    let mut name = self.draft.file_name.clone().unwrap_or_default();
                    if ui.text_edit_singleline(&mut name).changed() {
                        self.draft.file_name = (!name.is_empty()).then_some(name);
                    }
                    ui.end_row();
                    ui.label("Folder");
                    ui.text_edit_singleline(&mut self.directory);
                    ui.end_row();
                });
            ui.separator();
            let message = size_message(&self.draft, canvas);
            let p = theme::palette(ui.ctx());
            let colour = match &message {
                SizeMessage::Size(_) => p.text_primary,
                SizeMessage::Warning(_) => p.warn,
                SizeMessage::Error(_) => p.danger,
            };
            ui.colored_label(colour, message.text());
            if let Some(memory) = memory_estimate(&self.draft, canvas) {
                ui.label(
                    egui::RichText::new(format!("About {memory} of memory"))
                        .color(p.text_secondary),
                );
            }
            let valid = !matches!(message, SizeMessage::Error(_));
            dialog::buttons(ui, |ui| {
                if dialog::primary_button(ui, valid, "Export").clicked() {
                    action = Some(DialogAction::Export(self.draft.clone()));
                }
                if ui.add_enabled(valid, egui::Button::new("Save")).clicked() {
                    action = Some(DialogAction::Save(self.draft.clone()));
                }
                if ui.button("Cancel").clicked() {
                    cancelled = true;
                }
            });
        });
        if action.is_some() || cancelled || shown.cancelled {
            self.open = false;
        }
        action
    }

    fn size_ui(&mut self, ui: &mut egui::Ui, canvas: &Canvas) {
        let current = self.draft.size.clone();
        egui::Grid::new("export size")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Size");
                let mut chosen = None;
                egui::ComboBox::from_id_salt("export size mode")
                    .selected_text(mode_name(&current))
                    .show_ui(ui, |ui| {
                        for mode in [
                            SizeMode::Scale { factor: 1.0 },
                            SizeMode::Width { pixels: 1 },
                            SizeMode::Print {
                                width_inches: 1.0,
                                dpi: 1,
                            },
                        ] {
                            let same =
                                std::mem::discriminant(&mode) == std::mem::discriminant(&current);
                            if ui.selectable_label(same, mode_name(&mode)).clicked() && !same {
                                chosen = Some(mode);
                            }
                        }
                    });
                if let Some(mode) = chosen {
                    self.draft.size = switch_size_mode(&mode, &self.draft, canvas);
                }
                ui.end_row();
                // Presets that would pass the cap are disabled.
                let allowed = SCALE_PRESETS.map(|f| scale_allowed(f, &self.draft, canvas));
                match &mut self.draft.size {
                    SizeMode::Scale { factor } => {
                        ui.label("Scale");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(factor)
                                    .speed(0.05)
                                    .range(0.01..=64.0)
                                    .suffix("×"),
                            );
                            for (preset, allowed) in SCALE_PRESETS.into_iter().zip(allowed) {
                                let button = egui::Button::new(format!("{preset}×")).small();
                                if ui.add_enabled(allowed, button).clicked() {
                                    *factor = preset;
                                }
                            }
                        });
                        ui.end_row();
                    }
                    SizeMode::Width { pixels } => {
                        ui.label("Width");
                        ui.add(
                            egui::DragValue::new(pixels)
                                .range(1..=MAX_EXPORT_SIDE * 2)
                                .suffix(" px"),
                        );
                        ui.end_row();
                        ui.label("");
                        ui.label("The height keeps the region's proportions.");
                        ui.end_row();
                    }
                    SizeMode::Print { width_inches, dpi } => {
                        ui.label("Width");
                        ui.add(
                            egui::DragValue::new(width_inches)
                                .speed(0.1)
                                .range(0.1..=1000.0)
                                .suffix(" in"),
                        );
                        ui.end_row();
                        ui.label("Resolution");
                        ui.add(egui::DragValue::new(dpi).range(1..=2400).suffix(" DPI"));
                        ui.end_row();
                    }
                }
            });
    }

    fn region_ui(&mut self, ui: &mut egui::Ui, canvas: &Canvas, action: &mut Option<DialogAction>) {
        let before = matches!(self.draft.region, ExportRegion::Custom { .. });
        let mut selected = usize::from(before);
        ui.horizontal(|ui| {
            ui.label("Region");
            prop_widgets::segmented(ui, &["Whole Canvas", "Custom region"], &mut selected);
        });
        let custom = selected == 1;
        if custom != before {
            self.draft.region = region_switch(custom, &self.draft, canvas);
        }
        if custom && ui.button("Draw region on Canvas").clicked() {
            *action = Some(DialogAction::DrawRegion(self.draft.clone()));
        }
        if let ExportRegion::Custom { rect } = &mut self.draft.region {
            egui::Grid::new("export region")
                .num_columns(4)
                .show(ui, |ui| {
                    let edge = |ui: &mut egui::Ui, label: &str, value: &mut f64| {
                        ui.label(label);
                        ui.add(egui::DragValue::new(value).speed(1.0));
                    };
                    edge(ui, "Left", &mut rect.min[0]);
                    edge(ui, "Top", &mut rect.min[1]);
                    ui.end_row();
                    edge(ui, "Right", &mut rect.max[0]);
                    edge(ui, "Bottom", &mut rect.max[1]);
                    ui.end_row();
                });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssmap_core::{DistanceUnit, Map, Rect, Scale};

    fn canvas() -> Canvas {
        Canvas::new(4096, 3072, Scale::new(1.0, DistanceUnit::Miles))
    }

    fn with_size(size: SizeMode) -> ExportSettings {
        ExportSettings {
            size,
            ..ExportSettings::default()
        }
    }

    #[test]
    fn a_normal_size_shows_the_pixel_size() {
        let message = size_message(&with_size(SizeMode::Scale { factor: 2.0 }), &canvas());
        assert_eq!(message, SizeMessage::Size("8192 × 6144 px".into()));
    }

    #[test]
    fn a_size_over_the_cap_is_an_error() {
        let message = size_message(&with_size(SizeMode::Scale { factor: 16.0 }), &canvas());
        assert!(matches!(message, SizeMessage::Error(_)), "{message:?}");
        assert!(message.text().contains("32768"));
        let bad = size_message(&with_size(SizeMode::Width { pixels: 0 }), &canvas());
        assert!(matches!(bad, SizeMessage::Error(_)));
    }

    #[test]
    fn a_side_above_16k_is_a_warning_but_allowed() {
        let message = size_message(&with_size(SizeMode::Scale { factor: 5.0 }), &canvas());
        assert!(matches!(message, SizeMessage::Warning(_)), "{message:?}");
        let exactly = size_message(&with_size(SizeMode::Width { pixels: 16_384 }), &canvas());
        assert!(matches!(exactly, SizeMessage::Size(_)));
    }

    #[test]
    fn switching_modes_keeps_the_current_width() {
        let settings = with_size(SizeMode::Scale { factor: 2.0 });
        let to = SizeMode::Width { pixels: 1 };
        assert_eq!(
            switch_size_mode(&to, &settings, &canvas()),
            SizeMode::Width { pixels: 8192 }
        );
    }

    #[test]
    fn settings_round_trip_through_the_dialog_and_the_map() {
        let mut map = Map::new(canvas());
        let stored = ExportSettings {
            size: SizeMode::Print {
                width_inches: 24.0,
                dpi: 150,
            },
            region: ExportRegion::Custom {
                rect: Rect {
                    min: [10.0, 20.0],
                    max: [1000.0, 800.0],
                },
            },
            transparent_background: true,
            file_name: Some("world.png".into()),
            ..ExportSettings::default()
        };
        map.export_settings = stored.clone();

        let mut dialog = ExportDialog::default();
        dialog.open_with(&map.export_settings, Some(PathBuf::from("/maps")));
        assert!(dialog.open);
        assert_eq!(dialog.draft(), &stored);
        assert_eq!(dialog.directory, "/maps");

        // Change a setting; pressing Save stores the draft in the Map.
        dialog.draft.credits.png_chunk = false;
        map.export_settings = dialog.draft().clone();

        let mut reopened = ExportDialog::default();
        reopened.open_with(&map.export_settings, None);
        assert!(!reopened.draft().credits.png_chunk);
        assert_eq!(reopened.draft().file_name.as_deref(), Some("world.png"));
        assert_eq!(reopened.draft().size, stored.size);
        assert_eq!(reopened.draft().region, stored.region);
    }

    #[test]
    fn a_custom_region_starts_as_the_current_region() {
        let whole = Rect {
            min: [0.0, 0.0],
            max: [4096.0, 3072.0],
        };
        let settings = ExportSettings::default();
        assert_eq!(
            region_switch(true, &settings, &canvas()),
            ExportRegion::Custom { rect: whole }
        );
        assert_eq!(
            region_switch(false, &settings, &canvas()),
            ExportRegion::Canvas
        );
    }
}
