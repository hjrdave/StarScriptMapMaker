//! Turns a Map into what the Canvas draws, keeping derived data (Label layouts) between
//! edits so only what changed is rebuilt. Roughened outlines come from the caller's
//! [`RoughnessCache`], which picking shares.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
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
    /// The content last built, and what it was built from.
    built: Option<Built>,
    /// What each Stamp last drew, so an edit that leaves a Stamp alone does not rebuild it.
    stamps: HashMap<Id, CachedStamp>,
    /// The Stamps of `built`, in the order of its Stamps.
    slot_ids: Vec<Id>,
    /// Where each of `slot_ids` is, made when an edit first needs it.
    slots: Option<HashMap<Id, usize>>,
    /// Where each Object that draws vector groups has them among the groups of `built`.
    objects: HashMap<Id, ObjectGroups>,
    /// Where the Layers whose Objects are direct children end.
    ends: HashMap<Id, LayerEnd>,
    /// The Labels the build in progress has drawn, to forget the layouts of the rest.
    seen: HashSet<Id>,
    /// Counts builds, to tell the entries of `stamps` the last build used from the rest.
    build: u64,
}

/// The content last built, and the state of everything it was built from.
struct Built {
    /// The Map revision.
    revision: u64,
    /// The generation of the shore rings, which changes as they land in the background.
    rings: u64,
    /// The generation of the roughened outlines, which changes as they land in the background.
    outlines: u64,
    content: Content,
}

/// Where a stretch of groups is among the content's, and where it comes in draw order.
#[derive(Clone, Debug, Default)]
struct Slot {
    /// The groups, which may be none.
    range: Range<usize>,
    /// Counts the stretches in the order the build emitted them, empty ones included.
    ord: usize,
    /// How many Stamps the build had placed before it. A Stamp is drawn after the stretch
    /// when its number is at least this.
    stamps: usize,
}

/// Where an Object's vector groups are among the content's, so an edit can replace just them.
#[derive(Clone, Debug, Default)]
struct ObjectGroups {
    /// The groups of its fill, shading and outline, or of its text.
    own: Slot,
    /// The groups of its ripples, which draw beneath every fill of its Layer. Only Objects
    /// with a Shore have them.
    ripples: Option<Slot>,
    /// Whether it has a Shore, whose rings can land in the background.
    shore: bool,
    /// The roughened outline it was drawn from, which is another once a better one lands.
    outline: Option<Arc<MultiPolygon>>,
    /// The serial of its rings when it was drawn.
    serial: u64,
}

/// Where a Layer's output ends before its Lakes, which is where Stamps added to the top of the
/// Layer go.
#[derive(Clone, Copy, Debug)]
struct LayerEnd {
    /// How many Stamps the build had placed.
    stamps: usize,
    /// How many groups the build had emitted.
    groups: usize,
    /// How many stretches the build had emitted, as [`Slot::ord`] counts them.
    ord: usize,
}

/// A stretch of the content's groups, and the groups that now go there.
struct Replacement {
    id: Id,
    ripples: bool,
    slot: Slot,
    new: Vec<VectorGroup>,
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
            objects: HashMap::new(),
            ends: HashMap::new(),
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

    /// The content built last, if it is still current, or brought up to date by redrawing only
    /// what changed. `None` when it must be built again.
    fn reuse(&mut self, map: &Map, roughness: &mut RoughnessCache) -> Option<Content> {
        if self
            .built
            .as_ref()
            .is_none_or(|built| built.revision != map.revision())
        {
            self.use_embedded_fonts(map);
        }
        let rings = self.rings.generation();
        let outlines = roughness.generation();
        if let Some(built) = &self.built
            && built.revision == map.revision()
            && built.rings == rings
            && built.outlines == outlines
        {
            return Some(built.content.clone());
        }
        let previous = self.built.as_ref().map(|built| {
            let landed = built.rings != rings || built.outlines != outlines;
            (built.revision, landed, built.content.clone())
        });
        if let Some((since, landed, previous)) = previous
            && let Some(patched) = self.patch(map, roughness, since, landed, &previous)
        {
            self.built = Some(Built {
                revision: map.revision(),
                rings,
                outlines,
                content: patched.clone(),
            });
            return Some(patched);
        }
        None
    }

    /// What the Canvas draws for `map`, rebuilt only when the Map has changed since.
    /// Landmasses and Lakes use their roughened outlines from `roughness`.
    pub fn content(&mut self, map: &Map, roughness: &mut RoughnessCache) -> Content {
        let _stage = profile::stage(Stage::Update);
        if let Some(content) = self.reuse(map, roughness) {
            return content;
        }
        let (rings, outlines) = (self.rings.generation(), roughness.generation());
        let mut groups = Vec::new();
        let mut stamps = Vec::new();
        let mut slot_ids = Vec::new();
        let mut objects: HashMap<Id, ObjectGroups> = HashMap::new();
        let mut emitted = 0;
        let mut ends: HashMap<Id, LayerEnd> = HashMap::new();
        let mut latest: Option<(Id, LayerEnd)> = None;
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
                    let (first, placed) = (groups.len(), stamps.len());
                    self.ripples(map, roughness, sibling, &mut groups);
                    if map.shore(sibling).is_some() {
                        objects.entry(sibling).or_default().ripples = Some(Slot {
                            range: first..groups.len(),
                            ord: emitted,
                            stamps: placed,
                        });
                        emitted += 1;
                    }
                }
            }
            let first_stamp = stamps.len();
            let first_group = groups.len();
            let place = Place { opacity, placement };
            self.draw(map, roughness, id, place, &mut groups, &mut stamps);
            mark_object(map, id, placement, &mut groups[first_group..]);
            if !matches!(object.kind, ObjectKind::Stamp(_)) {
                let slot = objects.entry(id).or_default();
                slot.own = Slot {
                    range: first_group..groups.len(),
                    ord: emitted,
                    stamps: first_stamp,
                };
                emitted += 1;
                slot.shore = map.shore(id).is_some();
                slot.serial = self.rings.serial(id);
                slot.outline = slot.shore.then(|| outline(roughness, map, id)).flatten();
            }
            for placed in &mut stamps[first_stamp..] {
                placed.clip = clip;
            }
            if stamps.len() > first_stamp {
                slot_ids.push(id);
            }
            if let Some(scope) = clips.last_mut().filter(|_| open.is_some()) {
                scope.groups.end = groups.len();
            }
            if let Some(layer) = map.parent(id)
                && !object.kind.is_water()
            {
                let end = LayerEnd {
                    stamps: stamps.len(),
                    groups: groups.len(),
                    ord: emitted,
                };
                // Objects of a Layer come in runs, so only a change of Layer needs recording.
                match &mut latest {
                    Some((last, at)) if *last == layer => *at = end,
                    other => {
                        if let Some((last, at)) = other.replace((layer, end)) {
                            ends.insert(last, at);
                        }
                    }
                }
            }
        }
        if let Some((last, at)) = latest {
            ends.insert(last, at);
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
        self.objects = objects;
        self.ends = ends;
        let content = Content::with_clips(groups, stamps, clips);
        self.built = Some(Built {
            revision: map.revision(),
            rings,
            outlines,
            content: content.clone(),
        });
        content
    }

    /// `previous`, built when the Map was at revision `since`, brought up to date by redrawing
    /// only the Objects that were edited in place since (or whose rings or outlines have landed
    /// since, if `landed`), if that is all that changed. `None` when anything else changed, and the
    /// content must be built again.
    fn patch(
        &mut self,
        map: &Map,
        roughness: &mut RoughnessCache,
        since: u64,
        landed: bool,
        previous: &Content,
    ) -> Option<Content> {
        let changes = map.changes_since(since)?;
        // Clip scopes hold copies of outlines and refer to groups by index.
        if changes.other || !previous.clips().is_empty() {
            return None;
        }
        let mut stamp_ids = Vec::new();
        let mut dirty = Vec::new();
        for &id in &changes.edited {
            match map.node(id) {
                Some(Node::Object(object)) if matches!(object.kind, ObjectKind::Stamp(_)) => {
                    stamp_ids.push(id);
                }
                Some(Node::Object(_)) if self.objects.contains_key(&id) => dirty.push(id),
                _ => return None,
            }
        }
        if changes.defaults {
            // Roughness, Shore and Label styles reach every Object that draws groups, and
            // no Stamp.
            let have: HashSet<Id> = dirty.iter().copied().collect();
            dirty.extend(self.objects.keys().filter(|id| !have.contains(id)).copied());
        }
        if landed {
            self.pick_up_landed(map, roughness, &mut dirty)?;
        }
        let stamps = if stamp_ids.is_empty() {
            None
        } else {
            Some(self.patch_stamps(map, &stamp_ids, previous)?)
        };
        let replacements = self.replacements(map, roughness, &dirty)?;
        let content = if stamps.is_none() && replacements.is_empty() {
            previous.clone()
        } else {
            self.splice(previous, stamps, replacements)?
        };
        if changes.inserted.is_empty() {
            return Some(content);
        }
        self.insert_stamps(map, &changes.inserted, &content)
    }

    /// `content` with the Stamps `ids`, which were added to the top of their Layers, drawn
    /// after what those Layers drew. `None` if any is somewhere else, or not a Stamp.
    fn insert_stamps(&mut self, map: &Map, ids: &[Id], content: &Content) -> Option<Content> {
        if content.stamps().len() != self.slot_ids.len() {
            return None;
        }
        let mut by_layer: HashMap<Id, Vec<Id>> = HashMap::new();
        for &id in ids {
            let Some(Node::Object(object)) = map.node(id) else {
                return None;
            };
            if !matches!(object.kind, ObjectKind::Stamp(_)) {
                return None;
            }
            by_layer.entry(map.parent(id)?).or_default().push(id);
        }
        // Where each Layer's new Stamps go, in draw order.
        let mut places: Vec<(LayerEnd, Id, Vec<Id>)> = Vec::new();
        for (layer, added) in by_layer {
            let Some(Node::Layer(held)) = map.node(layer) else {
                return None;
            };
            if held.depth_sort {
                return None;
            }
            let end = *self.ends.get(&layer)?;
            places.push((end, layer, top_of_layer(map, layer, &added)?));
        }
        places.sort_by_key(|(end, _, _)| (end.ord, end.stamps));
        let total: usize = places.iter().map(|(_, _, added)| added.len()).sum();
        let mut stamps = Vec::with_capacity(content.stamps().len() + total);
        let mut slot_ids = Vec::with_capacity(stamps.capacity());
        let mut cursor = 0;
        for (end, _, added) in &places {
            stamps.extend_from_slice(&content.stamps()[cursor..end.stamps]);
            slot_ids.extend_from_slice(&self.slot_ids[cursor..end.stamps]);
            cursor = end.stamps;
            for &id in added {
                stamps.push(self.new_stamp(map, id, end.groups)?);
                slot_ids.push(id);
            }
        }
        stamps.extend_from_slice(&content.stamps()[cursor..]);
        slot_ids.extend_from_slice(&self.slot_ids[cursor..]);
        // What is drawn after an insertion point moves on by the Stamps put there.
        let put_before = |ord: usize, stamps: usize| -> usize {
            places
                .iter()
                .filter(|(end, _, _)| (end.ord, end.stamps) <= (ord, stamps))
                .map(|(_, _, added)| added.len())
                .sum()
        };
        for object in self.objects.values_mut() {
            let slots = [Some(&mut object.own), object.ripples.as_mut()];
            for slot in slots.into_iter().flatten() {
                slot.stamps += put_before(slot.ord, usize::MAX);
            }
        }
        for end in self.ends.values_mut() {
            end.stamps += put_before(end.ord, end.stamps);
        }
        self.slot_ids = slot_ids;
        self.slots = None;
        Some(content.replaced(None, Some(stamps)))
    }

    /// The Stamp `id` as the content draws it, placed after `groups` groups.
    fn new_stamp(&mut self, map: &Map, id: Id, groups: usize) -> Option<PlacedStamp> {
        let Some(Node::Object(object)) = map.node(id) else {
            return None;
        };
        let ObjectKind::Stamp(stamp) = &object.kind else {
            return None;
        };
        let opacity = layer_opacity(map, id);
        let draw = stamp_draw(stamp, opacity);
        self.stamps.insert(
            id,
            CachedStamp {
                version: map.version(id)?,
                opacity,
                build: self.build,
                draw: draw.clone(),
            },
        );
        Some(PlacedStamp {
            after: groups,
            clip: None,
            placement: placement_in(map, object.membership),
            draw,
        })
    }

    /// Adds to `dirty` the Objects whose outlines or rings have landed in the background since
    /// they were drawn. `None` if one of them can no longer be drawn.
    fn pick_up_landed(
        &mut self,
        map: &Map,
        roughness: &mut RoughnessCache,
        dirty: &mut Vec<Id>,
    ) -> Option<()> {
        let shored: Vec<(Id, u64, Option<Arc<MultiPolygon>>)> = self
            .objects
            .iter()
            .filter(|(_, slot)| slot.shore)
            .map(|(&id, slot)| (id, slot.serial, slot.outline.clone()))
            .collect();
        for (id, serial, drawn) in shored {
            if dirty.contains(&id) {
                continue;
            }
            let (shore, latest) = (map.shore(id)?, outline(roughness, map, id)?);
            // Until the roughened outline is ready, what was drawn stands.
            let ready = roughness.is_ready(map, id);
            let current = match (&drawn, ready) {
                (Some(drawn), false) => drawn.clone(),
                _ => latest,
            };
            if ready
                && !drawn
                    .as_ref()
                    .is_some_and(|drawn| Arc::ptr_eq(drawn, &current))
            {
                dirty.push(id);
                continue;
            }
            self.rings.prepare(id, &current, shore, false);
            if self.rings.serial(id) != serial {
                dirty.push(id);
            }
        }
        Some(())
    }

    /// The Stamps of `previous` with those of `ids` drawn anew in the same slots.
    fn patch_stamps(
        &mut self,
        map: &Map,
        ids: &[Id],
        previous: &Content,
    ) -> Option<Vec<PlacedStamp>> {
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
        for &id in ids {
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
        Some(placed)
    }

    /// What the Objects `ids` draw now, in place of what they drew. `None` if one of them is
    /// not one the build recorded.
    fn replacements(
        &mut self,
        map: &Map,
        roughness: &mut RoughnessCache,
        ids: &[Id],
    ) -> Option<Vec<Replacement>> {
        let mut out = Vec::new();
        for &id in ids {
            let Some(Node::Object(object)) = map.node(id) else {
                return None;
            };
            let slot = self.objects.get(&id)?.clone();
            let place = Place {
                opacity: layer_opacity(map, id),
                placement: placement_in(map, object.membership),
            };
            let mut ripples = Vec::new();
            self.ripples(map, roughness, id, &mut ripples);
            let mut own = Vec::new();
            self.draw(map, roughness, id, place, &mut own, &mut Vec::new());
            mark_object(map, id, place.placement, &mut own);
            let latest = outline(roughness, map, id);
            let recorded = self.objects.get_mut(&id)?;
            recorded.serial = self.rings.serial(id);
            recorded.outline = recorded.shore.then_some(latest).flatten();
            let stretches = [(true, slot.ripples, ripples), (false, Some(slot.own), own)];
            for (is_ripples, old, new) in stretches {
                let Some(old) = old else { continue };
                if old.range.is_empty() && new.is_empty() {
                    continue;
                }
                out.push(Replacement {
                    id,
                    ripples: is_ripples,
                    slot: old,
                    new,
                });
            }
        }
        Some(out)
    }

    /// `previous` with `replacements` swapped in for the groups they name, `stamps` for its
    /// Stamps if given, and the Stamps and `self.objects` moved to follow.
    fn splice(
        &mut self,
        previous: &Content,
        stamps: Option<Vec<PlacedStamp>>,
        mut replacements: Vec<Replacement>,
    ) -> Option<Content> {
        replacements.sort_by_key(|r| r.slot.ord);
        if !replacements
            .windows(2)
            .all(|pair| pair[0].slot.range.end <= pair[1].slot.range.start)
        {
            return None;
        }
        if replacements.is_empty() {
            return Some(previous.replaced(None, stamps));
        }
        let before = previous.groups();
        let mut groups = Vec::with_capacity(before.len());
        let mut cursor = 0;
        for replacement in &replacements {
            groups.extend_from_slice(&before[cursor..replacement.slot.range.start]);
            groups.extend(replacement.new.iter().cloned());
            cursor = replacement.slot.range.end;
        }
        groups.extend_from_slice(&before[cursor..]);
        // What comes after a replaced stretch moves by how much it grew.
        let grew = |r: &Replacement| r.new.len() as isize - r.slot.range.len() as isize;
        let mut placed = stamps;
        for r in &replacements {
            // Stamps drawn before a stretch end before it starts, and the rest begin after it.
            let list = placed.as_deref().unwrap_or(previous.stamps());
            let (earlier, later) = (r.slot.stamps.checked_sub(1), list.get(r.slot.stamps));
            let ok = earlier.is_none_or(|i| list[i].after <= r.slot.range.start)
                && later.is_none_or(|stamp| stamp.after >= r.slot.range.end);
            if !ok {
                return None;
            }
        }
        if replacements.iter().any(|r| grew(r) != 0) {
            let list = placed.get_or_insert_with(|| previous.stamps().to_vec());
            let (mut next, mut moved) = (0, 0isize);
            for (number, stamp) in list.iter_mut().enumerate() {
                while next < replacements.len() && replacements[next].slot.stamps <= number {
                    moved += grew(&replacements[next]);
                    next += 1;
                }
                stamp.after = stamp.after.checked_add_signed(moved)?;
            }
        }
        let replaced: HashMap<(Id, bool), usize> = replacements
            .iter()
            .map(|r| ((r.id, r.ripples), r.new.len()))
            .collect();
        let moved_before = |ord: usize| -> isize {
            replacements
                .iter()
                .take_while(|r| r.slot.ord < ord)
                .map(grew)
                .sum()
        };
        for (&id, object) in &mut self.objects {
            let slots = [
                (false, Some(&mut object.own)),
                (true, object.ripples.as_mut()),
            ];
            for (is_ripples, slot) in slots {
                let Some(slot) = slot else { continue };
                let start = slot
                    .range
                    .start
                    .checked_add_signed(moved_before(slot.ord))?;
                let len = replaced
                    .get(&(id, is_ripples))
                    .copied()
                    .unwrap_or_else(|| slot.range.len());
                slot.range = start..start + len;
            }
        }
        for end in self.ends.values_mut() {
            end.groups = end.groups.checked_add_signed(moved_before(end.ord))?;
        }
        Some(previous.replaced(Some(groups), placed))
    }

    /// Adds the ripples of `sibling`, which draw beneath every fill of its Layer.
    fn ripples(
        &mut self,
        map: &Map,
        roughness: &mut RoughnessCache,
        sibling: Id,
        groups: &mut Vec<VectorGroup>,
    ) {
        let (Some(shore), Some(outline)) = (map.shore(sibling), outline(roughness, map, sibling))
        else {
            return;
        };
        let first = groups.len();
        self.rings.prepare(sibling, &outline, shore, self.complete);
        ripple_groups(
            &mut |distance| self.rings.ring(sibling, distance),
            shore,
            layer_opacity(map, sibling),
            groups,
        );
        mark_object(
            map,
            sibling,
            placement_of(map, sibling),
            &mut groups[first..],
        );
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
                    // A River's mouth opens into the water it flows into, so its line and
                    // shading run along the banks only.
                    let river = match kind {
                        ObjectKind::River(river) => Some(river),
                        _ => None,
                    };
                    let reach = shore.shading.width / f64::from(SHADING_BANDS);
                    shading_groups(
                        &mut |distance| self.rings.ring(id, distance),
                        &|ring: &MultiPolygon, distance: f64| match river {
                            Some(river) => {
                                lines_path(&river.away_from_mouth(ring, distance.abs() + reach))
                            }
                            None => region_path(ring),
                        },
                        shore,
                        opacity,
                        groups,
                    );
                    if shore.outline.width > 0.0 {
                        let path = match river {
                            Some(river) => lines_path(&river.banks()),
                            None => region_path(&outline),
                        };
                        groups.push(stroke(
                            path,
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

/// The Stamps `added` in the order they draw, if they sit on top of Layer `layer`: after every
/// other child that draws before its Lakes, the last of which is an Object, not a Layer.
fn top_of_layer(map: &Map, layer: Id, added: &[Id]) -> Option<Vec<Id>> {
    let new: HashSet<Id> = added.iter().copied().collect();
    let mut top = Vec::with_capacity(added.len());
    for &child in map.children(Some(layer)).iter().rev() {
        if new.contains(&child) {
            top.push(child);
            continue;
        }
        match map.node(child) {
            Some(Node::Object(object)) if object.kind.is_water() => {}
            Some(Node::Object(_)) => break,
            _ => return None,
        }
    }
    top.reverse();
    (top.len() == added.len()).then_some(top)
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
            && object.kind.is_water()
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
            path: Arc::new(path),
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
    path: &impl Fn(&MultiPolygon, f64) -> Path,
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
                path(&ring, distance),
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

/// Open lines, each drawn from its first point to its last.
fn lines_path(lines: &[Vec<[f64; 2]>]) -> Path {
    let mut builder = Path::builder();
    for line in lines {
        let mut points = line.iter().map(|&[x, y]| point(x as f32, y as f32));
        let Some(first) = points.next() else {
            continue;
        };
        builder.begin(first);
        for p in points {
            builder.line_to(p);
        }
        builder.end(false);
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
    use ssmap_core::{Canvas, Command, DistanceUnit, Label, MultiPolygon, Polygon, Scale};

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

    fn square(x: f64, size: f64) -> MultiPolygon {
        MultiPolygon(vec![Polygon {
            exterior: vec![
                [x, 100.0],
                [x + size, 100.0],
                [x + size, 100.0 + size],
                [x, 100.0 + size],
            ],
            holes: Vec::new(),
        }])
    }

    /// Stamps under, three Landmasses in the middle and Stamps above with a Label.
    fn map_with_land() -> (Map, Vec<Id>) {
        let mut map = Map::new(Canvas::new(800, 600, Scale::new(1.0, DistanceUnit::Miles)));
        let layer = |map: &mut Map, name: &str| {
            let id = Id::new();
            map.execute(&Command::CreateLayer {
                id,
                parent: None,
                index: None,
                name: name.into(),
            })
            .unwrap();
            id
        };
        let stamp = |map: &mut Map, parent: Id, x: f64| {
            map.execute(&Command::CreateStamp {
                id: Id::new(),
                parent,
                index: None,
                stamp: Box::new(ssmap_core::Stamp::new(
                    "base/tree".parse().unwrap(),
                    [x, 10.0],
                )),
            })
            .unwrap();
        };
        let (below, land, above) = (
            layer(&mut map, "Below"),
            layer(&mut map, "Land"),
            layer(&mut map, "Above"),
        );
        for x in [1.0, 2.0] {
            stamp(&mut map, below, x);
        }
        let lands: Vec<Id> = (0..3).map(|_| Id::new()).collect();
        for (i, &id) in lands.iter().enumerate() {
            map.execute(&Command::CreateLandmass {
                id,
                parent: land,
                index: None,
                geometry: square(i as f64 * 200.0, 100.0),
                fill: None,
            })
            .unwrap();
            map.execute(&Command::SetMoveLocked { id, locked: false })
                .unwrap();
        }
        for x in [3.0, 4.0] {
            stamp(&mut map, above, x);
        }
        map.execute(&Command::CreateLabel {
            id: Id::new(),
            parent: above,
            index: None,
            label: Box::new(Label {
                text: "Thule".into(),
                ..Label::default()
            }),
        })
        .unwrap();
        (map, lands)
    }

    fn assert_same_content(a: &Content, b: &Content) {
        assert_eq!(a.groups().len(), b.groups().len(), "group count");
        for (i, (x, y)) in a.groups().iter().zip(b.groups()).enumerate() {
            assert_eq!(x, y, "group {i}");
        }
        assert_same_stamps(a, b);
    }

    #[test]
    fn editing_a_landmass_redraws_only_it() {
        let (mut map, lands) = map_with_land();
        let mut scene = MapScene::new(Fonts::bundled());
        let mut roughness = RoughnessCache::new();
        let before = scene.content(&map, &mut roughness);
        map.execute(&Command::Translate {
            ids: vec![lands[1]],
            offset: [30.0, 20.0],
        })
        .unwrap();
        let after = scene.content(&map, &mut roughness);
        let fresh = MapScene::new(Fonts::bundled()).content(&map, &mut RoughnessCache::new());
        assert_same_content(&after, &fresh);
        // The others' paths are the very ones built before; the moved one's are new.
        let shared = |id: Id| {
            let own = |content: &Content| {
                content
                    .groups()
                    .iter()
                    .filter(|g| g.object.is_some_and(|o| o.id == id) && !g.effect)
                    .map(|g| g.shapes[0].path.clone())
                    .collect::<Vec<_>>()
            };
            own(&before)
                .iter()
                .zip(own(&after))
                .all(|(x, y)| Arc::ptr_eq(x, &y))
        };
        assert!(shared(lands[0]) && shared(lands[2]));
        assert!(!shared(lands[1]));
    }

    #[test]
    fn a_landmass_that_draws_more_groups_moves_the_stamps_after_it() {
        let (mut map, lands) = map_with_land();
        let mut scene = MapScene::new(Fonts::bundled());
        let mut roughness = RoughnessCache::new();
        let before = scene.content(&map, &mut roughness);
        let mut shore = map.shore(lands[1]).unwrap().clone();
        shore.ripples.count += 3;
        map.execute(&Command::SetShore {
            ids: vec![lands[1]],
            shore: Some(shore),
        })
        .unwrap();
        let after = scene.content(&map, &mut roughness);
        assert!(after.groups().len() > before.groups().len());
        let fresh = MapScene::new(Fonts::bundled()).content(&map, &mut RoughnessCache::new());
        assert_same_content(&after, &fresh);
        // Edit again, to check the places it recorded moved too.
        map.execute(&Command::Translate {
            ids: vec![lands[0], lands[2]],
            offset: [5.0, 5.0],
        })
        .unwrap();
        let again = scene.content(&map, &mut roughness);
        let fresh = MapScene::new(Fonts::bundled()).content(&map, &mut RoughnessCache::new());
        assert_same_content(&again, &fresh);
    }

    #[test]
    fn editing_land_and_stamps_together_patches_both() {
        let (mut map, lands) = map_with_land();
        let mut scene = MapScene::new(Fonts::bundled());
        let mut roughness = RoughnessCache::new();
        let stamp_ids: Vec<Id> = map
            .draw_order()
            .into_iter()
            .filter(|&id| matches!(map.node(id), Some(Node::Object(o)) if matches!(o.kind, ObjectKind::Stamp(_))))
            .collect();
        scene.content(&map, &mut roughness);
        move_stamp(&mut map, stamp_ids[0], [77.0, 7.0]);
        map.execute(&Command::Translate {
            ids: vec![lands[2]],
            offset: [-20.0, 0.0],
        })
        .unwrap();
        move_stamp(&mut map, stamp_ids[3], [88.0, 8.0]);
        let patched = scene.content(&map, &mut roughness);
        let fresh = MapScene::new(Fonts::bundled()).content(&map, &mut RoughnessCache::new());
        assert_same_content(&patched, &fresh);
    }

    #[test]
    fn editing_a_clipped_map_rebuilds() {
        let (mut map, lands) = map_with_land();
        let layers: Vec<Id> = map.children(None).to_vec();
        map.execute(&Command::SetClip {
            id: layers[0],
            target: Some(lands[0]),
        })
        .unwrap();
        let mut scene = MapScene::new(Fonts::bundled());
        let mut roughness = RoughnessCache::new();
        scene.content(&map, &mut roughness);
        map.execute(&Command::Translate {
            ids: vec![lands[0]],
            offset: [10.0, 10.0],
        })
        .unwrap();
        let rebuilt = scene.content(&map, &mut roughness);
        let fresh = MapScene::new(Fonts::bundled()).content(&map, &mut RoughnessCache::new());
        assert_eq!(rebuilt.clips().len(), fresh.clips().len());
        assert_same_content(&rebuilt, &fresh);
    }

    #[test]
    fn rings_that_land_after_an_edit_patch_only_that_landmass() {
        let (mut map, lands) = map_with_land();
        map.execute(&Command::SetDefaultRoughness {
            roughness: ssmap_core::Roughness::new(8.0, 16.0, 3),
        })
        .unwrap();
        let (sender, landed) = std::sync::mpsc::channel();
        let sender = std::sync::Mutex::new(sender);
        let mut scene = MapScene::new(Fonts::bundled());
        scene.compute_rings_in_background(move || {
            let _ = sender.lock().unwrap().send(());
        });
        let mut roughness = RoughnessCache::new();
        // Let every Landmass's rings land.
        for _ in 0..30 {
            scene.content(&map, &mut roughness);
            if landed
                .recv_timeout(std::time::Duration::from_millis(300))
                .is_err()
            {
                break;
            }
        }
        let settled = scene.content(&map, &mut roughness);
        let mut shore = map.shore(lands[1]).unwrap().clone();
        shore.ripples.spacing += 7.0;
        map.execute(&Command::SetShore {
            ids: vec![lands[1]],
            shore: Some(shore),
        })
        .unwrap();
        scene.content(&map, &mut roughness);
        landed
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("the moved Landmass's rings land");
        let after = scene.content(&map, &mut roughness);
        let fresh = MapScene::new(Fonts::bundled()).content(&map, &mut RoughnessCache::new());
        assert_same_content(&after, &fresh);
        let first_path = |content: &Content, id: Id| {
            content
                .groups()
                .iter()
                .find(|g| g.object.is_some_and(|o| o.id == id) && !g.effect)
                .map(|g| g.shapes[0].path.clone())
                .unwrap()
        };
        assert!(
            Arc::ptr_eq(
                &first_path(&settled, lands[0]),
                &first_path(&after, lands[0])
            ),
            "the Landmass nobody touched was not drawn again"
        );
    }

    fn place(map: &mut Map, parent: Id, xs: &[f64]) -> Vec<Id> {
        let stamps: Vec<ssmap_core::PlacedStamp> = xs
            .iter()
            .map(|&x| ssmap_core::PlacedStamp {
                id: Id::new(),
                stamp: ssmap_core::Stamp::new("base/tree".parse().unwrap(), [x, 33.0]),
            })
            .collect();
        let ids = stamps.iter().map(|placed| placed.id).collect();
        map.execute(&Command::PlaceStamps { parent, stamps })
            .unwrap();
        ids
    }

    fn assert_matches_fresh(
        scene: &mut MapScene,
        map: &Map,
        roughness: &mut RoughnessCache,
    ) -> Content {
        let content = scene.content(map, roughness);
        let fresh = MapScene::new(Fonts::bundled()).content(map, &mut RoughnessCache::new());
        assert_same_content(&content, &fresh);
        content
    }

    #[test]
    fn stamps_placed_on_top_of_layers_are_patched_in() {
        let (mut map, lands) = map_with_land();
        let layers: Vec<Id> = map.children(None).to_vec();
        let mut scene = MapScene::new(Fonts::bundled());
        let mut roughness = RoughnessCache::new();
        let before = scene.content(&map, &mut roughness);
        place(&mut map, layers[0], &[10.0, 11.0, 12.0]);
        let after = assert_matches_fresh(&mut scene, &map, &mut roughness);
        assert!(
            std::ptr::eq(after.groups(), before.groups()),
            "the groups are shared, not rebuilt"
        );
        assert_eq!(after.stamps().len(), before.stamps().len() + 3);
        // Another Layer in the same patch, and one more patch after it.
        place(&mut map, layers[2], &[20.0]);
        place(&mut map, layers[0], &[30.0, 31.0]);
        assert_matches_fresh(&mut scene, &map, &mut roughness);
        // The places it recorded followed: edit land, a Stamp, and place again.
        map.execute(&Command::Translate {
            ids: vec![lands[0]],
            offset: [3.0, 3.0],
        })
        .unwrap();
        assert_matches_fresh(&mut scene, &map, &mut roughness);
        let stamp_ids: Vec<Id> = map
            .draw_order()
            .into_iter()
            .filter(|&id| matches!(map.node(id), Some(Node::Object(o)) if matches!(o.kind, ObjectKind::Stamp(_))))
            .collect();
        move_stamp(&mut map, stamp_ids[4], [5.0, 5.0]);
        place(&mut map, layers[2], &[40.0]);
        assert_matches_fresh(&mut scene, &map, &mut roughness);
    }

    #[test]
    fn placing_under_other_things_rebuilds() {
        let (mut map, _) = map_with_land();
        let layers: Vec<Id> = map.children(None).to_vec();
        let mut scene = MapScene::new(Fonts::bundled());
        let mut roughness = RoughnessCache::new();
        scene.content(&map, &mut roughness);
        // Below the Layer's other Stamps.
        map.execute(&Command::CreateStamp {
            id: Id::new(),
            parent: layers[0],
            index: Some(0),
            stamp: Box::new(ssmap_core::Stamp::new(
                "base/tree".parse().unwrap(),
                [1.0, 2.0],
            )),
        })
        .unwrap();
        assert_matches_fresh(&mut scene, &map, &mut roughness);
        // Into a Layer that holds a Layer on top of its Objects.
        let inner = Id::new();
        map.execute(&Command::CreateLayer {
            id: inner,
            parent: Some(layers[0]),
            index: None,
            name: "Inner".into(),
        })
        .unwrap();
        scene.content(&map, &mut roughness);
        place(&mut map, layers[0], &[8.0]);
        assert_matches_fresh(&mut scene, &map, &mut roughness);
        // A Landmass is not a Stamp.
        map.execute(&Command::CreateLandmass {
            id: Id::new(),
            parent: layers[1],
            index: None,
            geometry: square(600.0, 50.0),
            fill: None,
        })
        .unwrap();
        assert_matches_fresh(&mut scene, &map, &mut roughness);
    }

    #[test]
    fn changing_a_map_wide_default_redraws_the_land_and_leaves_the_stamps() {
        let (mut map, _) = map_with_land();
        let mut scene = MapScene::new(Fonts::bundled());
        let mut roughness = RoughnessCache::new();
        let before = scene.content(&map, &mut roughness);
        for (amount, scale) in [(6.0, 12.0), (9.0, 18.0)] {
            map.execute(&Command::SetDefaultRoughness {
                roughness: ssmap_core::Roughness::new(amount, scale, 3),
            })
            .unwrap();
            let after = assert_matches_fresh(&mut scene, &map, &mut roughness);
            assert!(
                std::ptr::eq(after.stamps(), before.stamps()),
                "no Stamp is copied when no group count changes"
            );
        }
        let mut shore = map
            .shore(map.children(Some(map.children(None)[1]))[0])
            .unwrap()
            .clone();
        shore.ripples.count += 2;
        map.execute(&Command::SetDefaultShore {
            kind: ssmap_core::ShoreKind::Landmass,
            shore,
        })
        .unwrap();
        let grown = assert_matches_fresh(&mut scene, &map, &mut roughness);
        assert!(grown.groups().len() > before.groups().len());
    }

    #[test]
    fn outlines_that_land_redraw_only_their_objects_and_until_then_the_last_ones_stand() {
        let (mut map, lands) = map_with_land();
        // One Landmass has its own Roughness, so a change to the default is not its.
        map.execute(&Command::SetRoughness {
            ids: vec![lands[0]],
            roughness: Some(ssmap_core::Roughness::new(4.0, 20.0, 9)),
        })
        .unwrap();
        map.execute(&Command::SetDefaultRoughness {
            roughness: ssmap_core::Roughness::new(6.0, 14.0, 2),
        })
        .unwrap();
        let mut scene = MapScene::new(Fonts::bundled());
        let mut roughness = RoughnessCache::new();
        let before = scene.content(&map, &mut roughness);
        // The default changes, and a rebuild is started but not yet run.
        map.execute(&Command::SetDefaultRoughness {
            roughness: ssmap_core::Roughness::new(11.0, 22.0, 4),
        })
        .unwrap();
        let rebuild = roughness.rebuild(&map);
        assert_eq!(rebuild.len(), 2, "the two without their own Roughness");
        let meanwhile = scene.content(&map, &mut roughness);
        assert_same_content(&meanwhile, &before);
        // The outlines land.
        rebuild.run(|| {});
        let after = scene.content(&map, &mut roughness);
        let fresh = MapScene::new(Fonts::bundled()).content(&map, &mut RoughnessCache::new());
        assert_same_content(&after, &fresh);
        let first_path = |content: &Content, id: Id| {
            content
                .groups()
                .iter()
                .find(|g| g.object.is_some_and(|o| o.id == id) && !g.effect)
                .map(|g| g.shapes[0].path.clone())
                .unwrap()
        };
        assert!(
            Arc::ptr_eq(
                &first_path(&meanwhile, lands[0]),
                &first_path(&after, lands[0])
            ),
            "the Landmass with its own Roughness was not drawn again"
        );
        assert!(!Arc::ptr_eq(
            &first_path(&meanwhile, lands[1]),
            &first_path(&after, lands[1])
        ));
    }
}
