use super::map::anchor_name;
use super::*;
use crate::asset_browser::install_test_pack;
use crate::tools::ToolId;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable};
use ssmap_core::{Align, Canvas, DistanceUnit, Label, MultiPolygon, Polygon, Roughness, Scale};

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

/// The Map section of the left sidebar, which holds the Map's own settings.
fn map_harness(fixture: Fixture) -> Harness<'static, Fixture> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(320.0, 900.0))
        .build_ui_state(
            |ui, fixture: &mut Fixture| {
                theme::install(&ui.ctx().clone());
                fixture.panel.map_panel(ui, &mut fixture.editor);
            },
            fixture,
        );
    harness.run();
    harness
}

/// The Art boards section, which holds the Canvas and the Art boards.
fn boards_harness(fixture: Fixture) -> Harness<'static, Fixture> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(320.0, 900.0))
        .build_ui_state(
            |ui, fixture: &mut Fixture| {
                theme::install(&ui.ctx().clone());
                let mut list = crate::board_list::BoardList::default();
                fixture
                    .panel
                    .boards_panel(ui, &mut fixture.editor, &mut list);
            },
            fixture,
        );
    harness.run();
    harness
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
fn nothing_selected_shows_no_map_settings() {
    let (fixture, _, _) = fixture();
    let harness = harness(fixture);
    // The Layer's fields are in the Layers panel.
    for label in ["Depth sort", "Opacity", "Clip to"] {
        assert!(harness.query_by_label(label).is_none(), "{label}");
    }
    // The Map's fields are in the Map section of the left sidebar, not here.
    for label in ["Map", "Canvas width", "Amount"] {
        assert!(harness.query_by_label(label).is_none(), "{label}");
    }
}

#[test]
fn a_selected_layer_shows_no_map_section_and_no_layer_section() {
    let (mut fixture, layer, _) = fixture();
    fixture.editor.select(vec![layer]);
    let harness = harness(fixture);
    for label in ["Depth sort", "Opacity", "Clip to", "Layer"] {
        assert!(harness.query_by_label(label).is_none(), "{label}");
    }
    assert!(harness.query_by_label("Map").is_none());
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

fn fill_style_of(editor: &Editor, id: Id) -> ssmap_core::FillStyle {
    let Some(Node::Object(object)) = editor.map().node(id) else {
        panic!("an Object")
    };
    object.kind.fill_style()
}

#[test]
fn a_landmass_fill_switches_between_flat_and_grain() {
    let (mut fixture, _, island) = fixture();
    fixture.editor.select(vec![island]);
    let mut harness = harness(fixture);
    // Flat to begin with: no grain fields.
    harness.get_by_label("Flat");
    assert!(harness.query_by_label("Grain amount").is_none());

    harness.get_by_label("Grain").click();
    harness.run();
    let editor = &harness.state().editor;
    assert_eq!(
        fill_style_of(editor, island),
        ssmap_core::FillStyle::Grain(ssmap_core::Grain::DEFAULT)
    );
    assert_eq!(editor.undo_label(), Some("Change Fill Style"));
    harness.get_by_label("Grain amount");
    harness.get_by_label("Grain size");

    harness.get_by_label("Flat").click();
    harness.run();
    assert_eq!(
        fill_style_of(&harness.state().editor, island),
        ssmap_core::FillStyle::Flat
    );
    assert!(harness.query_by_label("Grain amount").is_none());
}

#[test]
fn a_lake_has_no_fill_style() {
    let (mut fixture, layer, _) = fixture();
    let lake = Id::new();
    assert!(
        fixture
            .editor
            .run(Command::CreateLake {
                id: lake,
                parent: layer,
                index: None,
                geometry: square(50.0, 20.0),
                fill: None,
            })
            .is_ok()
    );
    fixture.editor.select(vec![lake]);
    let harness = harness(fixture);
    assert!(harness.query_by_label("Grain").is_none());
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
    // In the title and in the breadcrumb row.
    assert!(harness.query_all_by_label("2 Landmasses").next().is_some());
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

/// A Label that overrides every field the panel marks, selected, with the Select tool.
fn fixture_with_overridden_label() -> (Fixture, Id) {
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
                    size: Some(48.0),
                    line_height: Some(1.4),
                    letter_spacing: Some(0.05),
                    arch: Some(0.25),
                    fill: Some(Color::rgb(43, 33, 24)),
                    opacity: Some(0.8),
                    halo: Some(ssmap_core::Halo {
                        width: 2.0,
                        color: Color::rgb(244, 234, 210),
                        ..ssmap_core::Halo::default()
                    }),
                    ..Label::default()
                }),
            })
            .is_ok()
    );
    fixture.editor.select(vec![label]);
    (fixture, label)
}

#[test]
fn align_is_one_choice_and_format_is_two_independent_toggles() {
    let (fixture, id) = fixture_with_overridden_label();
    let mut harness = harness(fixture);
    let resolved = |harness: &Harness<'_, Fixture>| {
        let editor = &harness.state().editor;
        editor
            .label(id)
            .expect("a Label")
            .resolve(editor.map().label_styles())
    };
    let (italic, caps, align) = {
        let r = resolved(&harness);
        (r.font.italic, r.all_caps, r.align)
    };
    assert_ne!(align, Align::Right);

    harness.get_by_label("Align right").click();
    harness.run();
    assert_eq!(resolved(&harness).align, Align::Right);
    assert_eq!(harness.state().editor.undo_label(), Some("Edit Label"));

    // Italic and All caps flip on their own, each in one Undo step.
    harness.get_by_label("Italic").click();
    harness.run();
    let r = resolved(&harness);
    assert_eq!((r.font.italic, r.all_caps), (!italic, caps));
    harness.get_by_label("All caps").click();
    harness.run();
    let r = resolved(&harness);
    assert_eq!((r.font.italic, r.all_caps), (!italic, !caps));
    harness.get_by_label("Italic").click();
    harness.run();
    let r = resolved(&harness);
    assert_eq!((r.font.italic, r.all_caps), (italic, !caps));
    // Align is untouched by Format.
    assert_eq!(r.align, Align::Right);
}

#[test]
fn a_selected_label_is_titled_label_not_properties() {
    let (fixture, _) = fixture_with_overridden_label();
    let harness = harness(fixture);
    assert!(harness.query_by_label("Properties").is_none());
    // The title, and the lock button's "Lock Label" are the only things named Label.
    assert!(harness.query_all_by_label("Label").next().is_some());
    // The breadcrumb keeps the Layer and the Pin and Lock buttons under the title.
    harness.get_by_label("Pin to Canvas");
    harness.get_by_label("Lock Label");
}

#[test]
fn a_landmass_keeps_the_properties_title() {
    let (mut fixture, _, island) = fixture();
    fixture.editor.select(vec![island]);
    let harness = harness(fixture);
    assert!(harness.query_all_by_label("Label").next().is_none());
}

#[test]
fn the_label_panel_fits_at_its_narrowest_width() {
    const NARROW: f32 = 240.0;
    let (fixture, _) = fixture_with_overridden_label();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(NARROW, 1600.0))
        .build_ui_state(
            |ui, fixture: &mut Fixture| {
                theme::install(&ui.ctx().clone());
                fixture.panel.show(ui, &mut fixture.editor);
            },
            fixture,
        );
    harness.run();
    // Every override shows its reset arrow here, the widest a row gets.
    harness.get_by_label("Reset Size");
    harness.get_by_label("Reset Halo colour");
    let clipped: Vec<_> = harness
        .root()
        .children_recursive()
        .filter(|node| node.accesskit_node().bounding_box().is_some())
        .map(|node| node.rect())
        .filter(|rect| rect.right() > NARROW + 0.5 || rect.left() < -0.5)
        .collect();
    assert!(clipped.is_empty(), "outside the panel: {clipped:?}");
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
        "Scatter",
        "Randomise",
        "None chosen",
        "Choose Asset",
        "Size",
        "Tilt jitter",
        "Place on",
        "Random",
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
fn a_tool_with_settings_hides_the_selection_s_properties() {
    // With a brush active, a selected Landmass's properties stay out of the way.
    let (mut fixture, _, island) = fixture();
    fixture.editor.select(vec![island]);
    fixture.editor.active_tool = ToolId::LandBrush;
    let mut harness = harness(fixture);
    for label in ["Brush", "Irregularity", "Paint", "Paint Lasso", "Erase"] {
        assert!(
            harness.query_all_by_label(label).next().is_some(),
            "{label}"
        );
    }
    for label in ["Roughness", "Shore style", "Combine"] {
        assert!(harness.query_by_label(label).is_none(), "{label}");
    }
    // The Select tool shows the selection's, and not the brush's.
    harness.state_mut().editor.active_tool = ToolId::Select;
    harness.run();
    for label in ["Roughness", "Shore style", "Combine"] {
        assert!(harness.query_by_label(label).is_some(), "{label}");
    }
    assert!(harness.query_by_label("Irregularity").is_none());
}

#[test]
fn the_stamp_tool_shows_its_settings_and_asset_whatever_is_selected() {
    let (mut fixture, layer, _) = fixture();
    install_test_pack(&mut fixture.editor, false);
    fixture.editor.active_tool = ToolId::Stamp;
    let stamp = add_stamp(&mut fixture, layer, 10.0);
    fixture.editor.select(vec![stamp]);
    let harness = harness(fixture);
    for label in ["Choose Asset", "Density", "Tilt jitter"] {
        assert!(harness.query_by_label(label).is_some(), "{label}");
    }
    // The selected Stamp's own sections wait for the Select tool.
    for label in ["Transform", "Appearance"] {
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
fn the_land_brush_mode_is_three_icon_buttons_that_set_the_mode() {
    use crate::editor::LandMode;
    let (mut fixture, _, _) = fixture();
    fixture.editor.active_tool = ToolId::LandBrush;
    let mut harness = harness(fixture);
    assert!(harness.query_by_label("Size").is_some());
    // The old Draw and Behaviour sections are gone.
    for label in ["Draw", "Behaviour", "Draw mode", "Mode"] {
        assert!(harness.query_by_label(label).is_none(), "{label}");
    }

    harness.get_by_label("Erase").click();
    harness.run();
    assert_eq!(harness.state().editor.land_mode(), LandMode::Erase);
    assert!(harness.state().editor.erase_default);
    harness.get_by_label("Paint Lasso").click();
    harness.run();
    assert_eq!(harness.state().editor.land_mode(), LandMode::PaintLasso);
    assert!(!harness.state().editor.erase_default);
    // The lasso hides the Brush section, as before.
    assert!(harness.query_by_label("Irregularity").is_none());
    harness.get_by_label("Paint").click();
    harness.run();
    assert_eq!(harness.state().editor.land_mode(), LandMode::Paint);
    assert!(harness.query_by_label("Irregularity").is_some());
}

#[test]
fn the_lake_brush_has_paint_and_erase_only_and_the_stamp_places_from_the_top() {
    let (mut fixture, _, _) = fixture();
    fixture.editor.active_tool = ToolId::WaterBrush;
    let mut harness = harness(fixture);
    harness.get_by_label("Paint");
    harness.get_by_label("Erase");
    assert!(harness.query_by_label("Paint Lasso").is_none());
    harness.get_by_label("Erase").click();
    harness.run();
    assert!(harness.state().editor.erase_default);

    harness.state_mut().editor.active_tool = ToolId::Stamp;
    harness.run();
    for label in ["Place on", "Choose Asset"] {
        harness.get_by_label(label);
    }
    // Place on is above the Asset card.
    let place = harness.get_by_label("Place on").rect().top();
    let asset = harness.get_by_label("Choose Asset").rect().top();
    assert!(place < asset);
}

#[test]
fn only_the_land_brush_has_irregularity() {
    let (mut fixture, _, _) = fixture();
    fixture.editor.active_tool = ToolId::LandBrush;
    let mut harness = harness(fixture);
    assert!(harness.query_by_label("Irregularity").is_some());
    harness.state_mut().editor.active_tool = ToolId::WaterBrush;
    harness.run();
    assert!(harness.query_by_label("Irregularity").is_none());
}

#[test]
fn tools_without_settings_have_no_section_and_the_eraser_has_only_size() {
    let (mut fixture, _, _) = fixture();
    fixture.editor.active_tool = ToolId::StampEraser;
    let mut harness = harness(fixture);
    assert!(harness.query_by_label("Eraser").is_some());
    assert!(harness.query_by_label("Size").is_some());
    assert!(harness.query_by_label("Density").is_none());
    for tool in [
        ToolId::Select,
        ToolId::Pan,
        ToolId::Label,
        ToolId::ExportRegion,
    ] {
        harness.state_mut().editor.active_tool = tool;
        harness.run();
        // The header names the tool; the body has no section.
        assert!(harness.query_by_label(tool.label()).is_some(), "{tool:?}");
        for section in ["Size", "Density", "Eraser", "Brush"] {
            assert!(
                harness.query_by_label(section).is_none(),
                "{tool:?} {section}"
            );
        }
    }
}

#[test]
fn the_header_names_the_tool_and_a_chip_names_its_mode() {
    let (mut fixture, _, _) = fixture();
    fixture.editor.active_tool = ToolId::LandBrush;
    let mut harness = harness(fixture);
    harness.get_by_label("Land Brush");
    harness.get_by_label("Mode: Paint");

    // The lasso and the erase default change the chip, as the stroke would change.
    harness.state_mut().editor.draw_mode = crate::editor::DrawMode::Lasso;
    harness.run();
    harness.get_by_label("Mode: Paint Lasso");
    harness.state_mut().editor.erase_default = true;
    harness.run();
    harness.get_by_label("Mode: Erase Lasso");

    // The Lake Brush has no lasso; the Stamp's chip is where it places.
    harness.state_mut().editor.active_tool = ToolId::WaterBrush;
    harness.run();
    harness.get_by_label("Lake Brush");
    harness.get_by_label("Mode: Erase");
    harness.state_mut().editor.active_tool = ToolId::Stamp;
    harness.run();
    harness.get_by_label("Mode: Land");

    // The Stamp Eraser and the River have no modes, so no chip.
    harness.state_mut().editor.active_tool = ToolId::StampEraser;
    harness.run();
    harness.get_by_label("Stamp Eraser");
    assert!(harness.query_by_label_contains("Mode: ").is_none());
}

#[test]
fn sections_do_not_collapse() {
    let (mut fixture, _, _) = fixture();
    fixture.editor.active_tool = ToolId::StampEraser;
    let mut harness = harness(fixture);
    // Clicking the heading changes nothing: the section's body stays.
    harness.get_by_label("Eraser").click();
    harness.run();
    assert!(harness.query_by_label("Size").is_some());
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

fn click(harness: &mut Harness<'static, Fixture>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

fn geometry_of(fixture: &Fixture, id: Id) -> String {
    format!("{:?}", fixture.editor.map().node(id))
}

/// Types `text` into the `n`th number box (from the top) that shows `shown`.
fn type_into_nth(harness: &mut Harness<'static, Fixture>, shown: &str, n: usize, text: &str) {
    harness
        .get_all_by_value(shown)
        .nth(n)
        .expect("a box showing that value")
        .click();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run();
    harness
        .get_by(|n| n.role() == egui::accesskit::Role::TextInput && n.is_focused())
        .type_text(text);
    harness.key_press(egui::Key::Enter);
    harness.run();
}

#[test]
fn resizing_from_each_anchor_keeps_that_point_fixed_and_moves_no_object() {
    for y in 0..3u8 {
        for x in 0..3u8 {
            let (fixture, _, island) = fixture();
            let before = geometry_of(&fixture, island);
            let mut harness = boards_harness(fixture);
            click(&mut harness, &anchor_name(x, y));
            // The width box is the first showing 100, the height box the second.
            type_into_nth(&mut harness, "100", 0, "90");
            type_into_nth(&mut harness, "100", 0, "110");
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
    let mut harness = boards_harness(fixture);
    // Anchor bottom-right: shrinking pulls the left edge past the Landmass at 0..40.
    click(&mut harness, &anchor_name(2, 2));
    type_into_nth(&mut harness, "100", 0, "30");
    let state = harness.state();
    assert_eq!(state.editor.map().canvas().width, 30);
    assert_eq!(state.editor.map().canvas().origin[0], 70.0);
    assert_eq!(geometry_of(state, island), before);
    type_into_nth(&mut harness, "30", 0, "100");
    let state = harness.state();
    assert_eq!(state.editor.map().canvas().width, 100);
    assert_eq!(state.editor.map().canvas().origin[0], 0.0);
    assert_eq!(geometry_of(state, island), before);

    // Each typed size was one undo step, and undo only moves the Canvas.
    harness.state_mut().editor.undo();
    assert_eq!(harness.state().editor.map().canvas().width, 30);
    assert_eq!(geometry_of(harness.state(), island), before);
    harness.state_mut().editor.undo();
    assert_eq!(harness.state().editor.map().canvas().width, 100);
    assert_eq!(geometry_of(harness.state(), island), before);
    for _ in 0..2 {
        harness.state_mut().editor.redo();
        assert_eq!(geometry_of(harness.state(), island), before);
    }
    assert_eq!(harness.state().editor.map().canvas().width, 100);
}

#[test]
fn scrubbing_the_canvas_width_is_one_undo_step_that_holds_the_anchor() {
    let (fixture, _, island) = fixture();
    let before = geometry_of(&fixture, island);
    let mut harness = boards_harness(fixture);
    click(&mut harness, &anchor_name(2, 1));
    let steps = harness.state().editor.history_steps().0.len();
    let start = harness
        .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Canvas width")
        .rect()
        .center();
    harness.event(egui::Event::PointerMoved(start));
    harness.run();
    let button = |pressed| egui::Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    harness.event(button(true));
    harness.run();
    for step in 1..=4 {
        harness.event(egui::Event::PointerMoved(
            start - egui::vec2(step as f32, 0.0),
        ));
        harness.run();
    }
    harness.event(egui::Event::PointerButton {
        pos: start - egui::vec2(4.0, 0.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run();
    // Four points left at ten per point is 40 narrower, with the right edge held still.
    let canvas = harness.state().editor.map().canvas();
    assert_eq!(canvas.width, 60);
    assert_eq!(canvas.origin[0], 40.0);
    assert_eq!(geometry_of(harness.state(), island), before);
    assert_eq!(harness.state().editor.history_steps().0.len(), steps + 1);
    harness.state_mut().editor.undo();
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
fn the_style_tab_shows_the_template_cards_and_no_template_row() {
    let mut harness = map_harness(ocean_fixture());
    click(&mut harness, "Style");
    // The card of the current look is the one sign of which template it is.
    let ocean = harness.get_by_label("Ocean look");
    assert_eq!(
        ocean.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    assert!(harness.query_by_label("Template").is_none());
    assert!(harness.query_by_label("Ocean").is_none());
}

#[test]
fn the_style_tab_keeps_every_field_findable_by_its_whole_name() {
    let mut harness = map_harness(ocean_fixture());
    click(&mut harness, "Style");
    // The rows show "Colour", "Light", "Dark" and the numbers carry "Strength" and "Scale" in
    // the box, but assistive tools and tests still find them by the whole name.
    for name in [
        "Water colour",
        "Texture strength",
        "Texture scale",
        "Texture light",
        "Texture dark",
        "Amount",
        "Scale",
        "Seed",
        "Re-roll",
    ] {
        assert!(harness.query_by_label(name).is_some(), "{name}");
    }
}

#[test]
fn the_style_tab_swatches_start_at_one_x_at_every_width() {
    for width in [480.0, 320.0, 240.0, 200.0] {
        let (fixture, _, _) = fixture();
        let mut harness = narrow_harness(fixture, width, |ui, f| {
            let id = egui::Id::new("map panel tab");
            ui.ctx().data_mut(|d| d.insert_temp(id, 0_usize));
            f.panel.map_panel(ui, &mut f.editor);
        });
        harness.run();
        let lefts: Vec<f32> = ["Water colour", "Texture light", "Texture dark"]
            .into_iter()
            .map(|name| harness.get_by_label(name).rect().left())
            .collect();
        assert!(
            lefts.windows(2).all(|w| (w[0] - w[1]).abs() < 0.5),
            "swatches at {width}: {lefts:?}"
        );
        // The pair sits side by side, each in its own half, inside the panel.
        let strength = harness.get_by_label("Texture strength").rect();
        let scale = harness.get_by_label("Texture scale").rect();
        assert!(strength.right() <= scale.left(), "pair at {width}");
        assert!(scale.right() <= width, "pair at {width}");
    }
}

#[test]
fn a_long_label_does_not_move_its_field() {
    let (fixture, _, _) = fixture();
    let mut harness = narrow_harness(fixture, 320.0, |ui, _| {
        let mut short = 1.0;
        let mut long = 2.0;
        prop_widgets::scrub_field(ui, &mut short, &prop_widgets::ScrubSpec::new("Size"));
        prop_widgets::scrub_field(
            ui,
            &mut long,
            &prop_widgets::ScrubSpec::new("Texture strength jitter"),
        );
    });
    harness.run();
    // The label is the scrub handle: both columns are the same width, so the boxes line up.
    let short = harness.get_by_label("Size").rect();
    let long = harness.get_by_label("Texture strength jitter").rect();
    assert_eq!(short.width(), long.width());
    assert_eq!(short.right(), long.right());
}

#[test]
fn taking_another_templates_look_is_one_undo_step_and_keeps_the_template() {
    let fixture = ocean_fixture();
    let original = fixture.editor.map().canvas().background.clone();
    let mut harness = map_harness(fixture);
    click(&mut harness, "Style");
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
    let mut harness = map_harness(fixture);
    click(&mut harness, "Style");
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
    let mut harness = map_harness(ocean_fixture());
    click(&mut harness, "Style");
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
    assert!(harness.query_all_by_label("2 Stamps").next().is_some());
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

#[test]
fn a_selected_landmass_has_a_lock_in_its_header_and_no_locked_checkbox() {
    let (mut fixture, _, island) = fixture();
    fixture.editor.select(vec![island]);
    let mut harness = harness(fixture);
    for label in [
        "Lock Landmass",
        "Appearance",
        "Fill",
        "Roughness",
        "Amount",
        "Re-roll",
    ] {
        assert!(harness.query_by_label(label).is_some(), "{label}");
    }
    // Union, Subtract and Split Apart are icon buttons with their names.
    for label in ["Union", "Subtract", "Split Apart"] {
        assert!(harness.query_by_label(label).is_some(), "{label}");
    }
    assert!(harness.query_by_label("Locked").is_none());

    harness.get_by_label("Lock Landmass").click();
    harness.run();
    assert!(harness.query_by_label("Unlock Landmass").is_some());
    assert_eq!(harness.state().editor.undo_label(), Some("Lock"));
    harness.state_mut().editor.undo();
    harness.run();
    assert!(harness.query_by_label("Lock Landmass").is_some());
}

#[test]
fn a_labels_halo_colour_always_shows_and_overrides_can_be_reset_from_the_header() {
    let (mut fixture, layer, _) = fixture();
    let id = Id::new();
    fixture
        .editor
        .run(Command::CreateLabel {
            id,
            parent: layer,
            index: None,
            label: Box::new(Label {
                text: "Hello".to_owned(),
                size: Some(40.0),
                ..Label::default()
            }),
        })
        .expect("a Label is made");
    fixture.editor.select(vec![id]);
    let mut harness = harness(fixture);
    for label in [
        "Lock Label",
        "Manage Label styles…",
        "Italic",
        "All caps",
        "Align left",
        "Align centre",
        "Align right",
    ] {
        assert!(harness.query_by_label(label).is_some(), "{label}");
    }
    assert!(harness.query_by_label("Locked").is_none());
    // The halo's colour sits beside its size even while the halo has none, so the row keeps
    // its shape when a size is typed.
    harness.get_by_label("Halo colour");

    // One field is overridden, so the header offers the reset and says so.
    harness.get_by_label("Reset overrides").click();
    harness.run();
    assert_eq!(
        harness.state().editor.label(id).expect("a Label").size,
        None
    );
    assert!(harness.query_by_label("Reset overrides").is_none());
}

#[test]
fn a_labels_board_is_a_full_width_field_in_its_appearance_section() {
    use ssmap_core::Membership;
    let (mut fixture, layer, _) = fixture();
    let board = fixture.editor.new_art_board().unwrap();
    let id = Id::new();
    fixture
        .editor
        .run(Command::CreateLabel {
            id,
            parent: layer,
            index: None,
            label: Box::new(Label {
                text: "Hello".to_owned(),
                ..Label::default()
            }),
        })
        .unwrap();
    fixture.editor.select_board(None);
    fixture.editor.select(vec![id]);
    let mut harness = narrow_harness(fixture, 260.0, |ui, f| f.panel.show(ui, &mut f.editor));
    // The Board field is the combo that says "Canvas"; it spans the panel, not a default width.
    let boards: Vec<_> = harness
        .query_all_by_role(egui::accesskit::Role::ComboBox)
        .filter(|node| node.accesskit_node().value().as_deref() == Some("Canvas"))
        .collect();
    let [combo] = boards.as_slice() else {
        panic!("one Board field, found {}", boards.len())
    };
    let rect = combo.rect();
    assert!(rect.width() > 200.0, "{rect:?}");
    assert!(rect.right() <= 260.0, "{rect:?}");
    // Only one: the generic row under the sections is not drawn for a Label.
    assert_eq!(harness.query_all_by_label("Board").count(), 0);

    combo.click();
    harness.run();
    harness.get_by_label("Art board 1").click();
    harness.run();
    let member = match harness.state().editor.map().node(id) {
        Some(Node::Object(object)) => object.membership,
        _ => panic!("an Object"),
    };
    assert_eq!(member, Membership::Board(board));
}

fn pinned(harness: &Harness<'_, Fixture>, id: Id) -> bool {
    matches!(harness.state().editor.map().node(id), Some(ssmap_core::Node::Object(o)) if o.move_locked)
}

#[test]
fn the_header_pin_button_pins_and_unpins_and_a_lock_disables_it() {
    let (mut fixture, layer, _) = fixture();
    install_test_pack(&mut fixture.editor, false);
    let stamp = add_stamp(&mut fixture, layer, 10.0);
    fixture.editor.select(vec![stamp]);
    let mut harness = harness(fixture);
    // The checkbox is gone; the pin button sits beside the padlock.
    assert!(harness.query_by_label("Locked for movement").is_none());

    harness.get_by_label("Pin to Canvas").click();
    harness.run();
    assert!(pinned(&harness, stamp));
    assert_eq!(harness.state().editor.undo_label(), Some("Pin to Canvas"));

    harness.get_by_label("Unpin from Canvas").click();
    harness.run();
    assert!(!pinned(&harness, stamp));
    assert_eq!(
        harness.state().editor.undo_label(),
        Some("Unpin from Canvas")
    );

    // A locked Object refuses the pin: the button is there but does nothing.
    harness.get_by_label("Lock Stamp").click();
    harness.run();
    harness.get_by_label("Pin to Canvas").click();
    harness.run();
    assert!(!pinned(&harness, stamp));
}

fn board_size(harness: &Harness<'static, Fixture>) -> (u32, u32) {
    let canvas = harness.state().editor.map().canvas();
    (canvas.width, canvas.height)
}

#[test]
fn the_map_panel_has_style_and_shores_tabs_and_no_canvas_tab() {
    let mut harness = map_harness(ocean_fixture());
    for tab in ["Style", "Shores"] {
        harness.get_by_role_and_label(egui::accesskit::Role::Button, tab);
    }
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::Button, "Canvas")
            .is_none()
    );
    // It starts on Style; the Canvas is in the Art boards section.
    harness.get_by_label("Parchment look");
    assert!(harness.query_by_label("Canvas width").is_none());
    assert!(harness.query_by_label("Outline width").is_none());

    click(&mut harness, "Shores");
    harness.get_by_label("Outline width");
    assert!(harness.query_by_label("Parchment look").is_none());
}

#[test]
fn the_boards_settings_cover_the_canvas_and_an_art_board() {
    let (fixture, _, _) = fixture();
    let canvas = fixture.editor.map().primary_board();
    let harness = boards_harness(fixture);
    for label in [
        "Name",
        "Size",
        "Scale",
        "Resize from",
        "Position",
        "Background",
    ] {
        assert!(
            harness.query_all_by_label(label).next().is_some(),
            "{label}"
        );
    }
    // The Canvas has no Set as Canvas or Delete, and its name is read-only.
    assert!(harness.query_by_label("Set as Canvas").is_none());
    assert!(harness.query_by_label("Delete").is_none());
    harness.get_by_label("Duplicate");

    let mut fixture = harness.into_state();
    let board = fixture.editor.new_art_board().unwrap();
    fixture.editor.select_board(Some(board));
    let mut harness = boards_harness(fixture);
    assert!(harness.query_by_label("Scale").is_none());
    harness.get_by_label("Set as Canvas");
    harness.get_by_label("Delete").click();
    harness.run();
    harness.get_by_label("Duplicate").click();
    harness.run();
    let requests = harness.state_mut().panel.take_board_requests();
    assert_eq!(
        requests,
        [BoardRequest::Delete(board), BoardRequest::Duplicate(board)]
    );
    assert_ne!(board, canvas);
}

#[test]
fn the_background_colour_row_is_in_the_boards_panel_and_a_board_has_no_properties_section() {
    let (mut fixture, _, _) = fixture();
    let board = fixture.editor.new_art_board().unwrap();
    fixture.editor.select_board(Some(board));
    let harness = harness(fixture);
    // Properties no longer shows the selected board's settings.
    for label in ["Size", "Position", "Background", "Set as Canvas"] {
        assert!(harness.query_by_label(label).is_none(), "{label}");
    }
}

const SHORE_FIELDS: [&str; 9] = [
    "Outline width",
    "Outline colour",
    "Ripple count",
    "Ripple spacing",
    "Ripple line width",
    "Ripple fade",
    "Ripple line colour",
    "Shade width",
    "Shade colour",
];

#[test]
fn the_shores_tab_edits_one_shore_and_the_mode_buttons_pick_which() {
    use egui::accesskit::Toggled;
    let mut harness = map_harness(ocean_fixture());
    click(&mut harness, "Shores");
    for label in SHORE_FIELDS {
        harness.get_by_label(label);
    }
    // No heading row: the tab already says Shores, and the buttons say which kind it is.
    assert!(harness.query_by_label("Landmass shore").is_none());
    let toggled = |harness: &Harness<'_, Fixture>, name: &str| {
        harness.get_by_label(name).accesskit_node().toggled()
    };
    assert_eq!(toggled(&harness, "Landmass"), Some(Toggled::True));
    assert_eq!(toggled(&harness, "Lake"), Some(Toggled::False));
    let landmass = harness
        .state()
        .editor
        .map()
        .defaults()
        .shore_for(ssmap_core::ShoreKind::Landmass)
        .clone();
    click(&mut harness, "Lake");
    assert_eq!(toggled(&harness, "Landmass"), Some(Toggled::False));
    assert_eq!(toggled(&harness, "Lake"), Some(Toggled::True));
    // Switching only changes which shore is shown.
    assert_eq!(
        harness
            .state()
            .editor
            .map()
            .defaults()
            .shore_for(ssmap_core::ShoreKind::Landmass),
        &landmass
    );
}

#[test]
fn editing_the_lake_shore_leaves_the_landmass_shore_alone() {
    use ssmap_core::ShoreKind;
    let mut harness = map_harness(ocean_fixture());
    click(&mut harness, "Shores");
    click(&mut harness, "Lake");
    let before = |harness: &Harness<'_, Fixture>, kind| {
        harness
            .state()
            .editor
            .map()
            .defaults()
            .shore_for(kind)
            .clone()
    };
    let (landmass, lake) = (
        before(&harness, ShoreKind::Landmass),
        before(&harness, ShoreKind::Lake),
    );
    // Type a new Ripple count into the Lake shore.
    type_into(&mut harness, &lake.ripples.count.to_string(), "7");
    assert_eq!(before(&harness, ShoreKind::Landmass), landmass);
    assert_eq!(before(&harness, ShoreKind::Lake).ripples.count, 7);
    assert_ne!(before(&harness, ShoreKind::Lake), lake);
    assert_eq!(
        harness.state().editor.undo_label(),
        Some("Change Default Lake Shore Style")
    );
}

#[test]
fn the_shores_tab_lines_up_its_colours_and_pairs_its_numbers_at_every_width() {
    for width in [480.0, 320.0, 240.0, 200.0] {
        let (fixture, _, _) = fixture();
        let harness = narrow_harness(fixture, width, |ui, f| {
            let id = egui::Id::new("map panel tab");
            ui.ctx().data_mut(|d| d.insert_temp(id, 1_usize));
            f.panel.map_panel(ui, &mut f.editor);
        });
        let lefts: Vec<f32> = ["Outline colour", "Ripple line colour", "Shade colour"]
            .into_iter()
            .map(|name| harness.get_by_label(name).rect().left())
            .collect();
        assert!(
            lefts.windows(2).all(|w| (w[0] - w[1]).abs() < 0.5),
            "colours at {width}: {lefts:?}"
        );
        // Count | Spacing and Width | Fade sit side by side, inside the panel.
        for (a, b) in [
            ("Ripple count", "Ripple spacing"),
            ("Ripple line width", "Ripple fade"),
        ] {
            let (a, b) = (
                harness.get_by_label(a).rect(),
                harness.get_by_label(b).rect(),
            );
            assert!(
                a.right() <= b.left() && (a.top() - b.top()).abs() < 0.5,
                "{a:?} {b:?} at {width}"
            );
            assert!(b.right() <= width, "{b:?} at {width}");
        }
        // The two mode buttons sit directly under the tab bar, one row, 40 x 32.
        let (landmass, lake) = (
            harness.get_by_label("Landmass").rect(),
            harness.get_by_label("Lake").rect(),
        );
        assert_eq!((landmass.width(), landmass.height()), (40.0, 32.0));
        assert!(landmass.right() < lake.left() && (landmass.top() - lake.top()).abs() < 0.5);
    }
}

#[test]
fn a_translucent_shore_colour_shows_its_opacity_after_the_hex() {
    let mut harness = map_harness(ocean_fixture());
    click(&mut harness, "Shores");
    let shown: Vec<String> = harness
        .query_all_by_label_contains("#")
        .filter_map(|node| node.accesskit_node().value())
        .collect();
    // An opaque colour is the hex alone; a translucent one adds its percentage.
    assert!(shown.iter().any(|text| text.contains('%')), "{shown:?}");
    assert!(shown.iter().any(|text| !text.contains('%')), "{shown:?}");
}

#[test]
fn the_chain_is_off_at_first_and_keeps_the_aspect_ratio_once_on() {
    let (mut fixture, _, _) = fixture();
    fixture
        .editor
        .run(Command::ResizeCanvas {
            origin: [0.0, 0.0],
            width: 200,
            height: 100,
        })
        .unwrap();
    let mut harness = boards_harness(fixture);
    assert!(harness.query_by_label("Unlink width and height").is_none());

    // Off: the two fields are independent.
    type_into(&mut harness, "200", "300");
    assert_eq!(board_size(&harness), (300, 100));

    // On: the ratio is the one at this moment (3 : 1), so a new width sets the height.
    click(&mut harness, "Keep aspect ratio");
    harness.get_by_label("Unlink width and height");
    type_into(&mut harness, "300", "600");
    assert_eq!(board_size(&harness), (600, 200));
    assert_eq!(harness.state().editor.undo_label(), Some("Resize Canvas"));

    // The height changes the width the same way, and one Undo step puts both back.
    type_into(&mut harness, "200", "100");
    assert_eq!(board_size(&harness), (300, 100));
    harness.state_mut().editor.undo();
    assert_eq!(board_size(&harness), (600, 200));
}

#[test]
fn every_object_breadcrumb_has_a_layers_icon_while_the_selection_shares_a_layer() {
    let (mut fixture, layer, island) = fixture();
    install_test_pack(&mut fixture.editor, false);
    let stamp = add_stamp(&mut fixture, layer, 10.0);
    // One Object: the breadcrumb shows its Layer, so it has the icon.
    fixture.editor.select(vec![stamp]);
    let harness = harness(fixture);
    harness.get_by_label("Layer");
    let (mut fixture, ..) = (harness.into_state(), ());
    // Objects in the same Layer still share it.
    fixture.editor.select(vec![stamp, island]);
    let harness = self::harness(fixture);
    harness.get_by_label("Layer");
    // Objects in different Layers have no Layer to show, and no icon.
    let mut fixture = harness.into_state();
    let other = fixture.editor.new_layer().expect("a Layer");
    let apart = add_stamp(&mut fixture, other, 30.0);
    fixture.editor.select(vec![stamp, apart]);
    let harness = self::harness(fixture);
    assert!(harness.query_by_label("Layer").is_none());
}

#[test]
fn the_label_panel_groups_its_fields_into_divider_sections() {
    let (mut fixture, layer, _) = fixture();
    let id = Id::new();
    fixture
        .editor
        .run(Command::CreateLabel {
            id,
            parent: layer,
            index: None,
            label: Box::new(Label {
                text: "Hello".to_owned(),
                ..Label::default()
            }),
        })
        .expect("a Label is made");
    fixture.editor.select(vec![id]);
    let mut harness = harness(fixture);
    // "Fill" and "Halo" also name a field, so a section may match more than one node.
    for section in ["Text", "Typography", "Fill", "Halo", "Appearance"] {
        assert!(
            harness.query_all_by_label(section).next().is_some(),
            "{section}"
        );
    }
    // The sections are always open: no chevrons to collapse them.
    for field in [
        "Weight",
        "Size",
        "Line height",
        "Letter spacing",
        "Arch",
        "Opacity",
        "Rotation",
    ] {
        harness.get_by_label(field);
    }
    // Alignment is three icon buttons, and choosing one is one Undo step.
    harness.get_by_label("Align right").click();
    harness.run();
    assert_eq!(
        harness.state().editor.label(id).expect("a Label").align,
        ssmap_core::Align::Right
    );
    assert_eq!(harness.state().editor.undo_label(), Some("Edit Label"));
}

#[test]
fn properties_follow_the_select_tool_and_its_selection() {
    let (mut fixture, layer, island) = fixture();
    fixture.editor.active_tool = ToolId::Select;
    fixture.editor.select(vec![island]);
    let name = crate::layer_tree::node_name(fixture.editor.map(), island);
    let layer_name = crate::layer_tree::node_name(fixture.editor.map(), layer);
    let mut harness = harness(fixture);
    assert!(harness.query_all_by_label(&name).next().is_some());
    harness.get_by_label("Roughness");

    // Picking the Layer in the Layers panel changes the title.
    harness.state_mut().editor.select(vec![layer]);
    harness.run();
    assert!(harness.query_all_by_label(&layer_name).next().is_some());
    harness.state_mut().editor.select(vec![island]);
    harness.run();

    // Other tools hide the selection's properties and title, and keep the selection.
    for tool in [ToolId::LandBrush, ToolId::Stamp, ToolId::Pan, ToolId::Label] {
        harness.state_mut().editor.active_tool = tool;
        harness.run();
        assert!(harness.query_by_label(&name).is_none(), "{tool:?}");
        assert!(harness.query_by_label("Roughness").is_none(), "{tool:?}");
        assert_eq!(harness.state().editor.selection(), [island], "{tool:?}");
    }

    // Back to Select: the same selection and properties.
    harness.state_mut().editor.active_tool = ToolId::Select;
    harness.run();
    assert!(harness.query_all_by_label(&name).next().is_some());
    harness.get_by_label("Roughness");
}

/// Widgets of `harness` that reach outside a panel `width` wide.
fn outside(harness: &Harness<'_, Fixture>, width: f32) -> Vec<egui::Rect> {
    harness
        .root()
        .children_recursive()
        .filter(|node| node.accesskit_node().bounding_box().is_some())
        .map(|node| node.rect())
        .filter(|rect| rect.right() > width + 0.5 || rect.left() < -0.5)
        .collect()
}

/// Every state of the Properties panel, as a name and the way to set it up.
#[allow(clippy::type_complexity, reason = "a list of named set-ups")]
fn properties_states() -> Vec<(&'static str, fn(&mut Fixture, Id, Id, Id))> {
    vec![
        ("nothing selected", |_, _, _, _| {}),
        ("a Landmass", |f, _, island, _| {
            f.editor.select(vec![island])
        }),
        ("a Lake", |f, _, _, lake| f.editor.select(vec![lake])),
        ("a Landmass and a Lake", |f, _, island, lake| {
            f.editor.select(vec![island, lake]);
        }),
        ("the Land brush", |f, _, _, _| {
            f.editor.active_tool = ToolId::LandBrush;
        }),
        ("the Water brush", |f, _, _, _| {
            f.editor.active_tool = ToolId::WaterBrush;
        }),
        ("the River tool", |f, _, _, _| {
            f.editor.active_tool = ToolId::River;
        }),
        ("the Stamp tool", |f, _, _, _| {
            install_test_pack(&mut f.editor, false);
            f.editor.active_tool = ToolId::Stamp;
        }),
        ("the Stamp eraser", |f, _, _, _| {
            f.editor.active_tool = ToolId::StampEraser;
        }),
        ("a River", |f, layer, _, _| {
            let id = Id::new();
            let river = ssmap_core::River::new(vec![
                ssmap_core::RiverPoint::new([0.0, 0.0], 6.0),
                ssmap_core::RiverPoint::new([100.0, 0.0], 6.0),
                ssmap_core::RiverPoint::new([100.0, 100.0], 12.0),
            ]);
            f.editor
                .run(Command::CreateRiver {
                    id,
                    parent: layer,
                    index: None,
                    river: Box::new(river),
                })
                .unwrap();
            f.editor.select(vec![id]);
        }),
        ("a Stamp", |f, layer, _, _| {
            install_test_pack(&mut f.editor, false);
            let id = Id::new();
            f.editor
                .run(Command::CreateStamp {
                    id,
                    parent: layer,
                    index: None,
                    stamp: Box::new(ssmap_core::Stamp::default()),
                })
                .unwrap();
            f.editor.select(vec![id]);
        }),
        ("two Stamps", |f, layer, _, _| {
            install_test_pack(&mut f.editor, false);
            let ids = [Id::new(), Id::new()];
            for id in ids {
                f.editor
                    .run(Command::CreateStamp {
                        id,
                        parent: layer,
                        index: None,
                        stamp: Box::new(ssmap_core::Stamp::default()),
                    })
                    .unwrap();
            }
            f.editor.select(ids.to_vec());
        }),
    ]
}

/// A harness `width` wide and tall enough not to scroll, that draws with `show`.
fn narrow_harness(
    fixture: Fixture,
    width: f32,
    mut show: impl FnMut(&mut egui::Ui, &mut Fixture) + 'static,
) -> Harness<'static, Fixture> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(width, 1600.0))
        .build_ui_state(
            move |ui, fixture: &mut Fixture| {
                theme::install(&ui.ctx().clone());
                show(ui, fixture);
            },
            fixture,
        );
    harness.run();
    harness
}

/// The Properties panel goes down to 260 pt (`PROPERTIES_PANEL_MIN`); 240 leaves some room.
#[test]
fn no_properties_state_reaches_outside_a_narrow_panel() {
    for width in [260.0, 240.0] {
        for (name, setup) in properties_states() {
            let (mut fixture, layer, island) = fixture();
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
            setup(&mut fixture, layer, island, lake);
            let harness = narrow_harness(fixture, width, |ui, f| f.panel.show(ui, &mut f.editor));
            let out = outside(&harness, width);
            assert!(out.is_empty(), "{name} at {width}: {out:?}");
        }
    }
}

/// The Map section shares the sidebar column, which goes down to 200 pt (`LAYERS_PANEL_MIN`).
#[test]
fn the_map_panel_tabs_fit_down_to_the_sidebar_minimum() {
    for width in [200.0, 240.0] {
        for tab in 0..2_usize {
            let (fixture, _, _) = fixture();
            let harness = narrow_harness(fixture, width, move |ui, f| {
                let id = egui::Id::new("map panel tab");
                ui.ctx().data_mut(|d| d.insert_temp(id, tab));
                f.panel.map_panel(ui, &mut f.editor);
            });
            let out = outside(&harness, width);
            assert!(out.is_empty(), "tab {tab} at {width}: {out:?}");
        }
    }
}
