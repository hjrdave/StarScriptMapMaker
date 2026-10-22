//! The properties panel: shows and edits what is selected, through Commands.

mod label_stamp;
mod layer;
mod map;
mod object_style;

use eframe::egui;
use ssmap_core::{Color, Command, Id, Map, MapTemplate, Node, ObjectKind};

use crate::editor::Editor;
use crate::layer_tree::node_name;
use crate::prop_widgets::{self, Edit, LiveEdit, Provenance, SectionHeader};
use crate::theme;
use crate::tool_section::ToolSection;

#[derive(Default)]
pub struct PropertiesPanel {
    /// The text being typed for a Label, committed when the field loses focus.
    text_edit: Option<(Id, String)>,
    /// Turns the scrubs, sliders and swatches into one Undo step each.
    live: LiveEdit,
    /// "Manage Label styles…" was clicked; the window is opened by the owner of the tab.
    manage_label_styles: bool,
    /// The active tool's settings, above the selection.
    tool_section: ToolSection,
    /// The Canvas point that stays put when it is resized, as (column, row) of the 3×3
    /// anchor grid: 0 is left / top, 1 the middle, 2 right / bottom.
    canvas_anchor: (u8, u8),
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
    pub fn take_manage_label_styles(&mut self) -> bool {
        std::mem::take(&mut self.manage_label_styles)
    }

    /// Returns true when the collapse control at the bottom was clicked.
    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) -> bool {
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
                self.tool_section.show(ui, editor);
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
        match editor.selection() {
            [] if editor.board_selected() => {
                let canvas = editor.map.canvas.clone();
                heading(ui, "Board");
                self.canvas_fields(ui, editor, &canvas);
            }
            [] => {
                if let Some(layer) = editor.active_layer().filter(|&id| editor.map.contains(id))
                    && let Some(Node::Layer(data)) = editor.map.node(layer).cloned()
                {
                    breadcrumb(ui, &editor.map, layer);
                    self.layer_properties(ui, editor, layer, &data);
                }
                self.map_properties(ui, editor);
            }
            [id] => {
                let id = *id;
                breadcrumb(ui, &editor.map, id);
                match editor.map.node(id).cloned() {
                    Some(Node::Layer(layer)) => self.layer_properties(ui, editor, id, &layer),
                    Some(Node::Object(object)) if matches!(object.kind, ObjectKind::Label(_)) => {
                        let ObjectKind::Label(label) = object.kind else {
                            unreachable!("matched above")
                        };
                        self.label_properties(ui, editor, id, object.locked, &label);
                    }
                    Some(Node::Object(object)) if matches!(object.kind, ObjectKind::Stamp(_)) => {
                        let ObjectKind::Stamp(stamp) = object.kind else {
                            unreachable!("matched above")
                        };
                        self.stamp_properties(ui, editor, id, object.locked, &stamp);
                    }
                    Some(Node::Object(object)) => {
                        self.object_properties(ui, editor, id, &object);
                    }
                    None => {}
                }
            }
            many => {
                heading(ui, &selection_summary(&editor.map, many));
                // Landmasses and Lakes share Roughness and Shore style; a setting that differs
                // between them shows "Mixed", and an edit sets all of them.
                let many = many.to_vec();
                self.object_roughness(ui, editor, &many);
                self.object_shore(ui, editor, &many);
                self.combine(ui, editor);
            }
        }
    }

    /// A Landmass or Lake: Locked and Fill, then Roughness and Shore style, and Combine for a
    /// Landmass.
    fn object_properties(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        id: Id,
        object: &ssmap_core::Object,
    ) {
        let title = node_name(&editor.map, id);
        let header = SectionHeader {
            kind: "object",
            title: &title,
            badge: None,
            action: None,
        };
        prop_widgets::section(ui, header, |ui| {
            let mut locked = object.locked;
            if prop_widgets::checkbox_row(ui, "Locked", &mut locked) {
                editor.run(Command::SetLocked { id, locked });
            }
            if let Some(fill) = object.kind.fill() {
                self.fill(ui, editor, id, fill);
            }
        });
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
                if ui
                    .add_enabled(editor.can_combine(), egui::Button::new("Union"))
                    .on_hover_text("Merge the selected Landmasses into the topmost one")
                    .clicked()
                {
                    editor.union_selection();
                }
                if ui
                    .add_enabled(editor.can_combine(), egui::Button::new("Subtract"))
                    .on_hover_text("Cut the topmost selected Landmass out of the others")
                    .clicked()
                {
                    editor.subtract_selection();
                }
                if ui
                    .add_enabled(editor.can_split(), egui::Button::new("Split Apart"))
                    .on_hover_text(
                        "Make each part of the selected Landmasses a Landmass of its own",
                    )
                    .clicked()
                {
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
    const KINDS: [(&str, &str); 5] = [
        ("Layer", "Layers"),
        ("Landmass", "Landmasses"),
        ("Lake", "Lakes"),
        ("Stamp", "Stamps"),
        ("Label", "Labels"),
    ];
    let mut counts = [0usize; 5];
    for &id in ids {
        let kind = match map.node(id) {
            Some(Node::Layer(_)) => 0,
            Some(Node::Object(object)) => match object.kind {
                ObjectKind::Landmass(_) => 1,
                ObjectKind::Lake(_) => 2,
                ObjectKind::Stamp(_) => 3,
                ObjectKind::Label(_) => 4,
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

#[cfg(test)]
mod tests {
    use super::map::anchor_name;
    use super::*;
    use crate::tools::ToolId;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT as _, Queryable};
    use ssmap_core::{Canvas, DistanceUnit, Label, MultiPolygon, Polygon, Roughness, Scale};

    struct Fixture {
        editor: Editor,
        panel: PropertiesPanel,
    }

    /// A Map with one Layer holding one Landmass; nothing is selected.
    fn fixture() -> (Fixture, Id, Id) {
        let mut editor = Editor::new(Map::new(Canvas::new(
            100,
            100,
            Scale::new(1.0, DistanceUnit::Miles),
        )));
        let layer = editor.new_layer().expect("a Layer");
        let island = Id::new();
        assert!(editor.run(Command::CreateLandmass {
            id: island,
            parent: layer,
            index: None,
            geometry: square(0.0, 40.0),
            fill: None,
        }));
        editor.select(Vec::new());
        let panel = PropertiesPanel::default();
        (Fixture { editor, panel }, layer, island)
    }

    fn square(min: f64, size: f64) -> MultiPolygon {
        MultiPolygon(vec![Polygon {
            exterior: vec![
                [min, min],
                [min + size, min],
                [min + size, min + size],
                [min, min + size],
            ],
            holes: Vec::new(),
        }])
    }

    fn harness(fixture: Fixture) -> Harness<'static, Fixture> {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(320.0, 900.0))
            .build_ui_state(
                |ui, fixture: &mut Fixture| {
                    theme::install(&ui.ctx().clone());
                    fixture.panel.show(ui, &mut fixture.editor);
                },
                fixture,
            );
        harness.run();
        harness
    }

    #[test]
    fn layer_name_visibility_and_lock_are_managed_in_the_layers_panel() {
        let (fixture, _, _) = fixture();
        let harness = harness(fixture);
        for label in ["Name", "Visible", "Locked"] {
            assert!(harness.query_by_label(label).is_none(), "{label}");
        }
    }

    #[test]
    fn nothing_selected_shows_the_active_layer_and_a_collapsed_map_section() {
        let (fixture, _, _) = fixture();
        let harness = harness(fixture);
        // The active Layer's fields.
        for label in ["Depth sort", "Opacity", "Clip to"] {
            assert!(harness.query_by_label(label).is_some(), "{label}");
        }
        // The Map section is there, closed: its fields are not.
        assert!(harness.query_by_label("Map").is_some());
        assert!(harness.query_by_label("Canvas width").is_none());
        assert!(harness.query_by_label("Amount").is_none());
    }

    #[test]
    fn an_overridden_roughness_resets_to_the_map_default() {
        let (mut fixture, _, island) = fixture();
        let default = fixture.editor.map.defaults().roughness.clone();
        let own = Roughness::new(default.amount + 5.0, default.scale, default.seed);
        assert!(fixture.editor.run(Command::SetRoughness {
            ids: vec![island],
            roughness: Some(own),
        }));
        fixture.editor.select(vec![island]);
        let mut harness = harness(fixture);

        harness.get_by_label("Reset Amount").click();
        harness.run();
        let editor = &harness.state().editor;
        let Some(Node::Object(object)) = editor.map.node(island) else {
            panic!("an Object")
        };
        assert_eq!(object.kind.roughness(), None);
        // Inherited again: no reset arrow.
        assert!(harness.query_by_label("Reset Amount").is_none());
    }

    #[test]
    fn re_rolling_overrides_the_roughness_with_another_seed() {
        let (mut fixture, _, island) = fixture();
        fixture.editor.select(vec![island]);
        let mut harness = harness(fixture);

        harness.get_by_label("Re-roll").click();
        harness.run();
        let editor = &harness.state().editor;
        let default = editor.map.defaults().roughness.clone();
        let Some(Node::Object(object)) = editor.map.node(island) else {
            panic!("an Object")
        };
        let own = object.kind.roughness().expect("an override");
        assert_eq!((own.amount, own.scale), (default.amount, default.scale));
        assert_ne!(own.seed, default.seed);
        assert_eq!(editor.undo_label(), Some("Change Roughness"));
    }

    #[test]
    fn combine_is_disabled_with_one_landmass() {
        let (mut fixture, _, island) = fixture();
        fixture.editor.select(vec![island]);
        let harness = harness(fixture);
        assert!(harness.get_by_label("Union").accesskit_node().is_disabled());
        assert!(
            harness
                .get_by_label("Subtract")
                .accesskit_node()
                .is_disabled()
        );
        assert!(
            harness
                .query_by_label("Subtract removes the top Landmass from the one below it.")
                .is_some()
        );
    }

    #[test]
    fn a_multi_selection_is_summarised_and_can_be_combined() {
        let (mut fixture, layer, island) = fixture();
        let other = Id::new();
        assert!(fixture.editor.run(Command::CreateLandmass {
            id: other,
            parent: layer,
            index: None,
            geometry: square(10.0, 50.0),
            fill: None,
        }));
        fixture.editor.select(vec![island, other]);
        let mut harness = harness(fixture);
        assert!(harness.query_by_label("2 Landmasses").is_some());
        assert!(!harness.get_by_label("Union").accesskit_node().is_disabled());

        harness.get_by_label("Union").click();
        harness.run();
        assert_eq!(harness.state().editor.undo_label(), Some("Union"));
    }

    #[test]
    fn an_overridden_label_size_resets_to_the_styles_size() {
        let (mut fixture, layer, _) = fixture();
        let style_size = Label::default()
            .resolve(fixture.editor.map.label_styles())
            .size;
        let label = Id::new();
        assert!(fixture.editor.run(Command::CreateLabel {
            id: label,
            parent: layer,
            index: None,
            label: Box::new(Label {
                text: "Hello".to_owned(),
                size: Some(style_size + 10.0),
                ..Label::default()
            }),
        }));
        fixture.editor.select(vec![label]);
        let mut harness = harness(fixture);
        harness.get_by_label("Reset Size").click();
        harness.run();
        let editor = &harness.state().editor;
        let Some(Node::Object(object)) = editor.map.node(label) else {
            panic!("an Object")
        };
        let ObjectKind::Label(stored) = &object.kind else {
            panic!("a Label")
        };
        assert_eq!(stored.size, None);
        assert_eq!(stored.resolve(editor.map.label_styles()).size, style_size);
        // Inherited again: no reset arrow.
        assert!(harness.query_by_label("Reset Size").is_none());
    }

    #[test]
    fn manage_label_styles_asks_for_the_window() {
        let (mut fixture, layer, _) = fixture();
        let label = Id::new();
        assert!(fixture.editor.run(Command::CreateLabel {
            id: label,
            parent: layer,
            index: None,
            label: Box::new(Label {
                text: "Hello".to_owned(),
                ..Label::default()
            }),
        }));
        fixture.editor.select(vec![label]);
        let mut harness = harness(fixture);
        harness.get_by_label("Manage Label styles…").click();
        harness.run();
        assert!(harness.state_mut().panel.take_manage_label_styles());
    }

    #[test]
    fn shuffle_variant_in_properties_stays_within_the_variant_group() {
        use ssmap_core::asset::{AssetRef, PackLibrary, PackRoots};
        use ssmap_render::SpriteStore;
        let roots = PackRoots {
            built_in: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../crates/core/tests/packs"),
            user: None,
        };
        let (library, problems) = PackLibrary::load(&roots);
        assert!(problems.is_empty(), "{problems:?}");
        let (mut fixture, _, _) = fixture();
        fixture
            .editor
            .set_sprites(SpriteStore::new(std::sync::Arc::new(library)));
        fixture.editor.selected_asset = Some("test-pack/mountain-1".parse().unwrap());
        let stamp = fixture
            .editor
            .place_stamp([10.0, 10.0])
            .expect("a Stamp is placed");
        fixture.editor.select(vec![stamp]);
        // Placing picks any Asset of the Variant group.
        let before = fixture.editor.stamp(stamp).expect("a Stamp").asset.clone();
        let caption = fixture.editor.asset_caption(&before);
        assert!(caption == "Mountain · v1" || caption == "Mountain · v2");
        let mut harness = harness(fixture);
        assert!(harness.query_by_label(&caption).is_some());

        // The pick is random and may repeat the same Variant, so click until it changes.
        let group: Vec<AssetRef> = ["test-pack/mountain-1", "test-pack/mountain-2"]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
        let mut asset = before.clone();
        for _ in 0..64 {
            harness.get_by_label("Shuffle variant").click();
            harness.run();
            asset = harness
                .state()
                .editor
                .stamp(stamp)
                .expect("a Stamp")
                .asset
                .clone();
            assert!(group.contains(&asset));
            if asset != before {
                break;
            }
        }
        assert_ne!(asset, before);
        assert_eq!(harness.state().editor.undo_label(), Some("Shuffle Variant"));
    }

    /// The test pack installed in the Editor, optionally flagged `attribution_required`.
    fn install_test_pack(editor: &mut Editor, attribution_required: bool) {
        use ssmap_core::asset::{PackLibrary, PackSource, parse_manifest};
        use ssmap_render::SpriteStore;
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/core/tests/packs/test-pack/pack.toml");
        let mut text = std::fs::read_to_string(&path).unwrap();
        if attribution_required {
            text = text.replacen("[pack]\n", "[pack]\nattribution_required = true\n", 1);
        }
        let pack = parse_manifest(&path, &text, PackSource::BuiltIn).unwrap();
        let mut library = PackLibrary::default();
        let mut problems = Vec::new();
        library.add(pack, &mut problems);
        assert!(problems.is_empty(), "{problems:?}");
        editor.set_sprites(SpriteStore::new(std::sync::Arc::new(library)));
    }

    #[test]
    fn stamp_shows_the_picker_and_scatter_settings_and_select_shows_no_tool_section() {
        let (mut fixture, _, _) = fixture();
        install_test_pack(&mut fixture.editor, false);
        fixture.editor.active_tool = ToolId::Stamp;
        let mut harness = harness(fixture);
        for label in [
            "Stamp",
            "Size",
            "Place on",
            "Density",
            "Spacing",
            "Size jitter",
            "Rotation jitter",
            "Random flip",
            "Random variant",
        ] {
            assert!(harness.query_by_label(label).is_some(), "{label}");
        }
        assert!(harness.query_by_label("Tree (test-pack/tree)").is_some());

        harness.state_mut().editor.active_tool = ToolId::Select;
        harness.run();
        for label in ["Density", "Tree (test-pack/tree)"] {
            assert!(harness.query_by_label(label).is_none(), "{label}");
        }
    }

    #[test]
    fn picking_a_thumbnail_arms_its_asset() {
        let (mut fixture, _, _) = fixture();
        install_test_pack(&mut fixture.editor, false);
        fixture.editor.active_tool = ToolId::Stamp;
        let mut harness = harness(fixture);
        assert_eq!(harness.state().editor.selected_asset, None);
        harness.get_by_label("Tree (test-pack/tree)").click();
        harness.run();
        assert_eq!(
            harness.state().editor.selected_asset,
            Some("test-pack/tree".parse().unwrap())
        );
        assert_eq!(harness.state().editor.active_tool, ToolId::Stamp);
        // The picked Asset's pack · author · licence.
        assert!(
            harness
                .query_by_label_contains("Test pack · StarScript Map Maker contributors · CC0-1.0")
                .is_some()
        );
    }

    #[test]
    fn variants_show_their_count_and_attribution_shows_the_dot() {
        let (mut fixture, _, _) = fixture();
        install_test_pack(&mut fixture.editor, true);
        fixture.editor.active_tool = ToolId::Stamp;
        let mut harness = harness(fixture);
        assert!(
            harness
                .query_by_label("Tree (test-pack/tree), attribution required")
                .is_some()
        );
        assert!(
            harness
                .query_by_label("Mountain (test-pack/mountain-1), 2 variants, attribution required")
                .is_some()
        );
        harness
            .get_by_label("Tree (test-pack/tree), attribution required")
            .click();
        harness.run();
        assert!(
            harness
                .query_by_label_contains("Attribution required")
                .is_some()
        );
    }

    #[test]
    fn no_dot_without_attribution_and_an_empty_search_offers_a_way_out() {
        let (mut fixture, _, _) = fixture();
        install_test_pack(&mut fixture.editor, false);
        fixture.editor.active_tool = ToolId::Stamp;
        let mut harness = harness(fixture);
        assert!(
            harness
                .query_by_label_contains("attribution required")
                .is_none()
        );
        // The search field is the first text field, above the Layer's name.
        for step in 0..2 {
            let search = harness
                .get_all_by_role(egui::accesskit::Role::TextInput)
                .next()
                .expect("the search field");
            if step == 0 {
                search.click();
            } else {
                search.type_text("zzz");
            }
            harness.run();
        }
        harness.run();
        assert!(harness.query_by_label("No Assets match “zzz”").is_some());
        harness.get_by_label("Clear search").click();
        harness.run();
        assert!(harness.query_by_label("Tree (test-pack/tree)").is_some());
    }

    #[test]
    fn the_land_brush_section_sets_the_default_mode() {
        let (mut fixture, _, _) = fixture();
        fixture.editor.active_tool = ToolId::LandBrush;
        let mut harness = harness(fixture);
        assert!(harness.query_by_label("Size").is_some());
        harness.get_by_label("Erase").click();
        harness.run();
        assert!(harness.state().editor.erase_default);
        harness.get_by_label("Paint").click();
        harness.run();
        assert!(!harness.state().editor.erase_default);
    }

    #[test]
    fn tools_without_settings_have_no_section_and_the_eraser_has_only_size() {
        let (mut fixture, _, _) = fixture();
        fixture.editor.active_tool = ToolId::StampEraser;
        let mut harness = harness(fixture);
        assert!(harness.query_by_label("Stamp Eraser").is_some());
        assert!(harness.query_by_label("Size").is_some());
        assert!(harness.query_by_label("Density").is_none());
        for tool in [
            ToolId::Select,
            ToolId::Pan,
            ToolId::Lasso,
            ToolId::Label,
            ToolId::ExportRegion,
        ] {
            harness.state_mut().editor.active_tool = tool;
            harness.run();
            assert!(harness.query_by_label(tool.label()).is_none(), "{tool:?}");
        }
    }

    #[test]
    fn the_summary_counts_each_kind() {
        let (fixture, layer, island) = fixture();
        let map = &fixture.editor.map;
        assert_eq!(selection_summary(map, &[island]), "1 Landmass");
        assert_eq!(
            selection_summary(map, &[island, layer]),
            "1 Layer + 1 Landmass"
        );
        assert_eq!(selection_summary(map, &[]), "");
    }

    fn open_map_section(harness: &mut Harness<'static, Fixture>) {
        harness.get_by_label("Map").click();
        harness.run();
    }

    fn click(harness: &mut Harness<'static, Fixture>, label: &str) {
        harness.get_by_label(label).click();
        harness.run();
    }

    fn geometry_of(fixture: &Fixture, id: Id) -> String {
        format!("{:?}", fixture.editor.map.node(id))
    }

    #[test]
    fn resizing_from_each_anchor_keeps_that_point_fixed_and_moves_no_object() {
        for y in 0..3u8 {
            for x in 0..3u8 {
                let (fixture, _, island) = fixture();
                let before = geometry_of(&fixture, island);
                let mut harness = harness(fixture);
                open_map_section(&mut harness);
                click(&mut harness, &anchor_name(x, y));
                click(&mut harness, "Decrease Canvas width");
                click(&mut harness, "Increase Canvas height");
                let state = harness.state();
                let canvas = &state.editor.map.canvas;
                assert_eq!((canvas.width, canvas.height), (90, 110));
                let expected = [f64::from(x) * 10.0 / 2.0, f64::from(y) * -10.0 / 2.0];
                assert_eq!(canvas.origin, expected, "anchor {x},{y}");
                assert_eq!(geometry_of(state, island), before);
            }
        }
    }

    #[test]
    fn shrinking_past_objects_and_growing_back_then_undo_and_redo_touch_no_object() {
        let (fixture, _, island) = fixture();
        let before = geometry_of(&fixture, island);
        let mut harness = harness(fixture);
        open_map_section(&mut harness);
        // Anchor bottom-right: shrinking pulls the left edge past the Landmass at 0..40.
        click(&mut harness, &anchor_name(2, 2));
        for _ in 0..7 {
            click(&mut harness, "Decrease Canvas width");
        }
        let state = harness.state();
        assert_eq!(state.editor.map.canvas.width, 30);
        assert_eq!(state.editor.map.canvas.origin[0], 70.0);
        assert_eq!(geometry_of(state, island), before);
        for _ in 0..7 {
            click(&mut harness, "Increase Canvas width");
        }
        let state = harness.state();
        assert_eq!(state.editor.map.canvas.width, 100);
        assert_eq!(state.editor.map.canvas.origin[0], 0.0);
        assert_eq!(geometry_of(state, island), before);

        // Each click was one undo step, and undo only moves the Canvas.
        harness.state_mut().editor.undo();
        assert_eq!(harness.state().editor.map.canvas.width, 90);
        for _ in 0..13 {
            harness.state_mut().editor.undo();
            assert_eq!(geometry_of(harness.state(), island), before);
        }
        assert_eq!(harness.state().editor.map.canvas.width, 100);
        for _ in 0..14 {
            harness.state_mut().editor.redo();
            assert_eq!(geometry_of(harness.state(), island), before);
        }
        assert_eq!(harness.state().editor.map.canvas.width, 100);
    }

    /// Types `text` into the number box that shows `shown`.
    fn type_into(harness: &mut Harness<'static, Fixture>, shown: &str, text: &str) {
        harness.get_by_value(shown).click();
        harness.run();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.run();
        harness
            .get_by(|n| n.role() == egui::accesskit::Role::TextInput && n.is_focused())
            .type_text(text);
        harness.key_press(egui::Key::Enter);
        harness.run();
    }

    fn ocean_fixture() -> Fixture {
        let (mut fixture, _, _) = fixture();
        fixture.editor.map.canvas.template = Some("Ocean".to_owned());
        fixture
    }

    fn look_of(name: &str) -> ssmap_core::Background {
        let templates = MapTemplate::load_all().0;
        let template = templates.iter().find(|t| t.name() == name).unwrap();
        template.background().clone()
    }

    #[test]
    fn map_settings_name_the_template_the_map_was_made_from() {
        let mut harness = harness(ocean_fixture());
        open_map_section(&mut harness);
        harness.get_by_label("Template");
        harness.get_by_label("Ocean");
    }

    #[test]
    fn taking_another_templates_look_is_one_undo_step_and_keeps_the_template() {
        let fixture = ocean_fixture();
        let original = fixture.editor.map.canvas.background.clone();
        let mut harness = harness(fixture);
        open_map_section(&mut harness);
        click(&mut harness, "Parchment look");
        let state = harness.state();
        assert_eq!(state.editor.map.canvas.background, look_of("Parchment"));
        assert_eq!(state.editor.map.canvas.template.as_deref(), Some("Ocean"));
        assert_eq!(state.editor.undo_label(), Some("Change Background"));

        // One undo step puts the whole look back.
        harness.state_mut().editor.undo();
        let canvas = &harness.state().editor.map.canvas;
        assert_eq!(canvas.background, original);
        assert_eq!(canvas.template.as_deref(), Some("Ocean"));
    }

    #[test]
    fn changing_one_background_field_leaves_the_rest_and_is_one_undo_step() {
        let fixture = ocean_fixture();
        let original = fixture.editor.map.canvas.background.clone();
        let mut harness = harness(fixture);
        open_map_section(&mut harness);
        type_into(&mut harness, "1.0", "2.5");
        let state = harness.state();
        let background = &state.editor.map.canvas.background;
        assert_eq!(background.texture.scale, 2.5);
        assert_eq!(background.water, original.water);
        assert_eq!(background.texture.strength, original.texture.strength);
        assert_eq!(state.editor.map.canvas.template.as_deref(), Some("Ocean"));

        // One undo step puts the whole background back.
        harness.state_mut().editor.undo();
        assert_eq!(harness.state().editor.map.canvas.background, original);
        harness.state_mut().editor.redo();
        let scale = harness.state().editor.map.canvas.background.texture.scale;
        assert_eq!(scale, 2.5);
    }

    #[test]
    fn a_changed_background_and_the_template_name_survive_save_and_reopen() {
        let mut harness = harness(ocean_fixture());
        open_map_section(&mut harness);
        click(&mut harness, "Winter look");
        let shown = format!("{:.1}", look_of("Winter").texture.scale);
        type_into(&mut harness, &shown, "2.5");
        let map = &harness.state().editor.map;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("changed.ssmap");
        ssmap_core::file::MapFile::create(&path, map, None).unwrap();
        let (_, reopened) = ssmap_core::file::MapFile::open(&path).unwrap();
        assert_eq!(reopened.canvas.background, map.canvas.background);
        assert_ne!(reopened.canvas.background, look_of("Ocean"));
        assert_eq!(reopened.canvas.template.as_deref(), Some("Ocean"));
    }
}
