use std::borrow::Cow;

use ssmap_core::Polygon;

use super::*;

impl Editor {
    pub fn node_edit(&self) -> Option<&NodeEdit> {
        self.node_edit.as_ref()
    }

    /// The nodes of the Object in node-edit mode: the stored outline (not the roughened one)
    /// of a Landmass or Lake, or a River's path as one ring, which
    /// [`Self::edited_is_path`] says is open.
    pub fn edited_outline(&self) -> Option<Cow<'_, MultiPolygon>> {
        match self.map.node(self.node_edit.as_ref()?.id)? {
            Node::Object(object) => match &object.kind {
                ObjectKind::River(river) => Some(Cow::Owned(MultiPolygon(vec![Polygon {
                    exterior: river.path().iter().map(|p| p.position).collect(),
                    holes: Vec::new(),
                }]))),
                kind => kind.geometry().map(Cow::Borrowed),
            },
            Node::Layer(_) => None,
        }
    }

    /// Whether the nodes being edited are a River's path: open, from source to mouth, so no
    /// edge runs from the last point back to the first.
    pub fn edited_is_path(&self) -> bool {
        self.node_edit.as_ref().is_some_and(|edit| {
            matches!(self.map.node(edit.id), Some(Node::Object(o)) if matches!(o.kind, ObjectKind::River(_)))
        })
    }

    /// The area of the Object in node-edit mode: its outline, which for a River is the one
    /// generated from its path. A click inside it keeps node-edit mode.
    pub fn edited_area(&self) -> Option<&MultiPolygon> {
        match self.map.node(self.node_edit.as_ref()?.id)? {
            Node::Object(object) => object.kind.geometry(),
            Node::Layer(_) => None,
        }
    }

    /// Enters node-edit mode on a Landmass or Lake, selecting it.
    pub fn enter_node_edit(&mut self, id: Id) {
        self.node_edit = None;
        self.select(vec![id]);
        self.node_edit = Some(NodeEdit {
            id,
            nodes: Vec::new(),
        });
    }

    pub fn leave_node_edit(&mut self) {
        self.node_edit = None;
    }

    /// Selects nodes of the Object being edited.
    pub fn select_nodes(&mut self, nodes: Vec<NodeRef>) {
        if let Some(edit) = &mut self.node_edit {
            edit.nodes = nodes;
        }
    }

    /// The shape of the outline being edited: how many points each ring has. Node numbers
    /// stay meaningful while it does not change.
    pub fn edited_shape(&self) -> Vec<usize> {
        self.edited_outline()
            .map(|outline| {
                outline
                    .0
                    .iter()
                    .flat_map(|polygon| polygon.rings().map(<[Point]>::len))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Forgets the node selection unless the outline still has `shape`: a repair may have
    /// renumbered its nodes.
    pub fn keep_nodes_if_shape(&mut self, shape: &[usize]) {
        if self.edited_shape() != shape {
            self.select_nodes(Vec::new());
        }
    }

    /// Adds a node on an edge and selects it. One Undo step.
    pub fn insert_node(&mut self, edge: NodeRef, point: Point) {
        let Some(id) = self.node_edit.as_ref().map(|edit| edit.id) else {
            return;
        };
        if self.run(Command::InsertNode { id, edge, point }).is_ok() {
            let added = NodeRef {
                index: edge.index + 1,
                ..edge
            };
            let landed = self
                .edited_outline()
                .is_some_and(|outline| outline.node(added) == Some(point));
            self.select_nodes(if landed { vec![added] } else { Vec::new() });
        }
    }

    /// Deletes the selected nodes. One Undo step; refused if a ring would be left with fewer
    /// than three points.
    pub fn delete_selected_nodes(&mut self) {
        let Some(edit) = self.node_edit.clone() else {
            return;
        };
        if edit.nodes.is_empty() {
            return;
        }
        let count = edit.nodes.len();
        let command = Command::DeleteNodes {
            id: edit.id,
            nodes: edit.nodes,
        };
        if self.run(command).is_ok() {
            self.select_nodes(Vec::new());
            self.info_with_undo(match count {
                1 => "Deleted 1 point".to_string(),
                n => format!("Deleted {n} points"),
            });
        }
    }

    /// The selected Landmasses, topmost first.
    pub(super) fn selected_landmasses(&self) -> Vec<Id> {
        let order = self.map.draw_order();
        let mut ids: Vec<Id> = self
            .selection
            .iter()
            .copied()
            .filter(|&id| {
                matches!(self.map.node(id), Some(Node::Object(o)) if matches!(o.kind, ObjectKind::Landmass(_)))
            })
            .collect();
        ids.sort_by_key(|&id| std::cmp::Reverse(order.iter().position(|&o| o == id)));
        ids
    }

    /// Whether Union and Subtract have enough to work on: two or more selected Landmasses.
    pub fn can_combine(&self) -> bool {
        self.selected_landmasses().len() >= 2
    }

    /// Whether a selected Landmass has more than one part.
    pub fn can_split(&self) -> bool {
        self.selected_landmasses()
            .iter()
            .any(|&id| self.part_count(id) > 1)
    }

    pub(super) fn part_count(&self, id: Id) -> usize {
        match self.map.node(id) {
            Some(Node::Object(o)) => o.kind.geometry().map_or(0, |g| g.0.len()),
            _ => 0,
        }
    }

    /// Merges the selected Landmasses into the topmost one, which stays selected.
    pub fn union_selection(&mut self) {
        let ids = self.selected_landmasses();
        if let Some(&keeper) = ids.first() {
            let _ = self.run_selecting(Command::Union { ids }, vec![keeper]);
        }
    }

    /// Cuts the topmost selected Landmass out of the others.
    pub fn subtract_selection(&mut self) {
        let ids = self.selected_landmasses();
        let rest: Vec<Id> = ids.iter().skip(1).copied().collect();
        if !ids.is_empty() {
            let _ = self.run_selecting(Command::Subtract { ids }, rest);
        }
    }

    /// Makes each part of every selected Landmass a Landmass of its own, as one Undo step.
    pub fn split_selection(&mut self) {
        let targets: Vec<Id> = self
            .selected_landmasses()
            .into_iter()
            .filter(|&id| self.part_count(id) > 1)
            .collect();
        if targets.is_empty() {
            return;
        }
        let mut selection = Vec::new();
        let mut commands = Vec::new();
        for id in targets {
            let new_ids: Vec<Id> = (1..self.part_count(id)).map(|_| Id::new()).collect();
            selection.push(id);
            selection.extend(&new_ids);
            commands.push(Command::SplitApart { id, new_ids });
        }
        self.end_open_gesture();
        self.in_gesture("Split Apart", |g| {
            for command in commands {
                if !g.run(command) {
                    return Err(());
                }
            }
            g.editor.select(selection);
            Ok(())
        });
    }
}
