//! Turns a Map into what the Canvas draws, keeping derived data (Label layouts) between
//! edits so only what changed is rebuilt. Roughened outlines come from the caller's
//! [`RoughnessCache`], which picking shares.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use lyon::math::point;
use lyon::path::Path;
use ssmap_core::{
    ContentHash, Id, Landmass, Map, Membership, MultiPolygon, Node, ObjectKind, Polygon,
    ResolvedLabel, RoughnessCache, ShoreStyle,
};

use crate::canvas::{ClipScope, Content};
use crate::color::Color;
use crate::color::ColorExt as _;
use crate::profile::{self, Stage};
use crate::shore_rings::{RingStore, SHADING_BANDS, ripple_distances, shading_distances};
use crate::stamp::{PlacedStamp, StampDraw};
use crate::text::{Fonts, LabelLayout, LayoutKey};
use crate::vector::{FillRule, ObjectRef, Paint, Placement, VectorGroup, VectorShape};

/// The roughened outline of `id`, timed as the Roughness stage (a cache hit is near free).
fn outline(roughness: &mut RoughnessCache, map: &Map, id: Id) -> Option<Arc<MultiPolygon>> {
    let _stage = profile::stage(Stage::Roughness);
    roughness.outline(map, id)
}

/// The Map's drawn Objects and the derived data behind them. The editor keeps one per open
/// Map; export builds its own.
pub struct MapScene {
    fonts: Fonts,
    /// The embedded font files `fonts` was built with in place of the bundled ones.
    font_overrides: Vec<(String, ContentHash)>,
    layouts: HashMap<Id, (LayoutKey, Arc<LabelLayout>)>,
    /// The rings each Landmass and Lake draws its ripples and shading along, so an edit offsets
    /// only the outlines it changed.
    rings: RingStore,
    /// Set while a build must not draw rings still being computed in the background.
    complete: bool,
    /// The content last built, and the Map revision and ring generation it was built from.
    built: Option<(u64, u64, Content)>,
}

impl MapScene {
    /// A scene shaping Labels with `fonts`, which should be [`Fonts::bundled`]. A Map that
    /// embeds other font files gets those instead.
    pub fn new(fonts: Fonts) -> Self {
        Self {
            fonts,
            font_overrides: Vec::new(),
            layouts: HashMap::new(),
            rings: RingStore::default(),
            complete: false,
            built: None,
        }
    }

    /// Forgets everything derived, as when another Map opens.
    pub fn clear(&mut self) {
        self.layouts.clear();
        self.rings.clear();
        self.built = None;
    }

    /// Computes the rings ripples and shading follow on a thread of their own from now on, so a
    /// slider over a rough coastline never waits for them. Until an Object's new rings land it
    /// draws the ones it had; `repaint` is called from that thread as each lands. Export asks
    /// for [`MapScene::complete_content`] instead, which waits for none of this.
    pub fn compute_rings_in_background(&mut self, repaint: impl Fn() + Send + Sync + 'static) {
        self.rings.compute_in_background(repaint);
        self.built = None;
    }

    /// Like [`MapScene::content`], but always rebuilt, from roughened outlines only and with
    /// every ring computed for the outline it is drawn around, as export needs.
    pub fn complete_content(&mut self, map: &Map, roughness: &mut RoughnessCache) -> Content {
        self.built = None;
        self.complete = true;
        let content = self.content(map, roughness);
        self.complete = false;
        content
    }

    /// Drops the built content so the next call rebuilds it, keeping shaped text.
    pub fn invalidate(&mut self) {
        self.built = None;
    }

    /// Switches to the fonts the Map embeds, which win over the bundled ones, if they
    /// differ from the ones in use. Labels are shaped again when they do.
    fn use_embedded_fonts(&mut self, map: &Map) {
        let overrides = Fonts::embedded_overrides(map.embedded());
        if overrides == self.font_overrides {
            return;
        }
        self.fonts = if overrides.is_empty() {
            Fonts::bundled()
        } else {
            Fonts::with_embedded(map.embedded())
        };
        self.font_overrides = overrides;
        self.layouts.clear();
        self.built = None;
    }

    /// What the Canvas draws for `map`, rebuilt only when the Map has changed since.
    /// Landmasses and Lakes use their roughened outlines from `roughness`.
    pub fn content(&mut self, map: &Map, roughness: &mut RoughnessCache) -> Content {
        let _stage = profile::stage(Stage::Update);
        if self
            .built
            .as_ref()
            .is_none_or(|(r, _, _)| *r != map.revision())
        {
            self.use_embedded_fonts(map);
        }
        let generation = self.rings.generation();
        if let Some((revision, rings, content)) = &self.built
            && *revision == map.revision()
            && *rings == generation
        {
            return content.clone();
        }
        let mut groups = Vec::new();
        let mut stamps = Vec::new();
        let mut seen = Vec::new();
        let mut clips: Vec<ClipScope> = Vec::new();
        // The Landmass the last scope is clipped to, while that scope is still growing.
        let mut open: Option<Id> = None;
        let order = map.draw_order();
        // Each Layer's ripples draw beneath every fill of that Layer, so they never cover
        // land of that Layer.
        let mut rippled = HashSet::new();
        for &id in &order {
            // Consecutive Objects clipped to the same Landmass share a scope.
            let target = clip_target(map, id);
            if target != open {
                open = target;
                if let Some(target) = target {
                    clips.push(clip_scope(map, roughness, target, &order, groups.len()));
                }
            }
            let clip = open.map(|_| clips.len() - 1);
            if let Some(layer) = map.parent(id)
                && map.shore(id).is_some()
                && rippled.insert(layer)
            {
                for &sibling in order
                    .iter()
                    .filter(|&&other| map.parent(other) == Some(layer))
                {
                    if let (Some(shore), Some(outline)) =
                        (map.shore(sibling), outline(roughness, map, sibling))
                    {
                        let first = groups.len();
                        self.rings.prepare(sibling, &outline, shore, self.complete);
                        ripple_groups(
                            &mut |distance| self.rings.ring(sibling, distance),
                            shore,
                            layer_opacity(map, sibling),
                            &mut groups,
                        );
                        mark_object(map, sibling, &mut groups[first..]);
                    }
                }
            }
            let first_stamp = stamps.len();
            let first_group = groups.len();
            self.draw(map, roughness, id, &mut groups, &mut stamps, &mut seen);
            mark_object(map, id, &mut groups[first_group..]);
            for placed in &mut stamps[first_stamp..] {
                placed.clip = clip;
                placed.placement = placement_of(map, id);
            }
            if let Some(scope) = clips.last_mut().filter(|_| open.is_some()) {
                scope.groups.end = groups.len();
            }
        }
        self.layouts.retain(|id, _| seen.contains(id));
        self.rings.end_build();
        let content = Content::with_clips(groups, stamps, clips);
        self.built = Some((map.revision(), generation, content.clone()));
        content
    }

    /// The shaped text of a Label, shaping it only if its text or typography changed.
    pub fn label_layout(&mut self, id: Id, label: &ResolvedLabel) -> Arc<LabelLayout> {
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
        stamps: &mut Vec<PlacedStamp>,
        seen: &mut Vec<Id>,
    ) {
        let Some(Node::Object(object)) = map.node(id) else {
            return;
        };
        let opacity = layer_opacity(map, id);
        match &object.kind {
            ObjectKind::Label(label) => {
                seen.push(id);
                let label = label.resolve(map.label_styles());
                let layout = self.label_layout(id, &label);
                groups.push(layout.group(&label, opacity));
            }
            ObjectKind::Stamp(stamp) => stamps.push(PlacedStamp {
                after: groups.len(),
                clip: None,
                placement: Placement::Canvas,
                draw: StampDraw {
                    asset: stamp.asset.clone(),
                    position: stamp.position,
                    scale: stamp.scale,
                    rotation: stamp.rotation,
                    flip: stamp.flip,
                    tint: stamp.tint,
                    fill: stamp.fill.unwrap_or(Landmass::DEFAULT_FILL),
                    opacity: stamp.opacity * opacity,
                },
            }),
            kind => {
                let (Some(fill), Some(outline)) = (kind.fill(), outline(roughness, map, id)) else {
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
                    self.rings.prepare(id, &outline, shore, self.complete);
                    shading_groups(
                        &mut |distance| self.rings.ring(id, distance),
                        shore,
                        opacity,
                        groups,
                    );
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

/// Records that `groups` draw Object `id`, at its current version.
fn mark_object(map: &Map, id: Id, groups: &mut [VectorGroup]) {
    let Some(version) = map.version(id) else {
        return;
    };
    let placement = placement_of(map, id);
    for group in groups {
        group.object = Some(ObjectRef { id, version });
        group.placement = placement;
    }
}

/// Where the Object `id` is drawn: the board that clips it, or none. Art boards are counted
/// in the Map's order without the Canvas, as [`CanvasScene::for_map`](crate::CanvasScene::for_map) lists them.
fn placement_of(map: &Map, id: Id) -> Placement {
    let Some(Node::Object(object)) = map.node(id) else {
        return Placement::Canvas;
    };
    match object.membership {
        Membership::Canvas => Placement::Canvas,
        Membership::OffCanvas => Placement::OffCanvas,
        Membership::Board(board) => map
            .boards()
            .iter()
            .filter(|other| other.id != map.primary_board())
            .position(|other| other.id == board)
            .map_or(Placement::Canvas, Placement::Board),
    }
}

/// The Landmass `id` is clipped to: that of the nearest Layer above it that has a Clip. A
/// Layer's own Clip replaces any Clip of the Layers around it.
fn clip_target(map: &Map, id: Id) -> Option<Id> {
    std::iter::successors(map.parent(id), |&layer| map.parent(layer)).find_map(|layer| {
        match map.node(layer) {
            Some(Node::Layer(layer)) => layer.clip,
            _ => None,
        }
    })
}

/// A scope starting at group `start`, clipped to the Landmass `target`'s roughened outline
/// minus every Lake that draws after it.
fn clip_scope(
    map: &Map,
    roughness: &mut RoughnessCache,
    target: Id,
    order: &[Id],
    start: usize,
) -> ClipScope {
    let fills = |outline: &MultiPolygon| -> Vec<VectorGroup> {
        outline
            .0
            .iter()
            .map(|polygon| {
                VectorGroup::fill(
                    polygon_path(polygon),
                    Color::rgb(0, 0, 0),
                    FillRule::EvenOdd,
                )
            })
            .collect()
    };
    let land = outline(roughness, map, target)
        .map(|outline| fills(&outline))
        .unwrap_or_default();
    let after = order
        .iter()
        .position(|&id| id == target)
        .map_or(0, |at| at + 1);
    let mut holes = Vec::new();
    for &id in &order[after..] {
        if let Some(Node::Object(object)) = map.node(id)
            && matches!(object.kind, ObjectKind::Lake(_))
            && let Some(outline) = outline(roughness, map, id)
        {
            holes.extend(fills(&outline));
        }
    }
    ClipScope {
        groups: start..start,
        land,
        holes,
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
        effect: false,
        object: None,
        placement: Placement::Canvas,
    }
}

/// A colour with its alpha scaled by `factor`.
fn faded(mut color: Color, factor: f64) -> Color {
    color.a = (f64::from(color.a) * factor.clamp(0.0, 1.0)).round() as u8;
    color
}

/// Lines stepping outward from the coast, each fainter than the last.
fn ripple_groups(
    ring: &mut impl FnMut(f64) -> Option<Arc<MultiPolygon>>,
    shore: &ShoreStyle,
    opacity: f32,
    groups: &mut Vec<VectorGroup>,
) {
    let ripples = &shore.ripples;
    let distances = ripple_distances(shore);
    let last = distances.len().saturating_sub(1).max(1);
    for (k, distance) in distances.into_iter().enumerate() {
        let along = k as f64 / last as f64;
        // A ring still being computed with nothing to stand in for it is left out for now.
        let Some(ring) = ring(distance) else {
            continue;
        };
        groups.push(
            stroke(
                region_path(&ring),
                faded(ripples.color, 1.0 - ripples.fade * along),
                ripples.width as f32,
                opacity,
            )
            .with_effect(),
        );
    }
}

/// A glow inward from the coast: bands along rings inset from it, each fainter than the last.
fn shading_groups(
    ring: &mut impl FnMut(f64) -> Option<Arc<MultiPolygon>>,
    shore: &ShoreStyle,
    opacity: f32,
    groups: &mut Vec<VectorGroup>,
) {
    let shading = &shore.shading;
    let band = shading.width / f64::from(SHADING_BANDS);
    for distance in shading_distances(shore) {
        let Some(ring) = ring(distance) else {
            continue;
        };
        if ring.is_empty() {
            break;
        }
        let middle = -distance / shading.width;
        groups.push(
            stroke(
                region_path(&ring),
                faded(shading.color, 1.0 - middle),
                band as f32,
                opacity,
            )
            .with_effect(),
        );
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
    use ssmap_core::{Canvas, Command, DistanceUnit, Label, Scale};

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
        let before = scene.label_layout(id, &label.resolve(map.label_styles()));
        label.position = [400.0, 300.0];
        label.rotation = 30.0;
        map.execute(&Command::SetLabel {
            id,
            label: Box::new(label.clone()),
        })
        .unwrap();
        let moved = label.resolve(map.label_styles());
        assert!(Arc::ptr_eq(&before, &scene.label_layout(id, &moved)));
        label.size = Some(96.0);
        let bigger = label.resolve(map.label_styles());
        assert!(!Arc::ptr_eq(&before, &scene.label_layout(id, &bigger)));
    }

    #[test]
    fn content_is_rebuilt_when_background_rings_land() {
        let mut map = Map::new(Canvas::new(800, 600, Scale::new(1.0, DistanceUnit::Miles)));
        let layer = Id::new();
        map.execute(&Command::CreateLayer {
            id: layer,
            parent: None,
            index: None,
            name: "Land".into(),
        })
        .unwrap();
        map.execute(&Command::SetDefaultRoughness {
            roughness: ssmap_core::Roughness::new(8.0, 16.0, 3),
        })
        .unwrap();
        map.execute(&Command::CreateLandmass {
            id: Id::new(),
            parent: layer,
            index: None,
            geometry: MultiPolygon(vec![Polygon {
                exterior: vec![
                    [100.0, 100.0],
                    [500.0, 100.0],
                    [500.0, 400.0],
                    [100.0, 400.0],
                ],
                holes: Vec::new(),
            }]),
            fill: None,
        })
        .unwrap();
        let (sender, landed) = std::sync::mpsc::channel();
        let sender = std::sync::Mutex::new(sender);
        let mut scene = MapScene::new(Fonts::bundled());
        scene.compute_rings_in_background(move || {
            let _ = sender.lock().unwrap().send(());
        });
        let mut roughness = RoughnessCache::new();
        let effects = |content: &Content| content.groups().iter().filter(|g| g.effect).count();
        let before = scene.content(&map, &mut roughness);
        assert_eq!(effects(&before), 0, "no rings yet, and none to stand in");
        landed
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("the rings land");
        let after = scene.content(&map, &mut roughness);
        assert!(
            effects(&after) > 0,
            "the same Map revision is rebuilt with its rings"
        );
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
