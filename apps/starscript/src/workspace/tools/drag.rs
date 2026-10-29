//! Select-tool drags on the selection: move, scale, rotate and Option-drag copy.

use ssmap_core::{Affine, Command, Id, Label, LabelStyles, Node, Stamp};

use super::{ToolId, ToolState};
use crate::editor::Editor;
use crate::handles::{self, Bounds, Grip};

/// A Select-tool drag on the selection. Each frame re-issues the whole move, scale or
/// rotation from where it started, so nothing drifts.
pub(super) struct Drag {
    grip: Option<Grip>,
    /// Where the drag began, in Canvas units.
    start: [f64; 2],
    /// The selection's bounds when it began.
    bounds: Bounds,
    /// The selected Landmasses and Lakes, which move and resize but don't rotate.
    outlines: Vec<Id>,
    /// The selected Labels as they were, so every frame transforms from the start and
    /// rounding never builds up.
    labels: Vec<(Id, Label)>,
    /// The Label styles the Labels follow, which resizing needs to scale what they inherit.
    styles: LabelStyles,
    /// The selected Stamps as they were, for the same reason.
    stamps: Vec<(Id, Stamp)>,
    /// An Option-drag: the originals that stay put, and the ids their copies took.
    pub(super) copies: Option<(Vec<Id>, Vec<Id>)>,
    /// For an arch drag: the Label's arch and text block width when it began.
    arching: Option<(f64, f64)>,
}

/// Starts moving, scaling or rotating the selection from the Canvas point `point`, picking
/// what is under the pointer first if it isn't selected. `grip` is the handle under it.
pub(super) fn start_drag(
    editor: &mut Editor,
    state: &mut ToolState,
    mut bounds: Option<Bounds>,
    grip: Option<Grip>,
    point: [f64; 2],
    duplicate: bool,
) {
    if grip.is_none() && !bounds.is_some_and(|b| b.contains(point)) {
        let Some(id) = editor.pick(point) else {
            return;
        };
        if !editor.is_selected(id) {
            editor.select(vec![id]);
        }
        bounds = Bounds::of_selection(editor);
    }
    // Whatever is linked to the selection moves or transforms with it, as one step. A locked
    // linked node blocks it all. An Option-drag copies only the selection, unlinked.
    let along = if duplicate && grip.is_none() {
        Vec::new()
    } else {
        let Some(along) = editor.linked_movers() else {
            return;
        };
        along
    };
    let moving: Vec<Id> = editor.selection().iter().copied().chain(along).collect();
    let outlines: Vec<Id> = moving
        .iter()
        .copied()
        .filter(|&id| {
            matches!(editor.map().node(id), Some(ssmap_core::Node::Object(o)) if o.kind.geometry().is_some())
        })
        .collect();
    let labels: Vec<(Id, Label)> = moving
        .iter()
        .filter_map(|&id| editor.label(id).map(|label| (id, label.clone())))
        .collect();
    let stamps: Vec<(Id, Stamp)> = moving
        .iter()
        .filter_map(|&id| editor.stamp(id).map(|stamp| (id, stamp.clone())))
        .collect();
    let Some(bounds) =
        bounds.filter(|_| !(outlines.is_empty() && labels.is_empty() && stamps.is_empty()))
    else {
        return;
    };
    // Option held as a move begins: leave the originals and drag copies of them. Locked
    // ones can't move, so they aren't copied.
    let mut copies = None;
    if duplicate && grip.is_none() {
        let sources: Vec<Id> = editor
            .selection()
            .iter()
            .copied()
            .filter(|&id| {
                (outlines.contains(&id) || editor.label(id).is_some() || editor.stamp(id).is_some())
                    && !editor.map().node(id).is_some_and(ssmap_core::Node::locked)
            })
            .collect();
        if sources.is_empty() {
            return;
        }
        if !state.begin_held(editor, ToolId::Select, "Duplicate Objects") {
            return;
        }
        let (command, new_ids) = editor.duplicate_command(&sources, [0.0, 0.0]);
        if !state.run_held(editor, command) {
            state.cancel_held(editor);
            return;
        }
        editor.select(new_ids.clone());
        copies = Some((sources, new_ids));
    } else {
        let label = match grip {
            None => "Move",
            Some(Grip::Resize(_)) => "Resize",
            Some(Grip::Rotate) => "Rotate",
            Some(Grip::Arch) => "Change Label Arch",
        };
        if !state.begin_held(editor, ToolId::Select, label) {
            return;
        }
    }
    let arching = match (grip, labels.as_slice()) {
        (Some(Grip::Arch), [(id, _)]) => editor.label_layout(*id).map(|(label, layout)| {
            let [min, max] = layout.bounds();
            (label.arch, max[0] - min[0])
        }),
        _ => None,
    };
    if grip == Some(Grip::Arch) && arching.is_none() {
        state.cancel_held(editor);
        return;
    }
    state.drag = Some(Drag {
        arching,
        grip,
        start: point,
        bounds,
        outlines,
        labels,
        styles: editor.map().label_styles().clone(),
        stamps,
        copies,
    });
}

/// The `SetMembership` Command a move drag ends each frame with: the centre of the dragged
/// selection, moved by the drag to the pointer at `point`, decides which board the dragged
/// Objects belong to (the topmost one under it), or none when it is over no board. The whole
/// selection goes to one board, so a group is never split. `None` when nothing changes
/// membership, or for a resize or rotation.
pub(super) fn membership_command(editor: &Editor, drag: &Drag, point: [f64; 2]) -> Option<Command> {
    if drag.grip.is_some() {
        return None;
    }
    let centre = drag.bounds.centre();
    let target = editor.map().membership_at([
        centre[0] + point[0] - drag.start[0],
        centre[1] + point[1] - drag.start[1],
    ]);
    let moved: Vec<Id> = match &drag.copies {
        Some((_, new_ids)) => new_ids.clone(),
        None => drag
            .outlines
            .iter()
            .copied()
            .chain(drag.labels.iter().map(|(id, _)| *id))
            .chain(drag.stamps.iter().map(|(id, _)| *id))
            .collect(),
    };
    let ids: Vec<Id> = moved
        .into_iter()
        .filter(|&id| {
            matches!(editor.map().node(id), Some(Node::Object(o)) if o.membership != target)
                && !editor.map().is_locked(id)
        })
        .collect();
    (!ids.is_empty()).then_some(Command::SetMembership {
        ids,
        membership: target,
    })
}

/// What a drag does to each Label, from how it was when the drag began.
type LabelTransform = Box<dyn Fn(&Label) -> Label>;

/// What a drag does to each Stamp.
type StampTransform = Box<dyn Fn(&Stamp) -> Stamp>;

/// The Commands that put the dragged selection where the pointer at `point` says: one for
/// the Landmasses and Lakes, then one per Label and per Stamp. `None` keeps the last frame's,
/// as when a resize reaches the opposite edge and the box has no size.
pub(super) fn drag_commands(drag: &Drag, point: [f64; 2], snap: bool) -> Option<Vec<Command>> {
    let delta = [point[0] - drag.start[0], point[1] - drag.start[1]];
    if let Some((originals, new_ids)) = &drag.copies {
        return Some(vec![Command::Duplicate {
            ids: originals.clone(),
            new_ids: new_ids.clone(),
            offset: delta,
        }]);
    }
    let ids = drag.outlines.clone();
    let (outlines, transform, stamp_transform): (Option<Command>, LabelTransform, StampTransform) =
        match drag.grip {
            None => (
                Some(Command::Translate { ids, offset: delta }),
                Box::new(move |label| handles::moved(label, delta)),
                Box::new(move |stamp| handles::moved_stamp(stamp, delta)),
            ),
            Some(Grip::Resize(handle)) => {
                let resized = drag.bounds.resized(handle, delta);
                let transform = Affine::rect_to_rect(&drag.bounds.rect(), &resized.rect());
                if !transform.is_invertible() {
                    return None;
                }
                let (scale, fixed) = drag.bounds.scaling(handle, delta);
                let styles = drag.styles.clone();
                (
                    Some(Command::Transform { ids, transform }),
                    Box::new(move |label| handles::scaled(label, &styles, scale, fixed)),
                    Box::new(move |stamp| handles::scaled_stamp(stamp, scale, fixed)),
                )
            }
            Some(Grip::Arch) => {
                let (start, width) = drag.arching?;
                (
                    None,
                    Box::new(move |label| Label {
                        arch: Some(handles::arched_by(start, delta, label.rotation, width)),
                        ..label.clone()
                    }),
                    Box::new(Stamp::clone),
                )
            }
            Some(Grip::Rotate) => {
                let centre = drag.bounds.centre();
                let angle = |p: [f64; 2]| (p[1] - centre[1]).atan2(p[0] - centre[0]).to_degrees();
                let mut degrees = angle(point) - angle(drag.start);
                if snap {
                    degrees = (degrees / 15.0).round() * 15.0;
                }
                (
                    None,
                    Box::new(move |label| handles::rotated(label, degrees, centre)),
                    Box::new(move |stamp| handles::rotated_stamp(stamp, degrees, centre)),
                )
            }
        };
    let outlines = outlines.filter(|_| !drag.outlines.is_empty());
    let labels = drag.labels.iter().map(|(id, label)| Command::SetLabel {
        id: *id,
        label: Box::new(transform(label)),
    });
    let stamps = drag.stamps.iter().map(|(id, stamp)| Command::SetStamp {
        id: *id,
        stamp: Box::new(stamp_transform(stamp)),
    });
    Some(outlines.into_iter().chain(labels).chain(stamps).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handles::Handle;

    fn drag(grip: Option<Grip>) -> Drag {
        Drag {
            grip,
            start: [100.0, 0.0],
            bounds: Bounds {
                min: [-100.0, -50.0],
                max: [100.0, 50.0],
            },
            outlines: Vec::new(),
            labels: vec![(
                Id::new(),
                Label {
                    text: "x".into(),
                    size: Some(10.0),
                    ..Label::default()
                },
            )],
            styles: LabelStyles::default(),
            stamps: Vec::new(),
            copies: None,
            arching: Some((0.0, 200.0)),
        }
    }

    #[test]
    fn an_arch_drag_bends_the_label_by_how_far_the_knob_moved_up() {
        let arched = dragged(&drag(Some(Grip::Arch)), [100.0, -50.0], false);
        assert_eq!(arched.arch, Some(0.5));
        let sagged = dragged(&drag(Some(Grip::Arch)), [100.0, 500.0], false);
        assert_eq!(sagged.arch, Some(-1.0), "never past a half circle");
    }

    #[test]
    fn an_option_drag_re_issues_one_duplicate_with_the_pointer_offset() {
        let (original, copy) = (Id::new(), Id::new());
        let option_drag = Drag {
            copies: Some((vec![original], vec![copy])),
            ..drag(None)
        };
        let commands = drag_commands(&option_drag, [130.0, 20.0], false).unwrap();
        assert!(matches!(
            &commands[..],
            [Command::Duplicate { ids, new_ids, offset: [30.0, 20.0] }]
                if ids == &vec![original] && new_ids == &vec![copy]
        ));
    }

    fn dragged(drag: &Drag, point: [f64; 2], snap: bool) -> Label {
        match drag_commands(drag, point, snap).unwrap().remove(0) {
            Command::SetLabel { label, .. } => *label,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn drags_transform_from_where_they_began() {
        let moved = dragged(&drag(None), [130.0, 20.0], false);
        assert_eq!(moved.position, [30.0, 20.0]);
        let scaled = dragged(
            &drag(Some(Grip::Resize(Handle::Right))),
            [300.0, 0.0],
            false,
        );
        assert_eq!(scaled.size, Some(20.0));
        assert_eq!(scaled.position, [100.0, 0.0], "scaled about the left edge");
        let rotated = dragged(&drag(Some(Grip::Rotate)), [0.0, 100.0], false);
        assert!((rotated.rotation - 90.0).abs() < 1e-9);
        let snapped = dragged(&drag(Some(Grip::Rotate)), [100.0, 20.0], true);
        assert!((snapped.rotation - 15.0).abs() < 1e-9);
    }

    #[test]
    fn stamps_move_scale_and_rotate_about_the_drag() {
        let id = Id::new();
        let with_stamp = |grip| Drag {
            labels: Vec::new(),
            stamps: vec![(
                id,
                Stamp {
                    position: [10.0, 20.0],
                    ..Stamp::default()
                },
            )],
            ..drag(grip)
        };
        let stamp = |commands: Vec<Command>| match commands.into_iter().next() {
            Some(Command::SetStamp { stamp, .. }) => *stamp,
            other => panic!("expected SetStamp, got {other:?}"),
        };
        let moved = stamp(drag_commands(&with_stamp(None), [130.0, 20.0], false).unwrap());
        assert_eq!(moved.position, [40.0, 40.0]);
        // Doubling the box about the middle of its left edge doubles the Stamp and its
        // distance from that point (-100, 0).
        let grown = stamp(
            drag_commands(
                &with_stamp(Some(Grip::Resize(Handle::Right))),
                [300.0, 0.0],
                false,
            )
            .unwrap(),
        );
        assert_eq!(grown.scale, 2.0);
        assert_eq!(grown.position, [-100.0 + 110.0 * 2.0, 20.0 * 2.0]);
        let turned =
            stamp(drag_commands(&with_stamp(Some(Grip::Rotate)), [0.0, 100.0], false).unwrap());
        assert_eq!(turned.rotation, 90.0);
    }

    #[test]
    fn outlines_move_and_resize_with_labels_but_do_not_rotate() {
        let outline = Id::new();
        let mixed = |grip| Drag {
            outlines: vec![outline],
            ..drag(grip)
        };
        let commands = drag_commands(&mixed(None), [130.0, 20.0], false).unwrap();
        assert!(matches!(
            &commands[..],
            [Command::Translate { ids, offset: [30.0, 20.0] }, Command::SetLabel { .. }]
                if ids == &vec![outline]
        ));
        let commands = drag_commands(
            &mixed(Some(Grip::Resize(Handle::Right))),
            [300.0, 0.0],
            false,
        )
        .unwrap();
        assert!(matches!(
            &commands[..],
            [Command::Transform { transform, .. }, Command::SetLabel { .. }]
                if transform.0 == [2.0, 0.0, 0.0, 1.0, 100.0, 0.0]
        ));
        let commands = drag_commands(&mixed(Some(Grip::Rotate)), [0.0, 100.0], false).unwrap();
        assert!(matches!(&commands[..], [Command::SetLabel { .. }]));
        // Dragging the right edge onto the left leaves no box to map onto.
        let collapsed = drag_commands(
            &mixed(Some(Grip::Resize(Handle::Right))),
            [-100.0, 0.0],
            false,
        );
        assert!(collapsed.is_none());
    }
}

#[cfg(test)]
mod link_tests {
    use super::*;
    use ssmap_core::{
        Canvas, DistanceUnit, Map, MultiPolygon, Node, Polygon, Scale, Stamp as MapStamp,
    };

    fn square(x: f64) -> MultiPolygon {
        MultiPolygon(vec![Polygon {
            exterior: vec![[x, 0.0], [x + 10.0, 0.0], [x + 10.0, 10.0], [x, 10.0]],
            holes: Vec::new(),
        }])
    }

    /// Two Landmasses and a Stamp in three Layers, linked, with the first Landmass selected.
    fn linked() -> (Editor, [Id; 3]) {
        let mut editor = Editor::new(Map::new(Canvas::new(
            500,
            500,
            Scale::new(1.0, DistanceUnit::Miles),
        )));
        let mut ids = [Id::new(), Id::new(), Id::new()];
        for (n, id) in ids.iter_mut().enumerate() {
            let layer = Id::new();
            assert!(
                editor
                    .run(Command::CreateLayer {
                        id: layer,
                        parent: None,
                        index: None,
                        name: format!("Layer {n}"),
                    })
                    .is_ok()
            );
            let command = if n == 2 {
                Command::CreateStamp {
                    id: *id,
                    parent: layer,
                    index: None,
                    stamp: Box::new(MapStamp::new("pack/asset".parse().unwrap(), [200.0, 5.0])),
                }
            } else {
                Command::CreateLandmass {
                    id: *id,
                    parent: layer,
                    index: None,
                    geometry: square(n as f64 * 100.0),
                    fill: None,
                }
            };
            assert!(editor.run(command).is_ok());
        }
        editor.select(ids.to_vec());
        assert!(editor.link_selection());
        editor.select(vec![ids[0]]);
        (editor, ids)
    }

    fn left(editor: &Editor, id: Id) -> f64 {
        match editor.map().node(id) {
            Some(Node::Object(o)) => o.kind.geometry().unwrap().0[0].exterior[0][0],
            _ => panic!("not an Object"),
        }
    }

    #[test]
    fn dragging_one_linked_object_moves_the_others_in_one_undo_step() {
        let (mut editor, [a, b, s]) = linked();
        let undo_label = editor.undo_label().map(str::to_owned);
        let mut state = ToolState::default();
        let bounds = Bounds::of_selection(&mut editor);
        start_drag(&mut editor, &mut state, bounds, None, [5.0, 5.0], false);
        let drag = state.drag.as_ref().expect("the drag started");
        assert_eq!(drag.outlines, vec![a, b]);
        assert_eq!(drag.stamps.len(), 1);
        for command in drag_commands(drag, [25.0, 5.0], false).unwrap() {
            assert!(state.run_held(&mut editor, command));
        }
        state.drag = None;
        state.end_held(&mut editor);
        assert_eq!(left(&editor, a), 20.0);
        assert_eq!(left(&editor, b), 120.0);
        assert_eq!(editor.stamp(s).unwrap().position, [220.0, 5.0]);

        assert_eq!(editor.undo_label(), Some("Move"));
        editor.undo();
        assert_eq!(left(&editor, a), 0.0);
        assert_eq!(left(&editor, b), 100.0);
        assert_eq!(editor.stamp(s).unwrap().position, [200.0, 5.0]);
        assert_eq!(editor.undo_label().map(str::to_owned), undo_label);
    }

    #[test]
    fn a_locked_linked_object_blocks_the_drag_and_the_status_names_it() {
        let (mut editor, [a, b, _]) = linked();
        assert!(
            editor
                .run(Command::RenameObject {
                    id: b,
                    name: "Far Isle".into()
                })
                .is_ok()
        );
        assert!(
            editor
                .run(Command::SetLocked {
                    id: b,
                    locked: true
                })
                .is_ok()
        );
        let before = editor.map().view();
        let mut state = ToolState::default();
        let bounds = Bounds::of_selection(&mut editor);
        start_drag(&mut editor, &mut state, bounds, None, [5.0, 5.0], false);
        assert!(state.drag.is_none() && !editor.gesture_open());
        assert_eq!(editor.map().view(), before);
        let text = &editor.notice().expect("an error toast").text;
        assert!(
            text.contains("Far Isle") && text.contains("locked"),
            "{text}"
        );
        assert_eq!(editor.selection(), [a]);
    }

    #[test]
    fn an_option_drag_copies_only_the_selection() {
        let (mut editor, [a, b, _]) = linked();
        let mut state = ToolState::default();
        let bounds = Bounds::of_selection(&mut editor);
        start_drag(&mut editor, &mut state, bounds, None, [5.0, 5.0], true);
        let drag = state.drag.as_ref().expect("the drag started");
        assert_eq!(
            drag.copies.as_ref().map(|(from, _)| from.clone()),
            Some(vec![a])
        );
        state.drag = None;
        state.end_held(&mut editor);
        assert_eq!(left(&editor, b), 100.0);
        let copy = editor.selection()[0];
        assert_ne!(copy, a);
        assert!(!editor.map().is_linked(copy));
    }
}
