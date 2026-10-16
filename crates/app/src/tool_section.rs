//! The active tool's settings, at the top of Properties above the selection: the Land Brush's
//! size and mode, the Stamp's Asset picker, the Scatter Brush's picker and settings, and the
//! Stamp Eraser's size. Tools without settings get no section.

use eframe::egui;
use ssmap_core::scatter::Placement;

use crate::asset_browser::AssetBrowser;
use crate::editor::Editor;
use crate::icons::{self, Icon};
use crate::prop_widgets::{self, ScrubSpec};
use crate::theme;
use crate::tools::{self, ToolId};

#[derive(Default)]
pub struct ToolSection {
    browser: AssetBrowser,
}

/// The icon of a tool.
fn icon(tool: ToolId) -> Icon {
    match tool {
        ToolId::Select => Icon::Select,
        ToolId::Pan => Icon::Pan,
        ToolId::LandBrush => Icon::LandBrush,
        ToolId::Lasso => Icon::Lasso,
        ToolId::Label => Icon::Label,
        ToolId::Stamp => Icon::Stamp,
        ToolId::Scatter => Icon::Scatter,
        ToolId::StampEraser => Icon::StampEraser,
        ToolId::ExportRegion => Icon::ExportRegion,
    }
}

/// Whether the tool has settings to show.
pub fn has_section(tool: ToolId) -> bool {
    matches!(
        tool,
        ToolId::LandBrush | ToolId::Stamp | ToolId::Scatter | ToolId::StampEraser
    )
}

/// A slider that writes straight to `value`. Returns whether it changed.
fn slider(ui: &mut egui::Ui, value: &mut f64, spec: &ScrubSpec<'_>) -> bool {
    prop_widgets::slider(ui, value, spec).changed
}

impl ToolSection {
    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        let tool = editor.active_tool;
        if !has_section(tool) {
            return;
        }
        let palette = theme::palette(ui.ctx());
        egui::Frame::NONE
            .fill(palette.bg_tool_section)
            .inner_margin(egui::Margin::symmetric(
                theme::SPACING[4] as i8,
                theme::SPACING[3] as i8,
            ))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                header(ui, tool);
                ui.add_space(theme::SPACING[2]);
                match tool {
                    ToolId::LandBrush => land_brush(ui, editor),
                    ToolId::Stamp => self.browser.show(ui, editor),
                    ToolId::Scatter => {
                        self.browser.show(ui, editor);
                        ui.add_space(theme::SPACING[2]);
                        scatter_settings(ui, editor);
                    }
                    ToolId::StampEraser => size(ui, editor),
                    _ => {}
                }
            });
        ui.add_space(theme::SPACING[1]);
    }
}

/// The tool's icon, name and shortcut.
fn header(ui: &mut egui::Ui, tool: ToolId) {
    let palette = theme::palette(ui.ctx());
    ui.horizontal(|ui| {
        let (rect, _) =
            ui.allocate_exact_size(egui::Vec2::splat(icons::GRID), egui::Sense::hover());
        icons::paint(ui, rect, icon(tool), icons::GRID, palette.gold_text);
        ui.label(
            egui::RichText::new(tool.label())
                .size(theme::HEADING)
                .color(palette.text_primary),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(format!("{:?}", tool.shortcut()))
                    .size(theme::CAPTION)
                    .family(theme::mono_family())
                    .color(palette.text_tertiary),
            );
        });
    });
}

/// The brush diameter, which `[` and `]` also change.
fn size(ui: &mut egui::Ui, editor: &mut Editor) {
    let spec = ScrubSpec::new("Size").range(*tools::BRUSH_SIZES.start(), *tools::BRUSH_SIZES.end());
    slider(ui, &mut editor.brush_size, &spec);
}

fn land_brush(ui: &mut egui::Ui, editor: &mut Editor) {
    size(ui, editor);
    let mut mode = usize::from(editor.erase_default);
    ui.horizontal(|ui| {
        ui.label("Mode");
        if prop_widgets::segmented(ui, &["Paint", "Erase"], &mut mode) {
            editor.erase_default = mode == 1;
        }
    })
    .response
    .on_hover_text(format!(
        "Hold {} to get the other mode.",
        tools::ERASE_MODIFIER
    ));
}

fn scatter_settings(ui: &mut egui::Ui, editor: &mut Editor) {
    size(ui, editor);
    let brush = &mut editor.scatter;
    let mut place = match brush.placement {
        Placement::Land => 0,
        Placement::Water => 1,
        Placement::Anywhere => 2,
    };
    ui.horizontal(|ui| {
        ui.label("Place on");
        if prop_widgets::segmented(ui, &["Land", "Water", "Any"], &mut place) {
            brush.placement = [Placement::Land, Placement::Water, Placement::Anywhere][place];
        }
    });
    slider(
        ui,
        &mut brush.density,
        &ScrubSpec::new("Density").range(0.5, 100.0).decimals(1),
    );
    slider(
        ui,
        &mut brush.spacing,
        &ScrubSpec::new("Spacing").range(0.0, 200.0),
    );
    let mut jitter = brush.size_jitter * 100.0;
    if slider(
        ui,
        &mut jitter,
        &ScrubSpec::new("Size jitter").range(0.0, 90.0).unit("%"),
    ) {
        brush.size_jitter = jitter / 100.0;
    }
    slider(
        ui,
        &mut brush.rotation_jitter,
        &ScrubSpec::new("Rotation jitter")
            .range(0.0, 180.0)
            .unit("°"),
    );
    prop_widgets::checkbox_row(ui, "Random flip", &mut brush.random_flip);
    prop_widgets::checkbox_row(ui, "Random variant", &mut brush.random_variant);
}
