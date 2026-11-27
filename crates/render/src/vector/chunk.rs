//! Tessellation of groups, merged into draws and split to fit the device's buffers.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, LineCap, LineJoin, StrokeOptions,
    StrokeTessellator, StrokeVertex, VertexBuffers,
};
use ssmap_core::{
    FillStyle, Id, MAX_GRAIN_SIZE, MAX_MOTTLE_SCALE, MAX_TEXTURE_SCALE, MIN_GRAIN_SIZE,
    MIN_MOTTLE_SCALE, MIN_TEXTURE_SCALE,
};

use super::cache::{Previous, TessellationCache, mesh_object};
use super::{ObjectRef, Paint, VectorGroup, VectorShape};
use crate::color::ColorExt as _;
use crate::fill_texture::slot_for;
use crate::profile::{self, Stage};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Vertex {
    pub(super) position: [f32; 2],
    color: [f32; 4],
    /// Grain and mottle settings, packed as the shader unpacks them (see [`pack_style`]).
    look: [u8; 4],
    /// The mottle colour (sRGB) and the byte that offsets its patches.
    tint: [u8; 4],
    /// The texture: its slot (0 for none), its amount and its scale (see [`pack_texture`]).
    texture: [u8; 4],
}

/// A value on a log scale from `min` to `max`, as a byte. The shader undoes it with
/// `min * pow(max / min, byte / 255)`, so the ranges here and in `fill.wgsl` must agree.
fn log_byte(value: f64, min: f64, max: f64) -> u8 {
    ((value / min).ln() / (max / min).ln() * 255.0)
        .round()
        .clamp(0.0, 255.0) as u8
}

/// A fraction from 0 to 1 as a byte.
fn unit_byte(value: f64) -> u8 {
    (value * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Packs a fill's style into two 4-byte vertex attributes, so the vertex stays 32 bytes:
/// `look` is the grain amount, the grain size, the mottle contrast and the mottle scale, and
/// `tint` is the mottle colour and a byte that moves its patches about. An amount or contrast
/// of 0 means that part is off.
pub(super) fn pack_style(style: &FillStyle) -> ([u8; 4], [u8; 4]) {
    let mut look = [0; 4];
    let mut tint = [0; 4];
    if let Some(grain) = &style.grain {
        look[0] = unit_byte(grain.amount);
        look[1] = log_byte(grain.size, MIN_GRAIN_SIZE, MAX_GRAIN_SIZE);
    }
    if let Some(mottle) = &style.mottle {
        look[2] = unit_byte(mottle.contrast);
        look[3] = log_byte(mottle.scale, MIN_MOTTLE_SCALE, MAX_MOTTLE_SCALE);
        // Spread the seed over the byte, so nearby seeds do not look alike.
        let offset = mottle.seed.wrapping_mul(2_654_435_761) >> 24;
        tint = [mottle.color.r, mottle.color.g, mottle.color.b, offset as u8];
    }
    (look, tint)
}

/// Packs a fill's texture into a vertex attribute: the slot of its image (see
/// `fill_texture::slot_for`; 0 means none), the amount, and the scale on a log scale, as the
/// shader unpacks them. The fourth byte is unused.
pub(super) fn pack_texture(style: &FillStyle) -> [u8; 4] {
    let Some(texture) = &style.texture else {
        return [0; 4];
    };
    let slot = slot_for(&texture.asset);
    if slot == 0 || texture.amount <= 0.0 {
        return [0; 4];
    }
    [
        slot,
        unit_byte(texture.amount),
        log_byte(texture.scale, MIN_TEXTURE_SCALE, MAX_TEXTURE_SCALE),
        0,
    ]
}

/// A run of indices: either several groups that can't overlap themselves, drawn plainly,
/// or one group drawn once per sample.
pub(super) struct Draw {
    pub(super) indices: Range<u32>,
    pub(super) once: bool,
    pub(super) effect: bool,
    /// The Canvas area its triangles cover, as `[min_x, min_y, max_x, max_y]`.
    pub(super) bounds: [f32; 4],
}

/// Merged groups stop growing here, so one draw's bounds stay small enough to cull by.
pub(super) const MERGE_LIMIT: u32 = 30_000;

pub(super) const EMPTY_BOUNDS: [f32; 4] = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];

pub(super) fn union(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
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

/// Adds one shape's triangles. A path lyon cannot tessellate draws nothing rather than
/// failing the frame.
fn tessellate(
    shape: &VectorShape,
    geometry: &mut VertexBuffers<Vertex, u32>,
    fill: &mut FillTessellator,
    stroke: &mut StrokeTessellator,
) {
    let color = shape.color.to_linear_premultiplied();
    let (look, tint, texture) = match shape.paint {
        Paint::Fill(_) => {
            let (look, tint) = pack_style(&shape.style);
            (look, tint, pack_texture(&shape.style))
        }
        Paint::Stroke { .. } => ([0; 4], [0; 4], [0; 4]),
    };
    match shape.paint {
        Paint::Fill(rule) => {
            let options = FillOptions::tolerance(TOLERANCE).with_fill_rule(rule);
            let _ = fill.tessellate_path(
                &*shape.path,
                &options,
                &mut BuffersBuilder::new(geometry, |vertex: FillVertex| Vertex {
                    position: vertex.position().to_array(),
                    color,
                    look,
                    tint,
                    texture,
                }),
            );
        }
        Paint::Stroke { width } => {
            let options = StrokeOptions::tolerance(TOLERANCE)
                .with_line_width(width)
                .with_line_join(LineJoin::Round)
                .with_line_cap(LineCap::Round);
            let _ = stroke.tessellate_path(
                &*shape.path,
                &options,
                &mut BuffersBuilder::new(geometry, |vertex: StrokeVertex| Vertex {
                    position: vertex.position().to_array(),
                    color,
                    look,
                    tint,
                    texture,
                }),
            );
        }
    }
}

/// One group's triangles, with indices counted from its own first vertex.
pub(super) struct GroupMesh {
    pub(super) vertices: Vec<Vertex>,
    pub(super) indices: Vec<u32>,
    pub(super) once: bool,
    pub(super) effect: bool,
    pub(super) bounds: [f32; 4],
}

#[derive(Default)]
pub(super) struct Tessellators {
    fill: FillTessellator,
    stroke: StrokeTessellator,
}

pub(super) fn mesh_group(group: &VectorGroup, tessellators: &mut Tessellators) -> GroupMesh {
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
pub(super) struct Built {
    pub(super) geometry: VertexBuffers<Vertex, u32>,
    pub(super) draws: Vec<Draw>,
    pub(super) runs: Vec<Range<usize>>,
}

/// Merges the groups' triangles into draws, split into runs at `cuts` (see
/// `LyonRenderer::set_runs`). The triangles of an Object come from `cache`. While the cache
/// is being filled in the background, an Object that is not ready is left out, and appears in
/// a later frame; otherwise it is tessellated now and kept.
pub(super) fn build(groups: &[VectorGroup], cuts: &[usize], cache: &TessellationCache) -> Built {
    let mut tessellators = Tessellators::default();
    let meshes = resolve(groups, cache, &mut tessellators);
    merge(&meshes, cuts)
}

/// Merges the triangles of resolved groups into draws, split into runs at `cuts`.
pub(super) fn merge(meshes: &[Option<Arc<GroupMesh>>], cuts: &[usize]) -> Built {
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

/// Splits `built` into pieces whose vertex and index data each fit in `max_bytes`, so each
/// can be one GPU buffer (the device caps `max_buffer_size`). A piece has its own vertices,
/// indices counted from its first vertex, draws, and one range of draws per run of `built`
/// (empty if the piece has none of that run). A draw that alone is too big is left out.
pub(super) fn split(built: Built, max_bytes: u64) -> Vec<Built> {
    let max_vertices = (max_bytes / std::mem::size_of::<Vertex>() as u64).max(1);
    let max_indices = max_bytes / std::mem::size_of::<u32>() as u64;
    let fits = |vertices: usize, indices: usize| {
        vertices as u64 <= max_vertices && indices as u64 <= max_indices
    };
    if fits(built.geometry.vertices.len(), built.geometry.indices.len()) {
        return vec![built];
    }
    let Built {
        geometry,
        draws,
        runs,
    } = built;
    let mut run_of = vec![0; draws.len()];
    for (run, range) in runs.iter().enumerate() {
        run_of[range.clone()].fill(run);
    }
    // The vertices a draw uses, then greedily the draws that share a piece.
    let mut spans: Vec<(Range<usize>, Vec<usize>)> = Vec::new();
    for (position, draw) in draws.iter().enumerate() {
        let used = &geometry.indices[draw.indices.start as usize..draw.indices.end as usize];
        let (Some(&low), Some(&high)) = (used.iter().min(), used.iter().max()) else {
            continue;
        };
        let (low, high) = (low as usize, high as usize + 1);
        if !fits(high - low, used.len()) {
            continue;
        }
        if let Some((vertices, members)) = spans.last_mut() {
            let first = &draws[members[0]];
            if fits(
                high - vertices.start,
                (draw.indices.end - first.indices.start) as usize,
            ) {
                vertices.end = high;
                members.push(position);
                continue;
            }
        }
        spans.push((low..high, vec![position]));
    }
    spans
        .into_iter()
        .map(|(vertices, members)| {
            let first = draws[members[0]].indices.start;
            let last = draws[members[members.len() - 1]].indices.end;
            let mut piece_runs: Vec<Range<usize>> = vec![0..0; runs.len()];
            let mut seen = vec![false; runs.len()];
            let mut piece_draws = Vec::with_capacity(members.len());
            for (local, &position) in members.iter().enumerate() {
                let draw = &draws[position];
                let run = run_of[position];
                if !seen[run] {
                    seen[run] = true;
                    piece_runs[run].start = local;
                }
                piece_runs[run].end = local + 1;
                piece_draws.push(Draw {
                    indices: draw.indices.start - first..draw.indices.end - first,
                    once: draw.once,
                    effect: draw.effect,
                    bounds: draw.bounds,
                });
            }
            let base = vertices.start as u32;
            Built {
                geometry: VertexBuffers {
                    vertices: geometry.vertices[vertices].to_vec(),
                    indices: geometry.indices[first as usize..last as usize]
                        .iter()
                        .map(|&index| index - base)
                        .collect(),
                },
                draws: piece_draws,
                runs: piece_runs,
            }
        })
        .collect()
}

type ObjectMeshes = std::vec::IntoIter<Arc<GroupMesh>>;

/// The mesh of each group, `None` for the groups of an Object left out of the frame.
pub(super) fn resolve(
    groups: &[VectorGroup],
    cache: &TessellationCache,
    tessellators: &mut Tessellators,
) -> Vec<Option<Arc<GroupMesh>>> {
    let _stage = profile::stage(Stage::Tessellation);
    let mut by_object: HashMap<Id, Vec<&VectorGroup>> = HashMap::new();
    let mut objects: Vec<ObjectRef> = Vec::new();
    for group in groups {
        if let Some(object) = group.object {
            let sources = by_object.entry(object.id).or_default();
            if sources.is_empty() {
                objects.push(object);
            }
            sources.push(group);
            cache.note_version(object.id, object.version);
        }
    }
    let building = cache.is_building();
    // Every Object is looked up first, so the ones to tessellate are meshed together.
    let mut found: HashMap<Id, Option<ObjectMeshes>> = HashMap::new();
    let mut misses: Vec<(ObjectRef, Option<Previous>)> = Vec::new();
    for &object in &objects {
        if let Some(meshes) = cache.lookup(object, &by_object[&object.id]) {
            found.insert(object.id, Some(meshes.into_iter()));
        } else if building {
            found.insert(object.id, None);
        } else {
            misses.push((object, cache.previous(object.id)));
        }
    }
    let jobs: Vec<(&[&VectorGroup], Option<&Previous>)> = misses
        .iter()
        .map(|(object, previous)| (by_object[&object.id].as_slice(), previous.as_ref()))
        .collect();
    let meshed = mesh_objects(&jobs, tessellators);
    for ((object, _), meshes) in misses.iter().zip(meshed) {
        cache.store(*object, &by_object[&object.id], meshes.clone());
        found.insert(object.id, Some(meshes.into_iter()));
    }
    let out = groups
        .iter()
        .map(|group| {
            let Some(object) = group.object else {
                return Some(Arc::new(mesh_group(group, tessellators)));
            };
            found
                .get_mut(&object.id)
                .and_then(Option::as_mut)
                .and_then(Iterator::next)
        })
        .collect();
    if !building {
        cache.retain(|id| by_object.contains_key(&id));
    }
    out
}

/// Meshes the groups of each Object in `jobs` (see [`mesh_object`]), giving the same meshes in
/// the same order as meshing them one by one. Several Objects are meshed on a thread per core,
/// as when a Shore setting changes every Landmass's outline at once (#692); one is meshed here.
pub(super) fn mesh_objects(
    jobs: &[(&[&VectorGroup], Option<&Previous>)],
    tessellators: &mut Tessellators,
) -> Vec<Vec<Arc<GroupMesh>>> {
    let threads = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(jobs.len());
    if threads <= 1 {
        return jobs
            .iter()
            .map(|&(sources, previous)| mesh_object(sources, previous, tessellators))
            .collect();
    }
    let next = AtomicUsize::new(0);
    let mut meshed: Vec<Vec<Arc<GroupMesh>>> = vec![Vec::new(); jobs.len()];
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut tessellators = Tessellators::default();
                    let mut done = Vec::new();
                    loop {
                        let at = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&(sources, previous)) = jobs.get(at) else {
                            break;
                        };
                        done.push((at, mesh_object(sources, previous, &mut tessellators)));
                    }
                    done
                })
            })
            .collect();
        for worker in workers {
            match worker.join() {
                Ok(done) => {
                    for (at, meshes) in done {
                        meshed[at] = meshes;
                    }
                }
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
    });
    meshed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::vector::ObjectRef;
    use lyon::math::point;
    use lyon::path::Path;

    fn built_of(count: usize) -> Built {
        let mut tessellators = Tessellators::default();
        let groups: Vec<VectorGroup> = (0..count).map(|i| fill(i as f32 * 20.0, None)).collect();
        let meshes: Vec<_> = groups
            .iter()
            .map(|g| Some(Arc::new(mesh_group(g, &mut tessellators))))
            .collect();
        // A cut after every second group, and once-draws, so draws stay separate.
        merge(&meshes, &[2, 4])
    }

    #[test]
    fn split_keeps_everything_when_it_fits() {
        let built = built_of(6);
        let draws = built.draws.len();
        let pieces = split(built, u64::MAX);
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].draws.len(), draws);
    }

    #[test]
    fn split_pieces_fit_and_draw_the_same_triangles() {
        let whole = built_of(6);
        let all_vertices = whole.geometry.vertices.len();
        let all_indices = whole.geometry.indices.len();
        // Room for about two groups' vertices per piece.
        let max = (all_vertices / 3 * std::mem::size_of::<Vertex>()) as u64;
        let positions: Vec<Vec<[f32; 2]>> = {
            let g = &whole.geometry;
            whole
                .draws
                .iter()
                .map(|d| {
                    g.indices[d.indices.start as usize..d.indices.end as usize]
                        .iter()
                        .map(|&i| g.vertices[i as usize].position)
                        .collect()
                })
                .collect()
        };
        let runs = whole.runs.len();
        let pieces = split(whole, max);
        assert!(pieces.len() > 1);
        let mut drawn = Vec::new();
        let mut total_indices = 0;
        for piece in &pieces {
            assert!(piece.geometry.vertices.len() as u64 * 36 <= max);
            assert_eq!(piece.runs.len(), runs);
            total_indices += piece.geometry.indices.len();
            assert!(
                piece
                    .geometry
                    .indices
                    .iter()
                    .all(|&i| (i as usize) < piece.geometry.vertices.len())
            );
            for draw in &piece.draws {
                drawn.push(
                    piece.geometry.indices[draw.indices.start as usize..draw.indices.end as usize]
                        .iter()
                        .map(|&i| piece.geometry.vertices[i as usize].position)
                        .collect::<Vec<_>>(),
                );
            }
            // Every draw is in exactly one run.
            let in_runs: usize = piece.runs.iter().map(Range::len).sum();
            assert_eq!(in_runs, piece.draws.len());
        }
        assert_eq!(total_indices, all_indices);
        assert_eq!(drawn, positions);
    }

    #[test]
    fn split_leaves_out_a_draw_too_big_for_any_buffer() {
        let pieces = split(built_of(3), 36);
        assert!(pieces.is_empty());
    }

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
        let group = VectorGroup::fill(
            square(x),
            Color::rgb(10, 20, 30),
            lyon::tessellation::FillRule::EvenOdd,
        );
        match object {
            Some(object) => group.of_object(object),
            None => group,
        }
    }

    fn stroke(x: f32, object: ObjectRef) -> VectorGroup {
        VectorGroup {
            shapes: vec![VectorShape {
                path: Arc::new(square(x)),
                color: Color::rgb(200, 0, 0),
                paint: Paint::Stroke { width: 2.0 },
                style: FillStyle::FLAT,
            }],
            effect: false,
            object: Some(object),
            placement: crate::vector::Placement::Canvas,
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

    /// The corners of `mesh`'s triangles, in order.
    fn corners(mesh: &GroupMesh) -> Vec<[f32; 2]> {
        mesh.indices
            .iter()
            .map(|&i| mesh.vertices[i as usize].position)
            .collect()
    }

    #[test]
    fn objects_meshed_together_get_the_meshes_of_one_by_one() {
        let objects: Vec<ObjectRef> = (0..40).map(|_| object(1)).collect();
        let groups: Vec<VectorGroup> = objects
            .iter()
            .enumerate()
            .flat_map(|(i, &o)| {
                let x = 30.0 * i as f32;
                [stroke(x, o), fill(x, Some(o))]
            })
            .collect();
        let sources: Vec<Vec<&VectorGroup>> = objects
            .iter()
            .map(|o| groups.iter().filter(|g| g.object == Some(*o)).collect())
            .collect();
        let jobs: Vec<(&[&VectorGroup], Option<&Previous>)> =
            sources.iter().map(|s| (s.as_slice(), None)).collect();
        let together = mesh_objects(&jobs, &mut Tessellators::default());
        let mut tessellators = Tessellators::default();
        assert_eq!(together.len(), objects.len());
        for (job, meshes) in jobs.iter().zip(&together) {
            let alone = mesh_object(job.0, None, &mut tessellators);
            let corners_of = |meshes: &[Arc<GroupMesh>]| -> Vec<Vec<[f32; 2]>> {
                meshes.iter().map(|m| corners(m)).collect()
            };
            assert_eq!(corners_of(meshes), corners_of(&alone));
        }
        // And through `resolve`, which stores each in the cache.
        let cache = TessellationCache::new();
        let resolved = resolve(&groups, &cache, &mut Tessellators::default());
        assert_eq!(cache.len(), objects.len());
        for (group, mesh) in groups.iter().zip(&resolved) {
            let alone = mesh_group(group, &mut tessellators);
            assert_eq!(corners(mesh.as_ref().unwrap()), corners(&alone));
        }
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
    fn grain_and_mottle_reach_the_fill_vertices_and_only_those() {
        let style = FillStyle {
            grain: Some(ssmap_core::Grain {
                amount: 0.5,
                size: 4.0,
            }),
            mottle: Some(ssmap_core::Mottle {
                color: Color::rgb(10, 20, 30),
                scale: 100.0,
                contrast: 1.0,
                seed: 9,
            }),
            texture: None,
        };
        let object = object(1);
        let styled = fill(0.0, Some(object)).with_style(style.clone());
        let plain = fill(20.0, Some(object));
        let mut stroked = stroke(40.0, object);
        stroked.shapes[0].style = style.clone();
        let built = batched(&[styled, plain, stroked]);
        let (with, without): (Vec<&Vertex>, Vec<&Vertex>) =
            built.vertices.iter().partition(|v| v.position[0] < 15.0);
        let (look, tint) = pack_style(&style);
        assert!(with.iter().all(|v| v.look == look && v.tint == tint));
        assert_eq!(look[0], 128, "half the grain amount");
        assert_eq!(look[2], 255, "full contrast");
        assert_eq!(tint[..3], [10, 20, 30]);
        // Plain fills and strokes carry nothing, even if a stroke was given a style.
        assert!(without.iter().all(|v| v.look == [0; 4] && v.tint == [0; 4]));
    }

    #[test]
    fn sizes_and_scales_survive_packing_to_within_a_few_percent() {
        // The shader's decode: `min * pow(max / min, byte / 255)`.
        let decode = |byte: u8, min: f64, max: f64| min * (max / min).powf(f64::from(byte) / 255.0);
        for size in [0.25, 1.0, 2.0, 7.5, 16.0] {
            let style = FillStyle {
                grain: Some(ssmap_core::Grain { amount: 1.0, size }),
                mottle: None,
                texture: None,
            };
            let (look, _) = pack_style(&style);
            let back = decode(look[1], 0.25, 16.0);
            assert!(
                (back / size - 1.0).abs() < 0.02,
                "{size} came back as {back}"
            );
        }
        for scale in [10.0, 80.0, 150.0, 1200.0, 5000.0] {
            let style = FillStyle {
                grain: None,
                mottle: Some(ssmap_core::Mottle {
                    scale,
                    ..ssmap_core::Mottle::DEFAULT
                }),
                texture: None,
            };
            let (look, _) = pack_style(&style);
            let back = decode(look[3], 10.0, 5000.0);
            assert!(
                (back / scale - 1.0).abs() < 0.03,
                "{scale} came back as {back}"
            );
        }
    }

    #[test]
    fn a_flat_style_packs_to_nothing_and_each_part_is_off_alone() {
        assert_eq!(pack_style(&FillStyle::FLAT), ([0; 4], [0; 4]));
        let grain_only = FillStyle {
            grain: Some(ssmap_core::Grain::DEFAULT),
            mottle: None,
            texture: None,
        };
        let (look, tint) = pack_style(&grain_only);
        assert_eq!((look[2], look[3], tint), (0, 0, [0; 4]));
        let mottle_only = FillStyle {
            grain: None,
            mottle: Some(ssmap_core::Mottle::DEFAULT),
            texture: None,
        };
        let (look, _) = pack_style(&mottle_only);
        assert_eq!((look[0], look[1]), (0, 0));
    }

    #[test]
    fn nearby_mottle_seeds_pick_different_offsets() {
        let offset = |seed| {
            pack_style(&FillStyle {
                grain: None,
                mottle: Some(ssmap_core::Mottle {
                    seed,
                    ..ssmap_core::Mottle::DEFAULT
                }),
                texture: None,
            })
            .1[3]
        };
        assert_eq!(offset(5), offset(5));
        let offsets: std::collections::HashSet<u8> = (1..=8).map(offset).collect();
        assert!(offsets.len() >= 6, "{offsets:?}");
    }

    #[test]
    fn a_style_change_retessellates_even_at_the_same_version() {
        let a = object(1);
        let cache = TessellationCache::new();
        let flat = vec![fill(0.0, Some(a))];
        build(&flat, &[], &cache);
        assert!(cache.is_ready(a.id, 1, &flat));
        for style in [
            FillStyle {
                grain: Some(ssmap_core::Grain::DEFAULT),
                mottle: None,
                texture: None,
            },
            FillStyle {
                grain: None,
                mottle: Some(ssmap_core::Mottle::DEFAULT),
                texture: None,
            },
        ] {
            let styled = vec![fill(0.0, Some(a)).with_style(style)];
            assert!(!cache.is_ready(a.id, 1, &styled));
        }
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

    /// The meshes the cache keeps for Object `object` with these groups.
    fn kept(cache: &TessellationCache, groups: &[VectorGroup]) -> Vec<Arc<GroupMesh>> {
        let object = groups[0].object.expect("an Object's groups");
        let sources: Vec<&VectorGroup> = groups.iter().collect();
        cache.lookup(object, &sources).expect("tessellated")
    }

    /// A Landmass's fill and outline, the outline `width` wide, at `version`.
    fn landmass(id: Id, version: u64, x: f32, width: f32) -> Vec<VectorGroup> {
        let object = ObjectRef { id, version };
        let mut outline = stroke(0.0, object);
        outline.shapes[0].paint = Paint::Stroke { width };
        vec![fill(x, Some(object)), outline]
    }

    #[test]
    fn a_shore_edit_keeps_the_mesh_of_the_unchanged_fill() {
        let id = Id::new();
        let cache = TessellationCache::new();
        let before = landmass(id, 1, 0.0, 2.0);
        build(&before, &[], &cache);
        let old = kept(&cache, &before);
        // A new outline width, and a new version, as a Shore edit gives.
        let after = landmass(id, 2, 0.0, 6.0);
        let built = build(&after, &[], &cache);
        let new = kept(&cache, &after);
        assert!(
            Arc::ptr_eq(&old[0], &new[0]),
            "the fill is not tessellated again"
        );
        assert!(!Arc::ptr_eq(&old[1], &new[1]), "the outline is");
        assert_eq!(positions(&built.geometry), positions(&batched(&after)));
    }

    #[test]
    fn a_moved_fill_is_tessellated_again() {
        let id = Id::new();
        let cache = TessellationCache::new();
        let before = landmass(id, 1, 0.0, 2.0);
        build(&before, &[], &cache);
        let old = kept(&cache, &before);
        let after = landmass(id, 2, 5.0, 2.0);
        let built = build(&after, &[], &cache);
        let new = kept(&cache, &after);
        assert!(!Arc::ptr_eq(&old[0], &new[0]));
        // The outline is unchanged, so it keeps its mesh.
        assert!(Arc::ptr_eq(&old[1], &new[1]));
        assert_eq!(positions(&built.geometry), positions(&batched(&after)));
    }

    #[test]
    fn a_background_rebuild_also_keeps_unchanged_meshes() {
        let id = Id::new();
        let cache = TessellationCache::new();
        let before = landmass(id, 1, 0.0, 2.0);
        cache.rebuild(&before).run(|| {});
        let old = kept(&cache, &before);
        let after = landmass(id, 2, 0.0, 6.0);
        cache.rebuild(&after).run(|| {});
        let new = kept(&cache, &after);
        assert!(Arc::ptr_eq(&old[0], &new[0]));
        assert!(!Arc::ptr_eq(&old[1], &new[1]));
    }
}
