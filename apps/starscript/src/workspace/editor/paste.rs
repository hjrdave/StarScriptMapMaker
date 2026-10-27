#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "existing invariant panics; replace with error handling over time (#351)"
)]
use super::*;

impl Editor {
    /// Whether Paste has anything to paste.
    pub fn has_clipboard(&self) -> bool {
        self.clipboard.has_contents()
    }

    /// Makes `clipboard` what Paste pastes: a Copy or Cut in another tab or Map.
    pub fn set_clipboard(&mut self, clipboard: Clipboard) {
        self.clipboard.set(clipboard);
    }

    /// What Paste would need that this Map lacks. `None` when nothing is copied.
    pub fn paste_gaps(&mut self) -> Option<PasteGaps> {
        self.sync_embedded();
        let clipboard = self.clipboard.contents()?;
        Some(self.map.paste_gaps(
            &clipboard.subtrees,
            |asset| {
                self.caches
                    .sprites()
                    .is_none_or(|sprites| sprites.library().asset(asset).is_some())
            },
            bundled_family,
        ))
    }

    /// The text to mirror to the system clipboard after a Copy or Cut, once.
    pub fn take_clipboard_text(&mut self) -> Option<String> {
        self.clipboard.take_text()
    }

    /// The selected branches, in stacking order, without any that sit inside another selected
    /// branch.
    pub(super) fn selected_roots(&self) -> Vec<Id> {
        let listed: HashSet<Id> = self.selection.iter().copied().collect();
        let mut roots: Vec<(Vec<usize>, Id)> = Vec::new();
        for &id in &self.selection {
            if !self.map.contains(id) || roots.iter().any(|(_, r)| *r == id) {
                continue;
            }
            let inside = std::iter::successors(self.map.parent(id), |&p| self.map.parent(p))
                .any(|ancestor| listed.contains(&ancestor));
            if !inside {
                roots.push((self.tree_path(id), id));
            }
        }
        roots.sort();
        roots.into_iter().map(|(_, id)| id).collect()
    }

    /// The sibling positions from the root down to `id`: sorting by it gives stacking order.
    pub(super) fn tree_path(&self, id: Id) -> Vec<usize> {
        let mut path = Vec::new();
        let mut at = id;
        loop {
            let parent = self.map.parent(at);
            path.push(
                self.map
                    .children(parent)
                    .iter()
                    .position(|&c| c == at)
                    .unwrap_or(0),
            );
            match parent {
                Some(parent) => at = parent,
                None => break,
            }
        }
        path.reverse();
        path
    }

    pub(super) fn subtree(&self, id: Id) -> Subtree {
        Subtree {
            id,
            node: self.map.node(id).expect("listed ids exist").clone(),
            children: self
                .map
                .children(Some(id))
                .iter()
                .map(|&child| self.subtree(child))
                .collect(),
        }
    }

    /// Copies the selected Layers and Objects, with everything inside them, locked ones too.
    /// Returns false when nothing is selected.
    pub fn copy_selection(&mut self) -> bool {
        let roots = self.selected_roots();
        if roots.is_empty() {
            return false;
        }
        let clipboard = self.clipboard_of(&roots);
        self.clipboard.copied(clipboard);
        true
    }

    /// What dragging the Layer or Object `id` out of this Map would copy: the whole Selection
    /// when `id` is part of it, else `id` alone. `None` when `id` is not in this Map.
    pub fn drag_clipboard(&mut self, id: Id) -> Option<Clipboard> {
        if !self.map.contains(id) {
            return None;
        }
        let roots = if self.selection.contains(&id) {
            self.selected_roots()
        } else {
            vec![id]
        };
        Some(self.clipboard_of(&roots))
    }

    /// The branches `roots` as a clipboard, with the middle of their Objects as its centre.
    pub(super) fn clipboard_of(&mut self, roots: &[Id]) -> Clipboard {
        let mut objects = Vec::new();
        let mut pending = roots.to_vec();
        while let Some(id) = pending.pop() {
            match self.map.node(id) {
                Some(Node::Object(_)) => objects.push(id),
                _ => pending.extend(self.map.children(Some(id))),
            }
        }
        // A Stamp whose art is not installed has no box, so it counts as its position.
        let mut points: Vec<[f64; 2]> = objects
            .iter()
            .filter(|&&id| self.stamp(id).is_some() && self.stamp_bounds(id).is_none())
            .filter_map(|&id| self.stamp(id).map(|stamp| stamp.position))
            .collect();
        if let Some(bounds) = Bounds::of_ids(self, &objects) {
            points.extend([bounds.min, bounds.max]);
        }
        let centre = Bounds::around(&points).map(|bounds| bounds.centre());
        let subtrees = roots.iter().map(|&id| self.subtree(id)).collect();
        Clipboard::new(subtrees, centre)
    }

    /// Copies the selection, then removes the originals as one Undo step. Locked Layers and
    /// Objects are copied but stay.
    pub fn cut_selection(&mut self) {
        if self.gesture.is_some() || !self.copy_selection() {
            return;
        }
        let roots = self.selected_roots();
        let removable: Vec<Id> = roots
            .iter()
            .copied()
            .filter(|&id| !self.branch_locked(id))
            .collect();
        if removable.len() < roots.len() {
            self.info("Locked layers were copied but not cut");
        }
        if !removable.is_empty() {
            let _ = self.run_selecting(Command::Delete { ids: removable }, Vec::new());
        }
    }

    /// Whether `id`, a Layer above it or anything inside it is locked.
    pub(super) fn branch_locked(&self, id: Id) -> bool {
        let mut stack = vec![id];
        self.map.is_locked(id) || {
            let mut found = false;
            while let Some(at) = stack.pop() {
                found |= self.map.node(at).is_some_and(Node::locked);
                stack.extend(self.map.children(Some(at)));
            }
            found
        }
    }

    /// Pastes what Copy or Cut took, as one Undo step, with the middle of the pasted Objects
    /// at `at` (Canvas units). It goes above the selected Layer, or on top when nothing is
    /// selected, and the pasted items become the Selection. Returns their ids.
    pub fn paste(&mut self, at: [f64; 2]) -> Option<Vec<Id>> {
        if self.gesture.is_some() {
            return None;
        }
        let clipboard = self.clipboard.contents()?.clone();
        self.paste_clipboard(clipboard, at)
    }

    /// Pastes `clipboard` as [`Editor::paste`] does, leaving this Map's own clipboard alone:
    /// what a Layer or Object dragged in from another window's Map copies.
    pub fn paste_clipboard(&mut self, mut clipboard: Clipboard, at: [f64; 2]) -> Option<Vec<Id>> {
        if self.gesture.is_some() {
            return None;
        }
        let (parent, index) = self.paste_destination(clipboard.has_objects)?;
        self.map
            .fit_for_paste(&mut clipboard.subtrees, bundled_family, FALLBACK_FAMILY);
        let mut new_ids = Vec::new();
        let mut tops = Vec::new();
        for root in &clipboard.subtrees {
            tops.push(Id::new());
            new_ids.push(tops[tops.len() - 1]);
            let mut pending: Vec<&Subtree> = root.children.iter().rev().collect();
            while let Some(tree) = pending.pop() {
                new_ids.push(Id::new());
                pending.extend(tree.children.iter().rev());
            }
        }
        let offset = clipboard
            .centre
            .map_or([0.0, 0.0], |c| [at[0] - c[0], at[1] - c[1]]);
        let command = Command::Paste {
            parent,
            index,
            subtrees: clipboard.subtrees,
            new_ids,
            offset,
        };
        self.run_selecting(command, tops.clone())
            .is_ok()
            .then_some(tops)
    }

    /// The Layer (`None` for the root) and position pasted items go in. Layers go just above
    /// the selected item; Objects need a Layer, so they go on top of the selected Layer, or
    /// of the Layer holding the selected Object, or of the active Layer.
    pub(super) fn paste_destination(
        &mut self,
        has_objects: bool,
    ) -> Option<(Option<Id>, Option<usize>)> {
        let selected = self.selected_roots().last().copied();
        if !has_objects {
            return Some(match selected {
                Some(id) => {
                    let parent = self.map.parent(id);
                    let index = self.map.children(parent).iter().position(|&c| c == id)? + 1;
                    (parent, Some(index))
                }
                None => (None, None),
            });
        }
        let layer = match selected.and_then(|id| self.map.node(id).map(|n| (id, n.is_layer()))) {
            Some((id, true)) => Some(id),
            Some((id, false)) => self.map.parent(id),
            None => self.active_layer.filter(|&id| self.map.contains(id)),
        };
        match layer {
            Some(layer) => Some((Some(layer), None)),
            None => {
                self.info("Select a layer to paste into");
                None
            }
        }
    }
}
