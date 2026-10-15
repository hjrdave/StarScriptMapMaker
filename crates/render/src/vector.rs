//! Vector fills and strokes behind a trait, so the renderer can be swapped (ADR 0001).

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use lyon::path::Path;
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, LineCap, LineJoin, StrokeOptions,
    StrokeTessellator, StrokeVertex, VertexBuffers,
};
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

/// Shapes painted as one, such as a Label's fill and its halo. Every sample takes the colour
/// of the first shape in the group that covers it, so the group never blends with itself:
/// a translucent stroke doesn't darken where it overlaps itself, and a halo listed after the
/// fill shows only around it.
#[derive(Clone, Debug, Default)]
pub struct VectorGroup {
    pub shapes: Vec<VectorShape>,
    /// A Shore effect (ripples or inner shading), which a frame drawn mid-gesture skips.
    pub effect: bool,
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
        }
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
    /// Tessellated groups by content hash, so a changed Map re-tessellates only what changed.
    cache: HashMap<u64, Arc<Tessellated>>,
    pub(crate) background: Option<Background>,
}

/// One group's triangles, with indices counted from this group's first vertex.
struct Tessellated {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    bounds: [f32; 4],
}

/// Tessellates groups on background threads. A group that isn't done yet is left out of the
/// mesh until it is.
pub(crate) struct Background {
    wake: Arc<dyn Fn() + Send + Sync>,
    sender: mpsc::Sender<(u64, Tessellated)>,
    results: Mutex<mpsc::Receiver<(u64, Tessellated)>>,
    /// Hashes of groups being tessellated.
    in_flight: HashSet<u64>,
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
            cache: HashMap::new(),
            background: None,
        }
    }

    /// Tessellates groups on background threads from now on, calling `wake` as each finishes.
    /// Groups not done yet are skipped by [`set_runs`](Self::set_runs) until they are.
    pub(crate) fn set_progressive(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        let (sender, results) = mpsc::channel();
        self.background = Some(Background {
            wake,
            sender,
            results: Mutex::new(results),
            in_flight: HashSet::new(),
        });
    }

    /// Takes in groups the background threads finished. Returns whether any arrived, in which
    /// case [`set_runs`](Self::set_runs) should run again.
    pub(crate) fn poll(&mut self) -> bool {
        let Some(background) = &mut self.background else {
            return false;
        };
        let mut arrived = false;
        let Ok(results) = background.results.get_mut() else {
            return false;
        };
        while let Ok((hash, tessellated)) = results.try_recv() {
            background.in_flight.remove(&hash);
            self.cache.insert(hash, Arc::new(tessellated));
            arrived = true;
        }
        arrived
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
        let mut vertices: Vec<Vertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut fill = FillTessellator::new();
        let mut stroke = StrokeTessellator::new();
        let mut draws: Vec<Draw> = Vec::new();
        let mut runs: Vec<Range<usize>> = Vec::new();
        let mut run_start = 0;
        let mut remaining_cuts = cuts.iter().peekable();
        // The first group after a cut starts a new draw, so no draw spans two runs.
        let mut split = false;
        let mut used: HashSet<u64> = HashSet::new();
        let mut wanted: Vec<(u64, VectorGroup)> = Vec::new();
        for (position, group) in groups.iter().enumerate() {
            while remaining_cuts.next_if(|&&cut| cut <= position).is_some() {
                runs.push(run_start..draws.len());
                run_start = draws.len();
                split = true;
            }
            let hash = group_hash(group);
            used.insert(hash);
            let tessellated = match self.cache.get(&hash) {
                Some(cached) => cached.clone(),
                None => match &mut self.background {
                    Some(background) => {
                        if background.in_flight.insert(hash) {
                            wanted.push((hash, group.clone()));
                        }
                        continue;
                    }
                    None => {
                        let built = Arc::new(tessellate_group(group, &mut fill, &mut stroke));
                        self.cache.insert(hash, built.clone());
                        built
                    }
                },
            };
            if tessellated.indices.is_empty() {
                continue;
            }
            let base = vertices.len() as u32;
            let start = indices.len() as u32;
            vertices.extend_from_slice(&tessellated.vertices);
            indices.extend(tessellated.indices.iter().map(|i| i + base));
            let end = indices.len() as u32;
            let once = group.may_overlap();
            let bounds = tessellated.bounds;
            match draws.last_mut() {
                Some(last)
                    if !split
                        && !once
                        && !last.once
                        && last.effect == group.effect
                        && end - last.indices.start <= MERGE_LIMIT =>
                {
                    last.indices.end = end;
                    last.bounds = union(last.bounds, bounds);
                }
                _ => draws.push(Draw {
                    indices: start..end,
                    once,
                    effect: group.effect,
                    bounds,
                }),
            }
            split = false;
        }
        for _ in remaining_cuts {
            runs.push(run_start..draws.len());
            run_start = draws.len();
        }
        runs.push(run_start..draws.len());
        self.cache.retain(|hash, _| used.contains(hash));
        if let Some(background) = &mut self.background {
            // Groups no longer wanted may still finish; the retain above drops them next call.
            background.in_flight.retain(|hash| used.contains(hash));
            self.spawn_tessellators(wanted);
        }
        self.mesh = (!indices.is_empty()).then(|| Mesh {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vector vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vector indices"),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            draws,
            runs,
        });
    }

    fn spawn_tessellators(&self, wanted: Vec<(u64, VectorGroup)>) {
        let Some(background) = &self.background else {
            return;
        };
        if wanted.is_empty() {
            return;
        }
        let workers = thread::available_parallelism()
            .map_or(1, |n| n.get().saturating_sub(1).max(1))
            .min(wanted.len());
        let queue = Arc::new(Mutex::new(wanted));
        for _ in 0..workers {
            let queue = queue.clone();
            let sender = background.sender.clone();
            let wake = background.wake.clone();
            thread::spawn(move || {
                let mut fill = FillTessellator::new();
                let mut stroke = StrokeTessellator::new();
                while let Some((hash, group)) = queue.lock().ok().and_then(|mut q| q.pop()) {
                    let built = tessellate_group(&group, &mut fill, &mut stroke);
                    if sender.send((hash, built)).is_err() {
                        return;
                    }
                    wake();
                }
            });
        }
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

fn tessellate_group(
    group: &VectorGroup,
    fill: &mut FillTessellator,
    stroke: &mut StrokeTessellator,
) -> Tessellated {
    let mut geometry: VertexBuffers<Vertex, u32> = VertexBuffers::new();
    for shape in &group.shapes {
        tessellate(shape, &mut geometry, fill, stroke);
    }
    let bounds = bounds_of(&geometry.vertices);
    Tessellated {
        vertices: geometry.vertices,
        indices: geometry.indices,
        bounds,
    }
}

/// Identifies a group by everything that decides its triangles.
fn group_hash(group: &VectorGroup) -> u64 {
    use lyon::path::PathEvent;
    let mut hasher = DefaultHasher::new();
    group.effect.hash(&mut hasher);
    let point = |hasher: &mut DefaultHasher, p: lyon::math::Point| {
        p.x.to_bits().hash(hasher);
        p.y.to_bits().hash(hasher);
    };
    for shape in &group.shapes {
        shape.color.hash(&mut hasher);
        match shape.paint {
            Paint::Fill(rule) => (0u8, rule == FillRule::EvenOdd).hash(&mut hasher),
            Paint::Stroke { width } => (1u8, width.to_bits()).hash(&mut hasher),
        }
        for event in shape.path.iter() {
            match event {
                PathEvent::Begin { at } => {
                    0u8.hash(&mut hasher);
                    point(&mut hasher, at);
                }
                PathEvent::Line { to, .. } => {
                    1u8.hash(&mut hasher);
                    point(&mut hasher, to);
                }
                PathEvent::Quadratic { ctrl, to, .. } => {
                    2u8.hash(&mut hasher);
                    point(&mut hasher, ctrl);
                    point(&mut hasher, to);
                }
                PathEvent::Cubic {
                    ctrl1, ctrl2, to, ..
                } => {
                    3u8.hash(&mut hasher);
                    point(&mut hasher, ctrl1);
                    point(&mut hasher, ctrl2);
                    point(&mut hasher, to);
                }
                PathEvent::End { close, .. } => {
                    4u8.hash(&mut hasher);
                    close.hash(&mut hasher);
                }
            }
        }
        // Separates shapes so that moving a point between them changes the hash.
        5u8.hash(&mut hasher);
    }
    hasher.finish()
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
