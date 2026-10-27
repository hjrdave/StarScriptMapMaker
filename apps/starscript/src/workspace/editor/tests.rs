//! Editor tests. One file because they share the helpers below; each test drives the Editor
//! the way a tool would.

use super::*;
use ssmap_core::{Canvas, DistanceUnit, Scale};

fn editor() -> Editor {
    Editor::new(Map::new(Canvas::new(
        100,
        100,
        Scale::new(1.0, DistanceUnit::Miles),
    )))
}

#[test]
fn new_layers_go_above_the_active_one_and_get_selected() {
    let mut editor = editor();
    let first = editor.new_layer().unwrap();
    let second = editor.new_layer().unwrap();
    editor.select(vec![first]);
    let third = editor.new_layer().unwrap();
    assert_eq!(editor.map.children(None), &[first, third, second]);
    assert_eq!(editor.selection(), &[third]);
    assert_eq!(editor.active_layer(), Some(third));
}

#[test]
fn duplicating_selects_the_copy_and_is_one_named_undo_step() {
    let mut editor = editor();
    let first = editor.new_layer().unwrap();
    let second = editor.new_layer().unwrap();
    editor.select(vec![first]);
    let copy = editor.duplicate_selection().unwrap();
    assert_eq!(editor.map.children(None), &[first, copy[0], second]);
    assert_eq!(editor.selection(), &copy[..]);
    assert_eq!(editor.undo_label(), Some("Duplicate Layer"));
    editor.undo();
    assert_eq!(editor.map.children(None), &[first, second]);
}

fn add_stamp(editor: &mut Editor, layer: Id, x: f64, y: f64) -> Id {
    let id = Id::new();
    assert!(
        editor
            .run(Command::CreateStamp {
                id,
                parent: layer,
                index: None,
                stamp: Box::new(Stamp::new("pack/asset".parse().unwrap(), [x, y])),
            })
            .is_ok()
    );
    id
}

fn stamp_position(editor: &Editor, id: Id) -> [f64; 2] {
    editor.stamp(id).unwrap().position
}

#[test]
fn a_paste_puts_the_middle_of_the_copied_objects_under_the_point() {
    let mut editor = editor();
    let layer = editor.new_layer().unwrap();
    let a = add_stamp(&mut editor, layer, 10.0, 10.0);
    let b = add_stamp(&mut editor, layer, 30.0, 20.0);
    editor.select(vec![a, b]);
    assert!(editor.copy_selection());
    // The middle of the two Stamps is (20, 15).
    let pasted = editor.paste([100.0, 200.0]).unwrap();
    assert_eq!(pasted.len(), 2);
    assert_eq!(stamp_position(&editor, pasted[0]), [90.0, 195.0]);
    assert_eq!(stamp_position(&editor, pasted[1]), [110.0, 205.0]);
    assert_eq!(editor.selection(), &pasted[..]);
    // The originals stay where they were, and one undo removes the paste.
    assert_eq!(stamp_position(&editor, a), [10.0, 10.0]);
    assert_eq!(editor.undo_label(), Some("Paste"));
    editor.undo();
    assert_eq!(editor.map.children(Some(layer)), &[a, b]);
}

#[test]
fn pasted_layers_go_above_the_selected_layer_and_keep_their_structure() {
    let mut editor = editor();
    let first = editor.new_layer().unwrap();
    let stamp = add_stamp(&mut editor, first, 5.0, 5.0);
    let second = editor.new_layer().unwrap();
    editor.select(vec![first]);
    editor.copy_selection();
    let pasted = editor.paste([0.0, 0.0]).unwrap();
    assert_eq!(editor.map.children(None), &[first, pasted[0], second]);
    assert_eq!(editor.map.children(Some(pasted[0])).len(), 1);
    assert_ne!(editor.map.children(Some(pasted[0])), &[stamp]);
    assert_eq!(node_name(&editor.map, pasted[0]), "Layer 1 copy");
    // With nothing selected a paste lands on top.
    editor.deselect();
    let top = editor.paste([0.0, 0.0]).unwrap();
    assert_eq!(editor.map.children(None).last(), Some(&top[0]));
}

#[test]
fn cut_removes_the_originals_in_one_step_and_skips_locked_layers() {
    let mut editor = editor();
    let open = editor.new_layer().unwrap();
    let locked = editor.new_layer().unwrap();
    assert!(
        editor
            .run(Command::SetLocked {
                id: locked,
                locked: true
            })
            .is_ok()
    );
    editor.select(vec![open, locked]);
    editor.cut_selection();
    assert_eq!(editor.map.children(None), &[locked]);
    assert_eq!(editor.undo_label(), Some("Delete"));
    // Both were copied, the locked one with its lock.
    let pasted = editor.paste([0.0, 0.0]).unwrap();
    assert_eq!(pasted.len(), 2);
    editor.undo();
    editor.undo();
    assert_eq!(editor.map.children(None), &[open, locked]);
}

#[test]
fn pasting_objects_needs_a_layer() {
    let mut editor = editor();
    let layer = editor.new_layer().unwrap();
    let a = add_stamp(&mut editor, layer, 1.0, 1.0);
    editor.select(vec![a]);
    editor.copy_selection();
    let pasted = editor.paste([1.0, 1.0]).unwrap();
    assert_eq!(editor.map.parent(pasted[0]), Some(layer));
    assert_eq!(editor.map.children(Some(layer)).len(), 2);
}

fn add_label(editor: &mut Editor, layer: Id, mut label: Label) -> Id {
    label.text = "Hello".into();
    let id = Id::new();
    assert!(
        editor
            .run(Command::CreateLabel {
                id,
                parent: layer,
                index: None,
                label: Box::new(label),
            })
            .is_ok()
    );
    id
}

fn label_of(editor: &Editor, id: Id) -> Label {
    match editor.map.node(id) {
        Some(Node::Object(object)) => match &object.kind {
            ObjectKind::Label(label) => (**label).clone(),
            _ => panic!("not a Label"),
        },
        _ => panic!("not an Object"),
    }
}

/// Copies `ids` in one Map and hands the copy to a second, empty one.
fn copy_into_another_map(from: &mut Editor, ids: Vec<Id>) -> (Editor, Id) {
    from.select(ids);
    assert!(from.copy_selection());
    let text = from.take_clipboard_text().unwrap();
    let mut to = editor();
    let layer = to.new_layer().unwrap();
    to.set_clipboard(Clipboard::from_text(&text).unwrap());
    (to, layer)
}

#[test]
fn pasting_into_another_map_needs_nothing_when_it_has_it_all() {
    let mut from = editor();
    let layer = from.new_layer().unwrap();
    let label = add_label(&mut from, layer, Label::default());
    let (mut to, target) = copy_into_another_map(&mut from, vec![label]);
    assert_eq!(to.paste_gaps(), Some(PasteGaps::default()));
    let pasted = to.paste([5.0, 5.0]).unwrap();
    assert_eq!(to.map.parent(pasted[0]), Some(target));
    // Cancelling the popup is not pasting: the Map is as it was.
    assert_eq!(to.undo_label(), Some("Paste"));
    to.undo();
    assert!(to.map.children(Some(target)).is_empty());
}

#[test]
fn a_label_style_the_other_map_lacks_falls_back_to_its_default() {
    let mut from = editor();
    let layer = from.new_layer().unwrap();
    let style = Id::new();
    assert!(
        from.run(Command::CreateLabelStyle {
            style: ssmap_core::LabelStyle::new(style, "Realm"),
        })
        .is_ok()
    );
    let label = add_label(
        &mut from,
        layer,
        Label {
            style,
            size: Some(33.0),
            ..Label::default()
        },
    );
    let (mut to, _) = copy_into_another_map(&mut from, vec![label]);
    assert_eq!(to.paste_gaps().unwrap().label_styles, 1);
    let pasted = to.paste([0.0, 0.0]).unwrap();
    let label = label_of(&to, pasted[0]);
    assert_eq!(label.style, ssmap_core::LabelStyle::DEFAULT_ID);
    assert_eq!(label.size, Some(33.0));
    assert_eq!(to.map.label_styles().iter().count(), 1);
}

#[test]
fn a_font_the_other_map_cannot_draw_gets_the_fallback_font() {
    let mut from = editor();
    let layer = from.new_layer().unwrap();
    let label = add_label(
        &mut from,
        layer,
        Label {
            font: Some(ssmap_core::FontFace {
                family: "Papyrus".into(),
                ..Default::default()
            }),
            ..Label::default()
        },
    );
    let (mut to, _) = copy_into_another_map(&mut from, vec![label]);
    assert_eq!(to.paste_gaps().unwrap().fonts, vec!["Papyrus".to_owned()]);
    let pasted = to.paste([0.0, 0.0]).unwrap();
    let font = label_of(&to, pasted[0]).font.unwrap();
    assert_eq!(font.family, FALLBACK_FAMILY);
}

#[test]
fn an_asset_the_other_map_lacks_pastes_as_a_placeholder_and_cancel_pastes_nothing() {
    let mut from = editor();
    let layer = from.new_layer().unwrap();
    let stamp = add_stamp(&mut from, layer, 1.0, 1.0);
    let (mut to, target) = copy_into_another_map(&mut from, vec![stamp]);
    // No pack is installed here, and nothing is embedded.
    to.set_sprites(SpriteStore::new(Arc::new(
        ssmap_core::asset::PackLibrary::default(),
    )));
    let gaps = to.paste_gaps().unwrap();
    assert_eq!(gaps.assets, vec!["pack/asset".parse::<AssetRef>().unwrap()]);
    // Cancelling the popup never reaches paste: the Map is untouched.
    assert!(to.map.children(Some(target)).is_empty());
    assert_ne!(to.undo_label(), Some("Paste"));
    // Pasting anyway keeps the Stamp, which draws as a placeholder.
    let pasted = to.paste([0.0, 0.0]).unwrap();
    assert_eq!(
        to.stamp(pasted[0]).unwrap().asset,
        "pack/asset".parse::<AssetRef>().unwrap()
    );
    assert_eq!(to.undo_label(), Some("Paste"));
}

#[test]
fn a_paste_with_nothing_copied_does_nothing() {
    let mut editor = editor();
    editor.new_layer().unwrap();
    assert!(editor.paste([0.0, 0.0]).is_none());
    editor.deselect();
    assert!(!editor.copy_selection());
}

#[test]
fn a_duplicate_with_nothing_selected_does_nothing() {
    let mut editor = editor();
    editor.new_layer().unwrap();
    editor.deselect();
    assert!(editor.duplicate_selection().is_none());
}

#[test]
fn a_water_stroke_makes_a_lake_in_a_water_layer_in_one_undo_step() {
    let mut editor = editor();
    let stroke = |erase| BrushStroke {
        region: land::stroke_region(&[[10.0, 10.0], [60.0, 10.0]], 5.0),
        start: [10.0, 10.0],
        erase,
    };
    let gesture = editor.begin_gesture(Editor::water_label(false));
    editor.finish_water_stroke(gesture, stroke(false));
    assert_eq!(editor.undo_label(), Some("Paint Water"));
    let layer = editor.water_layer().expect("a Water layer");
    let lakes = editor.map.children(Some(layer)).to_vec();
    assert_eq!(lakes.len(), 1);
    assert_eq!(editor.selection(), lakes);

    // A second stroke extends the Lake and reuses the Layer.
    let gesture = editor.begin_gesture(Editor::water_label(false));
    editor.finish_water_stroke(gesture, stroke(false));
    assert_eq!(editor.map.children(Some(layer)), lakes);
    assert_eq!(editor.map.children(None), &[layer]);

    // Undoing both steps removes the Lake and the Layer it created.
    editor.undo();
    assert!(editor.map.contains(lakes[0]));
    editor.undo();
    assert!(!editor.map.contains(layer));
}

#[test]
fn undo_restores_the_selection_and_redo_reapplies() {
    let mut editor = editor();
    let layer = editor.new_layer().unwrap();
    editor.undo();
    assert!(!editor.map.contains(layer));
    assert!(editor.selection().is_empty());
    assert_eq!(editor.active_layer(), None);
    editor.redo();
    assert_eq!(editor.selection(), &[layer]);
}

#[test]
fn refusals_show_a_message_and_change_nothing() {
    let mut editor = editor();
    let layer = editor.new_layer().unwrap();
    editor
        .run(Command::SetLocked {
            id: layer,
            locked: true,
        })
        .unwrap();
    let before = editor.map.view();
    assert_eq!(
        editor.run(Command::RenameLayer {
            id: layer,
            name: "Nope".into(),
        }),
        Err(ExecuteError::Command(CommandError::Locked(layer)))
    );
    assert_eq!(editor.map.view(), before);
    let notice = editor.notice().expect("a visible message");
    assert!(notice.is_error());
    assert_eq!(notice.text, "“Layer 1” is locked");
}

#[test]
fn a_gesture_is_one_undo_step_and_cancel_restores() {
    let mut editor = editor();
    let layer = editor.new_layer().unwrap();
    let gesture = editor.begin_gesture("Change Opacity");
    for opacity in [0.8, 0.5, 0.2] {
        editor.gesture(&gesture, Command::SetOpacity { id: layer, opacity });
    }
    editor.end_gesture(gesture);
    assert_eq!(editor.undo_label(), Some("Change Opacity"));
    editor.undo();
    assert_eq!(opacity(&editor, layer), 1.0);

    let gesture = editor.begin_gesture("Change Opacity");
    editor.gesture(
        &gesture,
        Command::SetOpacity {
            id: layer,
            opacity: 0.3,
        },
    );
    editor.cancel_gesture(gesture);
    assert_eq!(opacity(&editor, layer), 1.0);
    assert_eq!(editor.redo_label(), Some("Change Opacity"));
}

#[test]
fn placing_a_label_creates_a_layer_and_is_one_undo_step() {
    let mut editor = editor();
    let (id, gesture) = editor.place_label([50.0, 50.0]).unwrap();
    let mut label = editor.label(id).unwrap().clone();
    label.text = "Thule".into();
    editor.gesture(
        &gesture,
        Command::SetLabel {
            id,
            label: Box::new(label),
        },
    );
    editor.end_gesture(gesture);
    assert_eq!(editor.selection(), &[id]);
    assert_eq!(editor.label(id).unwrap().text, "Thule");
    assert_eq!(editor.undo_label(), Some("Add Label"));
    editor.undo();
    assert!(editor.map.children(None).is_empty());
    assert!(editor.selection().is_empty());
}

#[test]
fn cancelling_a_placement_restores_the_selection() {
    let mut editor = editor();
    let layer = editor.new_layer().unwrap();
    let (_, gesture) = editor.place_label([50.0, 50.0]).unwrap();
    editor.cancel_gesture(gesture);
    assert_eq!(editor.map.children(Some(layer)).len(), 0);
    assert_eq!(editor.selection(), &[layer]);
}

#[test]
fn picking_finds_the_topmost_unlocked_visible_label() {
    let mut editor = editor();
    let (below, gesture) = editor.place_label([50.0, 50.0]).unwrap();
    editor.end_gesture(gesture);
    let (above, gesture) = editor.place_label([52.0, 50.0]).unwrap();
    editor.end_gesture(gesture);
    assert_eq!(editor.pick([50.0, 50.0]), Some(above));
    assert_eq!(editor.pick([-500.0, 50.0]), None);
    editor
        .run(Command::SetLocked {
            id: above,
            locked: true,
        })
        .unwrap();
    assert_eq!(editor.pick([50.0, 50.0]), Some(below));
    let layer = editor.map.parent(below).unwrap();
    editor
        .run(Command::SetVisible {
            id: layer,
            visible: false,
        })
        .unwrap();
    assert_eq!(editor.pick([50.0, 50.0]), None);
}

fn opacity(editor: &Editor, id: Id) -> f32 {
    match editor.map.node(id) {
        Some(Node::Layer(layer)) => layer.opacity,
        _ => panic!("a Layer"),
    }
}

fn editor_with_square() -> (Editor, Id) {
    let mut editor = editor();
    let layer = editor.new_layer().unwrap();
    let id = Id::new();
    editor
        .run(Command::CreateLandmass {
            id,
            parent: layer,
            index: None,
            geometry: MultiPolygon(vec![ssmap_core::Polygon {
                exterior: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                holes: Vec::new(),
            }]),
            fill: None,
        })
        .unwrap();
    (editor, id)
}

fn node(index: usize) -> NodeRef {
    NodeRef {
        part: 0,
        ring: 0,
        index,
    }
}

#[test]
fn node_edit_mode_follows_the_selection_and_edits_undo_cleanly() {
    let (mut editor, id) = editor_with_square();
    editor.enter_node_edit(id);
    assert_eq!(editor.selection(), &[id]);

    // A drag is one gesture and one Undo step, however many frames it takes.
    editor.select_nodes(vec![node(2)]);
    let gesture = editor.begin_gesture("Move Points");
    for step in 1..=5 {
        editor.gesture_replace(
            &gesture,
            Command::MoveNodes {
                id,
                nodes: vec![node(2)],
                offset: [f64::from(step), 0.0],
            },
        );
    }
    editor.end_gesture(gesture);
    assert_eq!(
        editor.edited_outline().unwrap().node(node(2)),
        Some([15.0, 10.0])
    );
    editor.undo();
    assert_eq!(
        editor.edited_outline().unwrap().node(node(2)),
        Some([10.0, 10.0])
    );
    assert!(editor.node_edit().is_some_and(|edit| edit.nodes.is_empty()));

    editor.insert_node(node(0), [5.0, -2.0]);
    assert_eq!(editor.node_edit().unwrap().nodes, vec![node(1)]);
    assert_eq!(editor.edited_shape(), vec![5]);
    editor.delete_selected_nodes();
    assert_eq!(editor.edited_shape(), vec![4]);

    // Leaving, or selecting something else, ends the mode.
    editor.select(Vec::new());
    assert!(editor.node_edit().is_none());
}

#[test]
fn deleting_offers_an_undo_that_undoes_it() {
    let (mut editor, id) = editor_with_square();
    editor.select(vec![id]);
    let before = editor.map.view();
    editor.delete_selection();
    assert_ne!(editor.map.view(), before);
    let toast = editor.notice().expect("a toast").clone();
    assert!(!toast.is_error());
    let action = toast.action.expect("an Undo action");
    editor.drop_stale_toast_actions();
    let live = editor.toasts.items()[0].action;
    assert_eq!(live, Some(action));
    editor.run_toast_action(action);
    assert_eq!(editor.map.view(), before);
}

#[test]
fn the_undo_action_goes_once_the_history_moves_on() {
    let (mut editor, id) = editor_with_square();
    editor.select(vec![id]);
    editor.delete_selection();
    let action = editor.notice().unwrap().action.unwrap();
    editor.new_layer();
    editor.drop_stale_toast_actions();
    assert!(editor.toasts.items()[0].action.is_none());
    let after = editor.map.view();
    editor.run_toast_action(action);
    assert_eq!(editor.map.view(), after);
}

#[test]
fn deleting_nodes_offers_an_undo() {
    let (mut editor, id) = editor_with_square();
    editor.enter_node_edit(id);
    editor.select_nodes(vec![node(0)]);
    editor.delete_selected_nodes();
    let toast = editor.notice().expect("a toast");
    assert_eq!(toast.text, "Deleted 1 point");
    assert!(toast.action.is_some());
}

#[test]
fn deleting_nodes_that_would_leave_fewer_than_three_is_refused() {
    let (mut editor, id) = editor_with_square();
    editor.enter_node_edit(id);
    editor.select_nodes(vec![node(0), node(1)]);
    editor.delete_selected_nodes();
    assert_eq!(editor.edited_shape(), vec![4]);
    assert!(editor.notice().is_some_and(|notice| notice.is_error()));
}

#[test]
fn union_and_split_apart_work_on_the_selection_in_one_undo_step() {
    let (mut editor, a) = editor_with_square();
    let layer = editor.map.parent(a).unwrap();
    let b = Id::new();
    editor
        .run(Command::CreateLandmass {
            id: b,
            parent: layer,
            index: None,
            geometry: MultiPolygon(vec![ssmap_core::Polygon {
                exterior: vec![[5.0, 5.0], [20.0, 5.0], [20.0, 20.0], [5.0, 20.0]],
                holes: Vec::new(),
            }]),
            fill: None,
        })
        .unwrap();
    editor.select(vec![a, b]);
    assert!(editor.can_combine() && !editor.can_split());
    editor.union_selection();
    assert_eq!(editor.selection(), &[b]);
    assert!(!editor.map.contains(a));

    editor.undo();
    assert!(editor.map.contains(a) && editor.map.contains(b));

    // Two islands split into two Landmasses with one Undo step.
    editor.select(vec![a]);
    editor
        .run(Command::AddRegion {
            id: a,
            region: MultiPolygon(vec![ssmap_core::Polygon {
                exterior: vec![[50.0, 50.0], [60.0, 50.0], [60.0, 60.0], [50.0, 60.0]],
                holes: Vec::new(),
            }]),
        })
        .unwrap();
    assert!(editor.can_split());
    let before = editor.map.children(Some(layer)).len();
    editor.split_selection();
    assert_eq!(editor.map.children(Some(layer)).len(), before + 1);
    assert_eq!(editor.selection().len(), 2);
    editor.undo();
    assert_eq!(editor.map.children(Some(layer)).len(), before);
}
/// An Editor whose sprites come from the core crate's test pack.
fn editor_with_assets() -> Editor {
    use ssmap_core::asset::{PackLibrary, PackRoots};
    let roots = PackRoots {
        built_in: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/core/tests/packs"),
        user: None,
    };
    let (library, problems) = PackLibrary::load(&roots);
    assert!(problems.is_empty(), "{problems:?}");
    let mut editor = editor();
    editor.set_sprites(SpriteStore::new(Arc::new(library)));
    editor
}

fn asset(reference: &str) -> AssetRef {
    reference.parse().unwrap()
}

/// An Editor whose only pack asks for attribution, with one Stamp of it placed.
fn editor_using_credited_asset() -> Editor {
    use ssmap_core::asset::{PackLibrary, PackSource, parse_manifest};
    let manifest = "[pack]\nid = \"icons\"\nname = \"Icons\"\nlicence = \"CC-BY-3.0\"\n\
                    credits = \"Icons made by Lorc.\"\nattribution_required = true\n\
                    modified = \"2026-01-01\"\n\n[[asset]]\nid = \"sword\"\nname = \"Sword\"\n\
                    category = \"c\"\nfile = \"sword.svg\"\nanchor = [0.5, 0.5]\nwidth = 8\n\
                    author = \"Lorc\"\n";
    let pack = parse_manifest(
        std::path::Path::new("icons/pack.toml"),
        manifest,
        PackSource::BuiltIn,
    )
    .unwrap();
    let mut library = PackLibrary::default();
    library.add(pack, &mut Vec::new());
    let mut editor = editor();
    editor.set_sprites(SpriteStore::new(Arc::new(library)));
    editor.selected_asset = Some(asset("icons/sword"));
    editor.place_stamp([40.0, 50.0]).unwrap();
    editor
}

#[test]
fn a_map_without_credited_assets_has_no_credit_lines() {
    let mut editor = editor_with_assets();
    editor.selected_asset = Some(asset("test-pack/castle"));
    editor.place_stamp([40.0, 50.0]).unwrap();
    assert!(editor.credit_lines().is_empty());
    assert_eq!(editor.add_credits_label(), None);
}

#[test]
fn adding_the_credits_as_a_label_is_one_undo_step() {
    let mut editor = editor_using_credited_asset();
    let lines = editor.credit_lines();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].starts_with("Sword by Lorc, Icons (CC-BY-3.0)"));

    let id = editor.add_credits_label().unwrap();
    assert_eq!(editor.label(id).unwrap().text, lines.join("\n"));
    assert_eq!(editor.undo_label(), Some("Add Credits Label"));
    editor.undo();
    assert!(!editor.map.contains(id));
    editor.redo();
    assert!(editor.map.contains(id));
}

#[test]
fn a_new_map_places_stamps_and_labels_in_the_template_layers_without_setup() {
    use ssmap_core::{Canvas, DistanceUnit, Scale};
    let canvas = Canvas::new(1000, 800, Scale::new(1.0, DistanceUnit::Miles));
    let (map, layers, problem) = crate::new_map::start_map(canvas);
    assert_eq!(problem, None);
    let mut editor = editor_with_assets();
    editor.map = map;
    editor.default_layers = layers;
    assert_eq!(editor.active_layer(), None);

    editor.selected_asset = Some(asset("test-pack/castle"));
    let stamp = editor.place_stamp([40.0, 50.0]).unwrap();
    assert_eq!(editor.map.parent(stamp), layers.assets);
    let (label, gesture) = editor.place_label([60.0, 50.0]).unwrap();
    editor.end_gesture(gesture);
    assert_eq!(editor.map.parent(label), layers.labels);
    assert_eq!(editor.default_layer(Placing::Land), layers.land);
    assert_eq!(editor.map.children(layers.assets), [stamp]);
}

#[test]
fn the_stamp_tool_places_the_selected_asset_in_one_undo_step() {
    let mut editor = editor_with_assets();
    assert_eq!(editor.place_stamp([40.0, 50.0]), None, "no Asset chosen");
    assert!(editor.map.children(None).is_empty(), "and nothing was made");

    editor.selected_asset = Some(asset("test-pack/castle"));
    let id = editor.place_stamp([40.0, 50.0]).unwrap();
    let stamp = editor.stamp(id).unwrap();
    assert_eq!(stamp.asset, asset("test-pack/castle"));
    assert_eq!(stamp.position, [40.0, 50.0]);
    assert_eq!(editor.selection(), &[id]);
    // The Layer the Map lacked was made in the same step.
    assert_eq!(editor.map.children(None).len(), 1);
    assert_eq!(editor.undo_label(), Some("Add Stamp"));
    editor.undo();
    assert!(editor.map.children(None).is_empty());
}

#[test]
fn a_variant_group_gives_its_members_and_stamps_keep_what_they_got() {
    let mut editor = editor_with_assets();
    editor.selected_asset = Some(asset("test-pack/mountain-1"));
    let mut seen = std::collections::HashSet::new();
    for i in 0..60 {
        let id = editor.place_stamp([f64::from(i), 0.0]).unwrap();
        let got = editor.stamp(id).unwrap().asset.to_string();
        assert!(
            got == "test-pack/mountain-1" || got == "test-pack/mountain-2",
            "{got}"
        );
        seen.insert(got);
    }
    assert_eq!(seen.len(), 2, "both variants turn up in 60 tries");
}

#[test]
fn picking_goes_by_silhouette_and_the_topmost_stamp_wins() {
    let mut editor = editor_with_assets();
    editor.selected_asset = Some(asset("test-pack/castle"));
    let below = editor.place_stamp([200.0, 200.0]).unwrap();
    let above = editor.place_stamp([210.0, 200.0]).unwrap();
    // The Anchor is on the art, so a point just above it is inside the Silhouette.
    assert_eq!(editor.pick([205.0, 190.0]), Some(above));
    assert_eq!(editor.pick([-50.0, -50.0]), None);
    // The box's corner, outside the outline, finds nothing.
    let bounds = editor.stamp_bounds(below).unwrap();
    assert_eq!(
        editor.pick([bounds.min[0] + 0.1, bounds.min[1] + 0.1]),
        None
    );

    // A locked Stamp can't be picked there, so what's beneath is.
    editor
        .run(Command::SetLocked {
            id: above,
            locked: true,
        })
        .unwrap();
    assert_eq!(editor.pick([205.0, 190.0]), Some(below));
}

fn square(min: f64, max: f64) -> MultiPolygon {
    MultiPolygon(vec![ssmap_core::Polygon {
        exterior: vec![[min, min], [max, min], [max, max], [min, max]],
        holes: Vec::new(),
    }])
}

/// An Editor with a Landmass over (0,0)-(200,200) holding a Lake over (80,80)-(120,120).
fn editor_with_island() -> (Editor, Id) {
    let mut editor = editor_with_assets();
    let layer = editor.new_layer().unwrap();
    editor
        .run(Command::CreateLandmass {
            id: Id::new(),
            parent: layer,
            index: None,
            geometry: square(0.0, 200.0),
            fill: None,
        })
        .unwrap();
    editor
        .run(Command::CreateLake {
            id: Id::new(),
            parent: layer,
            index: None,
            geometry: square(80.0, 120.0),
            fill: None,
        })
        .unwrap();
    editor.selected_asset = Some(asset("test-pack/mountain-1"));
    editor.brush_size = 400.0;
    editor.scatter.density = 40.0;
    editor.scatter.spacing = 0.0;
    (editor, layer)
}

fn stamps_in(editor: &Editor, layer: Id) -> Vec<Stamp> {
    editor
        .map
        .children(Some(layer))
        .iter()
        .filter_map(|&id| editor.stamp(id).cloned())
        .collect()
}

#[test]
fn a_scatter_stroke_is_one_undo_step_and_land_only_keeps_off_water_and_lakes() {
    let (mut editor, layer) = editor_with_island();
    let before = editor.map.children(Some(layer)).len();
    assert!(editor.scatter_stamps(&[[20.0, 100.0], [180.0, 100.0]]));
    assert_eq!(editor.undo_label(), Some("Scatter Stamps"));
    let placed = stamps_in(&editor, layer);
    assert!(placed.len() > 10, "{}", placed.len());
    let lake = square(80.0, 120.0);
    for stamp in &placed {
        let [x, y] = stamp.position;
        assert!((0.0..=200.0).contains(&x) && (0.0..=200.0).contains(&y));
        assert!(!lake.contains(stamp.position), "{:?}", stamp.position);
    }
    editor.undo();
    assert_eq!(editor.map.children(Some(layer)).len(), before);
}

#[test]
fn water_only_scatters_off_the_land() {
    let (mut editor, layer) = editor_with_island();
    editor.scatter.placement = ssmap_core::scatter::Placement::Water;
    assert!(editor.scatter_stamps(&[[100.0, 100.0], [300.0, 300.0]]));
    let land = square(0.0, 200.0);
    let lake = square(80.0, 120.0);
    for stamp in stamps_in(&editor, layer) {
        assert!(!land.contains(stamp.position) || lake.contains(stamp.position));
    }
}

#[test]
fn the_stamp_eraser_only_touches_the_active_layer() {
    let (mut editor, layer) = editor_with_island();
    editor.scatter.placement = ssmap_core::scatter::Placement::Anywhere;
    assert!(editor.scatter_stamps(&[[100.0, 100.0]]));
    let other = editor.new_layer().unwrap();
    assert!(editor.scatter_stamps(&[[100.0, 100.0]]));
    assert_eq!(editor.active_layer(), Some(other));
    let (here, there) = (
        stamps_in(&editor, other).len(),
        stamps_in(&editor, layer).len(),
    );
    assert!(here > 0 && there > 0);

    assert!(editor.erase_stamps(&[[100.0, 100.0]]));
    assert_eq!(editor.undo_label(), Some("Delete"));
    assert_eq!(stamps_in(&editor, other).len(), 0);
    assert_eq!(
        stamps_in(&editor, layer).len(),
        there,
        "the other Layer is intact"
    );
    editor.undo();
    assert_eq!(stamps_in(&editor, other).len(), here);
}

#[test]
fn shuffle_variant_is_one_undo_step() {
    let mut editor = editor_with_assets();
    editor.selected_asset = Some(asset("test-pack/mountain-1"));
    let ids: Vec<Id> = (0..20)
        .map(|i| editor.place_stamp([f64::from(i), 0.0]).unwrap())
        .collect();
    editor.select(ids.clone());
    let before: Vec<AssetRef> = ids
        .iter()
        .map(|&id| editor.stamp(id).unwrap().asset.clone())
        .collect();
    assert!(editor.can_shuffle());
    assert!(editor.shuffle_variants());
    assert_eq!(editor.undo_label(), Some("Shuffle Variant"));
    let after: Vec<AssetRef> = ids
        .iter()
        .map(|&id| editor.stamp(id).unwrap().asset.clone())
        .collect();
    assert_ne!(before, after);
    editor.undo();
    let restored: Vec<AssetRef> = ids
        .iter()
        .map(|&id| editor.stamp(id).unwrap().asset.clone())
        .collect();
    assert_eq!(before, restored);
    assert_ne!(editor.undo_label(), Some("Shuffle Variant"));
}

fn create_stamp(layer: Id) -> (Id, Command) {
    let id = Id::new();
    let command = Command::CreateStamp {
        id,
        parent: layer,
        index: None,
        stamp: Box::new(Stamp::new("pack/asset".parse().unwrap(), [1.0, 2.0])),
    };
    (id, command)
}

#[test]
fn a_gesture_commits_one_undo_step() {
    let mut editor = editor();
    let layer = editor.new_layer().unwrap();
    let undo_steps = editor.history.undo_len();
    let id = editor.begin_gesture("Two Stamps");
    assert!(editor.gesture_open());
    let (a, first) = create_stamp(layer);
    let (b, second) = create_stamp(layer);
    assert!(editor.gesture(&id, first));
    assert!(editor.gesture(&id, second));
    assert!(editor.end_gesture(id));
    assert!(!editor.gesture_open());
    assert_eq!(editor.history.undo_len(), undo_steps + 1);
    assert_eq!(editor.undo_label(), Some("Two Stamps"));
    editor.undo();
    assert!(editor.stamp(a).is_none() && editor.stamp(b).is_none());
}

#[test]
fn cancelling_a_gesture_rolls_it_back() {
    let mut editor = editor();
    let layer = editor.new_layer().unwrap();
    let undo_steps = editor.history.undo_len();
    let selection = editor.selection().to_vec();
    let id = editor.begin_gesture("Stamp");
    let (stamp, command) = create_stamp(layer);
    assert!(editor.gesture(&id, command));
    editor.select(vec![stamp]);
    assert!(editor.cancel_gesture(id));
    assert!(editor.stamp(stamp).is_none());
    assert_eq!(editor.selection(), &selection[..]);
    assert_eq!(editor.history.undo_len(), undo_steps);
}

#[test]
fn replacing_in_a_gesture_keeps_one_commands_worth() {
    let mut editor = editor();
    let layer = editor.new_layer().unwrap();
    let id = editor.begin_gesture("Stamp");
    let (first, command) = create_stamp(layer);
    assert!(editor.gesture(&id, command));
    let (second, command) = create_stamp(layer);
    assert!(editor.gesture_replace(&id, command));
    assert!(editor.end_gesture(id));
    assert!(editor.stamp(first).is_none());
    assert!(editor.stamp(second).is_some());
}

#[test]
fn a_stale_token_is_a_no_op() {
    let mut editor = editor();
    let layer = editor.new_layer().unwrap();
    let old = editor.begin_gesture("First");
    assert!(editor.cancel_open_gesture());
    let current = editor.begin_gesture("Second");
    let (stamp, command) = create_stamp(layer);
    assert!(!editor.gesture(&old, command.clone()));
    assert!(!editor.gesture_replace(&old, command));
    assert!(editor.stamp(stamp).is_none());
    assert!(!editor.end_gesture(old));
    assert!(editor.gesture_open());
    assert!(editor.end_gesture(current));
    // Stale again once its own gesture is over, and so is a foreign Editor's token.
    // Both Editors issue the same number of tokens, so per-Editor counters would collide.
    let mut other = self::tests::editor();
    for label in ["One", "Two"] {
        let token = other.begin_gesture(label);
        assert!(other.end_gesture(token));
    }
    let foreign = other.begin_gesture("Three");
    let own = editor.begin_gesture("Third");
    assert!(!editor.cancel_gesture(foreign));
    assert!(editor.gesture_open());
    assert!(editor.cancel_gesture(own));
    assert!(other.cancel_open_gesture());
}

#[test]
fn cancel_open_gesture_invalidates_the_token() {
    let mut editor = editor();
    let layer = editor.new_layer().unwrap();
    assert!(!editor.cancel_open_gesture());
    let id = editor.begin_gesture("Stamp");
    let (stamp, command) = create_stamp(layer);
    assert!(editor.gesture(&id, command));
    assert!(editor.cancel_open_gesture());
    assert!(!editor.gesture_open());
    assert!(editor.stamp(stamp).is_none());
    let (_, command) = create_stamp(layer);
    assert!(!editor.gesture(&id, command));
    assert!(!editor.end_gesture(id));
}

#[test]
fn in_gesture_commits_on_ok_and_rolls_back_on_err() {
    let mut editor = editor();
    let layer = editor.new_layer().unwrap();
    let undo_steps = editor.history.undo_len();
    let (kept, command) = create_stamp(layer);
    let result = editor.in_gesture("Keep", |g| {
        assert!(g.run(command));
        Ok::<_, &str>(7)
    });
    assert_eq!(result, Some(Ok(7)));
    assert!(editor.stamp(kept).is_some());
    assert_eq!(editor.history.undo_len(), undo_steps + 1);

    let (dropped, command) = create_stamp(layer);
    let result = editor.in_gesture("Drop", |g| {
        assert!(g.run(command));
        Err::<(), _>("nope")
    });
    assert_eq!(result, Some(Err("nope")));
    assert!(editor.stamp(dropped).is_none());
    assert!(!editor.gesture_open());
    assert_eq!(editor.history.undo_len(), undo_steps + 1);
}

#[test]
fn in_gesture_does_not_open_inside_another_gesture() {
    let mut editor = editor();
    let outer = editor.begin_gesture("Outer");
    assert!(editor.in_gesture("Inner", |_| Ok::<(), ()>(())).is_none());
    assert!(editor.gesture_open());
    editor.cancel_gesture(outer);
}

#[test]
fn history_begin_refuses_while_a_transaction_is_open() {
    let mut editor = editor();
    let id = editor.begin_gesture("Open");
    assert_eq!(
        editor.history.begin("Second", Vec::new()),
        Err(ssmap_core::HistoryError::TransactionOpen)
    );
    // The editor shows a refusal instead of panicking, and the first gesture survives.
    let second = editor.begin_gesture("Second");
    assert!(!editor.end_gesture(second));
    assert!(editor.end_gesture(id));
}
