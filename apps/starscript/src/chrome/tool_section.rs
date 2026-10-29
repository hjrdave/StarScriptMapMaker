//! The active tool's settings, at the top of Properties above the selection: the Land Brush's
//! size and mode, the River's width and bending, the Stamp's chosen Asset and its scatter
//! settings, and the Stamp Eraser's size. Tools without settings get no section.

use eframe::egui;
use ssmap_core::scatter::Placement;

use crate::editor::Editor;
use crate::icons::{self, Icon};
use crate::prop_widgets::{self, Handle, Provenance, ScrubSpec};
use crate::theme;
use crate::tools::{self, ToolId};

#[derive(Default)]
pub(crate) struct ToolSection {
    /// The Asset card was clicked since the last [`ToolSection::take_open_assets`].
    open_assets: bool,
}

/// The icon of a tool.
pub(crate) fn icon(tool: ToolId) -> Icon {
    match tool {
        ToolId::Select => Icon::Select,
        ToolId::Pan => Icon::Pan,
        ToolId::LandBrush => Icon::LandBrush,
        ToolId::WaterBrush => Icon::WaterBrush,
        ToolId::River => Icon::River,
        ToolId::Lasso => Icon::Lasso,
        ToolId::Label => Icon::Label,
        ToolId::Stamp => Icon::Stamp,
        ToolId::StampEraser => Icon::StampEraser,
        ToolId::Canvas | ToolId::ExportRegion => Icon::ExportRegion,
    }
}

/// Whether the tool has settings to show.
fn has_section(tool: ToolId) -> bool {
    matches!(
        tool,
        ToolId::LandBrush
            | ToolId::WaterBrush
            | ToolId::River
            | ToolId::Stamp
            | ToolId::StampEraser
    )
}

/// A scrub field that writes straight to `value`. Returns whether it changed.
fn field(ui: &mut egui::Ui, value: &mut f64, spec: &ScrubSpec<'_>) -> bool {
    prop_widgets::scrub_field(ui, value, spec).changed
}

impl ToolSection {
    /// Whether the Asset card was clicked since the last call.
    pub(crate) fn take_open_assets(&mut self) -> bool {
        std::mem::take(&mut self.open_assets)
    }

    pub(crate) fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
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
                    ToolId::LandBrush | ToolId::WaterBrush => land_brush(ui, editor),
                    ToolId::River => river(ui, editor),
                    ToolId::Stamp => {
                        self.chosen_asset(ui, editor);
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

impl ToolSection {
    /// The Asset the Stamp places, as a card that opens the Assets section where it is chosen.
    fn chosen_asset(&mut self, ui: &mut egui::Ui, editor: &Editor) {
        let palette = theme::palette(ui.ctx());
        let card = editor
            .selected_asset
            .as_ref()
            .map(|reference| editor.asset_card(reference));
        let (name, subtitle) = card
            .clone()
            .unwrap_or_else(|| ("None chosen".to_owned(), "Pick one in Assets".to_owned()));
        let label = match &card {
            Some((name, _)) => format!("Choose Asset: {name}"),
            None => "Choose Asset".to_owned(),
        };
        let response = egui::Frame::NONE
            .inner_margin(egui::Margin::same(6))
            .fill(palette.bg_raised)
            .stroke(egui::Stroke::new(1.0, palette.border_subtle))
            .corner_radius(theme::RADIUS_BUTTON)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(
                            egui::RichText::new(name)
                                .size(theme::BODY)
                                .strong()
                                .color(palette.text_primary),
                        );
                        ui.label(
                            egui::RichText::new(subtitle)
                                .size(theme::CAPTION)
                                .color(palette.text_tertiary),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (rect, _) = ui.allocate_exact_size(
                            egui::Vec2::splat(icons::GRID),
                            egui::Sense::hover(),
                        );
                        icons::paint(ui, rect, Icon::ChevronRight, 14.0, palette.text_tertiary);
                    });
                });
            })
            .response
            .interact(egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
        });
        if response.clicked() {
            self.open_assets = true;
        }
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
                egui::RichText::new(tool.shortcut_text())
                    .size(theme::CAPTION)
                    .family(theme::mono_family())
                    .color(palette.text_tertiary),
            );
        });
    });
}

/// The brush diameter in Canvas units, which `[` and `]` also change.
fn size_spec() -> ScrubSpec<'static> {
    ScrubSpec::new("Size")
        .unit(" u")
        .range(*tools::BRUSH_SIZES.start(), *tools::BRUSH_SIZES.end())
}

/// The brush size of the Land Brush, Water Brush and Stamp Eraser: a logarithmic slider,
/// where dragging along the track is the point.
fn size(ui: &mut egui::Ui, editor: &mut Editor) {
    let spec = ScrubSpec::new("Size")
        .range(*tools::BRUSH_SIZES.start(), *tools::BRUSH_SIZES.end())
        .slider_max(tools::BRUSH_SIZE_SLIDER_MAX)
        .logarithmic()
        .default_value(crate::editor::DEFAULT_BRUSH_SIZE);
    prop_widgets::slider(ui, &mut editor.brush_size, &spec);
}

fn land_brush(ui: &mut egui::Ui, editor: &mut Editor) {
    size(ui, editor);
    let mut mode = usize::from(editor.erase_default);
    prop_widgets::field_row(ui, "Mode", Provenance::Plain, |ui| {
        if prop_widgets::segmented(ui, &["Paint", "Erase"], &mut mode) {
            editor.erase_default = mode == 1;
        }
    });
    ui.label(
        egui::RichText::new(format!(
            "Hold {} to get the other mode.",
            tools::ERASE_MODIFIER
        ))
        .size(theme::CAPTION)
        .color(theme::palette(ui.ctx()).text_tertiary),
    );
}

/// The width the next point gets, which `[` and `]` also change, and how much new Rivers bend.
fn river(ui: &mut egui::Ui, editor: &mut Editor) {
    let spec = ScrubSpec::new("Width")
        .range(ssmap_core::MIN_RIVER_WIDTH, ssmap_core::MAX_RIVER_WIDTH)
        .slider_max(tools::BRUSH_SIZE_SLIDER_MAX)
        .logarithmic()
        .default_value(crate::editor::DEFAULT_RIVER_WIDTH);
    prop_widgets::slider(ui, &mut editor.river_width, &spec);
    let mut bends = editor.river_meander * 100.0;
    let spec = ScrubSpec::new("Bends")
        .range(0.0, ssmap_core::MAX_MEANDER * 100.0)
        .unit("%")
        .default_value(ssmap_core::River::DEFAULT_MEANDER * 100.0);
    if prop_widgets::slider(ui, &mut bends, &spec).changed {
        editor.river_meander = bends / 100.0;
    }
    ui.label(
        egui::RichText::new(format!(
            "Click from source to mouth · double-click or Enter to finish. Widen with ] as \
             you go. Shift snaps to 45°, {} runs straight. Shift + wheel changes the bends.",
            tools::ERASE_MODIFIER
        ))
        .size(theme::CAPTION)
        .color(theme::palette(ui.ctx()).text_tertiary),
    );
}

fn scatter_settings(ui: &mut egui::Ui, editor: &mut Editor) {
    let brush_size = &mut editor.brush_size;
    let brush = &mut editor.scatter;
    prop_widgets::pair(
        ui,
        |ui| field(ui, brush_size, &size_spec().handle(Handle::Text("S"))),
        |ui| {
            let spec = ScrubSpec::new("Rotation jitter")
                .handle(Handle::Icon(Icon::Rotate))
                .range(0.0, 180.0)
                .unit("°");
            field(ui, &mut brush.rotation_jitter, &spec)
        },
    );
    let mut place = match brush.placement {
        Placement::Land => 0,
        Placement::Water => 1,
        Placement::Anywhere => 2,
    };
    prop_widgets::field_row(ui, "Place on", Provenance::Plain, |ui| {
        if prop_widgets::segmented(ui, &["Land", "Water", "Any"], &mut place) {
            brush.placement = [Placement::Land, Placement::Water, Placement::Anywhere][place];
        }
    });
    prop_widgets::field_row(ui, "Randomise", Provenance::Plain, |ui| {
        prop_widgets::chip_toggle(ui, "Flip", &mut brush.random_flip);
        prop_widgets::chip_toggle(ui, "Variant", &mut brush.random_variant);
    });
    field(
        ui,
        &mut brush.density,
        &ScrubSpec::new("Density")
            .range(0.5, 100.0)
            .decimals(1)
            .step(0.1),
    );
    field(
        ui,
        &mut brush.spacing,
        &ScrubSpec::new("Spacing").range(0.0, 2000.0),
    );
    let mut jitter = brush.size_jitter * 100.0;
    if field(
        ui,
        &mut jitter,
        &ScrubSpec::new("Size jitter").range(0.0, 90.0).unit("%"),
    ) {
        brush.size_jitter = jitter / 100.0;
    }
    ui.label(
        egui::RichText::new("Click to place · drag to scatter")
            .size(theme::CAPTION)
            .color(theme::palette(ui.ctx()).text_tertiary),
    );
}
