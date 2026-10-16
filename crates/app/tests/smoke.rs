//! egui_kittest smoke tests: the real app, driven headlessly through its accessibility tree.
//! There is no GPU here, so the Canvas draws its flat placeholder.

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ssmap_app::MapMakerApp;
use ssmap_core::{Canvas, DistanceUnit, Scale};

fn app() -> Harness<'static, MapMakerApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| MapMakerApp::with_recovery_dir(cc, None));
    harness.run();
    // The app starts on Home with no Map; most tests here want one to work in.
    assert!(harness.state().tabs().is_empty());
    assert!(harness.state().home_active());
    harness.state_mut().new_map(Canvas::new(
        4096,
        3072,
        Scale::new(0.25, DistanceUnit::Miles),
    ));
    harness.run();
    harness
}

#[test]
fn launches_and_creates_a_layer() {
    let mut harness = app();
    // A new Map starts with the template's four Layers.
    assert_eq!(harness.state().editor().map.children(None).len(), 4);

    harness.get_by_label("New Layer").click();
    harness.run();
    let map = &harness.state().editor().map;
    assert_eq!(map.children(None).len(), 5);
    // The new Layer shows in the tree and is selected.
    harness.get_by_label("Layer 5");
    assert_eq!(
        harness.state().editor().selection(),
        &map.children(None)[4..]
    );

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run();
    assert_eq!(harness.state().editor().map.children(None).len(), 4);
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Z,
    );
    harness.run();
    assert_eq!(harness.state().editor().map.children(None).len(), 5);
}

#[test]
fn a_new_map_opens_with_the_starter_layers_and_label_styles() {
    let harness = app();
    let map = &harness.state().editor().map;
    let layers: Vec<String> = map
        .children(None)
        .iter()
        .map(|&id| match map.node(id) {
            Some(ssmap_core::Node::Layer(layer)) => layer.name.clone(),
            _ => panic!("an Object at the top level"),
        })
        .collect();
    assert_eq!(layers, ["Backdrop", "Land", "Assets", "Labels"]);
    assert_eq!(map.label_styles().len(), 5);
}

#[test]
fn the_theme_is_installed_dark_by_default() {
    let harness = app();
    let ctx = &harness.ctx;
    // The harness has no system theme, so the dark fallback applies.
    assert_eq!(ctx.theme(), egui::Theme::Dark);
    let style = ctx.style_of(ctx.theme());
    assert_eq!(
        style.visuals.panel_fill,
        ssmap_app::theme::Palette::DARK.bg_panel
    );
    assert_eq!(
        ctx.style_of(egui::Theme::Light).visuals.panel_fill,
        ssmap_app::theme::Palette::LIGHT.bg_panel
    );
    assert_eq!(style.text_styles[&egui::TextStyle::Body].size, 12.0);
}

#[test]
fn a_locked_layer_refuses_edits_with_a_visible_message() {
    let mut harness = app();
    harness.get_by_label("New Layer").click();
    harness.run();
    // Lock and eye show on hover; the Layers panel lists the new, topmost Layer first.
    harness.get_by_label("Layer 5").hover();
    harness.run();
    harness.query_all_by_label("Lock").next().unwrap().click();
    harness.run();
    let layer = *harness.state().editor().map.children(None).last().unwrap();
    assert!(harness.state().editor().map.node(layer).unwrap().locked());

    // The hide toggle is refused, and the reason shows as a toast.
    harness.get_by_label("Layer 5").hover();
    harness.run();
    harness.query_all_by_label("Hide").next().unwrap().click();
    harness.run();
    let editor = harness.state().editor();
    let message = "“Layer 5” is locked";
    assert_eq!(editor.notice().map(|n| n.text.as_str()), Some(message));
    harness.get_by_label(&format!("Error: {message}"));
    assert!(matches!(
        editor.map.node(layer),
        Some(ssmap_core::Node::Layer(l)) if l.visible
    ));
}

#[test]
fn a_layer_with_100k_objects_lays_out_only_visible_rows() {
    use ssmap_core::{
        Canvas, DistanceUnit, Id, Layer, Map, Node, Object, ObjectKind, Op, Scale, Stamp, Subtree,
    };
    let mut map = Map::new(Canvas::new(
        1000,
        1000,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    let stamps = (0..100_000)
        .map(|_| Subtree {
            id: Id::new(),
            node: Node::Object(Object::new(ObjectKind::Stamp(Stamp::default()))),
            children: Vec::new(),
        })
        .collect();
    map.apply(Op::Insert {
        parent: None,
        index: 0,
        subtree: Subtree {
            id: Id::new(),
            node: Node::Layer(Layer::new("Forest")),
            children: stamps,
        },
    })
    .unwrap();

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| MapMakerApp::with_map(cc, map, None));
    let started = std::time::Instant::now();
    harness.run();
    eprintln!("first frames with 100k rows: {:?}", started.elapsed());
    harness.get_by_label("Forest");
    // Only rows that fit on screen exist in the UI.
    let stamp_rows = harness.query_all_by_label("Stamp").count();
    assert!(
        (1..100).contains(&stamp_rows),
        "{stamp_rows} Stamp rows laid out"
    );
}

#[test]
fn save_as_then_open_round_trips_the_map() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("world");
    let mut harness = app();
    assert_eq!(harness.state().title(), "Untitled — StarScript Map Maker");
    harness.get_by_label("New Layer").click();
    harness.run();
    assert!(harness.state().title().starts_with("Untitled •"));

    assert!(harness.state_mut().save_as_path(&path));
    harness.run();
    let saved = dir.path().join("world.ssmap");
    let editor = harness.state().editor();
    assert_eq!(harness.state().document().path(), Some(saved.as_path()));
    assert_eq!(
        editor.notice().map(|n| n.text.as_str()),
        Some("Saved “world.ssmap”")
    );
    assert_eq!(
        harness.state().title(),
        "world.ssmap — StarScript Map Maker"
    );
    let saved_view = editor.map.view();

    // More edits, then open the saved file again: it is already open, so its tab is shown as
    // it was left, edits and all.
    harness.get_by_label("New Layer").click();
    harness.run();
    assert!(harness.state().title().starts_with("world.ssmap •"));
    let edited_view = harness.state().editor().map.view();
    assert!(harness.state_mut().open_path(&saved));
    harness.run();
    assert_eq!(harness.state().tabs().len(), 1);
    assert_eq!(harness.state().editor().map.view(), edited_view);
    assert!(harness.state().editor().can_undo());
    assert_ne!(edited_view, saved_view);
    harness.get_by_label("Layer 5");
    assert_eq!(harness.state().recent_files(), std::slice::from_ref(&saved));
    // Undo the extra edit so the Ctrl+S below leaves the file with one more Layer.
    harness.get_by_label("Undo").click();
    harness.run();

    // Ctrl+S saves to the open file.
    harness.get_by_label("New Layer").click();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    harness.run();
    assert_eq!(
        harness.state().title(),
        "world.ssmap — StarScript Map Maker"
    );
    let (_, reopened) = ssmap_core::file::MapFile::open(&saved).unwrap();
    assert_eq!(reopened.children(None).len(), 6);
}

#[test]
fn saving_an_untitled_map_asks_where() {
    let mut harness = app();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    harness.run();
    assert!(harness.state().document().path().is_none());
    harness.get_by_label("Save Map As");
}

#[test]
fn a_file_that_fails_to_open_shows_why() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.ssmap");
    std::fs::write(&path, "plain text ".repeat(100)).unwrap();
    let mut harness = app();
    assert!(!harness.state_mut().open_path(&path));
    harness.run();
    let message = "Couldn’t open “notes.ssmap”: this is not a StarScript Map file";
    assert_eq!(
        harness.state().editor().notice().map(|n| n.text.as_str()),
        Some(message)
    );
    harness.get_by_label(&format!("Error: {message}"));
    assert!(harness.state().document().path().is_none());
}

/// Clicks the Canvas at a screen point.
fn click_at(harness: &mut Harness<'static, MapMakerApp>, pos: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(pos));
    harness.run();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run();
    }
}

#[test]
fn the_label_tool_places_a_label_and_types_its_text_in_one_undo_step() {
    let mut harness = app();
    harness.key_press(egui::Key::T);
    harness.run();
    // The middle of the Canvas panel, between the Layers and Properties panels.
    let canvas = egui::pos2(640.0, 400.0);
    click_at(&mut harness, canvas);
    assert!(harness.state().editing_label_text());
    // The placeholder text is selected, so typing replaces it.
    harness.event(egui::Event::Text("Ærendál".into()));
    harness.run();
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(!harness.state().editing_label_text());

    let editor = harness.state().editor();
    let [id] = editor.selection() else {
        panic!("the new Label is selected")
    };
    let id = *id;
    assert_eq!(editor.label(id).map(|l| l.text.as_str()), Some("Ærendál"));
    assert_eq!(editor.undo_label(), Some("Add Label"));
    // With no Layer chosen, it went into the template's Labels Layer.
    let parent = editor.map.parent(id).unwrap();
    assert!(matches!(
        editor.map.node(parent),
        Some(ssmap_core::Node::Layer(layer)) if layer.name == "Labels"
    ));
    // Its first line names it in the Layers panel.
    harness.get_by_label("Ærendál");

    // Clicking empty Canvas deselects it, and clicking it picks it again.
    click_at(&mut harness, egui::pos2(300.0, 120.0));
    assert!(harness.state().editor().selection().is_empty());
    click_at(&mut harness, canvas);
    assert_eq!(harness.state().editor().selection(), &[id]);

    // Dragging it moves it, as one Undo step.
    let before = harness.state().editor().label(id).unwrap().position;
    harness.event(egui::Event::PointerButton {
        pos: canvas,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run();
    for step in 1..=5 {
        harness.event(egui::Event::PointerMoved(
            canvas + egui::vec2(10.0 * step as f32, 0.0),
        ));
        harness.run();
    }
    harness.event(egui::Event::PointerButton {
        pos: canvas + egui::vec2(50.0, 0.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run();
    let editor = harness.state().editor();
    let after = editor.label(id).unwrap().position;
    assert!(
        after[0] > before[0] && after[1] == before[1],
        "{before:?} -> {after:?}"
    );
    assert_eq!(editor.undo_label(), Some("Move"));

    for _ in 0..2 {
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        harness.run();
    }
    let editor = harness.state().editor();
    assert!(editor.map.children(Some(parent)).is_empty());
}

/// One Layer holding an island with a hole (0..400) and a Lake (100..200) above it.
fn island_map() -> (ssmap_core::Map, ssmap_core::Id, ssmap_core::Id) {
    use ssmap_core::{Canvas, Command, DistanceUnit, Id, Map, MultiPolygon, Polygon, Scale};
    let square = |min: f64, size: f64| {
        vec![
            [min, min],
            [min + size, min],
            [min + size, min + size],
            [min, min + size],
        ]
    };
    let mut map = Map::new(Canvas::new(
        1000,
        1000,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    let (layer, island, lake) = (Id::new(), Id::new(), Id::new());
    for command in [
        Command::CreateLayer {
            id: layer,
            parent: None,
            index: None,
            name: "Land".into(),
        },
        Command::CreateLandmass {
            id: island,
            parent: layer,
            index: None,
            geometry: MultiPolygon(vec![Polygon {
                exterior: square(0.0, 400.0),
                holes: vec![square(250.0, 100.0)],
            }]),
            fill: None,
        },
        Command::CreateLake {
            id: lake,
            parent: layer,
            index: Some(0),
            geometry: MultiPolygon(vec![Polygon {
                exterior: square(100.0, 100.0),
                holes: Vec::new(),
            }]),
            fill: None,
        },
    ] {
        map.execute(&command).unwrap();
    }
    (map, island, lake)
}

fn outline_start(harness: &Harness<'_, MapMakerApp>, id: ssmap_core::Id) -> [f64; 2] {
    match harness.state().editor().map.node(id) {
        Some(ssmap_core::Node::Object(object)) => object.kind.geometry().unwrap().0[0].exterior[0],
        _ => panic!("an Object"),
    }
}

/// Presses at `from`, moves through to `to` over a few frames, and optionally releases.
fn drag(harness: &mut Harness<'_, MapMakerApp>, from: egui::Pos2, to: egui::Pos2, release: bool) {
    harness.hover_at(from);
    harness.run();
    harness.drag_at(from);
    harness.run();
    for step in 1..=4 {
        harness.hover_at(from + (to - from) * (step as f32 / 4.0));
        harness.run();
    }
    if release {
        harness.drop_at(to);
        harness.run();
    }
}

#[test]
fn select_move_and_resize_landmasses_and_lakes() {
    let (map, island, lake) = island_map();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| MapMakerApp::with_map(cc, map, None));
    harness.run();
    let screen = |harness: &Harness<'_, MapMakerApp>, point| {
        harness.state().canvas_to_screen(point).unwrap()
    };

    // A click picks the topmost fill: the Lake over the land, the land, nothing in the hole.
    for (point, expected) in [
        ([150.0, 150.0], vec![lake]),
        ([50.0, 300.0], vec![island]),
        ([300.0, 300.0], vec![]),
    ] {
        let at = screen(&harness, point);
        harness.hover_at(at);
        harness.run();
        harness.drag_at(at);
        harness.drop_at(at);
        harness.run();
        assert_eq!(
            harness.state().editor().selection(),
            expected,
            "at {point:?}"
        );
    }

    // Dragging the island's body moves it by the pointer's Canvas distance.
    let from = screen(&harness, [50.0, 300.0]);
    let to = screen(&harness, [150.0, 350.0]);
    drag(&mut harness, from, to, true);
    let moved = outline_start(&harness, island);
    assert!(
        (moved[0] - 100.0).abs() < 2.0 && (moved[1] - 50.0).abs() < 2.0,
        "moved to {moved:?}"
    );
    assert_eq!(harness.state().editor().selection(), [island]);
    assert_eq!(harness.state().editor().undo_label(), Some("Move"));

    // Dragging the bottom-right handle out doubles the island from its top-left corner.
    let corner = screen(&harness, [500.0, 450.0]);
    let far = screen(&harness, [900.0, 850.0]);
    drag(&mut harness, corner, far, true);
    let editor = harness.state().editor();
    assert_eq!(editor.undo_label(), Some("Resize"));
    let bounds = match editor.map.node(island) {
        Some(ssmap_core::Node::Object(o)) => o.kind.geometry().unwrap().bounds().unwrap(),
        _ => unreachable!(),
    };
    assert!((bounds.width() - 800.0).abs() < 4.0, "{bounds:?}");
    assert!((bounds.min[0] - 100.0).abs() < 2.0, "{bounds:?}");

    // Escape mid-drag puts everything back and records nothing.
    let before = harness.state().editor().map.view();
    let undo_steps = harness.state().editor().undo_label().map(str::to_owned);
    let body = screen(&harness, [200.0, 600.0]);
    let away = screen(&harness, [300.0, 700.0]);
    drag(&mut harness, body, away, false);
    assert_ne!(harness.state().editor().map.view(), before);
    harness.key_press(egui::Key::Escape);
    harness.run();
    harness.drop_at(body);
    harness.run();
    assert_eq!(harness.state().editor().map.view(), before);
    assert_eq!(
        harness.state().editor().undo_label().map(str::to_owned),
        undo_steps
    );

    // Undo restores the outline exactly and the Selection with it.
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(harness.state().editor().selection().is_empty());
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run();
    assert_eq!(outline_start(&harness, island), [0.0, 0.0]);
    assert_eq!(harness.state().editor().selection(), [island]);
}

#[test]
fn a_landmass_gets_its_own_roughness_in_one_undo_step() {
    let (map, island, _) = island_map();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| MapMakerApp::with_map(cc, map, None));
    harness.run();
    let roughness =
        |harness: &Harness<'_, MapMakerApp>| match harness.state().editor().map.node(island) {
            Some(ssmap_core::Node::Object(object)) => object.kind.roughness().cloned(),
            _ => panic!("an Object"),
        };

    let at = harness.state().canvas_to_screen([50.0, 300.0]).unwrap();
    harness.hover_at(at);
    harness.run();
    harness.drag_at(at);
    harness.drop_at(at);
    harness.run();
    assert_eq!(harness.state().editor().selection(), [island]);
    assert_eq!(roughness(&harness), None);

    // Unticking "Use the Map default" gives the island its own copy of the default.
    harness.get_by_label("Use the Map default").click();
    harness.run();
    assert_eq!(
        roughness(&harness).as_ref(),
        Some(&harness.state().editor().map.defaults().roughness)
    );
    assert_eq!(
        harness.state().editor().undo_label(),
        Some("Change Roughness")
    );

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run();
    assert_eq!(roughness(&harness), None);
}

#[test]
fn the_land_brush_paints_merges_and_erases_one_undo_step_per_stroke() {
    use ssmap_core::{Node, ObjectKind};
    let (map, island, _) = island_map();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| MapMakerApp::with_map(cc, map, None));
    harness.run();
    let screen = |harness: &Harness<'_, MapMakerApp>, point| {
        harness.state().canvas_to_screen(point).unwrap()
    };
    let landmasses = |harness: &Harness<'_, MapMakerApp>| {
        let map = &harness.state().editor().map;
        map.draw_order()
            .into_iter()
            .filter(|&id| {
                matches!(map.node(id), Some(Node::Object(o)) if matches!(o.kind, ObjectKind::Landmass(_)))
            })
            .collect::<Vec<_>>()
    };
    let outline =
        |harness: &Harness<'_, MapMakerApp>, id| match harness.state().editor().map.node(id) {
            Some(Node::Object(o)) => o.kind.geometry().unwrap().clone(),
            _ => panic!("an Object"),
        };
    let stroke = |harness: &mut Harness<'_, MapMakerApp>, from: [f64; 2], to: [f64; 2]| {
        let (from, to) = (screen(harness, from), screen(harness, to));
        drag(harness, from, to, true);
    };

    // Clicking the island with Select makes its Layer the active one; then deselect.
    let at = screen(&harness, [50.0, 300.0]);
    harness.hover_at(at);
    harness.run();
    harness.drag_at(at);
    harness.drop_at(at);
    harness.run();
    harness.key_press(egui::Key::Escape);
    harness.key_press(egui::Key::B);
    harness.run();
    assert_eq!(
        harness.state().editor().active_tool,
        ssmap_app::tools::ToolId::LandBrush
    );

    // A stroke on water creates a Landmass, selected so the next stroke merges into it.
    stroke(&mut harness, [600.0, 100.0], [800.0, 100.0]);
    let created = *landmasses(&harness)
        .iter()
        .find(|&&id| id != island)
        .expect("a new Landmass");
    assert_eq!(harness.state().editor().undo_label(), Some("Paint Land"));
    assert_eq!(harness.state().editor().selection(), [created]);
    let painted = outline(&harness, created);
    assert!(painted.contains([700.0, 100.0]) && !painted.contains([700.0, 200.0]));

    // With nothing selected, a stroke starting on the island merges into it.
    harness.key_press(egui::Key::Escape);
    harness.run();
    stroke(&mut harness, [50.0, 300.0], [50.0, 600.0]);
    assert_eq!(landmasses(&harness).len(), 2);
    assert!(outline(&harness, island).contains([50.0, 560.0]));

    // Option/Alt erases from the Landmasses it touches in the active Layer: a cut across the
    // island leaves one Landmass in two parts.
    let before_erase = outline(&harness, island).0.len();
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
    stroke(&mut harness, [180.0, 0.0], [180.0, 650.0]);
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    harness.run();
    assert_eq!(harness.state().editor().undo_label(), Some("Erase Land"));
    let cut = outline(&harness, island);
    assert!(!cut.contains([180.0, 50.0]));
    assert!(cut.0.len() > before_erase, "{} parts", cut.0.len());

    // Each stroke is one Undo step.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run();
    assert!(outline(&harness, island).contains([180.0, 50.0]));

    // Escape drops a stroke in progress and records nothing.
    let before = harness.state().editor().map.view();
    let (from, to) = (
        screen(&harness, [700.0, 700.0]),
        screen(&harness, [900.0, 900.0]),
    );
    drag(&mut harness, from, to, false);
    harness.key_press(egui::Key::Escape);
    harness.run();
    harness.drop_at(to);
    harness.run();
    assert_eq!(harness.state().editor().map.view(), before);
    assert_eq!(harness.state().editor().undo_label(), Some("Paint Land"));
}

#[test]
fn a_landmass_gets_its_own_shore_style_in_one_undo_step() {
    let (map, island, _) = island_map();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| MapMakerApp::with_map(cc, map, None));
    harness.run();
    let shore = |harness: &Harness<'_, MapMakerApp>| match harness.state().editor().map.node(island)
    {
        Some(ssmap_core::Node::Object(object)) => object.kind.shore().cloned(),
        _ => panic!("an Object"),
    };

    let at = harness.state().canvas_to_screen([50.0, 300.0]).unwrap();
    harness.hover_at(at);
    harness.run();
    harness.drag_at(at);
    harness.drop_at(at);
    harness.run();
    assert_eq!(harness.state().editor().selection(), [island]);
    assert_eq!(shore(&harness), None);

    harness.get_by_label("Use the Map style").click();
    harness.run();
    assert_eq!(
        shore(&harness).as_ref(),
        Some(&harness.state().editor().map.defaults().shore)
    );
    assert_eq!(
        harness.state().editor().undo_label(),
        Some("Change Shore Style")
    );

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run();
    assert_eq!(shore(&harness), None);
}
