//! The lyon renderer: its pipelines, GPU buffers and draw calls.

use std::ops::Range;
use std::time::Duration;

use wgpu::util::DeviceExt;

use super::cache::TessellationCache;
use super::chunk::{Built, Chunked, Draw, EMPTY_BOUNDS, Vertex, build, union};
use super::{DrawFilter, VectorGroup, overlaps};
use crate::canvas::{CLIP_INSIDE, StencilUse};

/// lyon tessellation drawn as plain triangles; antialiasing comes from the target's MSAA.
pub(crate) struct LyonRenderer {
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
    /// Drawn instead of `mesh` while set: the groups applied chunk by chunk (see
    /// [`LyonRenderer::apply_chunks`]).
    chunked: Option<Chunked<ChunkMesh>>,
    cache: TessellationCache,
}

struct Mesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    draws: Vec<Draw>,
    /// Ranges of `draws`, one per run (see [`LyonRenderer::set_runs`]).
    runs: Vec<Range<usize>>,
}

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
                    include_str!("../shaders/view.wgsl"),
                    include_str!("../shaders/fill.wgsl")
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
            chunked: None,
            cache: TessellationCache::new(),
        }
    }

    /// The per-Object tessellation cache that [`LyonRenderer::set_runs`] draws from.
    pub(crate) fn cache(&self) -> &TessellationCache {
        &self.cache
    }

    /// Draws from another cache, dropping what was built from the old one.
    pub(crate) fn set_cache(&mut self, cache: TessellationCache) {
        self.cache = cache;
        self.mesh = None;
        self.chunked = None;
    }
}

impl LyonRenderer {
    /// Like [`LyonRenderer::set_groups`], but also splits the groups into runs that
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
        self.chunked = None;
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

    /// Whether the groups are drawn chunk by chunk (see [`LyonRenderer::start_chunks`]).
    pub(crate) fn is_chunked(&self) -> bool {
        self.chunked.is_some()
    }

    /// Like [`LyonRenderer::set_runs`], but only splits the groups into chunks, which
    /// [`LyonRenderer::apply_chunks`] then builds. What was drawn stays until its chunk is
    /// replaced.
    pub(crate) fn start_chunks(&mut self, groups: &[VectorGroup], cuts: &[usize]) {
        self.mesh = None;
        let previous = self.chunked.take();
        self.chunked = Some(Chunked::new(previous, groups, cuts, &self.cache));
    }

    /// Builds the chunks of the groups last given to [`LyonRenderer::start_chunks`] that are
    /// out of date, within `budget` (all of them if `None`). Returns whether all are built.
    pub(crate) fn apply_chunks(
        &mut self,
        device: &wgpu::Device,
        groups: &[VectorGroup],
        budget: Option<Duration>,
    ) -> bool {
        let Some(chunked) = &mut self.chunked else {
            return true;
        };
        chunked.apply(groups, &self.cache, budget, |built| upload(device, built))
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
        if let Some(chunked) = &self.chunked {
            let Some(range) = chunked.runs.get(run) else {
                return;
            };
            for part in chunked.chunks[range.clone()]
                .iter()
                .filter_map(|chunk| chunk.part.as_ref())
                .filter(|part| overlaps(part.bounds, filter.bounds))
            {
                let Some((vertices, indices)) = &part.buffers else {
                    continue;
                };
                pass.set_vertex_buffer(0, vertices.slice(..));
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                self.draw_draws(pass, &part.draws, clipped, filter);
            }
            return;
        }
        let Some(mesh) = &self.mesh else { return };
        let Some(range) = mesh.runs.get(run) else {
            return;
        };
        pass.set_vertex_buffer(0, mesh.vertices.slice(..));
        pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
        self.draw_draws(pass, &mesh.draws[range.clone()], clipped, filter);
    }

    /// Draws what `filter` lets through of `draws`, from the buffers already set on `pass`.
    fn draw_draws(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        draws: &[Draw],
        clipped: bool,
        filter: &DrawFilter,
    ) {
        pass.set_stencil_reference(if clipped { CLIP_INSIDE } else { 0 });
        let plain = if clipped { &self.clipped } else { &self.plain };
        let visible =
            |draw: &&Draw| (filter.effects || !draw.effect) && overlaps(draw.bounds, filter.bounds);
        for draw in draws.iter().filter(visible) {
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

impl LyonRenderer {
    /// Replaces what is drawn. Groups draw in order, first lowest.
    pub(crate) fn set_groups(&mut self, device: &wgpu::Device, groups: &[VectorGroup]) {
        self.set_runs(device, groups, &[]);
    }

    /// Draws everything into a Canvas render pass whose group 0 is the view uniform and which
    /// has a stencil attachment cleared to 0.
    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw_run(pass, 0, false, &DrawFilter::everything());
    }
}

/// A chunk's triangles on the GPU.
struct ChunkMesh {
    /// Vertex and index buffers; `None` for a chunk with nothing to draw.
    buffers: Option<(wgpu::Buffer, wgpu::Buffer)>,
    draws: Vec<Draw>,
    bounds: [f32; 4],
}

fn upload(device: &wgpu::Device, built: Built) -> ChunkMesh {
    let Built {
        geometry, draws, ..
    } = built;
    let bounds = draws
        .iter()
        .fold(EMPTY_BOUNDS, |all, d| union(all, d.bounds));
    let buffers = (!geometry.indices.is_empty()).then(|| {
        (
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("chunk vertices"),
                contents: bytemuck::cast_slice(&geometry.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("chunk indices"),
                contents: bytemuck::cast_slice(&geometry.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
        )
    });
    ChunkMesh {
        buffers,
        draws,
        bounds,
    }
}
