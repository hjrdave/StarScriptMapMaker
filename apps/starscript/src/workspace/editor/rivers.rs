use ssmap_core::{River, RiverPoint, ending_at_shore};

use super::*;

/// The River tool's starting width, in Canvas units.
pub const DEFAULT_RIVER_WIDTH: f64 = 8.0;

impl Editor {
    /// The River that finishing the River tool's `path` now would make: cut where it first
    /// leaves land, with the tool's bending and `seed`. Its problem, if it has one, is why it
    /// can't be made.
    pub fn planned_river(&mut self, path: &[RiverPoint], seed: u32) -> River {
        let terrain = Terrain::of(&self.map, self.caches.roughness());
        River::new(ending_at_shore(path, &terrain))
            .with_meander(self.river_meander)
            .with_seed(seed)
    }

    /// Adds the River the tool's `path` makes ([`Self::planned_river`]) to the "Water" Layer
    /// and selects it, as one Undo step. The first River or water stroke also creates that
    /// Layer, as the water brush does. Returns whether it was added.
    pub fn finish_river(&mut self, path: &[RiverPoint], seed: u32) -> bool {
        let river = self.planned_river(path, seed);
        if let Some(problem) = river.problem() {
            self.info(format!("Can't add the River: {problem}"));
            return false;
        }
        let existing = self.water_layer();
        let id = Id::new();
        let added = self.in_gesture("Add River", |g| {
            let layer = match existing {
                Some(layer) => layer,
                None => {
                    let layer = Id::new();
                    let created = g.run(Command::CreateLayer {
                        id: layer,
                        parent: None,
                        index: None,
                        name: WATER_LAYER.to_owned(),
                    });
                    if !created {
                        return Err(());
                    }
                    layer
                }
            };
            if !g.run(Command::CreateRiver {
                id,
                parent: layer,
                index: None,
                river: Box::new(river),
            }) {
                return Err(());
            }
            g.editor.select(vec![id]);
            Ok(())
        });
        matches!(added, Some(Ok(())))
    }

    /// The selected Rivers, with their contents.
    pub fn selected_rivers(&self) -> Vec<(Id, River)> {
        self.selection
            .iter()
            .filter_map(|&id| match self.map.node(id) {
                Some(Node::Object(object)) => match &object.kind {
                    ObjectKind::River(river) => Some((id, (**river).clone())),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    /// Gives every selected River the bends of the seed `step` along from its own, as one
    /// Undo step, as Shift and the mouse wheel do. Their points stay where they are.
    pub fn step_river_seeds(&mut self, step: i32) {
        let rivers = self.selected_rivers();
        if rivers.is_empty() {
            return;
        }
        self.end_open_gesture();
        self.in_gesture("Edit River", |g| {
            for (id, river) in rivers {
                let seed = river.seed().wrapping_add_signed(step);
                if !g.run(Command::SetRiver {
                    id,
                    river: Box::new(river.with_seed(seed)),
                }) {
                    return Err(());
                }
            }
            Ok(())
        });
    }

    /// Scales the widths of the River points selected in node-edit mode by `factor`, as `[`
    /// and `]` do, kept in range. One Undo step; nothing happens unless a River's points are
    /// selected.
    pub fn scale_selected_river_widths(&mut self, factor: f64) {
        let Some(edit) = self.node_edit.clone() else {
            return;
        };
        let Some(Node::Object(object)) = self.map.node(edit.id) else {
            return;
        };
        let ObjectKind::River(river) = &object.kind else {
            return;
        };
        if edit.nodes.is_empty() {
            return;
        }
        let mut path = river.path().to_vec();
        for node in &edit.nodes {
            if let Some(point) = path.get_mut(node.index) {
                point.width = (point.width * factor)
                    .clamp(ssmap_core::MIN_RIVER_WIDTH, ssmap_core::MAX_RIVER_WIDTH);
            }
        }
        let river = Box::new((**river).clone().with_path(path));
        let _ = self.run(Command::SetRiver { id: edit.id, river });
    }
}
