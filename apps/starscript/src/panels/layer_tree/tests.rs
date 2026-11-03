use super::*;
use ssmap_core::{Canvas, DistanceUnit, Layer, Object, ObjectKind, Op, Scale, Subtree};

/// Top level, bottom first: A, B, C. B holds b1 (bottom) and b2.
fn map() -> (Map, [Id; 5]) {
    let mut map = Map::new(Canvas::new(10, 10, Scale::new(1.0, DistanceUnit::Miles)));
    let ids = [Id::new(), Id::new(), Id::new(), Id::new(), Id::new()];
    let [a, b, c, b1, b2] = ids;
    let layer = |id, children| Subtree {
        id,
        node: Node::Layer(Layer::new("")),
        children,
    };
    for (index, subtree) in [
        layer(a, vec![]),
        layer(b, vec![layer(b1, vec![]), layer(b2, vec![])]),
        layer(c, vec![]),
    ]
    .into_iter()
    .enumerate()
    {
        map.apply(Op::Insert {
            parent: None,
            index,
            subtree,
        })
        .unwrap();
    }
    (map, ids)
}

#[test]
fn rows_list_the_top_of_the_stack_first() {
    let (map, [a, b, c, b1, b2]) = map();
    let ids: Vec<(Id, usize)> = rows(&map, &HashSet::new())
        .into_iter()
        .map(|r| (r.id, r.depth))
        .collect();
    assert_eq!(ids, vec![(c, 0), (b, 0), (b2, 1), (b1, 1), (a, 0)]);
    let collapsed = HashSet::from([b]);
    assert_eq!(rows(&map, &collapsed).len(), 3);
}

fn drop_on(map: &mut Map, dragged: Id, target: DropTarget) {
    let command = move_command(map, dragged, target).expect("a move");
    map.execute(&command).unwrap();
}

#[test]
fn dropping_above_a_row_puts_it_higher_in_the_stack() {
    let (mut map, [a, b, c, ..]) = map();
    // Drag A (bottom) above C (top).
    drop_on(&mut map, a, DropTarget::Above(c));
    assert_eq!(map.children(None), &[b, c, a]);
    // Drag A back below B.
    drop_on(&mut map, a, DropTarget::Below(b));
    assert_eq!(map.children(None), &[a, b, c]);
}

#[test]
fn dropping_into_a_layer_puts_it_on_top_inside() {
    let (mut map, [a, b, _, b1, b2]) = map();
    drop_on(&mut map, a, DropTarget::Into(b));
    assert_eq!(map.children(Some(b)), &[b1, b2, a]);
    // And out again, beside a nested row.
    drop_on(&mut map, b2, DropTarget::Below(b));
    assert_eq!(map.parent(b2), None);
}

#[test]
fn no_op_and_self_nesting_drops_do_nothing() {
    let (map, [a, b, c, b1, _]) = map();
    assert_eq!(move_command(&map, b, DropTarget::Above(a)), None);
    assert_eq!(move_command(&map, b, DropTarget::Below(c)), None);
    assert_eq!(move_command(&map, b, DropTarget::Into(b)), None);
    assert_eq!(move_command(&map, b, DropTarget::Above(b1)), None);
}

#[test]
fn a_depth_sorted_layer_lists_its_stamps_in_draw_order_and_refuses_reordering() {
    use ssmap_core::{Object, Stamp};
    let (mut map, [a, ..]) = map();
    let stamps = [Id::new(), Id::new()];
    for (index, (id, y)) in stamps.into_iter().zip([30.0, 10.0]).enumerate() {
        let stamp = Stamp::new("p/a".parse().unwrap(), [0.0, y]);
        map.apply(Op::Insert {
            parent: Some(a),
            index,
            subtree: Subtree {
                id,
                node: Node::Object(Object::new(ObjectKind::Stamp(stamp))),
                children: vec![],
            },
        })
        .unwrap();
    }
    let listed = |map: &Map| -> Vec<Id> {
        rows(map, &HashSet::new())
            .into_iter()
            .map(|r| r.id)
            .filter(|id| stamps.contains(id))
            .collect()
    };
    // Top of the list is the top of the draw order: the last tree child.
    assert_eq!(listed(&map), vec![stamps[1], stamps[0]]);
    map.execute(&Command::SetDepthSort {
        id: a,
        depth_sort: true,
    })
    .unwrap();
    assert_eq!(listed(&map), vec![stamps[0], stamps[1]]);
    assert!(is_depth_sorted_stamp(&map, stamps[0]));
    assert_eq!(
        move_command(&map, stamps[0], DropTarget::Above(stamps[1])),
        None
    );
}

#[test]
fn every_tree_action_undoes_and_redoes_cleanly() {
    use crate::editor::Editor;
    let mut editor = Editor::new(Map::new(Canvas::new(
        10,
        10,
        Scale::new(1.0, DistanceUnit::Miles),
    )));
    let empty = editor.map().view();
    let a = editor.new_layer().unwrap();
    let b = editor.new_layer().unwrap();
    let c = editor.new_layer().unwrap();
    let drag = |editor: &mut Editor, id, target| {
        let command = move_command(editor.map(), id, target).expect("a move");
        assert!(editor.run(command).is_ok());
    };
    assert!(
        editor
            .run(Command::RenameLayer {
                id: a,
                name: "Coast".into()
            })
            .is_ok()
    );
    drag(&mut editor, c, DropTarget::Into(a)); // nest
    drag(&mut editor, b, DropTarget::Below(a)); // reorder
    assert!(
        editor
            .run(Command::SetVisible {
                id: b,
                visible: false
            })
            .is_ok()
    );
    assert!(
        editor
            .run(Command::SetLocked {
                id: c,
                locked: true
            })
            .is_ok()
    );
    assert!(
        editor
            .run(Command::SetLocked {
                id: c,
                locked: false
            })
            .is_ok()
    );
    editor.select(vec![b]);
    editor.delete_selection();
    let done = editor.map().view();

    while editor.can_undo() {
        editor.undo();
        editor.map().validate().unwrap();
    }
    assert_eq!(editor.map().view(), empty);
    while editor.can_redo() {
        editor.redo();
    }
    assert_eq!(editor.map().view(), done);
}

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

struct Fixture {
    editor: Editor,
    tree: LayerTree,
}

fn harness(editor: Editor) -> Harness<'static, Fixture> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(260.0, 700.0))
        .build_ui_state(
            |ui, fixture: &mut Fixture| {
                theme::install(&ui.ctx().clone());
                fixture.tree.show(ui, &mut fixture.editor);
                // The sidebar draws the counts in its bottom bar.
                if let Some(counts) = fixture.tree.counts_text() {
                    ui.label(counts);
                }
            },
            Fixture {
                editor,
                tree: LayerTree::default(),
            },
        );
    harness.run();
    harness
}

fn editor_with_two_layers() -> (Editor, Id, Id) {
    let mut editor = Editor::new(Map::new(Canvas::new(
        100,
        100,
        Scale::new(1.0, DistanceUnit::Miles),
    )));
    let a = editor.new_layer().unwrap();
    let b = editor.new_layer().unwrap();
    (editor, a, b)
}

#[test]
fn a_hidden_row_is_italic_with_the_slashed_eye() {
    let (mut editor, a, b) = editor_with_two_layers();
    assert!(
        editor
            .run(Command::SetVisible {
                id: a,
                visible: false
            })
            .is_ok()
    );
    assert!(RowState::of(editor.map(), a).italic());
    assert!(!RowState::of(editor.map(), b).italic());
    let harness = harness(editor);
    // Only the hidden row shows its eye (as "Show") without hover.
    harness.get_by_label("Show");
    // The one "Hide" is the settings row's; the other rows show none without hover.
    assert_eq!(harness.query_all_by_label("Hide").count(), 1);
}

#[test]
fn a_child_of_a_locked_layer_shows_a_dashed_padlock_that_does_nothing() {
    let (mut editor, a, b) = editor_with_two_layers();
    let command = move_command(editor.map(), b, DropTarget::Into(a)).unwrap();
    assert!(editor.run(command).is_ok());
    assert!(
        editor
            .run(Command::SetLocked {
                id: a,
                locked: true
            })
            .is_ok()
    );
    let state = RowState::of(editor.map(), b);
    assert!(state.locked_by_parent && !state.locked);
    assert!(!RowState::of(editor.map(), a).locked_by_parent);

    let revision = editor.map().revision();
    let selection = editor.selection().to_vec();
    let mut harness = harness(editor);
    harness.get_by_label("Unlock"); // the parent's own filled padlock
    harness.get_by_label("Locked by parent Layer").click();
    harness.run();
    let editor = &harness.state().editor;
    assert_eq!(editor.map().revision(), revision);
    assert!(!editor.map().node(b).unwrap().locked());
    assert_eq!(editor.selection(), selection);
}

#[test]
fn the_active_layer_shows_the_target() {
    let (editor, a, _) = editor_with_two_layers();
    let mut harness = harness(editor);
    harness.state_mut().editor.select(vec![a]);
    harness.run();
    // One target, on the one active Layer.
    assert_eq!(
        harness
            .query_all_by_label("Active Layer (new Objects go here)")
            .count(),
        1
    );
}

#[test]
fn indicators_show_clip_depth_sort_and_opacity() {
    let (mut editor, a, _) = editor_with_two_layers();
    assert!(
        editor
            .run(Command::SetDepthSort {
                id: a,
                depth_sort: true
            })
            .is_ok()
    );
    let harness = harness(editor);
    harness.get_by_label("Depth-sorted");
    assert!(harness.query_by_label("Clipped to Landmass").is_none());
    assert!(harness.query_by_label_contains("%").is_none());
}

#[test]
fn linked_rows_show_the_link_icon_and_link_and_unlink_are_undoable() {
    let (mut editor, a, b) = editor_with_two_layers();
    editor.select(vec![a]);
    assert!(!editor.can_link(), "one row has nothing to link to");
    editor.select(vec![a, b]);
    assert!(editor.can_link() && !editor.can_unlink());
    let plain = editor.map().view();
    assert!(editor.link_selection());
    assert!(editor.map().is_linked(a) && editor.map().is_linked(b));
    assert!(editor.can_unlink());

    let mut harness = harness(editor);
    assert_eq!(harness.query_all_by_label("Linked").count(), 2);

    let editor = &mut harness.state_mut().editor;
    assert!(editor.unlink_selection());
    harness.run();
    assert!(harness.query_by_label("Linked").is_none());

    let editor = &mut harness.state_mut().editor;
    editor.undo();
    harness.run();
    assert_eq!(harness.query_all_by_label("Linked").count(), 2);
    let editor = &mut harness.state_mut().editor;
    editor.undo();
    assert_eq!(editor.map().view(), plain);
    editor.redo();
    assert!(editor.map().is_linked(a));
}

#[test]
fn a_stamp_can_be_renamed_and_an_empty_name_restores_the_default() {
    use ssmap_core::Stamp;
    let (mut editor, a, _) = editor_with_two_layers();
    let id = Id::new();
    let stamp = Stamp::new("p/a".parse().unwrap(), [0.0, 0.0]);
    editor
        .map_mut()
        .apply(Op::Insert {
            parent: Some(a),
            index: 0,
            subtree: Subtree {
                id,
                node: Node::Object(Object::new(ObjectKind::Stamp(stamp))),
                children: vec![],
            },
        })
        .unwrap();
    assert_eq!(node_name(editor.map(), id), "Stamp");
    assert!(
        editor
            .run(Command::RenameObject {
                id,
                name: "  Old Keep ".into()
            })
            .is_ok()
    );
    assert_eq!(node_name(editor.map(), id), "Old Keep");
    assert!(
        editor
            .run(Command::RenameObject {
                id,
                name: String::new()
            })
            .is_ok()
    );
    assert_eq!(node_name(editor.map(), id), "Stamp");
    editor.undo();
    assert_eq!(node_name(editor.map(), id), "Old Keep");
    // Layers are renamed with their own Command.
    assert!(
        editor
            .run(Command::RenameObject {
                id: a,
                name: "x".into()
            })
            .is_err()
    );
}

#[test]
fn the_footer_counts_objects_and_layers() {
    use ssmap_core::{MultiPolygon, Polygon};
    let (mut editor, a, _) = editor_with_two_layers();
    assert_eq!(footer_text(0, 2), "0 Objects · 2 Layers");
    assert_eq!(footer_text(1, 1), "1 Object · 1 Layer");
    assert!(
        editor
            .run(Command::CreateLandmass {
                id: Id::new(),
                parent: a,
                index: None,
                geometry: MultiPolygon(vec![Polygon {
                    exterior: vec![[0.0, 0.0], [50.0, 0.0], [50.0, 50.0], [0.0, 50.0]],
                    holes: vec![],
                }]),
                fill: None,
            })
            .is_ok()
    );
    assert_eq!(counts(editor.map()), (1, 2));
    let mut harness = harness(editor);
    harness.get_by_label("1 Object · 2 Layers");
    // The count follows the Map.
    harness.state_mut().editor.new_layer();
    harness.run();
    harness.get_by_label("1 Object · 3 Layers");
}

#[test]
fn grouping_is_one_undo_step_and_ungrouping_restores_the_layers() {
    let (mut editor, a, b) = editor_with_two_layers();
    let before = editor.map().view();
    editor.select(vec![a, b]);
    let group = editor.group_selection().expect("a Group");
    assert_eq!(editor.selection(), [group]);
    assert_eq!(editor.map().parent(a), Some(group));
    assert_eq!(editor.map().parent(b), Some(group));
    let grouped = editor.map().view();
    assert!(editor.ungroup_selection());
    assert_eq!(editor.map().parent(a), None);
    assert_eq!(editor.selection().len(), 2);
    editor.undo();
    assert_eq!(editor.map().view(), grouped);
    editor.undo();
    assert_eq!(editor.map().view(), before);
}

#[test]
fn an_empty_group_can_be_created_and_collapses_with_its_contents() {
    let (mut editor, a, _) = editor_with_two_layers();
    let group = editor.new_group().expect("a Group");
    assert!(editor.map().children(Some(group)).is_empty());
    let command = move_command(editor.map(), a, DropTarget::Into(group)).unwrap();
    assert!(editor.run(command).is_ok());
    let mut collapsed = HashSet::new();
    assert!(rows(editor.map(), &collapsed).iter().any(|r| r.id == a));
    collapsed.insert(group);
    let visible = rows(editor.map(), &collapsed);
    assert!(!visible.iter().any(|r| r.id == a));
    assert!(visible.iter().any(|r| r.id == group));
    let open = rows(editor.map(), &HashSet::new());
    let depth = |id| open.iter().find(|r| r.id == id).unwrap().depth;
    assert_eq!(depth(a), depth(group) + 1);
}

#[test]
fn layers_drag_into_and_out_of_a_group() {
    let (mut editor, a, b) = editor_with_two_layers();
    let group = editor.new_group().unwrap();
    let into = move_command(editor.map(), a, DropTarget::Into(group)).unwrap();
    assert!(editor.run(into).is_ok());
    assert_eq!(editor.map().parent(a), Some(group));
    let out = move_command(editor.map(), a, DropTarget::Above(b)).unwrap();
    assert!(editor.run(out).is_ok());
    assert_eq!(editor.map().parent(a), None);
    editor.undo();
    assert_eq!(editor.map().parent(a), Some(group));
}

#[test]
fn a_locked_group_locks_its_contents_and_refuses_drops() {
    let (mut editor, a, b) = editor_with_two_layers();
    let group = editor.new_group().unwrap();
    let into = move_command(editor.map(), a, DropTarget::Into(group)).unwrap();
    assert!(editor.run(into).is_ok());
    assert!(
        editor
            .run(Command::SetLocked {
                id: group,
                locked: true
            })
            .is_ok()
    );
    assert!(RowState::of(editor.map(), a).locked_by_parent);
    assert!(move_command(editor.map(), b, DropTarget::Into(group)).is_none());
    assert!(move_command(editor.map(), b, DropTarget::Above(a)).is_none());
}

#[test]
fn deleting_a_group_asks_first_naming_it_and_its_contents() {
    let (mut editor, a, _) = editor_with_two_layers();
    let group = editor.new_group().unwrap();
    let into = move_command(editor.map(), a, DropTarget::Into(group)).unwrap();
    assert!(editor.run(into).is_ok());
    assert!(
        editor
            .run(Command::RenameLayer {
                id: group,
                name: "Coast".into()
            })
            .is_ok()
    );
    assert_eq!(filled_group(editor.map(), &[group]), Some(group));
    assert_eq!(filled_group(editor.map(), &[a]), None);
    assert_eq!(contents(editor.map(), &[group]), 1);
    assert_eq!(
        delete_message("Coast", 1),
        "Delete the Group \"Coast\"? Its 1 item will be deleted too."
    );
    editor.select(vec![group]);
    let mut harness = harness(editor);
    harness.state_mut().tree.confirm_delete = Some((group, 1));
    harness.run();
    harness.get_by_label_contains("\"Coast\"");
    harness.get_by_label("Cancel").click();
    harness.run();
    assert!(harness.state().editor.map().contains(group));
    harness.state_mut().tree.confirm_delete = Some((group, 1));
    harness.run();
    harness.get_by_label("Delete").click();
    harness.run();
    let editor = &harness.state().editor;
    assert!(!editor.map().contains(group) && !editor.map().contains(a));
}

/// Three Layers, the lowest inside a Group: the Group, its Layer, then the two others.
fn editor_with_a_group() -> (Editor, Vec<Id>) {
    let (mut editor, a, _) = editor_with_two_layers();
    editor.new_layer().unwrap();
    let group = editor.new_group().unwrap();
    let command = move_command(editor.map(), a, DropTarget::Into(group)).unwrap();
    assert!(editor.run(command).is_ok());
    editor.select(Vec::new());
    let ids = rows(editor.map(), &HashSet::new())
        .iter()
        .map(|row| row.id)
        .collect();
    (editor, ids)
}

fn click_row(harness: &mut Harness<'static, Fixture>, id: Id, modifiers: egui::Modifiers) {
    let name = node_name(harness.state().editor.map(), id);
    harness.get_by_label(&name).click_modifiers(modifiers);
    harness.run();
}

fn sorted(mut ids: Vec<Id>) -> Vec<Id> {
    ids.sort();
    ids
}

#[test]
fn command_click_toggles_a_row_in_and_out_of_the_selection() {
    let (editor, ids) = editor_with_a_group();
    let mut harness = harness(editor);
    click_row(&mut harness, ids[0], egui::Modifiers::NONE);
    click_row(&mut harness, ids[2], egui::Modifiers::COMMAND);
    assert_eq!(
        sorted(harness.state().editor.selection().to_vec()),
        sorted(vec![ids[0], ids[2]])
    );
    click_row(&mut harness, ids[0], egui::Modifiers::COMMAND);
    assert_eq!(harness.state().editor.selection(), &[ids[2]]);
}

#[test]
fn shift_click_selects_the_rows_between_across_groups() {
    let (editor, ids) = editor_with_a_group();
    assert_eq!(ids.len(), 4, "the Group, its Layer and two more");
    let mut harness = harness(editor);
    // From the Group at the top down past its own Layer, then back up to the middle.
    click_row(&mut harness, ids[0], egui::Modifiers::NONE);
    click_row(&mut harness, ids[3], egui::Modifiers::SHIFT);
    assert_eq!(
        sorted(harness.state().editor.selection().to_vec()),
        sorted(ids.clone())
    );
    click_row(&mut harness, ids[2], egui::Modifiers::SHIFT);
    assert_eq!(
        sorted(harness.state().editor.selection().to_vec()),
        sorted(ids[0..=2].to_vec()),
        "the range grows from the same anchor"
    );
}

#[test]
fn a_panel_selection_is_the_canvas_selection() {
    // The panel writes the Editor's Selection, which the Canvas reads for its handles
    // and outline: one list, so the two cannot disagree.
    let (editor, ids) = editor_with_a_group();
    let mut harness = harness(editor);
    click_row(&mut harness, ids[1], egui::Modifiers::NONE);
    assert!(harness.state().editor.is_selected(ids[1]));
    // A selection made elsewhere (the Canvas) shows in the panel's menu: Link is enabled
    // only for a row that is part of a selection of two or more.
    harness.state_mut().editor.select(vec![ids[1], ids[2]]);
    harness.run();
    let name = node_name(harness.state().editor.map(), ids[1]);
    harness.get_by_label(&name).click_secondary();
    harness.run();
    assert!(!NodeT::accesskit_node(&harness.get_by_label("Link")).is_disabled());
}

fn open_menu(harness: &mut Harness<'static, Fixture>, id: Id) {
    let name = node_name(harness.state().editor.map(), id);
    harness.get_by_label(&name).click_secondary();
    harness.run();
}

#[test]
fn the_menu_copies_and_pastes_the_selection() {
    let (editor, ids) = editor_with_a_group();
    let mut harness = harness(editor);
    click_row(&mut harness, ids[2], egui::Modifiers::NONE);
    click_row(&mut harness, ids[3], egui::Modifiers::COMMAND);
    assert!(!harness.state().editor.has_clipboard());
    open_menu(&mut harness, ids[3]);
    harness.get_by_label("Copy").click();
    harness.run();
    assert!(harness.state().editor.has_clipboard());
    assert!(
        harness.state().editor.map().contains(ids[2]),
        "Copy keeps them"
    );

    open_menu(&mut harness, ids[3]);
    harness.get_by_label("Paste").click();
    harness.run();
    assert!(harness.state().tree.paste_requested);
}

#[test]
fn the_menu_cuts_groups_and_deletes_the_selection() {
    let (editor, ids) = editor_with_a_group();
    let mut harness = harness(editor);
    click_row(&mut harness, ids[2], egui::Modifiers::NONE);
    click_row(&mut harness, ids[3], egui::Modifiers::SHIFT);
    open_menu(&mut harness, ids[3]);
    harness.get_by_label("Group").click();
    harness.run();
    let editor = &harness.state().editor;
    let [group] = editor.selection() else {
        panic!("the new Group is selected")
    };
    assert_eq!(editor.map().children(Some(*group)).len(), 2);
    let group = *group;

    open_menu(&mut harness, group);
    harness.get_by_label("Cut").click();
    harness.run();
    assert!(harness.state().editor.has_clipboard());
    assert!(!harness.state().editor.map().contains(group));

    // The Layer inside the first Group is empty, so no confirmation is asked.
    let inner = ids[1];
    click_row(&mut harness, inner, egui::Modifiers::NONE);
    open_menu(&mut harness, inner);
    harness.get_by_label("Delete").click();
    harness.run();
    assert!(!harness.state().editor.map().contains(inner));
}

#[test]
fn the_menu_hides_locks_and_pins_the_whole_selection_in_one_undo_step() {
    let (mut editor, a, b) = editor_with_two_layers();
    editor.select(vec![a, b]);
    let mut harness = harness(editor);
    open_menu(&mut harness, a);
    harness.get_by_label("Hide 2 items").click();
    harness.run();
    let editor = &harness.state().editor;
    assert!(!layer_data(editor, a).visible && !layer_data(editor, b).visible);
    assert_eq!(editor.undo_label(), Some("Hide"));

    open_menu(&mut harness, a);
    harness.get_by_label("Lock 2 items").click();
    harness.run();
    let editor = &harness.state().editor;
    assert!(layer_data(editor, a).locked && layer_data(editor, b).locked);
    assert_eq!(editor.undo_label(), Some("Lock"));

    // Both locked now: the next menu offers Unlock, and one Undo undoes both.
    open_menu(&mut harness, b);
    harness.get_by_label("Unlock 2 items").click();
    harness.run();
    let editor = &harness.state().editor;
    assert!(!layer_data(editor, a).locked && !layer_data(editor, b).locked);
    harness.state_mut().editor.undo();
    harness.run();
    let editor = &harness.state().editor;
    assert!(layer_data(editor, a).locked && layer_data(editor, b).locked);
}

#[test]
fn the_menu_on_a_mixed_selection_turns_the_flag_on_for_all() {
    let (mut editor, a, b) = editor_with_two_layers();
    editor
        .run(Command::SetMoveLocked {
            id: a,
            locked: true,
        })
        .unwrap();
    editor.select(vec![a, b]);
    let mut harness = harness(editor);
    open_menu(&mut harness, b);
    harness.get_by_label("Pin 2 items to Canvas").click();
    harness.run();
    let editor = &harness.state().editor;
    assert!(layer_data(editor, a).pinned && layer_data(editor, b).pinned);
    assert_eq!(editor.undo_label(), Some("Pin to Canvas"));
}

#[test]
fn the_menu_on_an_unselected_row_acts_on_that_row_only() {
    let (mut editor, a, b) = editor_with_two_layers();
    editor.select(vec![a]);
    let mut harness = harness(editor);
    open_menu(&mut harness, b);
    // The menu item comes last, after the settings row's and the rows' own eyes.
    harness
        .get_all_by_label("Hide")
        .last()
        .expect("the menu item")
        .click();
    harness.run();
    let editor = &harness.state().editor;
    assert!(layer_data(editor, a).visible);
    assert!(!layer_data(editor, b).visible);
    assert_eq!(editor.selection(), &[b]);
}

fn layer_data(editor: &Editor, id: Id) -> Layer {
    let Some(Node::Layer(layer)) = editor.map().node(id) else {
        panic!("a Layer")
    };
    layer.clone()
}

fn pointer(harness: &mut Harness<'static, Fixture>, pos: egui::Pos2, pressed: Option<bool>) {
    harness.event(egui::Event::PointerMoved(pos));
    if let Some(pressed) = pressed {
        harness.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    harness.run();
}

#[test]
fn depth_sort_is_in_the_layers_menu_and_nothing_sits_under_the_list() {
    let (mut editor, a, b) = editor_with_two_layers();
    editor.select(vec![a]);
    let mut harness = harness(editor);
    // The settings row is the only place for Opacity; the old sections and the Clipping
    // section are gone from under the list.
    assert!(harness.query_by_label("Opacity").is_some());
    for label in [
        "Appearance",
        "Ordering",
        "Clipping",
        "Clip to",
        "Depth sort",
    ] {
        assert!(harness.query_by_label(label).is_none(), "{label}");
    }

    // Depth sort is a checked item in the Layer's right-click menu.
    open_menu(&mut harness, a);
    harness.get_by_label("Depth sort").click();
    harness.run();
    let editor = &harness.state().editor;
    assert!(layer_data(editor, a).depth_sort);
    assert!(!layer_data(editor, b).depth_sort);
}

#[test]
fn the_settings_row_acts_on_the_selection_or_the_active_layer() {
    let (mut editor, a, b) = editor_with_two_layers();
    editor.select(vec![b]);
    editor.select(Vec::new());
    assert_eq!(editor.active_layer(), Some(b));
    let mut harness = harness(editor);
    // Nothing selected: the active Layer. Hide, Lock and Pin are one Undo step each.
    harness.get_by_label("Hide").click();
    harness.run();
    assert!(!layer_data(&harness.state().editor, b).visible);
    assert_eq!(harness.state().editor.undo_label(), Some("Hide"));
    // The hidden Layer's own row shows an eye too; the settings row comes first.
    harness
        .get_all_by_label("Show")
        .next()
        .expect("the settings row")
        .click();
    harness.run();
    assert!(layer_data(&harness.state().editor, b).visible);

    harness.get_by_label("Pin to Canvas").click();
    harness.run();
    assert!(layer_data(&harness.state().editor, b).pinned);
    assert!(!layer_data(&harness.state().editor, a).pinned);
    harness
        .get_all_by_label("Unpin from Canvas")
        .next()
        .expect("the settings row")
        .click();
    harness.run();
    assert!(!layer_data(&harness.state().editor, b).pinned);

    // With two Layers selected it acts on both, as one Undo step.
    harness.state_mut().editor.select(vec![a, b]);
    harness.run();
    harness.get_by_label("Lock").click();
    harness.run();
    let editor = &harness.state().editor;
    assert!(layer_data(editor, a).locked && layer_data(editor, b).locked);
    assert_eq!(editor.undo_label(), Some("Lock"));
    harness.state_mut().editor.undo();
    harness.run();
    let editor = &harness.state().editor;
    assert!(!layer_data(editor, a).locked && !layer_data(editor, b).locked);
}

#[test]
fn the_opacity_field_sets_every_selected_layer_in_one_undo_step() {
    let (mut editor, a, b) = editor_with_two_layers();
    editor.select(vec![a, b]);
    let mut harness = harness(editor);
    let field = harness.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Opacity");
    let start = field.rect().center();
    harness.hover_at(start);
    harness.run();
    harness.drag_at(start);
    harness.run();
    for step in 1..=4 {
        harness.hover_at(start + egui::vec2(-8.0 * step as f32, 0.0));
        harness.run();
    }
    harness.drop_at(start + egui::vec2(-32.0, 0.0));
    harness.run();
    let editor = &harness.state().editor;
    let (oa, ob) = (layer_data(editor, a).opacity, layer_data(editor, b).opacity);
    assert!(oa < 1.0 && (oa - ob).abs() < f32::EPSILON, "{oa} {ob}");
    assert_eq!(editor.undo_label(), Some("Change Opacity"));
}

#[test]
fn a_layers_pin_shows_on_the_row_and_what_is_inside_it_shows_a_dashed_pin() {
    let (mut editor, a, _) = editor_with_two_layers();
    let island = Id::new();
    editor
        .run(Command::CreateLandmass {
            id: island,
            parent: a,
            index: None,
            geometry: ssmap_core::MultiPolygon(vec![ssmap_core::Polygon {
                exterior: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                holes: Vec::new(),
            }]),
            fill: None,
        })
        .unwrap();
    editor
        .run(Command::SetMoveLocked {
            id: island,
            locked: false,
        })
        .unwrap();
    editor
        .run(Command::SetMoveLocked {
            id: a,
            locked: true,
        })
        .unwrap();
    let state = RowState::of(editor.map(), island);
    assert!(!state.pinned && state.pinned_by_parent);
    assert!(RowState::of(editor.map(), a).pinned);
    let harness = harness(editor);
    harness.get_by_label("Unpin from Canvas");
    harness.get_by_label("Pinned by parent Layer");
}

#[test]
fn one_opacity_drag_in_the_layer_area_is_one_undo_step() {
    let (mut editor, a, _) = editor_with_two_layers();
    editor.select(vec![a]);
    let mut harness = harness(editor);
    let before = harness.state().editor.undo_label().map(str::to_owned);
    let from = harness.get_by_label("Opacity").rect().center();
    pointer(&mut harness, from, Some(true));
    for step in 1..=4 {
        pointer(
            &mut harness,
            from - egui::vec2(step as f32 * 10.0, 0.0),
            None,
        );
    }
    pointer(&mut harness, from - egui::vec2(40.0, 0.0), Some(false));
    harness.run();

    let state = harness.state_mut();
    assert!(layer_data(&state.editor, a).opacity < 1.0);
    assert_eq!(state.editor.undo_label(), Some("Change Opacity"));
    state.editor.undo();
    assert_eq!(layer_data(&state.editor, a).opacity, 1.0);
    assert_eq!(state.editor.undo_label().map(str::to_owned), before);
}

#[test]
fn a_rename_request_expands_the_row_and_opens_its_rename_box() {
    let (editor, ids) = editor_with_a_group();
    let mut harness = harness(editor);
    let map = harness.state().editor.map();
    let child = ids.iter().copied().find(|&id| map.parent(id).is_some());
    let child = child.unwrap();
    let group = map.parent(child).unwrap();
    harness.state_mut().tree.collapsed.insert(group);
    harness.state_mut().tree.collapse_generation += 1;
    harness.run();
    harness.state_mut().editor.request_rename(child);
    harness.run();
    let tree = &harness.state().tree;
    assert!(!tree.collapsed.contains(&group));
    assert_eq!(tree.renaming.as_ref().map(|(id, _)| *id), Some(child));
    assert!(harness.state_mut().editor.take_rename_request().is_none());
}

#[test]
fn attach_to_layer_below_is_in_the_menu_and_the_rows_show_the_pair() {
    // The lowest Layer has nothing below it, so it cannot attach.
    let (editor, a, _) = editor_with_two_layers();
    let mut lowest = harness(editor);
    open_menu(&mut lowest, a);
    assert!(
        lowest
            .get_by_label("Attach to Layer Below")
            .accesskit_node()
            .is_disabled()
    );

    // `a` is below `b`, so b can attach to it.
    let (editor, a, b) = editor_with_two_layers();
    let mut harness = harness(editor);
    open_menu(&mut harness, b);
    harness.get_by_label("Attach to Layer Below").click();
    harness.run();
    let editor = &harness.state().editor;
    assert!(layer_data(editor, b).attached);
    assert_eq!(editor.undo_label(), Some("Attach to Layer Below"));
    // The attached row is indented with a clip arrow, and the Layer below it is underlined;
    // these are the only signs.
    let attached = RowState::of(editor.map(), b);
    let below = RowState::of(editor.map(), a);
    assert!(attached.attached && !attached.attached_to);
    assert!(below.attached_to && !below.attached);

    // The menu now offers Release, and releasing is one Undo step too.
    open_menu(&mut harness, b);
    harness.get_by_label("Release from Layer Below").click();
    harness.run();
    let editor = &harness.state().editor;
    assert!(!layer_data(editor, b).attached);
    assert_eq!(editor.undo_label(), Some("Release from Layer Below"));
    assert!(!RowState::of(editor.map(), a).attached_to);
}
