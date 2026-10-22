//! Placing Stamps in space and finding them by position. A Stamp's art is a rectangle, sized
//! by its Asset, that scales, flips and rotates about the Asset's Anchor (ADR 0003). An rstar
//! R-tree of those rectangles' bounds is the broad phase; the caller's exact test, a lookup in
//! the Asset's Silhouette, is the narrow phase. Core knows nothing of art files, so the
//! renderer supplies sizes and the Silhouette test.

use rstar::primitives::{GeomWithData, Rectangle};
use rstar::{AABB, RTree};

use crate::asset::AssetRef;
use crate::geometry::{Point, Rect};
use crate::id::Id;
use crate::map::Map;
use crate::node::{Node, ObjectKind, Stamp};

/// The size of an Asset's art and where its Anchor sits.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SpriteShape {
    /// Width and height in Canvas units at a Stamp scale of 1.
    pub size: [f64; 2],
    /// The Anchor as fractions of the art's width and height from its top-left.
    pub anchor: [f64; 2],
}

impl Stamp {
    /// Where `point` falls on the art, as fractions of its width and height from the top-left,
    /// or `None` outside the art's rectangle.
    pub(crate) fn art_coords(&self, shape: &SpriteShape, point: Point) -> Option<[f64; 2]> {
        let [dx, dy] = [point[0] - self.position[0], point[1] - self.position[1]];
        let (sin, cos) = self.rotation.to_radians().sin_cos();
        // Undo the rotation (clockwise on screen, y down), then the flip, then the scale.
        let local = [
            (dx * cos + dy * sin) / self.scale,
            (-dx * sin + dy * cos) / self.scale,
        ];
        let x = if self.flip { -local[0] } else { local[0] };
        let u = x / shape.size[0] + shape.anchor[0];
        let v = local[1] / shape.size[1] + shape.anchor[1];
        ((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)).then_some([u, v])
    }

    /// The box around the placed art.
    pub fn bounds(&self, shape: &SpriteShape) -> Rect {
        let (sin, cos) = self.rotation.to_radians().sin_cos();
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for corner in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
            let x = (corner[0] - shape.anchor[0]) * shape.size[0] * self.scale;
            let y = (corner[1] - shape.anchor[1]) * shape.size[1] * self.scale;
            let x = if self.flip { -x } else { x };
            let placed = [
                self.position[0] + x * cos - y * sin,
                self.position[1] + x * sin + y * cos,
            ];
            for axis in 0..2 {
                min[axis] = min[axis].min(placed[axis]);
                max[axis] = max[axis].max(placed[axis]);
            }
        }
        Rect { min, max }
    }
}

#[derive(Clone, PartialEq, Debug)]
struct Entry {
    id: Id,
    /// Position in the draw order; higher draws above.
    rank: usize,
    locked: bool,
    stamp: Stamp,
    shape: SpriteShape,
}

type Indexed = GeomWithData<Rectangle<[f64; 2]>, Entry>;

/// A spatial index of the Map's visible Stamps. It is a snapshot: rebuild it when
/// [`StampIndex::is_current`] says the Map has changed.
pub struct StampIndex {
    revision: u64,
    tree: RTree<Indexed>,
}

impl StampIndex {
    /// Indexes `map`. `shape` gives each Asset's size; Stamps of Assets it doesn't know
    /// can't be picked.
    pub fn build(map: &Map, shape: impl Fn(&AssetRef) -> Option<SpriteShape>) -> Self {
        let entries = map
            .draw_order()
            .into_iter()
            .enumerate()
            .filter_map(|(rank, id)| {
                let Some(Node::Object(object)) = map.node(id) else {
                    return None;
                };
                let ObjectKind::Stamp(stamp) = &object.kind else {
                    return None;
                };
                let shape = shape(&stamp.asset)?;
                let bounds = stamp.bounds(&shape);
                let entry = Entry {
                    id,
                    rank,
                    locked: map.is_locked(id),
                    stamp: stamp.clone(),
                    shape,
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

    /// The topmost unlocked Stamp for which `inside` accepts the point's art coordinates (see
    /// [`Stamp::art_coords`]); `inside` is the Silhouette test.
    pub fn pick(&self, point: Point, inside: impl Fn(&AssetRef, [f64; 2]) -> bool) -> Option<Id> {
        self.tree
            .locate_all_at_point(&point)
            .filter(|candidate| {
                let entry = &candidate.data;
                !entry.locked
                    && entry
                        .stamp
                        .art_coords(&entry.shape, point)
                        .is_some_and(|uv| inside(&entry.stamp.asset, uv))
            })
            .max_by_key(|candidate| candidate.data.rank)
            .map(|candidate| candidate.data.id)
    }

    /// Stamps whose bounds meet `rect`, locked or not, first lowest.
    pub fn intersecting(&self, rect: &Rect) -> Vec<Id> {
        let mut found: Vec<&Entry> = self
            .tree
            .locate_in_envelope_intersecting(&AABB::from_corners(rect.min, rect.max))
            .map(|candidate| &candidate.data)
            .collect();
        found.sort_by_key(|entry| entry.rank);
        found.into_iter().map(|entry| entry.id).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;
    use crate::map::{Canvas, DistanceUnit, Scale};

    const SHAPE: SpriteShape = SpriteShape {
        size: [100.0, 50.0],
        anchor: [0.5, 1.0],
    };

    fn stamp(position: Point) -> Stamp {
        Stamp {
            position,
            ..Stamp::default()
        }
    }

    #[test]
    fn the_anchor_is_the_art_pivot() {
        let s = stamp([200.0, 200.0]);
        assert_eq!(s.art_coords(&SHAPE, [200.0, 200.0]), Some([0.5, 1.0]));
        assert_eq!(s.art_coords(&SHAPE, [150.0, 150.0]), Some([0.0, 0.0]));
        assert_eq!(s.art_coords(&SHAPE, [149.0, 150.0]), None);
        let b = s.bounds(&SHAPE);
        assert_eq!((b.min, b.max), ([150.0, 150.0], [250.0, 200.0]));
    }

    #[test]
    fn scale_flip_and_rotation_pivot_on_the_anchor() {
        let mut s = stamp([0.0, 0.0]);
        s.scale = 2.0;
        assert_eq!(s.art_coords(&SHAPE, [-100.0, -100.0]), Some([0.0, 0.0]));
        s.scale = 1.0;
        s.flip = true;
        // The art's left edge now sits to the right of the Anchor.
        assert_eq!(s.art_coords(&SHAPE, [50.0, -50.0]), Some([0.0, 0.0]));
        s.flip = false;
        s.rotation = 90.0;
        // Quarter turn clockwise: the art's top, above the Anchor, points right.
        let top = s.art_coords(&SHAPE, [50.0, 0.0]).unwrap();
        assert!(
            (top[0] - 0.5).abs() < 1e-9 && (top[1] - 0.0).abs() < 1e-9,
            "{top:?}"
        );
        let b = s.bounds(&SHAPE);
        assert!((b.max[0] - 50.0).abs() < 1e-9 && (b.min[0]).abs() < 1e-9);
    }

    #[test]
    fn the_topmost_stamp_under_the_point_wins() {
        let mut map = Map::new(Canvas::new(800, 600, Scale::new(1.0, DistanceUnit::Miles)));
        let layer = Id::new();
        map.execute(&Command::CreateLayer {
            id: layer,
            parent: None,
            index: None,
            name: "Assets".into(),
        })
        .unwrap();
        let ids: Vec<Id> = (0..2).map(|_| Id::new()).collect();
        for (id, x) in ids.iter().zip([200.0, 220.0]) {
            map.execute(&Command::CreateStamp {
                id: *id,
                parent: layer,
                index: None,
                stamp: Box::new(stamp([x, 200.0])),
            })
            .unwrap();
        }
        let index = StampIndex::build(&map, |_| Some(SHAPE));
        assert!(index.is_current(&map));
        assert_eq!(index.pick([210.0, 180.0], |_, _| true), Some(ids[1]));
        // Inside the box but outside the Silhouette finds nothing.
        assert_eq!(index.pick([160.0, 180.0], |_, _| false), None);
        assert_eq!(index.pick([160.0, 180.0], |_, _| true), Some(ids[0]));
        assert_eq!(index.pick([700.0, 500.0], |_, _| true), None);
        assert_eq!(
            index.intersecting(&Rect {
                min: [140.0, 140.0],
                max: [160.0, 160.0]
            }),
            vec![ids[0]]
        );
        map.execute(&Command::SetLocked {
            id: ids[1],
            locked: true,
        })
        .unwrap();
        let index = StampIndex::build(&map, |_| Some(SHAPE));
        assert_eq!(index.pick([210.0, 180.0], |_, _| true), Some(ids[0]));
    }
}
