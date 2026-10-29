//! The River section: how much a River bends, which bends it has, and how wide it is at its
//! source and its mouth. Its Fill and Shore style are the sections a Lake has.

use eframe::egui;
use ssmap_core::{Command, Id, MAX_MEANDER, MAX_RIVER_WIDTH, MIN_RIVER_WIDTH, River};

use super::{PropertiesPanel, caption};
use crate::editor::Editor;
use crate::icons::{self, Icon};
use crate::prop_widgets::{self, Edit, ScrubSpec, SectionHeader};
use crate::theme::metrics;

/// The room an icon button takes at the end of a row, with its gap.
const ICON_SLOT: f32 = 24.0;

impl PropertiesPanel {
    /// Bends, Seed with a re-roll button, and the widths at the source and the mouth. Each
    /// scrub is one Undo step.
    pub(super) fn river_section(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        id: Id,
        river: &River,
    ) {
        let header = SectionHeader {
            kind: "river",
            title: "River",
            badge: None,
            action: None,
        };
        prop_widgets::section(ui, header, |ui| {
            ui.push_id(("river", id), |ui| {
                self.river_fields(ui, editor, id, river);
            });
        });
    }

    fn river_fields(&mut self, ui: &mut egui::Ui, editor: &mut Editor, id: Id, river: &River) {
        // The new River is made only when the field changed: making one generates its outline.
        let set = |panel: &mut Self, editor: &mut Editor, edit: Edit, river: &dyn Fn() -> River| {
            panel
                .live
                .apply(editor, edit, "Edit River", || Command::SetRiver {
                    id,
                    river: Box::new(river()),
                });
        };

        let mut bends = river.meander() * 100.0;
        let edit = prop_widgets::slider(
            ui,
            &mut bends,
            &ScrubSpec::new("Bends")
                .range(0.0, MAX_MEANDER * 100.0)
                .unit("%")
                .default_value(River::DEFAULT_MEANDER * 100.0),
        );
        set(self, editor, edit, &|| {
            river.clone().with_meander(bends / 100.0)
        });

        let mut seed = f64::from(river.seed());
        let mut reroll = false;
        let mut edit = Edit::default();
        ui.horizontal(|ui| {
            let room = ui.available_width() - ICON_SLOT - ui.spacing().item_spacing.x;
            ui.allocate_ui(egui::vec2(room, metrics::FIELD[0]), |ui| {
                edit = prop_widgets::scrub_field(
                    ui,
                    &mut seed,
                    &ScrubSpec::new("Seed").range(0.0, f64::from(u32::MAX)),
                );
            });
            reroll = icons::icon_button(ui, Icon::ReRoll, "Re-roll").clicked();
        });
        set(self, editor, edit, &|| river.clone().with_seed(seed as u32));
        if reroll {
            let seed = Id::new().as_uuid().as_u128() as u32;
            set(self, editor, Edit::instant(), &|| {
                river.clone().with_seed(seed)
            });
        }

        let path = river.path();
        let last = path.len().saturating_sub(1);
        for (label, index) in [("Source width", 0), ("Mouth width", last)] {
            let Some(point) = path.get(index) else {
                continue;
            };
            let mut width = point.width;
            let edit = prop_widgets::scrub_field(
                ui,
                &mut width,
                &ScrubSpec::new(label)
                    .unit(" u")
                    .range(MIN_RIVER_WIDTH, MAX_RIVER_WIDTH)
                    .step(0.5)
                    .decimals(1),
            );
            set(self, editor, edit, &|| {
                let mut widened = path.to_vec();
                widened[index].width = width;
                river.clone().with_path(widened)
            });
        }
        caption(
            ui,
            "Double-click the River to move its points. [ and ] widen the selected points.",
        );
    }
}
