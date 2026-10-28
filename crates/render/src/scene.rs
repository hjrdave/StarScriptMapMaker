//! Turns a Map into what the Canvas draws, keeping derived data (Label layouts) between
//! edits so only what changed is rebuilt. Roughened outlines come from the caller's
//! [`RoughnessCache`], which picking shares.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use lyon::math::point;
use lyon::path::Path;
use ssmap_core::{
    ContentHash, Id, Landmass, Map, Membership, MultiPolygon, Node, ObjectKind, Polygon,
    ResolvedLabel, RoughnessCache, ShoreStyle, Stamp,
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
    /// What each Stamp last drew, so an edit that leaves a Stamp alone does not rebuild it.
    stamps: HashMap<Id, CachedStamp>,
    /// The Stamps of `built`, in the order of its Stamps.
    slot_ids: Vec<Id>,
    /// Where each of `slot_ids` is, made when an edit first needs it.
    slots: Option<HashMap<Id, usize>>,
    /// The Labels the build in progress has drawn, to forget the layouts of the rest.
    seen: HashSet<Id>,
    /// Counts builds, to tell the entries of `stamps` the last build used from the rest.
    build: u64,
}

/// A Stamp's draw, and what it was built from.
struct CachedStamp {
    /// The version of the Stamp when it was built.
    version: u64,
    /// The opacity of the Layers above it when it was built.
    opacity: f32,
    /// The build that last used it.
    build: u64,
    draw: Arc<StampDraw>,
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
            stamps: HashMap::new(),
            seen: HashSet::new(),
            slot_ids: Vec::new(),
            slots: None,
            build: 0,
        }
    }

    /// Forgets everything derived, as when another Map opens.
    pub fn clear(&mut self) {
        self.layouts.clear();
        self.rings.clear();
        self.stamps.clear();
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
        let previous = self
            .built
            .as_ref()
            .filter(|(_, rings, _)| *rings == generation)
            .map(|(revision, _, content)| (*revision, content.clone()));
        if let Some((since, previous)) = previous
            && let Some(patched) = self.patch_stamps(map, since, &previous)
        {
            self.built = Some((map.revision(), generation, patched.clone()));
            return patched;
        }
        let mut groups = Vec::new();
        let mut stamps = Vec::new();
        let mut slot_ids = Vec::new();
        self.seen.clear();
        let mut clips: Vec<ClipScope> = Vec::new();
        // The Landmass the last scope is clipped to, while that scope is still growing.
        let mut open: Option<Id> = None;
        let order = map.draw_order();
        self.build += 1;
        // What the Layers above an Object decide, worked out once per Layer and not per Object.
        let mut above: HashMap<Option<Id>, (Option<Id>, f32)> = HashMap::new();
        let mut placements: Vec<(Membership, Placement)> = Vec::new();
        // Each Layer's ripples draw beneath every fill of that Layer, so they never cover
        // land of that Layer.
        let mut rippled = HashSet::new();
        for &id in &order {
            let Some(Node::Object(object)) = map.node(id) else {
                continue;
            };
            let (target, opacity) = *above
                .entry(map.parent(id))
                .or_insert_with(|| (clip_target(map, id), layer_opacity(map, id)));
            let placement = match placements.iter().find(|(m, _)| *m == object.membership) {
                Some(&(_, placement)) => placement,
                None => {
                    let placement = placement_in(map, object.membership);
                    placements.push((object.membership, placement));
                    placement
                }
            };
            // Consecutive Objects clipped to the same Landmass share a scope.
            if target != open {
                open = target;
                if let Some(target) = target {
                    clips.push(clip_scope(map, roughness, target, &order, groups.len()));
                }
            }
            let clip = open.map(|_| clips.len() - 1);
            if let Some(layer) = map.parent(id)
                && !matches!(object.kind, ObjectKind::Stamp(_))
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
                        mark_object(
                            map,
                            sibling,
                            placement_of(map, sibling),
                            &mut groups[first..],
                        );
                    }
                }
            }
            let first_stamp = stamps.len();
            let first_group = groups.len();
            let place = Place { opacity, placement };
            self.draw(map, roughness, id, place, &mut groups, &mut stamps);
            mark_object(map, id, placement, &mut groups[first_group..]);
            for placed in &mut stamps[first_stamp..] {
                placed.clip = clip;
            }
            if stamps.len() > first_stamp {
                slot_ids.push(id);
            }
            if let Some(scope) = clips.last_mut().filter(|_| open.is_some()) {
                scope.groups.end = groups.len();
            }
        }
        let seen = &self.seen;
        self.layouts.retain(|id, _| seen.contains(id));
        if self.stamps.len() > stamps.len() {
            let build = self.build;
            self.stamps.retain(|_, cached| cached.build == build);
        }
        self.rings.end_build();
        self.slot_ids = slot_ids;
        self.slots = None;
        let content = Content::with_clips(groups, stamps, clips);
        self.built = Some((map.revision(), generation, content.clone()));
        content
    }

    /// `previous`, built when the Map was at revision `since`, with the Stamps that were edited
    /// since drawn anew, if that is all the edits did. `None` when anything else changed, and
    /// the content must be built again.
    fn patch_stamps(&mut self, map: &Map, since: u64, previous: &Content) -> Option<Content> {
        let changes = map.changes_since(since)?;
        if changes.other || changes.properties.is_empty() {
            return None;
        }
        if previous.stamps().len() != self.slot_ids.len() {
            return None;
        }
        let Self {
            slots,
            slot_ids,
            stamps: cache,
            build,
            ..
        } = self;
        let slots = slots.get_or_insert_with(|| {
            slot_ids
                .iter()
                .enumerate()
                .map(|(slot, &id)| (id, slot))
                .collect()
        });
        let mut placed = previous.stamps().to_vec();
        for &id in &changes.properties {
            let Some(Node::Object(object)) = map.node(id) else {
                return None;
            };
            let ObjectKind::Stamp(stamp) = &object.kind else {
                return None;
            };
            // A Layer that sorts by depth reorders its Stamps when one moves.
            let layer = map.parent(id).and_then(|layer| map.node(layer));
            if matches!(layer, Some(Node::Layer(layer)) if layer.depth_sort) {
                return None;
            }
            let slot = *slots.get(&id)?;
            let opacity = layer_opacity(map, id);
            let draw = stamp_draw(stamp, opacity);
            cache.insert(
                id,
                CachedStamp {
                    version: map.version(id)?,
                    opacity,
                    build: *build,
                    draw: draw.clone(),
                },
            );
            let slot = placed.get_mut(slot)?;
            slot.draw = draw;
            slot.placement = placement_in(map, object.membership);
        }
        Some(previous.with_stamps_replaced(placed))
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
        place: Place,
        groups: &mut Vec<VectorGroup>,
        stamps: &mut Vec<PlacedStamp>,
    ) {
        let Some(Node::Object(object)) = map.node(id) else {
            return;
        };
        let Place { opacity, placement } = place;
        match &object.kind {
            ObjectKind::Label(label) => {
                self.seen.insert(id);
                let label = label.resolve(map.label_styles());
                let layout = self.label_layout(id, &label);
                groups.push(layout.group(&label, opacity));
            }
            ObjectKind::Stamp(stamp) => {
                let version = map.version(id).unwrap_or_default();
                let build = self.build;
                let draw = match self.stamps.get_mut(&id) {
                    Some(cached) if cached.version == version && cached.opacity == opacity => {
                        cached.build = build;
                        cached.draw.clone()
                    }
                    _ => {
                        let draw = stamp_draw(stamp, opacity);
                        self.stamps.insert(
                            id,
                            CachedStamp {
                                version,
                                opacity,
                                build,
                                draw: draw.clone(),
                            },
                        );
                        draw
                    }
                };
                stamps.push(PlacedStamp {
                    after: groups.len(),
                    clip: None,
                    placement,
                    draw,
                });
            }
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

/// What the Layers above an Object and its membership decide about how it draws.
#[derive(Clone, Copy)]
struct Place {
    /// The product of the opacities of the Layers above it.
    opacity: f32,
    placement: Placement,
}

/// What the Stamp draws, faded by `opacity` as well as its own.
fn stamp_draw(stamp: &Stamp, opacity: f32) -> Arc<StampDraw> {
    Arc::new(StampDraw {
        asset: stamp.asset.clone(),
        position: stamp.position,
        scale: stamp.scale,
        rotation: stamp.rotation,
        flip: stamp.flip,
        tint: stamp.tint,
        fill: stamp.fill.unwrap_or(Landmass::DEFAULT_FILL),
        opacity: stamp.opacity * opacity,
    })
}

/// Records that `groups` draw Object `id`, at its current version.
fn mark_object(map: &Map, id: Id, placement: Placement, groups: &mut [VectorGroup]) {
    if groups.is_empty() {
        return;
    }
    let Some(version) = map.version(id) else {
        return;
    };
    for group in groups {
        group.object = Some(ObjectRef { id, version });
        group.placement = placement;
    }
}

/// Where the Object `id` is drawn: the board that clips it, or none. Art boards are counted
/// in the Map's order without the Canvas, as [`CanvasScene::for_map`](crate::CanvasScene::for_map) lists them.
fn placement_of(map: &Map, id: Id) -> Placement {
    match map.node(id) {
        Some(Node::Object(object)) => placement_in(map, object.membership),
        _ => Placement::Canvas,
    }
}

/// Where an Object with `membership` is drawn.
fn placement_in(map: &Map, membership: Membership) -> Placement {
    match membership {
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

    /// A Map with a Layer of `count` Stamps in a row; returns the Layer and the Stamps.
    fn map_with_stamps(count: usize) -> (Map, Id, Vec<Id>) {
        let mut map = Map::new(Canvas::new(800, 600, Scale::new(1.0, DistanceUnit::Miles)));
        let layer = Id::new();
        map.execute(&Command::CreateLayer {
            id: layer,
            parent: None,
            index: None,
            name: "Trees".into(),
        })
        .unwrap();
        let ids: Vec<Id> = (0..count).map(|_| Id::new()).collect();
        for (i, &id) in ids.iter().enumerate() {
            map.execute(&Command::CreateStamp {
                id,
                parent: layer,
                index: None,
                stamp: Box::new(ssmap_core::Stamp::new(
                    "base/tree".parse().unwrap(),
                    [i as f64 * 10.0, 50.0],
                )),
            })
            .unwrap();
        }
        (map, layer, ids)
    }

    fn move_stamp(map: &mut Map, id: Id, to: [f64; 2]) {
        let Some(Node::Object(object)) = map.node(id) else {
            panic!()
        };
        let ObjectKind::Stamp(stamp) = &object.kind else {
            panic!()
        };
        let mut stamp = stamp.clone();
        stamp.position = to;
        map.execute(&Command::SetStamp {
            id,
            stamp: Box::new(stamp),
        })
        .unwrap();
    }

    #[test]
    fn an_edit_rebuilds_only_the_stamps_it_touched() {
        let (mut map, _, ids) = map_with_stamps(5);
        let mut scene = MapScene::new(Fonts::bundled());
        let mut roughness = RoughnessCache::new();
        let before = scene.content(&map, &mut roughness);
        move_stamp(&mut map, ids[2], [300.0, 70.0]);
        let after = scene.content(&map, &mut roughness);
        for (i, (old, new)) in before.stamps().iter().zip(after.stamps()).enumerate() {
            assert_eq!(
                Arc::ptr_eq(&old.draw, &new.draw),
                i != 2,
                "Stamp {i}: only the moved one is rebuilt"
            );
        }
        assert_eq!(after.stamps()[2].draw.position, [300.0, 70.0]);
    }

    #[test]
    fn a_layers_opacity_rebuilds_its_stamps() {
        let (mut map, layer, _) = map_with_stamps(3);
        let mut scene = MapScene::new(Fonts::bundled());
        let mut roughness = RoughnessCache::new();
        let before = scene.content(&map, &mut roughness);
        map.execute(&Command::SetOpacity {
            id: layer,
            opacity: 0.5,
        })
        .unwrap();
        let after = scene.content(&map, &mut roughness);
        assert!(
            after
                .stamps()
                .iter()
                .all(|placed| placed.draw.opacity == 0.5)
        );
        assert!(
            before
                .stamps()
                .iter()
                .all(|placed| placed.draw.opacity == 1.0)
        );
    }

    #[test]
    fn a_cached_build_equals_a_fresh_one_through_edits_and_deletes() {
        let (mut map, _, ids) = map_with_stamps(6);
        let mut scene = MapScene::new(Fonts::bundled());
        let mut roughness = RoughnessCache::new();
        scene.content(&map, &mut roughness);
        let edits: [(usize, [f64; 2]); 3] = [(1, [5.0, 5.0]), (4, [90.0, 9.0]), (1, [7.0, 1.0])];
        for (which, to) in edits {
            move_stamp(&mut map, ids[which], to);
            let cached = scene.content(&map, &mut roughness);
            let fresh = MapScene::new(Fonts::bundled()).content(&map, &mut RoughnessCache::new());
            assert_eq!(cached.stamps().len(), fresh.stamps().len());
            for (a, b) in cached.stamps().iter().zip(fresh.stamps()) {
                assert_eq!(a, b);
            }
        }
        map.execute(&Command::Delete {
            ids: vec![ids[0], ids[3]],
        })
        .unwrap();
        let cached = scene.content(&map, &mut roughness);
        let fresh = MapScene::new(Fonts::bundled()).content(&map, &mut RoughnessCache::new());
        assert_eq!(cached.stamps().len(), 4);
        for (a, b) in cached.stamps().iter().zip(fresh.stamps()) {
            assert_eq!(a, b);
        }
        assert_eq!(
            scene.stamps.len(),
            4,
            "the cache forgets the Stamps that are gone"
        );
    }

    fn assert_same_stamps(a: &Content, b: &Content) {
        assert_eq!(a.stamps().len(), b.stamps().len());
        for (x, y) in a.stamps().iter().zip(b.stamps()) {
            assert_eq!(x, y);
        }
    }

    #[test]
    fn editing_only_stamps_patches_their_slots_and_shares_the_rest() {
        let (mut map, _, ids) = map_with_stamps(200);
        let mut scene = MapScene::new(Fonts::bundled());
        let mut roughness = RoughnessCache::new();
        let before = scene.content(&map, &mut roughness);
        for step in 0..3 {
            for &id in &ids[10..20] {
                move_stamp(&mut map, id, [f64::from(step), 99.0]);
            }
            let patched = scene.content(&map, &mut roughness);
            assert!(
                std::ptr::eq(patched.groups(), before.groups()),
                "the groups are shared, not rebuilt"
            );
            let fresh = MapScene::new(Fonts::bundled()).content(&map, &mut RoughnessCache::new());
            assert_same_stamps(&patched, &fresh);
        }
    }

    #[test]
    fn a_patched_build_follows_undo() {
        let (mut map, _, ids) = map_with_stamps(5);
        let mut scene = MapScene::new(Fonts::bundled());
        let mut roughness = RoughnessCache::new();
        let before = scene.content(&map, &mut roughness);
        let inverse = map
            .execute(&Command::SetStamp {
                id: ids[3],
                stamp: Box::new(ssmap_core::Stamp::new(
                    "base/tree".parse().unwrap(),
                    [1.0, 1.0],
                )),
            })
            .unwrap();
        let moved = scene.content(&map, &mut roughness);
        assert_eq!(moved.stamps()[3].draw.position, [1.0, 1.0]);
        map.revert(inverse);
        let undone = scene.content(&map, &mut roughness);
        assert_same_stamps(&undone, &before);
    }

    #[test]
    fn other_edits_are_not_patched() {
        let (mut map, layer, ids) = map_with_stamps(4);
        let mut scene = MapScene::new(Fonts::bundled());
        let mut roughness = RoughnessCache::new();
        let before = scene.content(&map, &mut roughness);
        // A Layer property, a deleted Stamp, and a Stamp edit together with an added one.
        map.execute(&Command::SetOpacity {
            id: layer,
            opacity: 0.25,
        })
        .unwrap();
        let faded = scene.content(&map, &mut roughness);
        assert!(faded.stamps().iter().all(|s| s.draw.opacity == 0.25));
        map.execute(&Command::Delete { ids: vec![ids[1]] }).unwrap();
        let fewer = scene.content(&map, &mut roughness);
        assert_eq!(fewer.stamps().len(), before.stamps().len() - 1);
        move_stamp(&mut map, ids[0], [3.0, 3.0]);
        map.execute(&Command::CreateStamp {
            id: Id::new(),
            parent: layer,
            index: None,
            stamp: Box::new(ssmap_core::Stamp::new(
                "base/tree".parse().unwrap(),
                [8.0, 8.0],
            )),
        })
        .unwrap();
        let both = scene.content(&map, &mut roughness);
        let fresh = MapScene::new(Fonts::bundled()).content(&map, &mut RoughnessCache::new());
        assert_same_stamps(&both, &fresh);
        assert_eq!(both.stamps().len(), 4);
    }

    #[test]
    fn a_stamp_in_a_hidden_layer_is_not_patched_in() {
        let (mut map, layer, ids) = map_with_stamps(3);
        let mut scene = MapScene::new(Fonts::bundled());
        let mut roughness = RoughnessCache::new();
        scene.content(&map, &mut roughness);
        map.execute(&Command::SetVisible {
            id: layer,
            visible: false,
        })
        .unwrap();
        assert!(scene.content(&map, &mut roughness).stamps().is_empty());
        move_stamp(&mut map, ids[0], [5.0, 5.0]);
        assert!(
            scene.content(&map, &mut roughness).stamps().is_empty(),
            "still hidden"
        );
    }

    #[test]
    fn moving_a_stamp_in_a_depth_sorted_layer_rebuilds_the_order() {
        let (mut map, layer, ids) = map_with_stamps(4);
        map.execute(&Command::SetDepthSort {
            id: layer,
            depth_sort: true,
        })
        .unwrap();
        let mut scene = MapScene::new(Fonts::bundled());
        let mut roughness = RoughnessCache::new();
        scene.content(&map, &mut roughness);
        // The first Stamp moves below the rest, so it must now draw last.
        move_stamp(&mut map, ids[0], [0.0, 500.0]);
        let patched = scene.content(&map, &mut roughness);
        let fresh = MapScene::new(Fonts::bundled()).content(&map, &mut RoughnessCache::new());
        assert_same_stamps(&patched, &fresh);
        assert_eq!(patched.stamps()[3].draw.position, [0.0, 500.0]);
    }
}
