//! Finding Objects by position: an rstar R-tree of outline boxes for the broad phase, then an
//! exact even-odd point-in-polygon test.

use rstar::primitives::{GeomWithData, Rectangle};
use rstar::{AABB, RTree};

use crate::geometry::{Point, Rect};
use crate::id::Id;
use crate::map::Map;
use crate::node::Node;

#[derive(Clone, Copy, PartialEq, Debug)]
struct Entry {
    id: Id,
    /// Position in the draw order; higher draws above.
    rank: usize,
    locked: bool,
}

type Indexed = GeomWithData<Rectangle<[f64; 2]>, Entry>;

/// A spatial index of the Map's visible Landmasses and Lakes. It is a snapshot: rebuild it
/// when [`PickIndex::is_current`] says the Map has changed.
pub struct PickIndex {
    revision: u64,
    tree: RTree<Indexed>,
}

impl PickIndex {
    pub fn build(map: &Map) -> Self {
        let entries = map
            .draw_order()
            .into_iter()
            .enumerate()
            .filter_map(|(rank, id)| {
                let Some(Node::Object(object)) = map.node(id) else {
                    return None;
                };
                let bounds = object.kind.geometry()?.bounds()?;
                let entry = Entry {
                    id,
                    rank,
                    locked: map.is_locked(id),
                };
                Some(GeomWithData::new(
                    Rectangle::from_corners(bounds.min, bounds.max),
                    entry,
                ))
            })
            .collect();
        Self {
            revision: map.revision(),
            tree: RTree::bulk_load(entries),
        }
    }

    /// Built from `map` as it is now.
    pub fn is_current(&self, map: &Map) -> bool {
        self.revision == map.revision()
    }

    /// The topmost unlocked Landmass or Lake whose fill covers `point`. Holes don't count, so
    /// a click in a hole finds whatever lies beneath.
    pub fn pick(&self, map: &Map, point: Point) -> Option<Id> {
        self.tree
            .locate_all_at_point(&point)
            .filter(|candidate| !candidate.data.locked)
            .filter(|candidate| {
                matches!(
                    map.node(candidate.data.id),
                    Some(Node::Object(object))
                        if object.kind.geometry().is_some_and(|g| g.contains(point))
                )
            })
            .max_by_key(|candidate| candidate.data.rank)
            .map(|candidate| candidate.data.id)
    }

    /// Landmasses and Lakes whose bounds meet `rect`, locked or not, first lowest.
    pub fn intersecting(&self, rect: &Rect) -> Vec<Id> {
        let mut found: Vec<Entry> = self
            .tree
            .locate_in_envelope_intersecting(&AABB::from_corners(rect.min, rect.max))
            .map(|candidate| candidate.data)
            .collect();
        found.sort_by_key(|entry| entry.rank);
        found.into_iter().map(|entry| entry.id).collect()
    }
}
