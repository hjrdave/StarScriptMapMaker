//! The properties panel: shows and edits what is selected, through Commands.

mod board;
mod header;
pub(crate) use board::BoardRequest;
mod label;
mod map;
mod object_style;
mod river;
mod shore;
mod shore_presets;
pub(crate) use shore_presets::install as install_shore_thumbnails;
mod stamp;
#[cfg(test)]
mod tests;

use eframe::egui;
use ssmap_core::{AssetRef, Color, Command, Id, Map, MapTemplate, Node, ObjectKind};

use self::object_style::{FillTarget, Frame};
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
    /// What the Stamp tool's Asset card asked for.
    tool_section: ToolSection,
    /// The Stamp section's "Replace Asset" was clicked; the window opens the Assets section.
    open_assets: bool,
    /// The selected Stamp's Asset and its thumbnail, loaded once per Asset.
    stamp_thumb: Option<(AssetRef, Option<ThumbArt>)>,
    /// The Canvas point that stays put when it is resized, as (column, row) of the 3×3
    /// anchor grid: 0 is left / top, 1 the middle, 2 right / bottom.
    canvas_anchor: (u8, u8),
    /// The board whose width and height are chained, and the aspect ratio (width over height)
    /// from when the chain was turned on, so repeated rounding cannot make it drift.
    chain_ratio: Option<(Id, f64)>,
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
    /// What each member inherits, where the fields show one by one which override it. `None`:
    /// every field shows `provenance`.
    defaults: Option<Defaults<'a, T>>,
}

/// The defaults behind the members of an [`Inherit`], one per member.
struct Defaults<'a, T> {
    /// The default member `index` falls back to.
    values: &'a [T],
    /// The Command that makes member `index` follow its default again.
    clear: &'a dyn Fn(usize) -> Command,
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

    /// How the setting `field` reads is shown: with defaults, overridden where it differs from
    /// the default of any member and plain where it follows them all; else `provenance`.
    fn provenance_of<V: PartialEq>(&self, field: impl Fn(&T) -> V) -> Provenance {
        let Some(defaults) = &self.defaults else {
            return self.provenance;
        };
        let differs = self
            .members
            .iter()
            .zip(defaults.values)
            .any(|(member, default)| field(member) != field(default));
        if differs {
            Provenance::Overridden
        } else {
            Provenance::Plain
        }
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

    /// The panel's header: the active tool's icon and name with its mode chip, while the tool
    /// has settings, is not Select or nothing is selected; with Select and a selection, the
    /// "Select" beside the Select tool's icon (the breadcrumb below names the selection).
    fn header(&self, ui: &mut egui::Ui, editor: &Editor) {
        let tool = editor.active_tool;
        let nothing = editor.selection().is_empty() && editor.selected_board().is_none();
        if !(crate::tool_section::has_settings(tool)
            || nothing
            || tool != crate::tools::ToolId::Select)
        {
            let tint = theme::palette(ui.ctx()).text_primary;
            // A selected Label is titled by its kind, with the Label tool's icon; the
            // breadcrumb under it carries its name.
            if let [id] = editor.selection()
                && is_label(editor.map(), *id)
            {
                crate::shell::icon_header(ui, Icon::Label, tint, "Label", |_| {});
                return;
            }
            // The breadcrumb under the header says which item is selected.
            let icon = crate::tool_section::icon(crate::tools::ToolId::Select);
            crate::shell::icon_header(ui, icon, tint, "Select", |_| {});
            return;
        }
        let alt = ui.input(|i| i.modifiers.alt);
        let header = crate::tool_section::tool_header(editor, alt);
        let palette = theme::palette(ui.ctx());
        let erasing = header.chip.is_some_and(|(_, erase)| erase);
        let tint = if erasing {
            palette.danger
        } else {
            palette.text_primary
        };
        crate::shell::icon_header(
            ui,
            crate::tool_section::icon(tool),
            tint,
            header.name,
            |ui| {
                if let Some((text, danger)) = header.chip {
                    crate::shell::mode_chip(ui, text, danger);
                }
            },
        );
    }

    pub(crate) fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        // A change ends when the pointer is released, or when Escape cancelled it.
        self.live.apply(editor, Edit::default(), "", || {
            unreachable!("nothing changed")
        });
        self.header(ui, editor);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // A tool with settings shows only those (the Stamp's with its Asset card), so
                // the selection's properties don't sit above them. Select, Pan, Label
                // and Canvas have none, and show the selection's.
                let tool = editor.active_tool;
                let alt = ui.input(|i| i.modifiers.alt);
                if crate::tool_section::has_settings(tool) {
                    if tool == crate::tools::ToolId::Stamp {
                        crate::tool_section::place_on(ui, editor);
                        self.tool_section.stamp_asset(ui, editor);
                        ui.add_space(theme::SPACING[2]);
                    }
                    crate::tool_section::tool_sections(ui, editor, alt);
                } else if tool == crate::tools::ToolId::Select || editor.selection().is_empty() {
                    // Other tools hide a selection but keep it for the way back; with none,
                    // the hint or a selected board still shows.
                    self.selection(ui, editor);
                }
            });
    }

    fn selection(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        // Stamps name themselves in their header and show the board under Transform.
        if let Some(members) = stamp::stamps(editor.map(), editor.selection()) {
            self.stamp_properties(ui, editor, &members);
            return;
        }
        match editor.selection() {
            [] => {
                if !crate::tool_section::has_settings(editor.active_tool) {
                    caption(
                        ui,
                        "Nothing selected. The Map's own settings are in the Map section of the left bar, and the Canvas and Art boards in the Art boards section.",
                    );
                }
            }
            [id] => {
                let id = *id;
                match editor.map().node(id).cloned() {
                    // A Layer's own fields are edited in the Layers panel.
                    Some(Node::Layer(_)) => {
                        breadcrumb(ui, editor.map(), id);
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
                self.object_roughness(ui, editor, &many, Frame::Section);
                self.object_shore(ui, editor, &many, Frame::Section);
                self.combine(ui, editor, Frame::Section);
            }
        }
        // A Label shows its Board inside its Appearance section, and a Landmass or Lake on
        // its Shape tab.
        let own_board = |id: Id| is_label(editor.map(), id) || is_tabbed(editor.map(), id);
        if !matches!(editor.selection(), [id] if own_board(*id)) {
            self.membership_row(ui, editor, false);
        }
    }

    /// A Landmass, Lake or River: the header with the lock, then for a River its own settings,
    /// Appearance and Shore style, and for a Landmass or Lake the Fill, Shore and Shape tabs.
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
            if let Some(fill) = object.kind.fill() {
                let header = SectionHeader {
                    kind: "appearance",
                    title: "Appearance",
                    badge: None,
                    action: None,
                };
                prop_widgets::section(ui, header, |ui| self.fill(ui, editor, id, fill));
            }
            self.object_shore(ui, editor, &[id], Frame::Section);
            return;
        }
        let landmass = matches!(object.kind, ObjectKind::Landmass(_));
        match object_tab(ui) {
            ObjectTab::Fill => {
                if let Some(fill) = object.kind.fill() {
                    ui.add_space(theme::SPACING[2]);
                    self.fill(ui, editor, id, fill);
                    if landmass {
                        let style = editor.map().fill_style(id);
                        self.fill_style(ui, editor, FillTarget::Landmass(id), style);
                    }
                }
            }
            ObjectTab::Shore => {
                ui.add_space(theme::SPACING[2]);
                self.object_shore(ui, editor, &[id], Frame::Tab);
            }
            ObjectTab::Shape => {
                self.object_roughness(ui, editor, &[id], Frame::Tab);
                if landmass {
                    ui.add_space(theme::SPACING[3]);
                    self.combine(ui, editor, Frame::Tab);
                }
                ui.add_space(theme::SPACING[3]);
                self.membership_row(ui, editor, false);
            }
        }
    }

    /// The fill swatch, whose popover changes the fill live. Everything picked until the
    /// pointer is released is one Undo step.
    fn fill(&mut self, ui: &mut egui::Ui, editor: &mut Editor, id: Id, fill: Color) {
        let mut colour = Some(fill);
        let response = prop_widgets::swatch_shown_as(
            ui,
            "Fill colour",
            "Colour",
            &mut colour,
            false,
            Provenance::Plain,
        );
        let fill = colour.unwrap_or(fill);
        self.live
            .apply(editor, response.edit, "Change Fill", || Command::SetFill {
                ids: vec![id],
                fill,
            });
    }

    /// Union, Subtract and Split Apart, with the same rules as the Edit actions.
    fn combine(&mut self, ui: &mut egui::Ui, editor: &mut Editor, frame: Frame) {
        let header = SectionHeader {
            kind: "combine",
            title: "Combine",
            badge: None,
            action: None,
        };
        let body = |ui: &mut egui::Ui| {
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
        };
        match frame {
            Frame::Section => {
                prop_widgets::section(ui, header, body);
            }
            Frame::Tab => prop_widgets::divider_section(ui, header.title, |_| {}, body),
        }
    }
}

/// The tabs of a selected Landmass or Lake.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ObjectTab {
    Fill,
    Shore,
    Shape,
}

impl ObjectTab {
    const ALL: [ObjectTab; 3] = [ObjectTab::Fill, ObjectTab::Shore, ObjectTab::Shape];
    const NAMES: [&str; 3] = ["Fill", "Shore", "Shape"];
}

/// Where the chosen tab is kept in UI memory: for as long as the app is open, and the same for
/// every Landmass and Lake, so selecting another keeps the tab.
fn object_tab_id() -> egui::Id {
    egui::Id::new("object panel tab")
}

/// The tab bar of a Landmass or Lake, and the tab chosen. Shore until another is picked.
fn object_tab(ui: &mut egui::Ui) -> ObjectTab {
    let id = object_tab_id();
    let mut tab = ui
        .ctx()
        .data(|d| d.get_temp::<usize>(id))
        .unwrap_or(1)
        .min(ObjectTab::ALL.len() - 1);
    if prop_widgets::tab_bar(ui, &ObjectTab::NAMES, &mut tab) {
        ui.ctx().data_mut(|d| d.insert_temp(id, tab));
    }
    ObjectTab::ALL[tab]
}

/// Whether `id` is a Landmass or Lake, which show their properties on tabs.
fn is_tabbed(map: &Map, id: Id) -> bool {
    matches!(
        map.node(id),
        Some(Node::Object(object))
            if matches!(object.kind, ObjectKind::Landmass(_) | ObjectKind::Lake(_))
    )
}

/// "Layer › Object" for a selected Object, or the Layer's own name.
/// Whether `id` is a Label.
fn is_label(map: &Map, id: Id) -> bool {
    matches!(
        map.node(id),
        Some(Node::Object(object)) if matches!(object.kind, ObjectKind::Label(_))
    )
}

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

/// A quiet caption that starts a group of rows inside a section ("Water", "Texture"): 8 above
/// and 4 below, sentence case. Painted, so it adds no accessible node.
fn group_caption(ui: &mut egui::Ui, text: &str) {
    let palette = theme::palette(ui.ctx());
    ui.add_space(theme::SPACING[3]);
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(theme::CAPTION),
        palette.text_tertiary,
    );
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), galley.size().y),
        egui::Sense::hover(),
    );
    ui.painter().galley(rect.min, galley, palette.text_tertiary);
    ui.add_space(theme::SPACING[1]);
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
