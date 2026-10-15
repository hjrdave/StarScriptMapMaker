//! Vector fills and strokes behind a trait, so the renderer can be swapped (ADR 0001).

use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use lyon::path::Path;
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, LineCap, LineJoin, StrokeOptions,
    StrokeTessellator, StrokeVertex, VertexBuffers,
};
use ssmap_core::Id;
use wgpu::util::DeviceExt;

pub use lyon::tessellation::FillRule;

use crate::canvas::{CLIP_INSIDE, StencilUse};
use crate::color::{Color, ColorExt as _};

/// How a path is painted.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Paint {
    Fill(FillRule),
    /// A stroke centred on the path, `width` Canvas units across, with round joins and caps.
    Stroke {
        width: f32,
    },
}

/// One painted path in Canvas units.
#[derive(Clone, Debug)]
pub struct VectorShape {
    pub path: Path,
    pub color: Color,
    pub paint: Paint,
}

impl PartialEq for VectorShape {
    fn eq(&self, other: &Self) -> bool {
        self.color == other.color
            && self.paint == other.paint
            && self.path.iter().eq(other.path.iter())
    }
}

/// Shapes painted as one, such as a Label's fill and its halo. Every sample takes the colour
/// of the first shape in the group that covers it, so the group never blends with itself:
/// a translucent stroke doesn't darken where it overlaps itself, and a halo listed after the
/// fill shows only around it.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct VectorGroup {
    pub shapes: Vec<VectorShape>,
    /// A Shore effect (ripples or inner shading), which a frame drawn mid-gesture skips.
    pub effect: bool,
    /// The Object the group draws, whose tessellation is cached as a whole. Groups of no
    /// Object (Clip masks, the backdrop) are tessellated afresh every time.
    pub object: Option<ObjectRef>,
}

/// An Object at the version its groups were built from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ObjectRef {
    pub id: Id,
    pub version: u64,
}

impl VectorGroup {
    pub fn fill(path: Path, color: Color, rule: FillRule) -> Self {
        Self {
            shapes: vec![VectorShape {
                path,
                color,
                paint: Paint::Fill(rule),
            }],
            effect: false,
            object: None,
        }
    }

    /// Marks the group as drawing `object`.
    pub fn of_object(mut self, object: ObjectRef) -> Self {
        self.object = Some(object);
        self
    }

    /// Marks the group as a Shore effect.
    pub fn into_effect(mut self) -> Self {
        self.effect = true;
        self
    }

    /// Whether drawing the group's triangles plainly could cover a sample twice. Fill
    /// tessellation never overlaps itself; strokes and stacked shapes do.
    fn may_overlap(&self) -> bool {
        match self.shapes.as_slice() {
            [] => false,
            [shape] => matches!(shape.paint, Paint::Stroke { .. }),
            _ => true,
        }
    }
}

/// What a frame draws of the groups: those that reach `bounds`, and Shore effects only if
/// `effects`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DrawFilter {
    /// The visible Canvas area as `[min_x, min_y, max_x, max_y]`.
    pub bounds: [f32; 4],
    pub effects: bool,
}

impl DrawFilter {
    pub(crate) fn everything() -> Self {
        Self {
            bounds: [f32::MIN, f32::MIN, f32::MAX, f32::MAX],
            effects: true,
        }
    }
}

/// Whether two `[min_x, min_y, max_x, max_y]` boxes overlap.
pub(crate) fn overlaps(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

/// Draws vector groups into a Canvas render pass whose group 0 is the view uniform and which
/// has a stencil attachment cleared to 0.
pub trait VectorRenderer {
    /// Replaces what is drawn. Groups draw in order, first lowest.
    fn set_groups(&mut self, device: &wgpu::Device, groups: &[VectorGroup]);

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>);
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    color: [f32; 4],
}

/// lyon tessellation drawn as plain triangles; antialiasing comes from the target's MSAA.
pub struct LyonRenderer {
    /// Draws without touching the stencil.
    plain: wgpu::RenderPipeline,
    /// Draws only inside the active Clip.
    clipped: wgpu::RenderPipeline,
    /// Draws only where the stencil equals the reference, and marks what it draws.
    once: wgpu::RenderPipeline,
    /// Draws no colour, and clears the marks `once` left.
    reset: wgpu::RenderPipeline,
    /// Draws no colour, and sets the stencil to the reference.
    mask: wgpu::RenderPipeline,
    mesh: Option<Mesh>,
    cache: TessellationCache,
}

struct Mesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    draws: Vec<Draw>,
    /// Ranges of `draws`, one per run (see [`LyonRenderer::set_runs`]).
    runs: Vec<Range<usize>>,
}

/// A run of indices: either several groups that can't overlap themselves, drawn plainly,
/// or one group drawn once per sample.
struct Draw {
    indices: Range<u32>,
    once: bool,
    effect: bool,
    /// The Canvas area its triangles cover, as `[min_x, min_y, max_x, max_y]`.
    bounds: [f32; 4],
}

/// Merged groups stop growing here, so one draw's bounds stay small enough to cull by.
const MERGE_LIMIT: u32 = 30_000;

const EMPTY_BOUNDS: [f32; 4] = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];

fn union(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

fn bounds_of(vertices: &[Vertex]) -> [f32; 4] {
    vertices.iter().fold(EMPTY_BOUNDS, |bounds, v| {
        let [x, y] = v.position;
        union(bounds, [x, y, x, y])
    })
}

/// Curve flattening tolerance in Canvas units.
const TOLERANCE: f32 = 0.05;

impl LyonRenderer {
    pub(crate) fn new(
        device: &wgpu::Device,
        view_layout: &wgpu::BindGroupLayout,
        samples: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fill"),
            source: wgpu::ShaderSource::Wgsl(
                concat!(
                    include_str!("shaders/view.wgsl"),
                    include_str!("shaders/fill.wgsl")
                )
                .into(),
            ),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fill"),
            bind_group_layouts: &[Some(view_layout)],
            immediate_size: 0,
        });
        let pipeline = |label, stencil| {
            crate::canvas::canvas_pipeline(
                device,
                label,
                &layout,
                &shader,
                &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
                })],
                samples,
                stencil,
            )
        };
        Self {
            plain: pipeline("fill", StencilUse::Ignore),
            clipped: pipeline("fill clipped", StencilUse::Clipped),
            once: pipeline("fill once", StencilUse::PaintOnce),
            reset: pipeline("fill reset", StencilUse::Reset),
            mask: pipeline("fill mask", StencilUse::Mask),
            mesh: None,
            cache: TessellationCache::new(),
        }
    }

    /// The per-Object tessellation cache that [`LyonRenderer::set_runs`] draws from.
    pub(crate) fn cache(&self) -> &TessellationCache {
        &self.cache
    }
}

impl LyonRenderer {
    /// Like [`VectorRenderer::set_groups`], but also splits the groups into runs that
    /// [`LyonRenderer::draw_run`] draws separately, so something else can draw between them.
    /// `cuts` lists, in increasing order, the group counts to split at: run `i` ends before
    /// group `cuts[i]`, and the last run takes the rest. There is one more run than cuts.
    pub(crate) fn set_runs(
        &mut self,
        device: &wgpu::Device,
        groups: &[VectorGroup],
        cuts: &[usize],
    ) {
        let Built {
            geometry,
            draws,
            runs,
        } = build(groups, cuts, &self.cache);
        self.mesh = (!geometry.indices.is_empty()).then(|| Mesh {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vector vertices"),
                contents: bytemuck::cast_slice(&geometry.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vector indices"),
                contents: bytemuck::cast_slice(&geometry.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            draws,
            runs,
        });
    }

    /// Draws run `run` of the groups last given to [`LyonRenderer::set_runs`], only inside the
    /// active Clip if `clipped`, and only what `filter` lets through.
    pub(crate) fn draw_run(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        run: usize,
        clipped: bool,
        filter: &DrawFilter,
    ) {
        let Some(mesh) = &self.mesh else { return };
        let Some(range) = mesh.runs.get(run) else {
            return;
        };
        pass.set_vertex_buffer(0, mesh.vertices.slice(..));
        pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.set_stencil_reference(if clipped { CLIP_INSIDE } else { 0 });
        let plain = if clipped { &self.clipped } else { &self.plain };
        let visible =
            |draw: &&Draw| (filter.effects || !draw.effect) && overlaps(draw.bounds, filter.bounds);
        for draw in mesh.draws[range.clone()].iter().filter(visible) {
            if draw.once {
                pass.set_pipeline(&self.once);
                pass.draw_indexed(draw.indices.clone(), 0, 0..1);
                pass.set_pipeline(&self.reset);
                pass.draw_indexed(draw.indices.clone(), 0, 0..1);
            } else {
                pass.set_pipeline(plain);
                pass.draw_indexed(draw.indices.clone(), 0, 0..1);
            }
        }
    }

    /// Sets the stencil to `value` under run `run` of the groups last given to
    /// [`LyonRenderer::set_runs`], without drawing colour.
    pub(crate) fn draw_mask(&self, pass: &mut wgpu::RenderPass<'_>, run: usize, value: u32) {
        let Some(mesh) = &self.mesh else { return };
        let Some(range) = mesh.runs.get(run) else {
            return;
        };
        pass.set_vertex_buffer(0, mesh.vertices.slice(..));
        pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.set_stencil_reference(value);
        pass.set_pipeline(&self.mask);
        for draw in &mesh.draws[range.clone()] {
            pass.draw_indexed(draw.indices.clone(), 0, 0..1);
        }
    }
}

impl VectorRenderer for LyonRenderer {
    fn set_groups(&mut self, device: &wgpu::Device, groups: &[VectorGroup]) {
        self.set_runs(device, groups, &[]);
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw_run(pass, 0, false, &DrawFilter::everything());
    }
}

/// Adds one shape's triangles. A path lyon cannot tessellate draws nothing rather than
/// failing the frame.
fn tessellate(
    shape: &VectorShape,
    geometry: &mut VertexBuffers<Vertex, u32>,
    fill: &mut FillTessellator,
    stroke: &mut StrokeTessellator,
) {
    let color = shape.color.to_linear_premultiplied();
    match shape.paint {
        Paint::Fill(rule) => {
            let options = FillOptions::tolerance(TOLERANCE).with_fill_rule(rule);
            let _ = fill.tessellate_path(
                &shape.path,
                &options,
                &mut BuffersBuilder::new(geometry, |vertex: FillVertex| Vertex {
                    position: vertex.position().to_array(),
                    color,
                }),
            );
        }
        Paint::Stroke { width } => {
            let options = StrokeOptions::tolerance(TOLERANCE)
                .with_line_width(width)
                .with_line_join(LineJoin::Round)
                .with_line_cap(LineCap::Round);
            let _ = stroke.tessellate_path(
                &shape.path,
                &options,
                &mut BuffersBuilder::new(geometry, |vertex: StrokeVertex| Vertex {
                    position: vertex.position().to_array(),
                    color,
                }),
            );
        }
    }
}

/// One group's triangles, with indices counted from its own first vertex.
struct GroupMesh {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    once: bool,
    effect: bool,
    bounds: [f32; 4],
}

#[derive(Default)]
struct Tessellators {
    fill: FillTessellator,
    stroke: StrokeTessellator,
}

fn mesh_group(group: &VectorGroup, tessellators: &mut Tessellators) -> GroupMesh {
    let mut geometry: VertexBuffers<Vertex, u32> = VertexBuffers::new();
    for shape in &group.shapes {
        tessellate(
            shape,
            &mut geometry,
            &mut tessellators.fill,
            &mut tessellators.stroke,
        );
    }
    GroupMesh {
        bounds: bounds_of(&geometry.vertices),
        vertices: geometry.vertices,
        indices: geometry.indices,
        once: group.may_overlap(),
        effect: group.effect,
    }
}

/// The merged buffers and draws for a list of groups.
struct Built {
    geometry: VertexBuffers<Vertex, u32>,
    draws: Vec<Draw>,
    runs: Vec<Range<usize>>,
}

/// Merges the groups' triangles into draws, split into runs at `cuts` (see
/// [`LyonRenderer::set_runs`]). The triangles of an Object come from `cache`. While the cache
/// is being filled in the background, an Object that is not ready is left out, and appears in
/// a later frame; otherwise it is tessellated now and kept.
fn build(groups: &[VectorGroup], cuts: &[usize], cache: &TessellationCache) -> Built {
    let mut tessellators = Tessellators::default();
    let meshes = resolve(groups, cache, &mut tessellators);
    let mut geometry: VertexBuffers<Vertex, u32> = VertexBuffers::new();
    let mut draws: Vec<Draw> = Vec::new();
    let mut runs: Vec<Range<usize>> = Vec::new();
    let mut run_start = 0;
    let mut remaining_cuts = cuts.iter().peekable();
    // The first group after a cut starts a new draw, so no draw spans two runs.
    let mut split = false;
    for (position, mesh) in meshes.iter().enumerate() {
        while remaining_cuts.next_if(|&&cut| cut <= position).is_some() {
            runs.push(run_start..draws.len());
            run_start = draws.len();
            split = true;
        }
        let Some(mesh) = mesh else { continue };
        if mesh.indices.is_empty() {
            continue;
        }
        let start = geometry.indices.len() as u32;
        let base = geometry.vertices.len() as u32;
        geometry.vertices.extend_from_slice(&mesh.vertices);
        geometry
            .indices
            .extend(mesh.indices.iter().map(|&index| index + base));
        let end = geometry.indices.len() as u32;
        match draws.last_mut() {
            Some(last)
                if !split
                    && !mesh.once
                    && !last.once
                    && last.effect == mesh.effect
                    && end - last.indices.start <= MERGE_LIMIT =>
            {
                last.indices.end = end;
                last.bounds = union(last.bounds, mesh.bounds);
            }
            _ => draws.push(Draw {
                indices: start..end,
                once: mesh.once,
                effect: mesh.effect,
                bounds: mesh.bounds,
            }),
        }
        split = false;
    }
    for _ in remaining_cuts {
        runs.push(run_start..draws.len());
        run_start = draws.len();
    }
    runs.push(run_start..draws.len());
    Built {
        geometry,
        draws,
        runs,
    }
}

type ObjectMeshes = std::vec::IntoIter<Arc<GroupMesh>>;

/// The mesh of each group, `None` for the groups of an Object left out of the frame.
fn resolve(
    groups: &[VectorGroup],
    cache: &TessellationCache,
    tessellators: &mut Tessellators,
) -> Vec<Option<Arc<GroupMesh>>> {
    let mut by_object: HashMap<Id, Vec<&VectorGroup>> = HashMap::new();
    for group in groups {
        if let Some(object) = group.object {
            by_object.entry(object.id).or_default().push(group);
            cache.note_version(object.id, object.version);
        }
    }
    let building = cache.is_building();
    let mut found: HashMap<Id, Option<ObjectMeshes>> = HashMap::new();
    let out = groups
        .iter()
        .map(|group| {
            let Some(object) = group.object else {
                return Some(Arc::new(mesh_group(group, tessellators)));
            };
            found
                .entry(object.id)
                .or_insert_with(|| {
                    let sources = &by_object[&object.id];
                    if let Some(meshes) = cache.lookup(object, sources) {
                        return Some(meshes.into_iter());
                    }
                    if building {
                        return None;
                    }
                    let meshes: Vec<_> = sources
                        .iter()
                        .map(|group| Arc::new(mesh_group(group, tessellators)))
                        .collect();
                    cache.store(object, sources, meshes.clone());
                    Some(meshes.into_iter())
                })
                .as_mut()
                .and_then(Iterator::next)
        })
        .collect();
    if !building {
        cache.retain(|id| by_object.contains_key(&id));
    }
    out
}

/// Tessellated Objects, kept so only an edited Object is tessellated again.
///
/// Clones share one store, so a worker thread can fill it (see [`TessellationCache::rebuild`])
/// while the editor keeps drawing. While a rebuild is running, an Object that is not ready is
/// left out of the frame instead of being tessellated on the drawing thread.
#[derive(Default, Clone)]
pub struct TessellationCache {
    shared: Arc<TessellationShared>,
}

#[derive(Default)]
struct TessellationShared {
    inner: Mutex<TessellationInner>,
    /// Bumped whenever an Object lands or a rebuild ends, so a frame drawn before is out of date.
    generation: AtomicU64,
    /// How many rebuilds are still running.
    building: AtomicUsize,
}

#[derive(Default)]
struct TessellationInner {
    entries: HashMap<Id, TessellationEntry>,
    /// The newest version of each Object a frame has asked for. A job that finishes with an
    /// older one is stale.
    current: HashMap<Id, u64>,
}

struct TessellationEntry {
    version: u64,
    /// What `meshes` were made from: a Layer's opacity changes a group without changing its
    /// Object's version, so the groups themselves are compared too.
    source: Vec<VectorGroup>,
    meshes: Vec<Arc<GroupMesh>>,
}

impl TessellationCache {
    pub fn new() -> Self {
        Self::default()
    }

    fn inner(&self) -> MutexGuard<'_, TessellationInner> {
        self.shared.inner.lock().expect("tessellation cache lock")
    }

    fn note_version(&self, id: Id, version: u64) {
        let mut inner = self.inner();
        let current = inner.current.entry(id).or_insert(version);
        *current = (*current).max(version);
    }

    fn lookup(&self, object: ObjectRef, sources: &[&VectorGroup]) -> Option<Vec<Arc<GroupMesh>>> {
        let inner = self.inner();
        let entry = inner.entries.get(&object.id)?;
        (entry.version == object.version
            && entry.source.len() == sources.len()
            && entry.source.iter().zip(sources).all(|(a, &b)| a == b))
        .then(|| entry.meshes.clone())
    }

    /// Keeps the meshes unless a newer version of the Object has been seen since.
    fn store(
        &self,
        object: ObjectRef,
        sources: &[&VectorGroup],
        meshes: Vec<Arc<GroupMesh>>,
    ) -> bool {
        let mut inner = self.inner();
        if inner
            .current
            .get(&object.id)
            .is_some_and(|&current| current > object.version)
        {
            return false;
        }
        inner.entries.insert(
            object.id,
            TessellationEntry {
                version: object.version,
                source: sources.iter().map(|&group| group.clone()).collect(),
                meshes,
            },
        );
        true
    }

    fn retain(&self, keep: impl Fn(Id) -> bool) {
        let mut inner = self.inner();
        inner.entries.retain(|&id, _| keep(id));
        inner.current.retain(|&id, _| keep(id));
    }

    /// Whether Object `id` is tessellated for exactly these groups.
    pub fn is_ready(&self, id: Id, version: u64, groups: &[VectorGroup]) -> bool {
        let sources: Vec<&VectorGroup> = groups.iter().collect();
        self.lookup(ObjectRef { id, version }, &sources).is_some()
    }

    /// How many Objects of `groups` are not tessellated yet, as a rebuild would find. Starts
    /// nothing, so it is cheap to ask every frame.
    pub fn pending(&self, groups: &[VectorGroup]) -> usize {
        let mut by_object: HashMap<Id, (ObjectRef, Vec<&VectorGroup>)> = HashMap::new();
        for group in groups {
            if let Some(object) = group.object {
                by_object
                    .entry(object.id)
                    .or_insert_with(|| (object, Vec::new()))
                    .1
                    .push(group);
            }
        }
        by_object
            .values()
            .filter(|(object, list)| self.lookup(*object, list).is_none())
            .count()
    }

    /// How many Objects are tessellated.
    pub fn len(&self) -> usize {
        self.inner().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Empties the cache, as when another Map is opened.
    pub fn clear(&self) {
        let mut inner = self.inner();
        inner.entries.clear();
        inner.current.clear();
    }

    /// Whether a [`TessellationRebuild`] is still running.
    pub fn is_building(&self) -> bool {
        self.shared.building.load(Ordering::SeqCst) > 0
    }

    /// Changes whenever a rebuild lands an Object or ends, which is when what was drawn from
    /// a partly filled cache is out of date.
    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::SeqCst)
    }

    /// Starts tessellating every Object of `groups` that is not ready. Each job snapshots the
    /// Object's groups and version, so the worker never reads anything that is being edited.
    /// Until the returned rebuild is run or dropped, frames leave out Objects that are not
    /// ready. It can be sent to another thread.
    pub fn rebuild(&self, groups: &[VectorGroup]) -> TessellationRebuild {
        let mut order: Vec<ObjectRef> = Vec::new();
        let mut by_object: HashMap<Id, Vec<&VectorGroup>> = HashMap::new();
        for group in groups {
            if let Some(object) = group.object {
                let list = by_object.entry(object.id).or_default();
                if list.is_empty() {
                    order.push(object);
                }
                list.push(group);
            }
        }
        let jobs = order
            .into_iter()
            .filter(|object| self.lookup(*object, &by_object[&object.id]).is_none())
            .map(|object| TessellationJob {
                object,
                groups: by_object[&object.id].iter().map(|&g| g.clone()).collect(),
            })
            .collect();
        self.shared.building.fetch_add(1, Ordering::SeqCst);
        TessellationRebuild {
            cache: self.clone(),
            jobs,
        }
    }
}

struct TessellationJob {
    object: ObjectRef,
    groups: Vec<VectorGroup>,
}

/// Objects still to tessellate, from [`TessellationCache::rebuild`]. Owns its inputs, so it
/// can run on a thread of its own. Dropping it unrun ends the rebuild.
pub struct TessellationRebuild {
    cache: TessellationCache,
    jobs: Vec<TessellationJob>,
}

impl TessellationRebuild {
    /// How many Objects this will tessellate.
    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Tessellates each Object, calling `ready` after each lands in the cache (so the frame can
    /// be rebuilt and repainted) and once more when the rebuild is over. A result for an Object
    /// that has been edited meanwhile is discarded. Tessellating holds no lock, so drawing is
    /// never blocked by it.
    pub fn run(mut self, mut ready: impl FnMut()) {
        let mut tessellators = Tessellators::default();
        for job in std::mem::take(&mut self.jobs) {
            let meshes = job
                .groups
                .iter()
                .map(|group| Arc::new(mesh_group(group, &mut tessellators)))
                .collect();
            let sources: Vec<&VectorGroup> = job.groups.iter().collect();
            if self.cache.store(job.object, &sources, meshes) {
                self.cache.shared.generation.fetch_add(1, Ordering::SeqCst);
                ready();
            }
        }
        // Ends the rebuild first, so the last repaint sees it over.
        drop(self);
        ready();
    }
}

impl Drop for TessellationRebuild {
    fn drop(&mut self) {
        self.cache.shared.building.fetch_sub(1, Ordering::SeqCst);
        self.cache.shared.generation.fetch_add(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lyon::math::point;

    fn square(x: f32) -> Path {
        let mut builder = Path::builder();
        builder.begin(point(x, 0.0));
        builder.line_to(point(x + 10.0, 0.0));
        builder.line_to(point(x + 10.0, 10.0));
        builder.line_to(point(x, 10.0));
        builder.close();
        builder.build()
    }

    fn fill(x: f32, object: Option<ObjectRef>) -> VectorGroup {
        let group = VectorGroup::fill(square(x), Color::rgb(10, 20, 30), FillRule::EvenOdd);
        match object {
            Some(object) => group.of_object(object),
            None => group,
        }
    }

    fn stroke(x: f32, object: ObjectRef) -> VectorGroup {
        VectorGroup {
            shapes: vec![VectorShape {
                path: square(x),
                color: Color::rgb(200, 0, 0),
                paint: Paint::Stroke { width: 2.0 },
            }],
            effect: false,
            object: Some(object),
        }
    }

    fn object(version: u64) -> ObjectRef {
        ObjectRef {
            id: Id::new(),
            version,
        }
    }

    /// Every group tessellated into one shared buffer, as the batched pass did before the cache.
    fn batched(groups: &[VectorGroup]) -> VertexBuffers<Vertex, u32> {
        let mut geometry = VertexBuffers::new();
        let mut fill = FillTessellator::new();
        let mut stroke = StrokeTessellator::new();
        for shape in groups.iter().flat_map(|group| &group.shapes) {
            tessellate(shape, &mut geometry, &mut fill, &mut stroke);
        }
        geometry
    }

    fn positions(geometry: &VertexBuffers<Vertex, u32>) -> Vec<[f32; 2]> {
        geometry.vertices.iter().map(|v| v.position).collect()
    }

    #[test]
    fn a_full_frame_equals_the_batched_pass() {
        let (a, b, c) = (object(1), object(2), object(3));
        let groups = vec![
            fill(0.0, Some(a)),
            stroke(20.0, b),
            fill(40.0, None),
            fill(60.0, Some(c)),
            stroke(80.0, a),
        ];
        let expected = batched(&groups);
        let cache = TessellationCache::new();
        // Tessellated on the spot, then again from the cache.
        for _ in 0..2 {
            let built = build(&groups, &[2], &cache);
            assert_eq!(built.geometry.indices, expected.indices);
            assert_eq!(positions(&built.geometry), positions(&expected));
            assert_eq!(built.runs.len(), 2);
        }
        // Filled by a worker instead.
        let background = TessellationCache::new();
        background.rebuild(&groups).run(|| {});
        let built = build(&groups, &[2], &background);
        assert_eq!(built.geometry.indices, expected.indices);
        assert_eq!(positions(&built.geometry), positions(&expected));
    }

    #[test]
    fn an_object_not_ready_is_left_out_until_it_is() {
        let (a, b) = (object(1), object(1));
        let groups = vec![fill(0.0, Some(a)), fill(20.0, Some(b)), fill(40.0, Some(a))];
        let cache = TessellationCache::new();
        let rebuild = cache.rebuild(&groups);
        assert_eq!(rebuild.len(), 2);
        assert!(cache.is_building());
        let during = build(&groups, &[1], &cache);
        assert!(during.geometry.indices.is_empty(), "nothing is ready");
        assert_eq!(during.runs.len(), 2, "runs stay as cut");
        assert!(cache.is_empty());

        let mut landed = Vec::new();
        let seen = cache.clone();
        rebuild.run(|| {
            let ready = seen.len();
            let built = build(&groups, &[], &seen);
            landed.push((ready, built.geometry.indices.len()));
        });
        assert!(!cache.is_building());
        // After the first Object only its two groups (a), then all three, then the end.
        let one = batched(&[groups[0].clone(), groups[2].clone()])
            .indices
            .len();
        let all = batched(&groups).indices.len();
        assert_eq!(landed, vec![(1, one), (2, all), (2, all)]);
    }

    #[test]
    fn a_stale_result_is_discarded() {
        let id = Id::new();
        let old = ObjectRef { id, version: 1 };
        let new = ObjectRef { id, version: 2 };
        let before = vec![fill(0.0, Some(old))];
        let after = vec![fill(5.0, Some(new))];
        let cache = TessellationCache::new();
        let rebuild = cache.rebuild(&before);
        // The Object is edited while the job is queued, and a frame sees the new version.
        let during = build(&after, &[], &cache);
        assert!(during.geometry.indices.is_empty());
        let mut repaints = 0;
        rebuild.run(|| repaints += 1);
        assert_eq!(repaints, 1, "only the end of the rebuild");
        assert!(cache.is_empty());
        assert!(!cache.is_ready(id, 1, &before));
        // The new version is tessellated by the next pass and then ready.
        cache.rebuild(&after).run(|| {});
        assert!(cache.is_ready(id, 2, &after));
        assert!(!cache.is_ready(id, 1, &before));
    }

    #[test]
    fn an_edit_that_keeps_the_version_still_retessellates() {
        // A Layer's opacity changes the group but not the Object.
        let a = object(1);
        let cache = TessellationCache::new();
        let first = vec![fill(0.0, Some(a))];
        build(&first, &[], &cache);
        assert!(cache.is_ready(a.id, 1, &first));
        let mut faded = first.clone();
        faded[0].shapes[0].color.a = 128;
        assert!(!cache.is_ready(a.id, 1, &faded));
        let built = build(&faded, &[], &cache);
        assert_eq!(
            built.geometry.vertices[0].color,
            batched(&faded).vertices[0].color
        );
    }

    #[test]
    fn objects_that_left_the_frame_are_forgotten() {
        let (a, b) = (object(1), object(1));
        let cache = TessellationCache::new();
        build(&[fill(0.0, Some(a)), fill(20.0, Some(b))], &[], &cache);
        assert_eq!(cache.len(), 2);
        build(&[fill(0.0, Some(a))], &[], &cache);
        assert_eq!(cache.len(), 1);
    }
}
