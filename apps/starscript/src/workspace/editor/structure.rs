#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "existing invariant panics; replace with error handling over time (#351)"
)]
use super::*;

impl Editor {
    /// Creates a Layer above the active one (or on top), selects it and returns its id.
    pub fn new_layer(&mut self) -> Option<Id> {
        self.new_container("Layer")
    }

    /// Creates an empty Group (a Layer named "Group N") where `new_layer` would put a Layer.
    pub fn new_group(&mut self) -> Option<Id> {
        self.new_container("Group")
    }

    pub(super) fn new_container(&mut self, kind: &str) -> Option<Id> {
        let (parent, index) = match self.active_layer.filter(|&id| self.map.contains(id)) {
            Some(active) => {
                let parent = self.map.parent(active);
                let position = self
                    .map
                    .children(parent)
                    .iter()
                    .position(|&id| id == active)
                    .expect("a node is listed by its parent");
                (parent, Some(position + 1))
            }
            None => (None, None),
        };
        let id = Id::new();
        let name = format!("{kind} {}", self.layer_count() + 1);
        let command = Command::CreateLayer {
            id,
            parent,
            index,
            name,
        };
        self.run_selecting(command, vec![id]).is_ok().then_some(id)
    }

    /// Wraps the selection in a new Group and selects it. One Undo step; refused with a
    /// toast when the selection doesn't share a parent.
    pub fn group_selection(&mut self) -> Option<Id> {
        let ids = self.selected_roots();
        if ids.is_empty() {
            return None;
        }
        let id = Id::new();
        let name = format!("Group {}", self.layer_count() + 1);
        self.run_selecting(Command::CreateGroup { id, ids, name }, vec![id])
            .is_ok()
            .then_some(id)
    }

    /// Dissolves the selected Layers and Groups, selecting what was inside. One Undo step.
    pub fn ungroup_selection(&mut self) -> bool {
        let ids: Vec<Id> = self
            .selected_roots()
            .into_iter()
            .filter(|&id| self.map.node(id).is_some_and(Node::is_layer))
            .collect();
        if ids.is_empty() {
            return false;
        }
        let freed: Vec<Id> = ids
            .iter()
            .flat_map(|&id| self.map.children(Some(id)).to_vec())
            .collect();
        self.run_selecting(Command::Ungroup { ids }, freed).is_ok()
    }

    /// The Objects that move or transform with the selection because of Links. When a linked
    /// Layer or Object is locked nothing may move, so this says so in the status bar and
    /// returns `None`.
    pub fn linked_movers(&mut self) -> Option<Vec<Id>> {
        match self.map.linked_movers(&self.selection) {
            Ok(movers) => Some(movers),
            Err(err) => {
                self.refused(&err.into());
                None
            }
        }
    }

    /// Whether the selection holds two or more Layers or Objects to link.
    pub fn can_link(&self) -> bool {
        self.selection
            .iter()
            .filter(|&&id| self.map.contains(id))
            .count()
            > 1
    }

    /// Links the selected Layers and Objects so they move and transform together, wherever
    /// they sit in the tree. One Undo step.
    pub fn link_selection(&mut self) -> bool {
        if !self.can_link() {
            return false;
        }
        let ids = self.selection.clone();
        self.run(Command::Link { id: Id::new(), ids }).is_ok()
    }

    /// Whether any selected Layer or Object is linked.
    pub fn can_unlink(&self) -> bool {
        self.selection.iter().any(|&id| self.map.is_linked(id))
    }

    /// Takes the selected Layers and Objects out of their Link sets. One Undo step.
    pub fn unlink_selection(&mut self) -> bool {
        let ids: Vec<Id> = self
            .selection
            .iter()
            .copied()
            .filter(|&id| self.map.is_linked(id))
            .collect();
        !ids.is_empty() && self.run(Command::Unlink { ids }).is_ok()
    }

    pub fn delete_selection(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        let ids = self.selection.clone();
        let count = ids.len();
        if self
            .run_selecting(Command::Delete { ids }, Vec::new())
            .is_ok()
        {
            self.info_with_undo(match count {
                1 => "Deleted 1 item".to_string(),
                n => format!("Deleted {n} items"),
            });
        }
    }

    /// A Duplicate of `ids` with fresh ids, and the ids of the copies at the top of each
    /// duplicated item. Ids inside another listed id come with it.
    pub fn duplicate_command(&self, ids: &[Id], offset: [f64; 2]) -> (Command, Vec<Id>) {
        let listed: HashSet<Id> = ids.iter().copied().collect();
        let mut seen = HashSet::new();
        let mut new_ids = Vec::new();
        let mut tops = Vec::new();
        for &id in ids {
            let mut inside = false;
            let mut above = self.map.parent(id);
            while let Some(ancestor) = above {
                inside |= listed.contains(&ancestor);
                above = self.map.parent(ancestor);
            }
            if inside || !seen.insert(id) {
                continue;
            }
            tops.push(Id::new());
            new_ids.push(tops[tops.len() - 1]);
            let mut pending = self.map.children(Some(id)).to_vec();
            while let Some(child) = pending.pop() {
                new_ids.push(Id::new());
                pending.extend(self.map.children(Some(child)));
            }
        }
        let command = Command::Duplicate {
            ids: ids.to_vec(),
            new_ids,
            offset,
        };
        (command, tops)
    }

    /// Duplicates the selection in place, just above the originals, and selects the copies.
    /// One Undo step. Returns the copies' ids.
    pub fn duplicate_selection(&mut self) -> Option<Vec<Id>> {
        if self.selection.is_empty() || self.gesture.is_some() {
            return None;
        }
        let ids = self.selection.clone();
        let only_layers = ids
            .iter()
            .all(|&id| self.map.node(id).is_some_and(Node::is_layer));
        let (command, copies) = self.duplicate_command(&ids, [0.0, 0.0]);
        let label = if only_layers {
            "Duplicate Layer"
        } else {
            "Duplicate Objects"
        };
        let done = self.in_gesture(label, |g| {
            if !g.run(command) {
                return Err(());
            }
            g.editor.select(copies.clone());
            Ok(())
        });
        done.is_some_and(|result| result.is_ok()).then_some(copies)
    }

    /// Select All: selects every Object on the active Layer.
    pub fn select_all(&mut self) {
        let objects = self.active_layer_objects();
        if !objects.is_empty() {
            self.select(objects);
        }
    }

    /// Deselect: empties the Selection; the active Layer stays.
    pub fn deselect(&mut self) {
        self.select(Vec::new());
    }

    /// Deletes the active Layer with what it holds. One Undo step.
    pub fn delete_active_layer(&mut self) {
        if let Some(layer) = self.active_layer.filter(|&id| self.map.contains(id)) {
            self.select(vec![layer]);
            self.delete_selection();
        }
    }
}
