//! The properties panel: shows and edits what is selected, through Commands.

mod board;
mod header;
pub(crate) use board::BoardRequest;
mod label;
mod map;
mod object_style;
mod river;
mod stamp;
#[cfg(test)]
mod tests;

use eframe::egui;
use ssmap_core::{AssetRef, Color, Command, Id, Map, MapTemplate, Node, ObjectKind};

use crate::asset_browser::ThumbArt;
use crate::editor::Editor;
use crate::icons::Icon;
use crate::layer_tree::node_name;
use crate::prop_widgets::{self, Edit, LiveEdit, Provenance, SectionHeader};
use crate::theme;
use crate::tool_section::ToolSection;

#[derive(Default)]
pub(crate) struct PropertiesPanel {
    /// The text being typed for a Label, committed when the field loses focus.
    text_edit: Option<(Id, String)>,
    /// Turns the scrubs, sliders and swatches into one Undo step each.
    live: LiveEdit,
    /// "Manage Label styles…" was clicked; the window is opened by the owner of the tab.
    manage_label_styles: bool,
    /// What the board buttons asked the window for.
    board_requests: Vec<board::BoardRequest>,
    /// The active tool's settings, above the selection.
    tool_section: ToolSection,
    /// The Stamp section's "Replace Asset" was clicked; the window opens the Assets section.
    open_assets: bool,
    /// The selected Stamp's Asset and its thumbnail, loaded once per Asset.
    stamp_thumb: Option<(AssetRef, Option<ThumbArt>)>,
    /// The Canvas point that stays put when it is resized, as (column, row) of the 3×3
    /// anchor grid: 0 is left / top, 1 the middle, 2 right / bottom.
    canvas_anchor: (u8, u8),
    /// The board being resized by a drag, and its box when the drag began.
    resize_base: Option<(Id, ssmap_core::Canvas)>,
    /// The templates whose look the background can take, read when the Map section first
    /// opens.
    templates: Option<Vec<MapTemplate>>,
}

/// A group of settings that may be inherited from a default: the value shown, where it
/// comes from, how an edit is written and how it is reset.
struct Inherit<'a, T> {
    /// The value of each selected item (one for a single item or a Map default). The fields
    /// show the first; where the others differ they show "Mixed". Never empty: every
    /// `Inherit` is built from at least one selected item or a Map default.
    members: &'a [T],
    provenance: Provenance,
    label: &'a str,
    /// The Command that gives member `index` the new value.
    set: &'a dyn Fn(usize, T) -> Command,
    /// Goes back to the default. `None` where there is nothing to go back to.
    reset: Option<Command>,
}

impl<T: Clone> Inherit<'_, T> {
    fn current(&self) -> &T {
        // Never empty, see `members`.
        &self.members[0]
    }

    /// Whether the setting `field` reads differs between the members.
    fn mixed(&self, field: impl Fn(&T) -> f64) -> bool {
        let Some(first) = self.members.first().map(&field) else {
            return false;
        };
        self.members.iter().any(|m| field(m) != first)
    }

    /// One Command per member, each with `apply` done to its own value, so the other settings
    /// of each item are kept.
    fn commands(&self, apply: impl Fn(&mut T)) -> Vec<Command> {
        self.members
            .iter()
            .enumerate()
            .map(|(i, member)| {
                let mut value = member.clone();
                apply(&mut value);
                (self.set)(i, value)
            })
            .collect()
    }
}

impl PropertiesPanel {
    /// Whether "Manage Label styles…" was clicked since the last call.
    pub(crate) fn take_manage_label_styles(&mut self) -> bool {
        std::mem::take(&mut self.manage_label_styles)
    }

    /// Whether the Stamp tool's Asset card or "Replace Asset" on a selected Stamp was clicked
    /// since the last call.
    pub(crate) fn take_open_assets(&mut self) -> bool {
        let replace = std::mem::take(&mut self.open_assets);
        self.tool_section.take_open_assets() || replace
    }

    /// Returns true when the collapse control at the bottom was clicked.
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) -> bool {
        // A change ends when the pointer is released, or when Escape cancelled it.
        self.live.apply(editor, Edit::default(), "", || {
            unreachable!("nothing changed")
        });
        // The Map section starts collapsed; the others start open.
        prop_widgets::default_closed(ui.ctx(), "map");
        crate::shell::panel_header(ui, "Properties", |_| {});
        let footer = crate::layer_tree::FOOTER_HEIGHT;
        // The control is pinned to the panel's bottom edge, where the collapsed strip has it too.
        let panel = ui.max_rect();
        let rect = egui::Rect::from_min_max(
            egui::pos2(panel.left(), panel.bottom() - footer),
            panel.right_bottom(),
        );
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height((rect.top() - ui.cursor().top()).max(0.0))
            .show(ui, |ui| {
                let stamp_selected = stamp::stamps(editor.map(), editor.selection()).is_some();
                self.tool_section.show(ui, editor, stamp_selected);
                ui.add_space(theme::SPACING[1]);
                self.selection(ui, editor);
            });
        ui.painter().hline(
            rect.x_range(),
            rect.top(),
            egui::Stroke::new(1.0, theme::palette(ui.ctx()).border_subtle),
        );
        let mut bar = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        bar.add_space(theme::SPACING[2]);
        crate::shell::properties_collapse_button(&mut bar, false).clicked()
    }

    fn selection(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        // Stamps name themselves in their header and show the board under Transform.
        if let Some(members) = stamp::stamps(editor.map(), editor.selection()) {
            self.stamp_properties(ui, editor, &members);
            return;
        }
        match editor.selection() {
            [] if let Some(board) = editor.selected_board() => {
                self.board_properties(ui, editor, board);
            }
            [] => {
                self.map_properties(ui, editor);
            }
            [id] => {
                let id = *id;
                match editor.map().node(id).cloned() {
                    // A Layer's own fields are edited in the Layers panel.
                    Some(Node::Layer(_)) => {
                        breadcrumb(ui, editor.map(), id);
                        self.map_properties(ui, editor);
                    }
                    Some(Node::Object(object)) if matches!(object.kind, ObjectKind::Label(_)) => {
                        let ObjectKind::Label(label) = object.kind else {
                            unreachable!("matched above")
                        };
                        let title = node_name(editor.map(), id);
                        self.object_header(ui, editor, &[id], &title, "Label");
                        self.label_properties(ui, editor, id, &label);
                    }
                    Some(Node::Object(object)) => {
                        self.object_properties(ui, editor, id, &object);
                    }
                    None => {}
                }
            }
            many => {
                let many = many.to_vec();
                let summary = selection_summary(editor.map(), &many);
                let all_objects = many
                    .iter()
                    .all(|&id| matches!(editor.map().node(id), Some(Node::Object(_))));
                if all_objects {
                    self.object_header(ui, editor, &many, &summary, "Objects");
                } else {
                    heading(ui, &summary);
                }
                // Landmasses and Lakes share Roughness and Shore style; a setting that differs
                // between them shows "Mixed", and an edit sets all of them.
                self.object_roughness(ui, editor, &many);
                self.object_shore(ui, editor, &many);
                self.combine(ui, editor);
            }
        }
        self.membership_row(ui, editor);
    }

    /// A Landmass, Lake or River: the header with the lock, a River's own settings, its Fill,
    /// then Roughness (not for a River) and Shore style, and Combine for a Landmass.
    fn object_properties(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        id: Id,
        object: &ssmap_core::Object,
    ) {
        let title = node_name(editor.map(), id);
        let noun = match object.kind {
            ObjectKind::Lake(_) => "Lake",
            ObjectKind::River(_) => "River",
            _ => "Landmass",
        };
        self.object_header(ui, editor, &[id], &title, noun);
        if let ObjectKind::River(river) = &object.kind {
            self.river_section(ui, editor, id, river);
        }
        if let Some(fill) = object.kind.fill() {
            let header = SectionHeader {
                kind: "appearance",
                title: "Appearance",
                badge: None,
                action: None,
            };
            prop_widgets::section(ui, header, |ui| self.fill(ui, editor, id, fill));
        }
        self.object_roughness(ui, editor, &[id]);
        self.object_shore(ui, editor, &[id]);
        if matches!(object.kind, ObjectKind::Landmass(_)) {
            self.combine(ui, editor);
        }
    }

    /// The fill swatch, whose popover changes the fill live. Everything picked until the
    /// pointer is released is one Undo step.
    fn fill(&mut self, ui: &mut egui::Ui, editor: &mut Editor, id: Id, fill: Color) {
        let mut colour = Some(fill);
        let response = prop_widgets::swatch(ui, "Fill", &mut colour, false, Provenance::Plain);
        let fill = colour.unwrap_or(fill);
        self.live
            .apply(editor, response.edit, "Change Fill", || Command::SetFill {
                ids: vec![id],
                fill,
            });
    }

    /// Union, Subtract and Split Apart, with the same rules as the Edit actions.
    fn combine(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        let header = SectionHeader {
            kind: "combine",
            title: "Combine",
            badge: None,
            action: None,
        };
        prop_widgets::section(ui, header, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(theme::SPACING[1]);
                let combine = editor.can_combine();
                if prop_widgets::icon_action_enabled(
                    ui,
                    Icon::Union,
                    "Union",
                    combine,
                    "Merge the selected Landmasses into the topmost one",
                ) {
                    editor.union_selection();
                }
                if prop_widgets::icon_action_enabled(
                    ui,
                    Icon::Subtract,
                    "Subtract",
                    combine,
                    "Cut the topmost selected Landmass out of the others",
                ) {
                    editor.subtract_selection();
                }
                if prop_widgets::icon_action_enabled(
                    ui,
                    Icon::SplitApart,
                    "Split Apart",
                    editor.can_split(),
                    "Make each part of the selected Landmasses a Landmass of its own",
                ) {
                    editor.split_selection();
                }
            });
            caption(
                ui,
                "Subtract removes the top Landmass from the one below it.",
            );
        });
    }
}

/// "Layer › Object" for a selected Object, or the Layer's own name.
fn breadcrumb(ui: &mut egui::Ui, map: &Map, id: Id) {
    let text = match (map.node(id), map.parent(id)) {
        (Some(Node::Object(_)), Some(layer)) => {
            format!("{} › {}", node_name(map, layer), node_name(map, id))
        }
        _ => node_name(map, id),
    };
    heading(ui, &text);
}

fn heading(ui: &mut egui::Ui, text: &str) {
    let palette = theme::palette(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_space(theme::SPACING[3]);
        ui.label(
            egui::RichText::new(text)
                .size(theme::BODY)
                .color(palette.text_secondary),
        );
    });
}

fn caption(ui: &mut egui::Ui, text: &str) {
    let palette = theme::palette(ui.ctx());
    ui.label(
        egui::RichText::new(text)
            .size(theme::CAPTION)
            .color(palette.text_tertiary),
    );
}

/// A label with something other than a scrub field next to it.
fn row(ui: &mut egui::Ui, label: &str, body: impl FnOnce(&mut egui::Ui)) {
    let palette = theme::palette(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_sized(
            [64.0, 22.0],
            egui::Label::new(
                egui::RichText::new(label)
                    .size(theme::BODY)
                    .color(palette.text_secondary),
            ),
        );
        body(ui);
    });
}

/// "2 Landmasses + 1 Lake": what a multi-selection holds.
fn selection_summary(map: &Map, ids: &[Id]) -> String {
    const KINDS: [(&str, &str); 6] = [
        ("Layer", "Layers"),
        ("Landmass", "Landmasses"),
        ("Lake", "Lakes"),
        ("River", "Rivers"),
        ("Stamp", "Stamps"),
        ("Label", "Labels"),
    ];
    let mut counts = [0usize; 6];
    for &id in ids {
        let kind = match map.node(id) {
            Some(Node::Layer(_)) => 0,
            Some(Node::Object(object)) => match object.kind {
                ObjectKind::Landmass(_) => 1,
                ObjectKind::Lake(_) => 2,
                ObjectKind::River(_) => 3,
                ObjectKind::Stamp(_) => 4,
                ObjectKind::Label(_) => 5,
            },
            None => continue,
        };
        counts[kind] += 1;
    }
    KINDS
        .iter()
        .zip(counts)
        .filter(|&(_, n)| n > 0)
        .map(|(&(one, many), n)| format!("{n} {}", if n == 1 { one } else { many }))
        .collect::<Vec<_>>()
        .join(" + ")
}
