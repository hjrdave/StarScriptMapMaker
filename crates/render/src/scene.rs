//! Turns a Map into what the Canvas draws, keeping derived data (Label layouts) between
//! edits so only what changed is rebuilt. Roughened outlines come from the caller's
//! [`RoughnessCache`], which picking shares.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use lyon::math::point;
use lyon::path::Path;
use ssmap_core::land;
use ssmap_core::{
    Id, Label, Map, MultiPolygon, Node, ObjectKind, Polygon, RoughnessCache, ShoreStyle,
};

use crate::canvas::Content;
use crate::color::Color;
use crate::color::ColorExt as _;
use crate::text::{Fonts, LabelLayout, LayoutKey};
use crate::vector::{FillRule, Paint, VectorGroup, VectorShape};

/// The Map's drawn Objects and the derived data behind them. The editor keeps one per open
/// Map; export builds its own.
pub struct MapScene {
    fonts: Fonts,
    layouts: HashMap<Id, (LayoutKey, Arc<LabelLayout>)>,
    /// The content last built, and the Map revision it was built from.
    built: Option<(u64, Content)>,
}

impl MapScene {
    pub fn new(fonts: Fonts) -> Self {
        Self {
            fonts,
            layouts: HashMap::new(),
            built: None,
        }
    }

    /// Forgets everything derived, as when another Map opens.
    pub fn clear(&mut self) {
        self.layouts.clear();
        self.built = None;
    }

    /// What the Canvas draws for `map`, rebuilt only when the Map has changed since.
    /// Landmasses and Lakes use their roughened outlines from `roughness`.
    pub fn content(&mut self, map: &Map, roughness: &mut RoughnessCache) -> Content {
        if let Some((revision, content)) = &self.built
            && *revision == map.revision()
        {
            return content.clone();
        }
        let mut groups = Vec::new();
        let mut seen = Vec::new();
        let order = map.draw_order();
        // Each Layer's ripples draw beneath every fill of that Layer, so they never cover
        // land of that Layer.
        let mut rippled = HashSet::new();
        for &id in &order {
            if let Some(layer) = map.parent(id)
                && map.shore(id).is_some()
                && rippled.insert(layer)
            {
                for &sibling in order
                    .iter()
                    .filter(|&&other| map.parent(other) == Some(layer))
                {
                    if let (Some(shore), Some(outline)) =
                        (map.shore(sibling), roughness.outline(map, sibling))
                    {
                        ripple_groups(&outline, shore, layer_opacity(map, sibling), &mut groups);
                    }
                }
            }
            self.draw(map, roughness, id, &mut groups, &mut seen);
        }
        self.layouts.retain(|id, _| seen.contains(id));
        let content = Content::new(groups);
        self.built = Some((map.revision(), content.clone()));
        content
    }

    /// The shaped text of a Label, shaping it only if its text or typography changed.
    pub fn label_layout(&mut self, id: Id, label: &Label) -> Arc<LabelLayout> {
        let key = LayoutKey::of(label);
        if let Some((cached, layout)) = self.layouts.get(&id)
            && *cached == key
        {
            return layout.clone();
        }
        let layout = Arc::new(LabelLayout::shape(&mut self.fonts, label));
        self.layouts.insert(id, (key, layout.clone()));
        layout
    }

    /// Adds what the Object `id` draws, faded by the opacity of the Layers holding it.
    fn draw(
        &mut self,
        map: &Map,
        roughness: &mut RoughnessCache,
        id: Id,
        groups: &mut Vec<VectorGroup>,
        seen: &mut Vec<Id>,
    ) {
        let Some(Node::Object(object)) = map.node(id) else {
            return;
        };
        let opacity = layer_opacity(map, id);
        match &object.kind {
            ObjectKind::Label(label) => {
                seen.push(id);
                let layout = self.label_layout(id, label);
                groups.push(layout.group(label, opacity));
            }
            kind => {
                let (Some(fill), Some(outline)) = (kind.fill(), roughness.outline(map, id)) else {
                    return;
                };
                // One path per part, each holding its exterior and holes, filled even-odd so
                // holes stay open. Parts are separate groups so overlapping parts never
                // cancel out.
                let color = fill.faded(opacity);
                groups.extend(outline.0.iter().map(|polygon| {
                    VectorGroup::fill(polygon_path(polygon), color, FillRule::EvenOdd)
                }));
                if let Some(shore) = map.shore(id) {
                    shading_groups(&outline, shore, opacity, groups);
                    if shore.outline.width > 0.0 {
                        groups.push(stroke(
                            region_path(&outline),
                            shore.outline.color,
                            shore.outline.width as f32,
                            opacity,
                        ));
                    }
                }
            }
        }
    }
}

/// The product of the opacities of every Layer above `id`.
fn layer_opacity(map: &Map, id: Id) -> f32 {
    std::iter::successors(map.parent(id), |&layer| map.parent(layer))
        .filter_map(|layer| match map.node(layer) {
            Some(Node::Layer(layer)) => Some(layer.opacity),
            _ => None,
        })
        .product()
}

fn stroke(path: Path, color: Color, width: f32, opacity: f32) -> VectorGroup {
    VectorGroup {
        shapes: vec![VectorShape {
            path,
            color: color.faded(opacity),
            paint: Paint::Stroke { width },
        }],
    }
}

/// A colour with its alpha scaled by `factor`.
fn faded(mut color: Color, factor: f64) -> Color {
    color.a = (f64::from(color.a) * factor.clamp(0.0, 1.0)).round() as u8;
    color
}

/// Lines stepping outward from the coast, each fainter than the last.
fn ripple_groups(
    outline: &MultiPolygon,
    shore: &ShoreStyle,
    opacity: f32,
    groups: &mut Vec<VectorGroup>,
) {
    let ripples = &shore.ripples;
    if ripples.count == 0 || ripples.width <= 0.0 {
        return;
    }
    for k in 1..=ripples.count {
        let along = if ripples.count > 1 {
            f64::from(k - 1) / f64::from(ripples.count - 1)
        } else {
            0.0
        };
        let ring = land::offset(outline, ripples.spacing * f64::from(k));
        groups.push(stroke(
            region_path(&ring),
            faded(ripples.color, 1.0 - ripples.fade * along),
            ripples.width as f32,
            opacity,
        ));
    }
}

/// How many bands approximate the inward glow.
const SHADING_BANDS: u32 = 6;

/// A glow inward from the coast: bands along rings inset from it, each fainter than the last.
fn shading_groups(
    outline: &MultiPolygon,
    shore: &ShoreStyle,
    opacity: f32,
    groups: &mut Vec<VectorGroup>,
) {
    let shading = &shore.shading;
    if shading.width <= 0.0 || shading.color.a == 0 {
        return;
    }
    let band = shading.width / f64::from(SHADING_BANDS);
    for i in 0..SHADING_BANDS {
        let middle = (f64::from(i) + 0.5) / f64::from(SHADING_BANDS);
        let ring = land::offset(outline, -middle * shading.width);
        if ring.is_empty() {
            break;
        }
        groups.push(stroke(
            region_path(&ring),
            faded(shading.color, 1.0 - middle),
            band as f32,
            opacity,
        ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use ssmap_core::{Canvas, Command, DistanceUnit, Scale};

    fn map_with_label() -> (Map, Id, Id) {
        let mut map = Map::new(Canvas::new(800, 600, Scale::new(1.0, DistanceUnit::Miles)));
        let layer = Id::new();
        let label = Id::new();
        map.execute(&Command::CreateLayer {
            id: layer,
            parent: None,
            index: None,
            name: "Labels".into(),
        })
        .unwrap();
        map.execute(&Command::CreateLabel {
            id: label,
            parent: layer,
            index: None,
            label: Box::new(Label {
                text: "Thule".into(),
                ..Label::default()
            }),
        })
        .unwrap();
        (map, layer, label)
    }

    #[test]
    fn content_is_rebuilt_only_after_an_edit() {
        let (mut map, layer, _) = map_with_label();
        let mut scene = MapScene::new(Fonts::bundled());
        let first = scene.content(&map, &mut RoughnessCache::new());
        assert_eq!(first.groups().len(), 1);
        assert_eq!(
            scene.content(&map, &mut RoughnessCache::new()),
            first,
            "same build"
        );
        map.execute(&Command::SetOpacity {
            id: layer,
            opacity: 0.5,
        })
        .unwrap();
        let faded = scene.content(&map, &mut RoughnessCache::new());
        assert_ne!(faded, first);
        assert_eq!(faded.groups()[0].shapes[0].color.a, 128);
    }

    #[test]
    fn moving_a_label_reuses_its_layout() {
        let (mut map, _, id) = map_with_label();
        let mut scene = MapScene::new(Fonts::bundled());
        let Some(Node::Object(object)) = map.node(id) else {
            panic!()
        };
        let ObjectKind::Label(label) = &object.kind else {
            panic!()
        };
        let mut label = (**label).clone();
        let before = scene.label_layout(id, &label);
        label.position = [400.0, 300.0];
        label.rotation = 30.0;
        map.execute(&Command::SetLabel {
            id,
            label: Box::new(label.clone()),
        })
        .unwrap();
        assert!(Arc::ptr_eq(&before, &scene.label_layout(id, &label)));
        label.size *= 2.0;
        assert!(!Arc::ptr_eq(&before, &scene.label_layout(id, &label)));
    }

    #[test]
    fn hidden_layers_draw_nothing() {
        let (mut map, layer, _) = map_with_label();
        map.execute(&Command::SetVisible {
            id: layer,
            visible: false,
        })
        .unwrap();
        let mut scene = MapScene::new(Fonts::bundled());
        assert!(
            scene
                .content(&map, &mut RoughnessCache::new())
                .groups()
                .is_empty()
        );
    }
}
