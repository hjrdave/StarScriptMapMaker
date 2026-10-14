//! The Map's Landmass and Lake fills, as the Canvas renderer draws them.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use lyon::math::point;
use lyon::path::Path;
use ssmap_core::land;
use ssmap_core::{Id, Map, MultiPolygon, Node, ObjectKind, Polygon, RoughnessCache, ShoreStyle};

use crate::color::Color;
use crate::vector::FillPath;

/// One Landmass or Lake fill.
#[derive(Clone, PartialEq, Debug)]
pub struct ShapeFill {
    /// The roughened outline, shared with the Roughness cache.
    pub geometry: Arc<MultiPolygon>,
    pub color: Color,
    /// How the coastline is dressed.
    pub shore: ShoreStyle,
    /// The Layer holding the Object: its ripples draw beneath every fill of that Layer.
    pub layer: Option<Id>,
    /// Where this may show: the roughened outline of the Landmass a Layer around it is clipped
    /// to, minus Lakes over it. Fills of one clip share the `Arc`.
    pub clip: Option<Arc<MultiPolygon>>,
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

    /// Every visible Landmass and Lake of `map`, in [`Map::draw_order`], with its roughened
    /// outline from `roughness`.
    pub fn from_map(map: &Map, roughness: &mut RoughnessCache) -> Self {
        let order = map.draw_order();
        let mut clips = Clips::default();
        let fills = order
            .iter()
            .filter_map(|&id| match map.node(id) {
                Some(Node::Object(object)) => Some(ShapeFill {
                    color: object.kind.fill()?.into(),
                    geometry: roughness.outline(map, id)?,
                    shore: map.shore(id)?.clone(),
                    layer: map.parent(id),
                    clip: clips.of(map, roughness, &order, id),
                }),
                _ => None,
            })
            .collect();
        Self::new(fills)
    }

    pub fn fills(&self) -> &[ShapeFill] {
        &self.0
    }

    /// The paths to draw, lowest first. Each Layer's ripples come before its first fill, so
    /// they never cover land of that Layer. Each fill is followed by its inner shading, then its
    /// outline. Fills are one path per part, holding its exterior and holes (even-odd), so
    /// overlapping parts never cancel out.
    pub(crate) fn fill_paths(&self) -> Vec<FillPath> {
        // One path per distinct clip region, so fills sharing one share a stencil.
        let mut regions: Vec<(*const MultiPolygon, Arc<Path>)> = Vec::new();
        let clips: Vec<Option<Arc<Path>>> = self
            .0
            .iter()
            .map(|fill| {
                let region = fill.clip.as_ref()?;
                let key = Arc::as_ptr(region);
                Some(match regions.iter().find(|(k, _)| *k == key) {
                    Some((_, path)) => path.clone(),
                    None => {
                        let path = Arc::new(region_path(region));
                        regions.push((key, path.clone()));
                        path
                    }
                })
            })
            .collect();

        let mut paths = Vec::new();
        let mut rippled = HashSet::new();
        for (i, fill) in self.0.iter().enumerate() {
            if rippled.insert(fill.layer) {
                for (j, sibling) in self.0.iter().enumerate() {
                    if sibling.layer == fill.layer {
                        let start = paths.len();
                        ripple_paths(sibling, &mut paths);
                        for path in &mut paths[start..] {
                            path.clip = clips[j].clone();
                        }
                    }
                }
            }
            let start = paths.len();
            paths.extend(fill.geometry.0.iter().map(|polygon| FillPath {
                path: polygon_path(polygon),
                color: fill.color,
                stroke_width: None,
                clip: None,
            }));
            shading_paths(fill, &mut paths);
            if fill.shore.outline.width > 0.0 {
                paths.push(FillPath {
                    path: region_path(&fill.geometry),
                    color: fill.shore.outline.color.into(),
                    stroke_width: Some(fill.shore.outline.width as f32),
                    clip: None,
                });
            }
            for path in &mut paths[start..] {
                path.clip = clips[i].clone();
            }
        }
        paths
    }
}

/// The clip regions of a Map's fills, worked out once each.
#[derive(Default)]
struct Clips(HashMap<Vec<Id>, Option<Arc<MultiPolygon>>>);

impl Clips {
    /// Where the Object `id` may show: the roughened outlines of the Landmasses its Layers are
    /// clipped to, minus the Lakes drawn above them. `None` if no Layer around it is clipped.
    fn of(
        &mut self,
        map: &Map,
        roughness: &mut RoughnessCache,
        order: &[Id],
        id: Id,
    ) -> Option<Arc<MultiPolygon>> {
        let mut targets: Vec<Id> = std::iter::successors(map.parent(id), |&p| map.parent(p))
            .filter_map(|layer| match map.node(layer) {
                Some(Node::Layer(layer)) => layer.clip,
                _ => None,
            })
            .filter(|&target| map.contains(target))
            .collect();
        targets.sort();
        targets.dedup();
        if targets.is_empty() {
            return None;
        }
        self.0
            .entry(targets.clone())
            .or_insert_with(|| {
                let mut region: Option<MultiPolygon> = None;
                for &target in &targets {
                    let mut shown = (*roughness.outline(map, target)?).clone();
                    let rank = order.iter().position(|&o| o == target);
                    for (lake_rank, &lake) in order.iter().enumerate() {
                        let above = rank.is_none_or(|r| lake_rank > r);
                        let is_lake = matches!(
                            map.node(lake),
                            Some(Node::Object(o)) if matches!(o.kind, ObjectKind::Lake(_))
                        );
                        if above
                            && is_lake
                            && let Some(outline) = roughness.outline(map, lake)
                        {
                            shown = land::difference(&shown, &outline);
                        }
                    }
                    region = Some(match region {
                        Some(so_far) => land::intersection(&so_far, &shown),
                        None => shown,
                    });
                }
                region.map(Arc::new)
            })
            .clone()
    }
}

/// A colour with its alpha scaled by `factor`.
fn faded(color: ssmap_core::Color, factor: f64) -> Color {
    let mut color: Color = color.into();
    color.a = (f64::from(color.a) * factor.clamp(0.0, 1.0)).round() as u8;
    color
}

/// Lines stepping outward from the coast, each fainter than the last.
fn ripple_paths(fill: &ShapeFill, paths: &mut Vec<FillPath>) {
    let ripples = &fill.shore.ripples;
    if ripples.count == 0 || ripples.width <= 0.0 {
        return;
    }
    for k in 1..=ripples.count {
        let along = if ripples.count > 1 {
            f64::from(k - 1) / f64::from(ripples.count - 1)
        } else {
            0.0
        };
        let ring = land::offset(&fill.geometry, ripples.spacing * f64::from(k));
        paths.push(FillPath {
            path: region_path(&ring),
            color: faded(ripples.color, 1.0 - ripples.fade * along),
            stroke_width: Some(ripples.width as f32),
            clip: None,
        });
    }
}

/// How many bands approximate the inward glow.
const SHADING_BANDS: u32 = 6;

/// A glow inward from the coast: bands along rings inset from it, each fainter than the last.
fn shading_paths(fill: &ShapeFill, paths: &mut Vec<FillPath>) {
    let shading = &fill.shore.shading;
    if shading.width <= 0.0 || shading.color.a == 0 {
        return;
    }
    let band = shading.width / f64::from(SHADING_BANDS);
    for i in 0..SHADING_BANDS {
        let middle = (f64::from(i) + 0.5) / f64::from(SHADING_BANDS);
        let ring = land::offset(&fill.geometry, -middle * shading.width);
        if ring.is_empty() {
            break;
        }
        paths.push(FillPath {
            path: region_path(&ring),
            color: faded(shading.color, 1.0 - middle),
            stroke_width: Some(band as f32),
            clip: None,
        });
    }
}

/// Every ring of a region as closed subpaths of one path.
fn region_path(region: &MultiPolygon) -> Path {
    let mut builder = Path::builder();
    for ring in region.0.iter().flat_map(Polygon::rings) {
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
