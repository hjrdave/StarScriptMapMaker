//! The lyon renderer: its pipelines, GPU buffers and draw calls.

use std::ops::Range;
use std::time::Duration;

use wgpu::util::DeviceExt;

use super::cache::TessellationCache;
use super::chunk::{
    Built, Chunked, Draw, EMPTY_BOUNDS, Tessellators, Vertex, build, resolve, split, union,
};
use super::resident::{Pages, Resident};
use super::{DrawFilter, VectorGroup, overlaps};
use crate::canvas::{CLIP_INSIDE, StencilUse};
use crate::fill_texture::FillTextures;

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
    /// Drawn instead of `mesh` while `drawing_resident`: meshes kept on the GPU between
    /// updates (see [`LyonRenderer::update_runs`]).
    resident: Option<Resident>,
    resident_pages: Vec<GpuPage>,
    drawing_resident: bool,
    cache: TessellationCache,
    /// The texture array fills sample, as bind group 1.
    textures: wgpu::BindGroup,
}

/// The whole Map's triangles, in as many buffer pairs as the device's `max_buffer_size` needs.
struct Mesh {
    parts: Vec<Part>,
}

/// One vertex buffer and one index buffer, with the draws that use them.
struct Part {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    draws: Vec<Draw>,
    /// Ranges of `draws`, one per run (see [`LyonRenderer::set_runs`]).
    runs: Vec<Range<usize>>,
}

impl Part {
    fn bind(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
    }
}

/// Uploads `built` as buffers that each fit the device's `max_buffer_size`.
fn upload_parts(device: &wgpu::Device, built: Built, label: &str) -> Vec<Part> {
    split(built, device.limits().max_buffer_size)
        .into_iter()
        .filter(|piece| !piece.geometry.indices.is_empty())
        .map(|piece| Part {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(&piece.geometry.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(&piece.geometry.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            draws: piece.draws,
            runs: piece.runs,
        })
        .collect()
}

impl LyonRenderer {
    pub(crate) fn new(
        device: &wgpu::Device,
        view_layout: &wgpu::BindGroupLayout,
        textures: &FillTextures,
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
            bind_group_layouts: &[Some(view_layout), Some(textures.layout())],
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
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4, 2 => Unorm8x4, 3 => Unorm8x4, 4 => Unorm8x4],
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
            resident: None,
            resident_pages: Vec::new(),
            drawing_resident: false,
            cache: TessellationCache::new(),
            textures: textures.bind_group().clone(),
        }
    }

    /// Draws fills with the textures in `bind_group`, which uses the layout this was made with.
    pub(crate) fn set_textures(&mut self, bind_group: &wgpu::BindGroup) {
        self.textures = bind_group.clone();
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
        self.resident = None;
        self.resident_pages.clear();
        self.drawing_resident = false;
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
        let built = build(groups, cuts, &self.cache);
        self.chunked = None;
        self.drawing_resident = false;
        let parts = upload_parts(device, built, "vector");
        self.mesh = (!parts.is_empty()).then_some(Mesh { parts });
    }

    /// Like [`LyonRenderer::set_runs`], but keeps every mesh on the GPU between calls and writes
    /// only the meshes that are not there yet, so an edit uploads only what it changed
    /// (ADR 0019). The draw list is rebuilt in stacking order each time.
    pub(crate) fn update_runs(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        groups: &[VectorGroup],
        cuts: &[usize],
    ) {
        let meshes = resolve(groups, &self.cache, &mut Tessellators::default());
        self.chunked = None;
        self.mesh = None;
        let resident = self
            .resident
            .get_or_insert_with(|| Resident::new(device.limits().max_buffer_size));
        resident.update(
            &meshes,
            cuts,
            &mut GpuPages {
                device,
                queue,
                pages: &mut self.resident_pages,
            },
        );
        self.drawing_resident = true;
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
        self.drawing_resident = false;
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
            for chunk in chunked.chunks[range.clone()]
                .iter()
                .filter_map(|chunk| chunk.part.as_ref())
                .filter(|part| overlaps(part.bounds, filter.bounds))
            {
                for part in &chunk.parts {
                    part.bind(pass);
                    self.draw_draws(pass, &part.draws, clipped, filter);
                }
            }
            return;
        }
        if let Some(resident) = self.drawn_resident() {
            let Some(range) = resident.runs.get(run) else {
                return;
            };
            for segment in &resident.segments[range.clone()] {
                let Some(page) = self.resident_pages.get(segment.page) else {
                    continue;
                };
                page.bind(pass);
                self.draw_draws(
                    pass,
                    &resident.draws[segment.draws.clone()],
                    clipped,
                    filter,
                );
            }
            return;
        }
        let Some(mesh) = &self.mesh else { return };
        for part in &mesh.parts {
            let Some(range) = part.runs.get(run) else {
                continue;
            };
            part.bind(pass);
            self.draw_draws(pass, &part.draws[range.clone()], clipped, filter);
        }
    }

    fn drawn_resident(&self) -> Option<&Resident> {
        self.resident.as_ref().filter(|_| self.drawing_resident)
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
        pass.set_bind_group(1, &self.textures, &[]);
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
        pass.set_stencil_reference(value);
        pass.set_bind_group(1, &self.textures, &[]);
        pass.set_pipeline(&self.mask);
        if let Some(resident) = self.drawn_resident() {
            let Some(range) = resident.runs.get(run) else {
                return;
            };
            for segment in &resident.segments[range.clone()] {
                let Some(page) = self.resident_pages.get(segment.page) else {
                    continue;
                };
                page.bind(pass);
                for draw in &resident.draws[segment.draws.clone()] {
                    pass.draw_indexed(draw.indices.clone(), 0, 0..1);
                }
            }
            return;
        }
        let Some(mesh) = &self.mesh else { return };
        for part in &mesh.parts {
            let Some(range) = part.runs.get(run) else {
                continue;
            };
            part.bind(pass);
            for draw in &part.draws[range.clone()] {
                pass.draw_indexed(draw.indices.clone(), 0, 0..1);
            }
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
    /// Empty for a chunk with nothing to draw.
    parts: Vec<Part>,
    bounds: [f32; 4],
}

fn upload(device: &wgpu::Device, built: Built) -> ChunkMesh {
    let bounds = built
        .draws
        .iter()
        .fold(EMPTY_BOUNDS, |all, d| union(all, d.bounds));
    ChunkMesh {
        parts: upload_parts(device, built, "chunk"),
        bounds,
    }
}

/// One page of [`Resident`]: a vertex buffer and an index buffer that meshes are written into.
struct GpuPage {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
}

impl GpuPage {
    fn bind(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
    }
}

/// [`Resident`]'s pages on the GPU. Writes go through `queue.write_buffer`, which stages only
/// the bytes written, so nothing else in the buffer is copied or cleared again.
struct GpuPages<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    pages: &'a mut Vec<GpuPage>,
}

impl Pages for GpuPages<'_> {
    fn reset(&mut self, sizes: &[(u32, u32)]) {
        let buffer = |count: u32, size: usize, usage: wgpu::BufferUsages| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("vector page"),
                size: u64::from(count) * size as u64,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        *self.pages = sizes
            .iter()
            .map(|&(vertices, indices)| GpuPage {
                vertices: buffer(
                    vertices,
                    std::mem::size_of::<Vertex>(),
                    wgpu::BufferUsages::VERTEX,
                ),
                indices: buffer(
                    indices,
                    std::mem::size_of::<u32>(),
                    wgpu::BufferUsages::INDEX,
                ),
            })
            .collect();
    }

    fn write(
        &mut self,
        page: usize,
        vertex_at: u32,
        vertices: &[Vertex],
        index_at: u32,
        indices: &[u32],
    ) {
        let Some(target) = self.pages.get(page) else {
            return;
        };
        if !vertices.is_empty() {
            self.queue.write_buffer(
                &target.vertices,
                u64::from(vertex_at) * std::mem::size_of::<Vertex>() as u64,
                bytemuck::cast_slice(vertices),
            );
        }
        if !indices.is_empty() {
            self.queue.write_buffer(
                &target.indices,
                u64::from(index_at) * std::mem::size_of::<u32>() as u64,
                bytemuck::cast_slice(indices),
            );
        }
    }
}
