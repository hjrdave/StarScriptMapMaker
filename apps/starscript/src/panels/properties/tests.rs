use super::map::anchor_name;
use super::*;
use crate::asset_browser::install_test_pack;
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
    assert!(
        editor
            .run(Command::CreateLandmass {
                id: island,
                parent: layer,
                index: None,
                geometry: square(0.0, 40.0),
                fill: None,
            })
            .is_ok()
    );
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
fn nothing_selected_shows_only_a_collapsed_map_section() {
    let (fixture, _, _) = fixture();
    let harness = harness(fixture);
    // The Layer's fields are in the Layers panel.
    for label in ["Depth sort", "Opacity", "Clip to"] {
        assert!(harness.query_by_label(label).is_none(), "{label}");
    }
    // The Map section is there, closed: its fields are not.
    assert!(harness.query_by_label("Map").is_some());
    assert!(harness.query_by_label("Canvas width").is_none());
    assert!(harness.query_by_label("Amount").is_none());
}

#[test]
fn a_selected_layer_shows_the_map_section_and_no_layer_section() {
    let (mut fixture, layer, _) = fixture();
    fixture.editor.select(vec![layer]);
    let harness = harness(fixture);
    for label in ["Depth sort", "Opacity", "Clip to", "Layer"] {
        assert!(harness.query_by_label(label).is_none(), "{label}");
    }
    assert!(harness.query_by_label("Map").is_some());
}

#[test]
fn an_overridden_roughness_resets_to_the_map_default() {
    let (mut fixture, _, island) = fixture();
    let default = fixture.editor.map().defaults().roughness.clone();
    let own = Roughness::new(default.amount + 5.0, default.scale, default.seed);
    assert!(
        fixture
            .editor
            .run(Command::SetRoughness {
                ids: vec![island],
                roughness: Some(own),
            })
            .is_ok()
    );
    fixture.editor.select(vec![island]);
    let mut harness = harness(fixture);

    harness.get_by_label("Reset Amount").click();
    harness.run();
    let editor = &harness.state().editor;
    let Some(Node::Object(object)) = editor.map().node(island) else {
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
    let default = editor.map().defaults().roughness.clone();
    let Some(Node::Object(object)) = editor.map().node(island) else {
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
    assert!(
        fixture
            .editor
            .run(Command::CreateLandmass {
                id: other,
                parent: layer,
                index: None,
                geometry: square(10.0, 50.0),
                fill: None,
            })
            .is_ok()
    );
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
        .resolve(fixture.editor.map().label_styles())
        .size;
    let label = Id::new();
    assert!(
        fixture
            .editor
            .run(Command::CreateLabel {
                id: label,
                parent: layer,
                index: None,
                label: Box::new(Label {
                    text: "Hello".to_owned(),
                    size: Some(style_size + 10.0),
                    ..Label::default()
                }),
            })
            .is_ok()
    );
    fixture.editor.select(vec![label]);
    let mut harness = harness(fixture);
    harness.get_by_label("Reset Size").click();
    harness.run();
    let editor = &harness.state().editor;
    let Some(Node::Object(object)) = editor.map().node(label) else {
        panic!("an Object")
    };
    let ObjectKind::Label(stored) = &object.kind else {
        panic!("a Label")
    };
    assert_eq!(stored.size, None);
    assert_eq!(stored.resolve(editor.map().label_styles()).size, style_size);
    // Inherited again: no reset arrow.
    assert!(harness.query_by_label("Reset Size").is_none());
}

#[test]
fn manage_label_styles_asks_for_the_window() {
    let (mut fixture, layer, _) = fixture();
    let label = Id::new();
    assert!(
        fixture
            .editor
            .run(Command::CreateLabel {
                id: label,
                parent: layer,
                index: None,
                label: Box::new(Label {
                    text: "Hello".to_owned(),
                    ..Label::default()
                }),
            })
            .is_ok()
    );
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
    let (name, variant) = fixture.editor.asset_card(&before);
    assert_eq!(name, "Mountain");
    assert!(variant == "Variant 1 of 2" || variant == "Variant 2 of 2");
    let mut harness = harness(fixture);
    assert!(harness.query_by_label(&variant).is_some());

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
#[test]
fn stamp_shows_its_asset_and_scatter_settings_and_select_shows_no_tool_section() {
    let (mut fixture, _, _) = fixture();
    install_test_pack(&mut fixture.editor, false);
    fixture.editor.active_tool = ToolId::Stamp;
    let mut harness = harness(fixture);
    for label in [
        "Stamp",
        "None chosen",
        "Choose Asset",
        "Size",
        "Rotation jitter",
        "Place on",
        "Randomise",
        "Flip",
        "Variant",
        "Density",
        "Spacing",
        "Size jitter",
    ] {
        assert!(harness.query_by_label(label).is_some(), "{label}");
    }
    // The picker itself is the Assets section of the sidebar.
    assert!(harness.query_by_label("Tree (test-pack/tree)").is_none());

    harness.state_mut().editor.active_tool = ToolId::Select;
    harness.run();
    for label in ["Density", "Choose Asset"] {
        assert!(harness.query_by_label(label).is_none(), "{label}");
    }
}

#[test]
fn the_stamp_section_names_the_chosen_asset_and_asks_for_the_assets_section() {
    let (mut fixture, _, _) = fixture();
    install_test_pack(&mut fixture.editor, false);
    fixture.editor.active_tool = ToolId::Stamp;
    fixture.editor.selected_asset = Some("test-pack/tree".parse().unwrap());
    let mut harness = harness(fixture);
    harness.get_by_label("Tree");
    assert!(!harness.state_mut().panel.take_open_assets());
    harness.get_by_label("Choose Asset: Tree").click();
    harness.run();
    assert!(harness.state_mut().panel.take_open_assets());
    assert!(!harness.state_mut().panel.take_open_assets());
}

#[test]
fn the_board_row_moves_selected_objects_between_boards_in_one_undo_step() {
    use ssmap_core::Membership;
    let (mut fixture, _, island) = fixture();
    let board = fixture.editor.new_art_board().unwrap();
    fixture.editor.select_board(None);
    fixture.editor.select(vec![island]);
    let member = |fixture: &Fixture| match fixture.editor.map().node(island) {
        Some(Node::Object(object)) => object.membership,
        _ => panic!("an Object"),
    };
    let mut harness = harness(fixture);
    assert_eq!(member(harness.state()), Membership::Canvas);
    harness.get_by_label("Board");
    harness.get_by_role(egui::accesskit::Role::ComboBox).click();
    harness.run();
    harness.get_by_label("Art board 1").click();
    harness.run();
    assert_eq!(member(harness.state()), Membership::Board(board));
    assert_eq!(
        harness.state().editor.undo_label(),
        Some("Move On or Off Canvas")
    );

    harness.get_by_role(egui::accesskit::Role::ComboBox).click();
    harness.run();
    harness.get_by_label("Off the boards").click();
    harness.run();
    assert_eq!(member(harness.state()), Membership::OffCanvas);
    harness.state_mut().editor.undo();
    assert_eq!(member(harness.state()), Membership::Board(board));
}

#[test]
fn the_board_row_says_mixed_for_objects_on_different_boards() {
    use ssmap_core::Membership;
    let (mut fixture, layer, island) = fixture();
    let board = fixture.editor.new_art_board().unwrap();
    let lake = Id::new();
    fixture
        .editor
        .run(Command::CreateLake {
            id: lake,
            parent: layer,
            index: None,
            geometry: square(50.0, 10.0),
            fill: None,
        })
        .unwrap();
    fixture
        .editor
        .run(Command::SetMembership {
            ids: vec![lake],
            membership: Membership::Board(board),
        })
        .unwrap();
    fixture.editor.select_board(None);
    fixture.editor.select(vec![island, lake]);
    let harness = harness(fixture);
    let combo = harness.get_by_role(egui::accesskit::Role::ComboBox);
    assert_eq!(combo.accesskit_node().value().as_deref(), Some("Mixed"));
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
    let map = &fixture.editor.map();
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
    format!("{:?}", fixture.editor.map().node(id))
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
            let canvas = &state.editor.map().canvas();
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
    assert_eq!(state.editor.map().canvas().width, 30);
    assert_eq!(state.editor.map().canvas().origin[0], 70.0);
    assert_eq!(geometry_of(state, island), before);
    for _ in 0..7 {
        click(&mut harness, "Increase Canvas width");
    }
    let state = harness.state();
    assert_eq!(state.editor.map().canvas().width, 100);
    assert_eq!(state.editor.map().canvas().origin[0], 0.0);
    assert_eq!(geometry_of(state, island), before);

    // Each click was one undo step, and undo only moves the Canvas.
    harness.state_mut().editor.undo();
    assert_eq!(harness.state().editor.map().canvas().width, 90);
    for _ in 0..13 {
        harness.state_mut().editor.undo();
        assert_eq!(geometry_of(harness.state(), island), before);
    }
    assert_eq!(harness.state().editor.map().canvas().width, 100);
    for _ in 0..14 {
        harness.state_mut().editor.redo();
        assert_eq!(geometry_of(harness.state(), island), before);
    }
    assert_eq!(harness.state().editor.map().canvas().width, 100);
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
    let mut canvas = fixture.editor.map().canvas().clone();
    canvas.template = Some("Ocean".to_owned());
    fixture.editor.map_mut().set_canvas(canvas);
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
    let original = fixture.editor.map().canvas().background.clone();
    let mut harness = harness(fixture);
    open_map_section(&mut harness);
    click(&mut harness, "Parchment look");
    let state = harness.state();
    assert_eq!(state.editor.map().canvas().background, look_of("Parchment"));
    assert_eq!(
        state.editor.map().canvas().template.as_deref(),
        Some("Ocean")
    );
    assert_eq!(state.editor.undo_label(), Some("Change Background"));

    // One undo step puts the whole look back.
    harness.state_mut().editor.undo();
    let canvas = &harness.state().editor.map().canvas();
    assert_eq!(canvas.background, original);
    assert_eq!(canvas.template.as_deref(), Some("Ocean"));
}

#[test]
fn changing_one_background_field_leaves_the_rest_and_is_one_undo_step() {
    let fixture = ocean_fixture();
    let original = fixture.editor.map().canvas().background.clone();
    let mut harness = harness(fixture);
    open_map_section(&mut harness);
    type_into(&mut harness, "1.0", "2.5");
    let state = harness.state();
    let background = &state.editor.map().canvas().background;
    assert_eq!(background.texture.scale, 2.5);
    assert_eq!(background.water, original.water);
    assert_eq!(background.texture.strength, original.texture.strength);
    assert_eq!(
        state.editor.map().canvas().template.as_deref(),
        Some("Ocean")
    );

    // One undo step puts the whole background back.
    harness.state_mut().editor.undo();
    assert_eq!(harness.state().editor.map().canvas().background, original);
    harness.state_mut().editor.redo();
    let scale = harness
        .state()
        .editor
        .map()
        .canvas()
        .background
        .texture
        .scale;
    assert_eq!(scale, 2.5);
}

#[test]
fn a_changed_background_and_the_template_name_survive_save_and_reopen() {
    let mut harness = harness(ocean_fixture());
    open_map_section(&mut harness);
    click(&mut harness, "Winter look");
    let shown = format!("{:.1}", look_of("Winter").texture.scale);
    type_into(&mut harness, &shown, "2.5");
    let map = &harness.state().editor.map();

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("changed.ssmap");
    ssmap_core::file::MapFile::create(&path, map, None).unwrap();
    let (_, reopened) = ssmap_core::file::MapFile::open(&path).unwrap();
    assert_eq!(reopened.canvas().background, map.canvas().background);
    assert_ne!(reopened.canvas().background, look_of("Ocean"));
    assert_eq!(reopened.canvas().template.as_deref(), Some("Ocean"));
}

/// A Stamp of the test pack's tree on the fixture's Layer, at `x`.
fn add_stamp(fixture: &mut Fixture, layer: Id, x: f64) -> Id {
    let id = Id::new();
    let stamp = ssmap_core::Stamp::new("test-pack/tree".parse().expect("an Asset ref"), [x, 10.0]);
    fixture
        .editor
        .run(Command::CreateStamp {
            id,
            parent: layer,
            index: None,
            stamp: Box::new(stamp),
        })
        .expect("a Stamp is made");
    id
}

fn stamp_of(harness: &Harness<'_, Fixture>, id: Id) -> ssmap_core::Stamp {
    harness.state().editor.stamp(id).expect("a Stamp").clone()
}

#[test]
fn a_selected_stamp_shows_the_header_transform_and_appearance_only() {
    let (mut fixture, layer, _) = fixture();
    install_test_pack(&mut fixture.editor, false);
    let stamp = add_stamp(&mut fixture, layer, 10.0);
    fixture.editor.select(vec![stamp]);
    let mut harness = harness(fixture);
    for label in [
        "Tree",
        "Lock Stamp",
        "Shuffle variant",
        "Replace Asset",
        "Transform",
        "X",
        "Y",
        "Scale",
        "Rotation",
        "Flip horizontal",
        "Flip vertical",
        "Board",
        "Appearance",
        "Original",
        "Tint",
        "Fill",
        "Opacity",
    ] {
        assert!(harness.query_by_label(label).is_some(), "{label}");
    }
    // The old fields are gone: the lock is an icon and the colours hide until chosen.
    for label in [
        "Locked",
        "Position X",
        "Tint colour",
        "Fill colour",
        "Own colour",
    ] {
        assert!(harness.query_by_label(label).is_none(), "{label}");
    }

    harness.get_by_label("Lock Stamp").click();
    harness.run();
    assert!(harness.query_by_label("Unlock Stamp").is_some());
    assert_eq!(harness.state().editor.undo_label(), Some("Lock"));

    assert!(!harness.state_mut().panel.take_open_assets());
    harness.get_by_label("Replace Asset").click();
    harness.run();
    assert!(harness.state_mut().panel.take_open_assets());
}

#[test]
fn the_colour_control_reveals_the_colour_of_the_option_chosen() {
    let (mut fixture, layer, _) = fixture();
    let stamp = add_stamp(&mut fixture, layer, 10.0);
    fixture.editor.select(vec![stamp]);
    let mut harness = harness(fixture);

    harness.get_by_label("Tint").click();
    harness.run();
    let placed = stamp_of(&harness, stamp);
    assert!(placed.tint.is_some() && placed.fill.is_none());
    assert!(harness.query_by_label("Tint colour").is_some());
    assert!(harness.query_by_label("Fill colour").is_none());

    harness.get_by_label("Fill").click();
    harness.run();
    let placed = stamp_of(&harness, stamp);
    assert!(placed.tint.is_none() && placed.fill.is_some());
    assert!(harness.query_by_label("Fill colour").is_some());
    assert!(harness.query_by_label("Tint colour").is_none());

    harness.get_by_label("Original").click();
    harness.run();
    let placed = stamp_of(&harness, stamp);
    assert!(placed.tint.is_none() && placed.fill.is_none());
    assert_eq!(harness.state().editor.undo_label(), Some("Edit Stamp"));
}

#[test]
fn the_flip_buttons_mirror_the_stamp_each_in_one_undo_step() {
    let (mut fixture, layer, _) = fixture();
    let stamp = add_stamp(&mut fixture, layer, 10.0);
    fixture.editor.select(vec![stamp]);
    let mut harness = harness(fixture);

    harness.get_by_label("Flip horizontal").click();
    harness.run();
    assert!(stamp_of(&harness, stamp).flip);
    harness.get_by_label("Flip horizontal").click();
    harness.run();
    assert!(!stamp_of(&harness, stamp).flip);

    harness.get_by_label("Flip vertical").click();
    harness.run();
    let placed = stamp_of(&harness, stamp);
    assert!(placed.flip);
    assert_eq!(placed.rotation, 180.0);
    harness.state_mut().editor.undo();
    let placed = stamp_of(&harness, stamp);
    assert!(!placed.flip);
    assert_eq!(placed.rotation, 0.0);
}

#[test]
fn several_stamps_share_one_section_with_mixed_fields_and_one_undo_step() {
    let (mut fixture, layer, _) = fixture();
    let a = add_stamp(&mut fixture, layer, 10.0);
    let b = add_stamp(&mut fixture, layer, 30.0);
    fixture.editor.select(vec![a, b]);
    let mut harness = harness(fixture);
    assert!(harness.query_by_label("2 Stamps").is_some());
    assert!(harness.query_by_label("Lock Stamps").is_some());
    // No Asset card for several Stamps.
    assert!(harness.query_by_label("Replace Asset").is_none());
    // X differs, Y does not.
    let mixed = harness
        .query_all(egui_kittest::kittest::by().value(prop_widgets::MIXED))
        .count();
    assert_eq!(mixed, 1);

    harness.get_by_label("Flip horizontal").click();
    harness.run();
    assert!(stamp_of(&harness, a).flip && stamp_of(&harness, b).flip);
    harness.state_mut().editor.undo();
    assert!(!stamp_of(&harness, a).flip && !stamp_of(&harness, b).flip);
}
