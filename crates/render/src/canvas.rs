//! The Canvas renderer: an offscreen, multisampled target holding the backdrop and the Map's
//! Objects.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use lyon::math::Box2D;
use lyon::path::{Path, Winding};
use wgpu::util::DeviceExt;

use crate::color::{Color, ColorExt as _};
use crate::gpu::GpuCapabilities;
use crate::ocean::{OceanPass, OceanUniform};
use crate::profile::{self, Stage};
use crate::sprite::SpriteStore;
use crate::stamp::{PlacedStamp, StampRenderer};
use crate::vector::{
    APPLY_BUDGET, DrawFilter, FillRule, LyonRenderer, Placement, TessellationCache,
    TessellationRebuild, VectorGroup,
};
use crate::view::View;

/// The resolved Canvas target's format. Blending happens in linear space, and readback bytes
/// are already sRGB-encoded for PNG.
pub const CANVAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// The Canvas pass's stencil, which lets a [`VectorGroup`] paint each sample once.
const STENCIL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Stencil8;

/// The ocean: a background fill plus a texture over it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Backdrop {
    pub water: Color,
    /// How strongly the ocean texture lightens and darkens the water, from 0 to 1. At 0 the
    /// water is flat.
    pub texture_strength: f32,
    /// Stretches the texture's noise; 1 is the default size.
    pub texture_scale: f32,
    /// The colour the texture lightens towards.
    pub texture_light: Color,
    /// The colour the texture darkens towards.
    pub texture_dark: Color,
}

impl Default for Backdrop {
    fn default() -> Self {
        Self::from(&ssmap_core::Background::default())
    }
}

impl From<&ssmap_core::Background> for Backdrop {
    fn from(background: &ssmap_core::Background) -> Self {
        let texture = &background.texture;
        Self {
            water: background.water,
            texture_strength: texture.strength,
            texture_scale: texture.scale,
            texture_light: texture.light,
            texture_dark: texture.dark,
        }
    }
}

/// A board of the scene besides the Canvas: an Art board's rectangle and background.
#[derive(Clone, PartialEq, Debug)]
pub struct BoardArea {
    /// Size in Canvas units.
    pub size: [f64; 2],
    /// The top-left corner in workspace coordinates.
    pub origin: [f64; 2],
    pub backdrop: Backdrop,
}

impl From<&ssmap_core::Canvas> for BoardArea {
    fn from(board: &ssmap_core::Canvas) -> Self {
        Self {
            size: [f64::from(board.width), f64::from(board.height)],
            origin: board.origin,
            backdrop: Backdrop::from(&board.background),
        }
    }
}

/// The Map's Objects: vector groups, first drawn lowest, with Stamps placed among them, as
/// [`MapScene`](crate::MapScene) builds them. Two are equal only if they are the same build
/// (or both empty), so comparing scenes every frame stays cheap.
#[derive(Clone, Debug, Default)]
pub struct Content {
    groups: Arc<[VectorGroup]>,
    stamps: Arc<[PlacedStamp]>,
    clips: Arc<[ClipScope]>,
}

/// A stretch of the content that shows only inside a shape, as a Layer clipped to a Landmass
/// does. Scopes are disjoint and in draw order.
#[derive(Clone, Debug, Default)]
pub struct ClipScope {
    /// The vector groups it clips. Stamps join it by their [`PlacedStamp::clip`].
    pub groups: Range<usize>,
    /// Fills of the Landmass the scope is clipped to.
    pub land: Vec<VectorGroup>,
    /// Fills of the Lakes over the Landmass, cut out of it.
    pub holes: Vec<VectorGroup>,
}

impl Content {
    pub fn new(groups: Vec<VectorGroup>) -> Self {
        Self::with_stamps(groups, Vec::new())
    }

    /// `stamps` must be in draw order: their `after` values never decrease, and none exceeds
    /// the number of groups.
    pub fn with_stamps(groups: Vec<VectorGroup>, stamps: Vec<PlacedStamp>) -> Self {
        Self::with_clips(groups, stamps, Vec::new())
    }

    /// `clips` must be disjoint and in order, inside the groups, and each Stamp's `clip` must
    /// index one.
    pub fn with_clips(
        groups: Vec<VectorGroup>,
        stamps: Vec<PlacedStamp>,
        clips: Vec<ClipScope>,
    ) -> Self {
        debug_assert!(stamps.windows(2).all(|pair| pair[0].after <= pair[1].after));
        debug_assert!(stamps.iter().all(|stamp| stamp.after <= groups.len()));
        debug_assert!(
            stamps
                .iter()
                .all(|s| s.clip.is_none_or(|c| c < clips.len()))
        );
        debug_assert!(
            clips
                .windows(2)
                .all(|pair| pair[0].groups.end <= pair[1].groups.start)
        );
        debug_assert!(clips.iter().all(|clip| clip.groups.end <= groups.len()));
        Self {
            groups: groups.into(),
            stamps: stamps.into(),
            clips: clips.into(),
        }
    }

    /// This content with its Stamps replaced by `stamps`, which must be the same Stamps in the
    /// same slots with other draws. The groups and clip scopes are shared, not copied.
    pub(crate) fn with_stamps_replaced(&self, stamps: Vec<PlacedStamp>) -> Self {
        debug_assert_eq!(stamps.len(), self.stamps.len());
        Self {
            groups: self.groups.clone(),
            stamps: stamps.into(),
            clips: self.clips.clone(),
        }
    }

    pub fn clips(&self) -> &[ClipScope] {
        &self.clips
    }

    pub fn groups(&self) -> &[VectorGroup] {
        &self.groups
    }

    pub fn stamps(&self) -> &[PlacedStamp] {
        &self.stamps
    }

    fn is_empty(&self) -> bool {
        self.groups.is_empty() && self.stamps.is_empty()
    }
}

impl PartialEq for Content {
    fn eq(&self, other: &Self) -> bool {
        (Arc::ptr_eq(&self.groups, &other.groups)
            && Arc::ptr_eq(&self.stamps, &other.stamps)
            && Arc::ptr_eq(&self.clips, &other.clips))
            || (self.is_empty() && other.is_empty())
    }
}

/// How much of the Map a frame draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Quality {
    /// Everything. Export always uses this.
    #[default]
    Full,
    /// Without Shore effects (ripples and inner shading), for frames drawn while the view is
    /// panning or zooming. The editor redraws at [`Quality::Full`] when the gesture stops.
    Interactive,
}

/// Everything the Canvas renderer draws.
#[derive(Clone, PartialEq, Debug)]
pub struct CanvasScene {
    /// Canvas size in Canvas units.
    pub canvas_size: [f64; 2],
    /// The Canvas's top-left corner in workspace coordinates.
    pub canvas_origin: [f64; 2],
    pub backdrop: Backdrop,
    /// The Art boards, in the Map's order without the Canvas: [`Placement::Board`] counts
    /// them. Empty for export, which draws the Canvas only.
    pub art_boards: Vec<BoardArea>,
    /// The area around the boards. Editor-only; export never shows it.
    pub off_canvas: Color,
    /// Drawn over the backdrop.
    pub content: Content,
    pub quality: Quality,
    /// Whether Objects off the Canvas are drawn. Export turns it off.
    pub off_canvas_objects: bool,
}

impl CanvasScene {
    pub fn new(canvas: &ssmap_core::Canvas) -> Self {
        Self {
            canvas_size: [f64::from(canvas.width), f64::from(canvas.height)],
            canvas_origin: canvas.origin,
            backdrop: Backdrop::from(&canvas.background),
            art_boards: Vec::new(),
            off_canvas: Color::rgb(0x1c, 0x1c, 0x1f),
            content: Content::default(),
            quality: Quality::Full,
            off_canvas_objects: true,
        }
    }

    /// A scene of every board of `map`: the Canvas and its Art boards, each with its own
    /// background. Export uses [`CanvasScene::new`], which has the Canvas only.
    pub fn for_map(map: &ssmap_core::Map) -> Self {
        let mut scene = Self::new(map.canvas());
        scene.art_boards = map
            .boards()
            .iter()
            .filter(|board| board.id != map.primary_board())
            .map(|board| BoardArea::from(&board.canvas))
            .collect();
        scene
    }

    pub fn with_quality(mut self, quality: Quality) -> Self {
        self.quality = quality;
        self
    }

    pub fn with_content(mut self, content: Content) -> Self {
        self.content = content;
        self
    }
}

/// The parts of a scene that change the backdrop.
#[derive(Clone, PartialEq, Debug)]
struct BackdropKey {
    canvas_size: [f64; 2],
    canvas_origin: [f64; 2],
    backdrop: Backdrop,
    art_boards: Vec<BoardArea>,
}

pub struct CanvasRenderer {
    capabilities: GpuCapabilities,
    view_uniform: wgpu::Buffer,
    view_bind_group: wgpu::BindGroup,
    /// The water under the ocean texture.
    backdrop: LyonRenderer,
    ocean: OceanPass,
    /// The Map's Objects, over the ocean.
    content: LyonRenderer,
    stamps: StampRenderer,
    /// The shapes of the content's Clips: for each scope, its land then its Lakes.
    masks: LyonRenderer,
    drawn_backdrop: Option<BackdropKey>,
    drawn_content: Option<Content>,
    /// The generation of the content's tessellation cache when it was last drawn from.
    drawn_tessellation: u64,
    /// Where the content's vector groups are split to draw Stamps between them.
    content_cuts: Vec<usize>,
    /// What a frame may spend applying finished results; `None` is no limit.
    apply_budget: Option<Duration>,
    /// Results were left over by the last frame's budget.
    carry: bool,
    target: Option<Target>,
}

struct Target {
    size: [u32; 2],
    /// Multisampled colour, absent when MSAA is unsupported.
    msaa: Option<wgpu::TextureView>,
    resolved: wgpu::Texture,
    resolved_view: wgpu::TextureView,
    /// At the multisampled size; its contents are never kept between frames.
    stencil: wgpu::TextureView,
}

impl CanvasRenderer {
    pub fn new(device: &wgpu::Device, capabilities: GpuCapabilities) -> Self {
        let samples = capabilities.msaa_samples;
        let view_layout = uniform_layout(device, "view");
        let view_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("view"),
            contents: bytemuck::bytes_of(&View::fit([1.0, 1.0], [1, 1], 0.0).uniform()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let view_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("view"),
            layout: &view_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: view_uniform.as_entire_binding(),
            }],
        });
        Self {
            backdrop: LyonRenderer::new(device, &view_layout, samples),
            ocean: OceanPass::new(device, &view_layout, samples),
            content: LyonRenderer::new(device, &view_layout, samples),
            masks: LyonRenderer::new(device, &view_layout, samples),
            stamps: StampRenderer::new(
                device,
                &view_layout,
                samples,
                crate::atlas::page_size_for(&capabilities.limits),
            ),
            capabilities,
            view_uniform,
            view_bind_group,
            drawn_backdrop: None,
            drawn_content: None,
            drawn_tessellation: 0,
            content_cuts: Vec::new(),
            apply_budget: Some(APPLY_BUDGET),
            carry: false,
            target: None,
        }
    }

    /// The per-Object tessellation cache behind the content. A background thread can fill it
    /// (see [`CanvasRenderer::start_tessellation`]); each frame draws what is ready.
    pub fn tessellation_cache(&self) -> TessellationCache {
        self.content.cache().clone()
    }

    /// Draws the content from `cache` instead of the one it has been using, as when the editor
    /// shows another Map: each Map keeps its own cache, so coming back to one that is fully
    /// tessellated only merges its meshes again. Does nothing if `cache` is already in use.
    pub fn use_tessellation_cache(&mut self, cache: &TessellationCache) {
        if self.content.cache().same_as(cache) {
            return;
        }
        self.content.set_cache(cache.clone());
        self.drawn_content = None;
        self.drawn_tessellation = 0;
        self.content_cuts.clear();
        self.carry = false;
    }

    /// Starts tessellating the Objects of `content` that are not ready, so a Map that has just
    /// opened can show Objects as they complete. Run the result on a thread of its own, with a
    /// `ready` that requests a repaint; frames drawn meanwhile leave out Objects that are not
    /// ready yet, and the next frame after each completion draws it.
    pub fn start_tessellation(&self, content: &Content) -> TessellationRebuild {
        self.content.cache().rebuild(content.groups())
    }

    /// Blocks until the derived data behind a frame of `content` is complete: the Stamp art
    /// it draws is rasterised and no tessellation rebuild is running. A frame drawn afterwards
    /// has no placeholders and leaves out no Object. (Roughened outlines are part of
    /// `content`; wait on the [`RoughnessCache`](ssmap_core::RoughnessCache) before building it.)
    pub fn wait_until_ready(&self, content: &Content) {
        if let Some(store) = self.stamps.store() {
            store.prefetch(content.stamps().iter().map(|placed| &placed.draw.asset));
            store.wait_until_ready();
        }
        self.content.cache().wait_until_ready();
    }

    /// Whether [`CanvasRenderer::wait_until_ready`] would return at once: the Stamp art of
    /// `content` is rasterised and no tessellation rebuild is running. Starts the Stamp art
    /// loading, but never blocks, so it can be asked every frame.
    pub fn is_ready(&self, content: &Content) -> bool {
        if let Some(store) = self.stamps.store() {
            store.prefetch(content.stamps().iter().map(|placed| &placed.draw.asset));
            if store.pending() > 0 {
                return false;
            }
        }
        !self.content.cache().is_building()
    }

    /// How long a frame may spend applying finished background results before carrying the
    /// rest to the next frame ([`APPLY_BUDGET`] by default); `None` applies them all.
    /// Returns the previous budget.
    pub fn set_apply_budget(&mut self, budget: Option<Duration>) -> Option<Duration> {
        std::mem::replace(&mut self.apply_budget, budget)
    }

    /// Whether the last frame left results to apply, so another frame should follow.
    pub fn apply_pending(&self) -> bool {
        self.carry
    }

    pub fn capabilities(&self) -> &GpuCapabilities {
        &self.capabilities
    }

    /// Where Stamp art comes from. Without it, Stamps draw nothing.
    pub fn set_sprites(&mut self, sprites: SpriteStore) {
        self.stamps.set_store(sprites);
        // Force the content to be prepared again with the new art.
        self.drawn_content = None;
    }

    /// Records the Canvas render into `encoder` and returns the resolved target, which holds
    /// the result once the encoder is submitted.
    #[allow(clippy::expect_used, reason = "ensured above")]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene: &CanvasScene,
        view: &View,
    ) -> &wgpu::Texture {
        let upload = profile::stage(Stage::Upload);
        let backdrop = BackdropKey {
            canvas_size: scene.canvas_size,
            canvas_origin: scene.canvas_origin,
            backdrop: scene.backdrop,
            art_boards: scene.art_boards.clone(),
        };
        if self.drawn_backdrop.as_ref() != Some(&backdrop) {
            self.set_backdrop(device, queue, backdrop);
        }
        // Art embedded in the Map changed (Update from pack, Undo, another Map): redraw even
        // if the Stamps are the same.
        let new_art = self.stamps.refresh_store();
        // Read first, so an Object that lands during the build draws in the next frame.
        let tessellation = self.content.cache().generation();
        let building = self.content.cache().is_building();
        let content_changed = new_art || self.drawn_content.as_ref() != Some(&scene.content);
        if content_changed {
            self.stamps.prepare(device, queue, scene.content.stamps());
            let clips = scene.content.clips();
            let cuts: Vec<usize> = {
                let mut cuts: Vec<usize> = self
                    .stamps
                    .cuts()
                    .chain(clips.iter().flat_map(|c| [c.groups.start, c.groups.end]))
                    .collect();
                // Each board scissors its members, and Objects off the boards draw unscissored,
                // so a run never mixes placements.
                let groups = scene.content.groups();
                cuts.extend(
                    (1..groups.len()).filter(|&i| groups[i].placement != groups[i - 1].placement),
                );
                cuts.sort_unstable();
                cuts.dedup();
                cuts
            };
            // While a rebuild runs (or a carried apply is unfinished), take the new content in
            // chunks within the frame budget; otherwise merge it all at once.
            if building || self.carry {
                self.content.start_chunks(scene.content.groups(), &cuts);
            } else {
                self.content.set_runs(device, scene.content.groups(), &cuts);
            }
            // Run 2n of the masks is scope n's land and run 2n + 1 its Lakes.
            let mut mask_groups = Vec::new();
            let mut mask_cuts = Vec::new();
            for clip in clips {
                mask_groups.extend_from_slice(&clip.land);
                mask_cuts.push(mask_groups.len());
                mask_groups.extend_from_slice(&clip.holes);
                mask_cuts.push(mask_groups.len());
            }
            self.masks.set_runs(device, &mask_groups, &mask_cuts);
            self.content_cuts = cuts;
            self.drawn_content = Some(scene.content.clone());
        }
        if content_changed || tessellation != self.drawn_tessellation || self.carry {
            self.drawn_tessellation = tessellation;
            if self.content.is_chunked() {
                self.carry =
                    !self
                        .content
                        .apply_chunks(device, scene.content.groups(), self.apply_budget);
            } else if !content_changed {
                // Results landed with no chunks to take them: merge everything.
                self.content
                    .set_runs(device, scene.content.groups(), &self.content_cuts);
            }
        }
        self.ensure_target(device, view.size);
        queue.write_buffer(&self.view_uniform, 0, bytemuck::bytes_of(&view.uniform()));
        drop(upload);
        let _draw = profile::stage(Stage::Draw);

        let target = self.target.as_ref().expect("ensured above");
        let (attachment, resolve_target) = match &target.msaa {
            Some(msaa) => (msaa, Some(&target.resolved_view)),
            None => (&target.resolved_view, None),
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("canvas"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: attachment,
                depth_slice: None,
                resolve_target,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(scene.off_canvas.to_wgpu()),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &target.stencil,
                depth_ops: None,
                stencil_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0),
                    store: wgpu::StoreOp::Discard,
                }),
            }),
            ..Default::default()
        });
        pass.set_bind_group(0, &self.view_bind_group, &[]);
        self.backdrop.draw(&mut pass);
        self.ocean.draw(&mut pass);
        let clips = scene.content.clips();
        let filter = DrawFilter {
            bounds: view.visible(),
            effects: scene.quality == Quality::Full,
        };
        let mut scissor = Scissor::new(scene, view);
        let mut active = None;
        let mut start = 0;
        for run in 0..=self.content_cuts.len() {
            // A run lies inside one Clip scope or outside them all, as scope edges are cuts.
            let scope = clips.iter().position(|clip| clip.groups.contains(&start));
            let placement = scene
                .content
                .groups()
                .get(start)
                .map_or(Placement::Canvas, |group| group.placement);
            self.enter(&mut pass, &mut scissor, &mut active, scope);
            if scissor.set(&mut pass, placement) {
                self.content
                    .draw_run(&mut pass, run, scope.is_some(), &filter);
            }
            let Some(&cut) = self.content_cuts.get(run) else {
                break;
            };
            for (clip, placement, batches) in self.stamps.runs_after(cut) {
                self.enter(&mut pass, &mut scissor, &mut active, clip);
                if scissor.set(&mut pass, placement) {
                    self.stamps
                        .draw(&mut pass, batches, clip.is_some(), filter.bounds);
                }
            }
            start = cut;
        }
        drop(pass);
        &target.resolved
    }

    /// Makes `scope` the active Clip: removes the previous one's shape from the stencil and
    /// writes the new one's. The shapes are written across the whole target, so a run on the
    /// off-Canvas in the same scope still finds them outside the Canvas.
    fn enter(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        scissor: &mut Scissor,
        active: &mut Option<usize>,
        scope: Option<usize>,
    ) {
        if *active == scope {
            return;
        }
        scissor.set(pass, Placement::OffCanvas);
        if let Some(old) = *active {
            self.masks.draw_mask(pass, 2 * old, 0);
        }
        if let Some(new) = scope {
            self.masks.draw_mask(pass, 2 * new, CLIP_INSIDE);
            self.masks.draw_mask(pass, 2 * new + 1, 0);
        }
        *active = scope;
    }

    fn set_backdrop(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, scene: BackdropKey) {
        // The Canvas first, then the Art boards. Where boards overlap, the later one's water
        // and ocean show.
        let boards: Vec<BoardArea> = std::iter::once(BoardArea {
            size: scene.canvas_size,
            origin: scene.canvas_origin,
            backdrop: scene.backdrop,
        })
        .chain(scene.art_boards.iter().cloned())
        .collect();
        let fills: Vec<VectorGroup> = boards
            .iter()
            .map(|board| {
                let [w, h] = board.size.map(|v| v as f32);
                let [x, y] = board.origin.map(|v| v as f32);
                let mut builder = Path::builder();
                builder.add_rectangle(
                    &Box2D::new((x, y).into(), (x + w, y + h).into()),
                    Winding::Positive,
                );
                VectorGroup::fill(builder.build(), board.backdrop.water, FillRule::NonZero)
            })
            .collect();
        self.backdrop.set_groups(device, &fills);
        let oceans: Vec<OceanUniform> = boards
            .iter()
            .map(|board| OceanUniform {
                canvas_size: board.size.map(|v| v as f32),
                origin: board.origin.map(|v| v as f32),
                _pad: [0.0; 2],
                strength: board.backdrop.texture_strength.clamp(0.0, 1.0),
                // Keeps the shader's division finite.
                scale: board.backdrop.texture_scale.max(0.01),
                light: board.backdrop.texture_light.to_linear_premultiplied(),
                dark: board.backdrop.texture_dark.to_linear_premultiplied(),
            })
            .collect();
        self.ocean.set(device, queue, &oceans);
        self.drawn_backdrop = Some(scene);
    }

    fn ensure_target(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        let size = size.map(|v| v.max(1));
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let samples = self.capabilities.msaa_samples;
        let msaa = (samples > 1).then(|| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("canvas msaa"),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format: CANVAS_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        });
        let resolved = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("canvas"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CANVAS_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            // Hosts with a non-sRGB frame read the encoded bytes through this view.
            view_formats: &[CANVAS_FORMAT.remove_srgb_suffix()],
        });
        let resolved_view = resolved.create_view(&Default::default());
        let stencil = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("canvas stencil"),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format: STENCIL_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        self.target = Some(Target {
            size,
            msaa,
            resolved,
            resolved_view,
            stencil,
        });
    }
}

/// Keeps Objects that belong to a board inside it by scissoring their draws to the board's
/// rectangle in target pixels. Objects off the boards draw across the whole target.
struct Scissor {
    /// The Canvas, then the Art boards: each rectangle as `[x, y, width, height]` clamped to
    /// the target; `None` when none of it is in view.
    boards: Vec<Option<[u32; 4]>>,
    size: [u32; 2],
    off_canvas_objects: bool,
    /// The kind of draw the pass is currently set up for.
    current: Option<Placement>,
}

impl Scissor {
    fn new(scene: &CanvasScene, view: &View) -> Self {
        let size = view.size.map(|v| v.max(1));
        let pixel = |axis: usize, canvas: f64| (canvas - view.origin[axis]) * view.zoom;
        let clamp = |axis: usize, v: f64| v.round().clamp(0.0, f64::from(size[axis])) as u32;
        let rect = |origin: [f64; 2], extent: [f64; 2]| {
            let min = [0, 1].map(|axis| clamp(axis, pixel(axis, origin[axis])));
            let max = [0, 1].map(|axis| clamp(axis, pixel(axis, origin[axis] + extent[axis])));
            (max[0] > min[0] && max[1] > min[1]).then_some([
                min[0],
                min[1],
                max[0] - min[0],
                max[1] - min[1],
            ])
        };
        let boards = std::iter::once(rect(scene.canvas_origin, scene.canvas_size))
            .chain(
                scene
                    .art_boards
                    .iter()
                    .map(|board| rect(board.origin, board.size)),
            )
            .collect();
        Self {
            boards,
            size,
            off_canvas_objects: scene.off_canvas_objects,
            current: None,
        }
    }

    /// The rectangle clipping `placement`: `None` for Objects off the boards. The inner
    /// `None` is a board with none of it in view; a board the scene lacks counts as that.
    fn rect(&self, placement: Placement) -> Option<Option<[u32; 4]>> {
        match placement {
            Placement::OffCanvas => None,
            Placement::Canvas => Some(self.boards[0]),
            Placement::Board(index) => Some(self.boards.get(index + 1).copied().flatten()),
        }
    }

    /// Sets the scissor for a draw of an Object off the boards or of a board's member, and
    /// says whether the draw can show anything.
    fn set(&mut self, pass: &mut wgpu::RenderPass<'_>, placement: Placement) -> bool {
        let rect = self.rect(placement);
        if self.current != Some(placement) {
            self.current = Some(placement);
            match rect.flatten() {
                Some([x, y, w, h]) => pass.set_scissor_rect(x, y, w, h),
                None => pass.set_scissor_rect(0, 0, self.size[0], self.size[1]),
            }
        }
        match rect {
            None => self.off_canvas_objects,
            Some(rect) => rect.is_some(),
        }
    }
}

pub(crate) fn uniform_layout(device: &wgpu::Device, label: &str) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

/// The stencil value of samples inside the active Clip. Paint-once marks count up from it.
pub(crate) const CLIP_INSIDE: u32 = 0x80;

/// How a Canvas pipeline uses the stencil, which starts each frame at 0. Samples inside the
/// active Clip hold [`CLIP_INSIDE`] and the rest 0; draws that respect a Clip run with that
/// value as their stencil reference, and with 0 when nothing is clipped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum StencilUse {
    Ignore,
    /// Draws only where the stencil equals the reference, which is the Clip's inside.
    Clipped,
    /// Draws only where the stencil equals the reference and marks what it draws, so
    /// overlapping triangles in one draw paint each sample once.
    PaintOnce,
    /// Draws no colour and puts the reference back where `PaintOnce` marked, ready for the
    /// next `PaintOnce` draw.
    Reset,
    /// Draws no colour and sets the stencil to the reference everywhere it covers: the Clip's
    /// inside with [`CLIP_INSIDE`], or its removal with 0.
    Mask,
}

impl StencilUse {
    fn state(self) -> wgpu::DepthStencilState {
        let face = |compare, pass_op| wgpu::StencilFaceState {
            compare,
            fail_op: wgpu::StencilOperation::Keep,
            depth_fail_op: wgpu::StencilOperation::Keep,
            pass_op,
        };
        let (face, write_mask) = match self {
            StencilUse::Ignore => (wgpu::StencilFaceState::IGNORE, 0),
            StencilUse::Clipped => (
                face(wgpu::CompareFunction::Equal, wgpu::StencilOperation::Keep),
                0,
            ),
            StencilUse::PaintOnce => (
                face(
                    wgpu::CompareFunction::Equal,
                    wgpu::StencilOperation::IncrementClamp,
                ),
                0xff,
            ),
            // Passes where the stencil is above the reference, that is, marked.
            StencilUse::Reset => (
                face(wgpu::CompareFunction::Less, wgpu::StencilOperation::Replace),
                0xff,
            ),
            StencilUse::Mask => (
                face(
                    wgpu::CompareFunction::Always,
                    wgpu::StencilOperation::Replace,
                ),
                0xff,
            ),
        };
        wgpu::DepthStencilState {
            format: STENCIL_FORMAT,
            depth_write_enabled: None,
            depth_compare: None,
            stencil: wgpu::StencilState {
                front: face,
                back: face,
                read_mask: 0xff,
                write_mask,
            },
            bias: Default::default(),
        }
    }
}

/// A pipeline drawing premultiplied colour into the Canvas target.
pub(crate) fn canvas_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
    samples: u32,
    stencil: StencilUse,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers,
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: CANVAS_FORMAT,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: if matches!(stencil, StencilUse::Reset | StencilUse::Mask) {
                    wgpu::ColorWrites::empty()
                } else {
                    wgpu::ColorWrites::ALL
                },
            })],
            compilation_options: Default::default(),
        }),
        primitive: Default::default(),
        depth_stencil: Some(stencil.state()),
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        multiview_mask: None,
        cache: None,
    })
}
