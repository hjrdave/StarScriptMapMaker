//! The Map's Landmass and Lake fills, as the Canvas renderer draws them.

use std::sync::Arc;

use lyon::math::point;
use lyon::path::Path;
use ssmap_core::{Map, MultiPolygon, Node, Polygon};

use crate::color::Color;
use crate::vector::FillPath;

/// One Landmass or Lake fill.
#[derive(Clone, PartialEq, Debug)]
pub struct ShapeFill {
    pub geometry: MultiPolygon,
    pub color: Color,
}

/// Fills in draw order, first lowest. Clones share one list, and equality is identity (or
/// both empty): building a new list is what tells the renderer to tessellate again, so a
/// scene can be cloned and compared every frame for free.
#[derive(Clone, Debug, Default)]
pub struct Shapes(Arc<[ShapeFill]>);

impl PartialEq for Shapes {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || (self.0.is_empty() && other.0.is_empty())
    }
}

impl Shapes {
    pub fn new(fills: Vec<ShapeFill>) -> Self {
        Self(fills.into())
    }

    /// Every visible Landmass and Lake of `map`, in [`Map::draw_order`].
    pub fn from_map(map: &Map) -> Self {
        let fills = map
            .draw_order()
            .into_iter()
            .filter_map(|id| match map.node(id) {
                Some(Node::Object(object)) => Some(ShapeFill {
                    geometry: object.kind.geometry()?.clone(),
                    color: object.kind.fill()?.into(),
                }),
                _ => None,
            })
            .collect();
        Self::new(fills)
    }

    pub fn fills(&self) -> &[ShapeFill] {
        &self.0
    }

    /// One path per part, each holding its exterior and holes, filled even-odd so holes
    /// stay open. Parts are separate paths so overlapping parts never cancel out.
    pub(crate) fn fill_paths(&self) -> Vec<FillPath> {
        self.0
            .iter()
            .flat_map(|fill| {
                fill.geometry.0.iter().map(|polygon| FillPath {
                    path: polygon_path(polygon),
                    color: fill.color,
                })
            })
            .collect()
    }
}

fn polygon_path(polygon: &Polygon) -> Path {
    let mut builder = Path::builder();
    for ring in polygon.rings() {
        let mut points = ring.iter().map(|&[x, y]| point(x as f32, y as f32));
        let Some(first) = points.next() else {
            continue;
        };
        builder.begin(first);
        for p in points {
            builder.line_to(p);
        }
        builder.close();
    }
    builder.build()
}
