//! Group meshes kept in persistent GPU buffers between frames, so an edit uploads only the
//! meshes that changed (ADR 0019).
//!
//! Each mesh gets a range of vertices and a range of indices in one page (a vertex buffer and
//! an index buffer, each under the device's `max_buffer_size`). Its indices are stored counted
//! from the page's first vertex, so the meshes of neighbouring groups that sit next to each
//! other in a page still merge into one draw, as `chunk::merge` merges them. Stacking
//! order lives in the draw list, which is rebuilt on every update; the buffer layout follows it
//! only until an edit moves something.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use super::chunk::{Draw, GroupMesh, MERGE_LIMIT, Vertex, union};

/// Space left over when the pages are laid out again, as a fraction of what is in them, so the
/// next edits fit without another layout.
const SPARE: (u64, u64) = (1, 2);

/// The smallest page, in vertices and indices, so a small Map does not lay out again on every
/// edit that grows it a little.
const MIN_PAGE: (u32, u32) = (4096, 16_384);

/// Free ranges of a page's vertices or indices, kept sorted and merged with their neighbours.
#[derive(Debug)]
pub(super) struct Allocator {
    capacity: u32,
    free: Vec<Range<u32>>,
}

impl Allocator {
    pub(super) fn new(capacity: u32) -> Self {
        Self {
            capacity,
            free: std::iter::once(0..capacity)
                .filter(|range| range.start < range.end)
                .collect(),
        }
    }

    /// The first free range that holds `len`, or `None` if none does.
    pub(super) fn allocate(&mut self, len: u32) -> Option<Range<u32>> {
        let at = self
            .free
            .iter()
            .position(|range| range.len() >= len as usize)?;
        let range = &mut self.free[at];
        let taken = range.start..range.start + len;
        range.start += len;
        if range.start == range.end {
            self.free.remove(at);
        }
        Some(taken)
    }

    /// Gives `range` back, merging it with the free ranges either side.
    pub(super) fn release(&mut self, range: Range<u32>) {
        if range.start >= range.end {
            return;
        }
        let at = self.free.partition_point(|free| free.start < range.start);
        let joins_before = at > 0 && self.free[at - 1].end == range.start;
        let joins_after = at < self.free.len() && self.free[at].start == range.end;
        match (joins_before, joins_after) {
            (true, true) => {
                self.free[at - 1].end = self.free[at].end;
                self.free.remove(at);
            }
            (true, false) => self.free[at - 1].end = range.end,
            (false, true) => self.free[at].start = range.start,
            (false, false) => self.free.insert(at, range),
        }
    }

    pub(super) fn capacity(&self) -> u32 {
        self.capacity
    }
}

/// Where a page's buffers live: the GPU in the renderer, plain arrays in tests.
pub(super) trait Pages {
    /// Drops every page and makes new, empty ones of these sizes (vertices, indices).
    fn reset(&mut self, sizes: &[(u32, u32)]);
    /// Writes `vertices` from vertex `vertex_at` and `indices` from index `index_at` of `page`.
    fn write(
        &mut self,
        page: usize,
        vertex_at: u32,
        vertices: &[Vertex],
        index_at: u32,
        indices: &[u32],
    );
}

/// One mesh's place in a page. It holds the mesh, so the address it is found by is never
/// reused by another mesh while it is here.
struct Slot {
    _mesh: Arc<GroupMesh>,
    page: usize,
    vertices: Range<u32>,
    indices: Range<u32>,
}

struct Page {
    vertices: Allocator,
    indices: Allocator,
}

/// Draws that read one page, which is bound once for all of them.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Segment {
    pub(super) page: usize,
    pub(super) draws: Range<usize>,
}

/// Every mesh drawn, where it sits in the pages, and the draws in stacking order.
pub(super) struct Resident {
    /// The most vertices and indices one page may hold (the device's `max_buffer_size`).
    max: (u32, u32),
    pages: Vec<Page>,
    /// Found by the mesh's address (see [`key`]).
    slots: HashMap<usize, Slot>,
    pub(super) draws: Vec<Draw>,
    pub(super) segments: Vec<Segment>,
    /// Ranges of `segments`, one per run (see `LyonRenderer::set_runs`).
    pub(super) runs: Vec<Range<usize>>,
}

/// The most elements of `size` bytes a buffer of `max_bytes` holds, as a `u32`.
fn elements(max_bytes: u64, size: usize) -> u32 {
    (max_bytes / size as u64).min(u64::from(u32::MAX)) as u32
}

/// A mesh's address, which its [`Slot`] keeps from being reused while the mesh is there.
fn key(mesh: &Arc<GroupMesh>) -> usize {
    Arc::as_ptr(mesh) as usize
}

impl Resident {
    /// No pages yet; each buffer will stay under `max_buffer_size` bytes.
    pub(super) fn new(max_buffer_size: u64) -> Self {
        Self {
            max: (
                elements(max_buffer_size, std::mem::size_of::<Vertex>()),
                elements(max_buffer_size, std::mem::size_of::<u32>()),
            ),
            pages: Vec::new(),
            slots: HashMap::new(),
            draws: Vec::new(),
            segments: Vec::new(),
            runs: Vec::new(),
        }
    }

    /// Whether a mesh this size fits in a page at all. One that doesn't is left out, as the
    /// device could not hold it in any buffer.
    fn fits_a_page(&self, mesh: &GroupMesh) -> bool {
        mesh.vertices.len() as u64 <= u64::from(self.max.0)
            && mesh.indices.len() as u64 <= u64::from(self.max.1)
    }

    /// Draws `meshes` in order, split into runs at `cuts` (as `merge` does), writing to `pages`
    /// only the meshes that are not there yet. Meshes no longer drawn give their space back.
    /// Returns how many vertices were written.
    pub(super) fn update(
        &mut self,
        meshes: &[Option<Arc<GroupMesh>>],
        cuts: &[usize],
        pages: &mut impl Pages,
    ) -> usize {
        let drawn: Vec<&Arc<GroupMesh>> = meshes
            .iter()
            .flatten()
            .filter(|mesh| !mesh.indices.is_empty() && self.fits_a_page(mesh))
            .collect();
        let wanted: HashSet<usize> = drawn.iter().map(|&mesh| key(mesh)).collect();
        let gone: Vec<usize> = self
            .slots
            .keys()
            .filter(|key| !wanted.contains(key))
            .copied()
            .collect();
        for key in gone {
            if let Some(slot) = self.slots.remove(&key) {
                let page = &mut self.pages[slot.page];
                page.vertices.release(slot.vertices);
                page.indices.release(slot.indices);
            }
        }
        let mut written = 0;
        let mut placed = true;
        for &mesh in &drawn {
            if self.slots.contains_key(&key(mesh)) {
                continue;
            }
            match self.place(mesh, pages) {
                Some(count) => written += count,
                None => {
                    placed = false;
                    break;
                }
            }
        }
        if !placed || self.mostly_empty() {
            written = self.lay_out(&drawn, pages);
        }
        self.list_draws(meshes, cuts);
        written
    }

    /// Finds room for `mesh` in some page and writes it there. `None` if no page has room.
    fn place(&mut self, mesh: &Arc<GroupMesh>, pages: &mut impl Pages) -> Option<usize> {
        let (vertices, indices) = (mesh.vertices.len() as u32, mesh.indices.len() as u32);
        for (index, page) in self.pages.iter_mut().enumerate() {
            let Some(vertex_range) = page.vertices.allocate(vertices) else {
                continue;
            };
            let Some(index_range) = page.indices.allocate(indices) else {
                page.vertices.release(vertex_range);
                continue;
            };
            write(pages, index, mesh, &vertex_range, &index_range);
            self.slots.insert(
                key(mesh),
                Slot {
                    _mesh: mesh.clone(),
                    page: index,
                    vertices: vertex_range,
                    indices: index_range,
                },
            );
            return Some(vertices as usize);
        }
        None
    }

    /// Whether the pages hold under a quarter of their room, so laying them out again (with
    /// the usual spare room) gives memory back.
    fn mostly_empty(&self) -> bool {
        let capacity: u64 = self
            .pages
            .iter()
            .map(|page| u64::from(page.vertices.capacity()))
            .sum();
        let used: u64 = self
            .slots
            .values()
            .map(|slot| slot.vertices.len() as u64)
            .sum();
        capacity > u64::from(MIN_PAGE.0) && used * 4 < capacity
    }

    /// Makes new pages for `drawn`, with spare room, and writes every mesh into them in
    /// stacking order, so neighbouring groups sit next to each other again and merge.
    fn lay_out(&mut self, drawn: &[&Arc<GroupMesh>], pages: &mut impl Pages) -> usize {
        self.slots.clear();
        let mut unique: Vec<&Arc<GroupMesh>> = Vec::with_capacity(drawn.len());
        let mut seen = HashSet::new();
        for &mesh in drawn {
            if seen.insert(key(mesh)) {
                unique.push(mesh);
            }
        }
        let total = |count: fn(&GroupMesh) -> usize| -> u64 {
            unique.iter().map(|mesh| count(mesh) as u64).sum()
        };
        let with_spare = |n: u64| n + n * SPARE.0 / SPARE.1;
        let want = (
            with_spare(total(|m| m.vertices.len())),
            with_spare(total(|m| m.indices.len())),
        );
        // Every page is filled to leave the same share of spare room.
        let page_size = (
            want.0.max(u64::from(MIN_PAGE.0)).min(u64::from(self.max.0)) as u32,
            want.1.max(u64::from(MIN_PAGE.1)).min(u64::from(self.max.1)) as u32,
        );
        let one_page = want.0 <= u64::from(self.max.0) && want.1 <= u64::from(self.max.1);
        let fill = if one_page {
            (u64::MAX, u64::MAX)
        } else {
            (
                (u64::from(page_size.0) * SPARE.1 / (SPARE.0 + SPARE.1)).max(1),
                (u64::from(page_size.1) * SPARE.1 / (SPARE.0 + SPARE.1)).max(1),
            )
        };
        // Which page each mesh goes in.
        let mut assigned: Vec<usize> = Vec::with_capacity(unique.len());
        let mut used = (0u64, 0u64);
        let mut count = 1;
        for mesh in &unique {
            let size = (mesh.vertices.len() as u64, mesh.indices.len() as u64);
            if used != (0, 0) && (used.0 + size.0 > fill.0 || used.1 + size.1 > fill.1) {
                count += 1;
                used = (0, 0);
            }
            used = (used.0 + size.0, used.1 + size.1);
            assigned.push(count - 1);
        }
        let sizes = vec![page_size; count];
        pages.reset(&sizes);
        self.pages = sizes
            .iter()
            .map(|&(vertices, indices)| Page {
                vertices: Allocator::new(vertices),
                indices: Allocator::new(indices),
            })
            .collect();
        let mut written = 0;
        for (mesh, page) in unique.into_iter().zip(assigned) {
            let target = &mut self.pages[page];
            let (Some(vertex_range), Some(index_range)) = (
                target.vertices.allocate(mesh.vertices.len() as u32),
                target.indices.allocate(mesh.indices.len() as u32),
            ) else {
                continue;
            };
            write(pages, page, mesh, &vertex_range, &index_range);
            written += mesh.vertices.len();
            self.slots.insert(
                key(mesh),
                Slot {
                    _mesh: mesh.clone(),
                    page,
                    vertices: vertex_range,
                    indices: index_range,
                },
            );
        }
        written
    }

    /// The draws for `meshes` in stacking order, merging neighbours as `merge` does when they
    /// also sit next to each other in one page.
    fn list_draws(&mut self, meshes: &[Option<Arc<GroupMesh>>], cuts: &[usize]) {
        self.draws.clear();
        self.segments.clear();
        self.runs.clear();
        let mut run_start = 0;
        let mut remaining_cuts = cuts.iter().peekable();
        let mut split = false;
        for (position, mesh) in meshes.iter().enumerate() {
            while remaining_cuts.next_if(|&&cut| cut <= position).is_some() {
                self.runs.push(run_start..self.segments.len());
                run_start = self.segments.len();
                split = true;
            }
            let Some(mesh) = mesh else { continue };
            let Some(slot) = self.slots.get(&key(mesh)) else {
                continue;
            };
            let same_page = !split
                && self
                    .segments
                    .last()
                    .is_some_and(|segment| segment.page == slot.page);
            let next = self.draws.len();
            match self.draws.last_mut() {
                Some(last)
                    if same_page
                        && !mesh.once
                        && !last.once
                        && last.effect == mesh.effect
                        && last.indices.end == slot.indices.start
                        && slot.indices.end - last.indices.start <= MERGE_LIMIT =>
                {
                    last.indices.end = slot.indices.end;
                    last.bounds = union(last.bounds, mesh.bounds);
                }
                _ => {
                    self.draws.push(Draw {
                        indices: slot.indices.clone(),
                        once: mesh.once,
                        effect: mesh.effect,
                        bounds: mesh.bounds,
                    });
                    match self.segments.last_mut() {
                        Some(segment) if same_page => segment.draws.end = next + 1,
                        _ => self.segments.push(Segment {
                            page: slot.page,
                            draws: next..next + 1,
                        }),
                    }
                }
            }
            split = false;
        }
        for _ in remaining_cuts {
            self.runs.push(run_start..self.segments.len());
            run_start = self.segments.len();
        }
        self.runs.push(run_start..self.segments.len());
    }

    /// How many pages there are.
    #[cfg(test)]
    pub(super) fn page_count(&self) -> usize {
        self.pages.len()
    }
}

/// Writes `mesh` at its ranges, its indices counted from the page's first vertex.
fn write(
    pages: &mut impl Pages,
    page: usize,
    mesh: &GroupMesh,
    vertices: &Range<u32>,
    indices: &Range<u32>,
) {
    let base = vertices.start;
    let rebased: Vec<u32> = mesh.indices.iter().map(|&index| index + base).collect();
    pages.write(page, base, &mesh.vertices, indices.start, &rebased);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::vector::VectorGroup;
    use crate::vector::chunk::{Tessellators, merge, mesh_group};
    use lyon::math::point;
    use lyon::path::Path;

    /// Pages as plain arrays, counting what is written.
    #[derive(Default)]
    struct Arrays {
        pages: Vec<(Vec<Vertex>, Vec<u32>)>,
        resets: usize,
    }

    impl Pages for Arrays {
        fn reset(&mut self, sizes: &[(u32, u32)]) {
            self.resets += 1;
            self.pages = sizes
                .iter()
                .map(|&(v, i)| {
                    (
                        vec![bytemuck::Zeroable::zeroed(); v as usize],
                        vec![u32::MAX; i as usize],
                    )
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
            let (v, i) = &mut self.pages[page];
            v[vertex_at as usize..vertex_at as usize + vertices.len()].copy_from_slice(vertices);
            i[index_at as usize..index_at as usize + indices.len()].copy_from_slice(indices);
        }
    }

    /// The corners `run` draws, in draw order, read back from the pages.
    fn drawn(resident: &Resident, arrays: &Arrays, run: usize) -> Vec<[f32; 2]> {
        resident.segments[resident.runs[run].clone()]
            .iter()
            .flat_map(|segment| {
                let (vertices, indices) = &arrays.pages[segment.page];
                resident.draws[segment.draws.clone()]
                    .iter()
                    .flat_map(|draw| draw.indices.clone())
                    .map(|i| vertices[indices[i as usize] as usize].position)
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// The corners `merge` gives for run `run` of the same meshes.
    fn merged(meshes: &[Option<Arc<GroupMesh>>], cuts: &[usize], run: usize) -> Vec<[f32; 2]> {
        let built = merge(meshes, cuts);
        built.draws[built.runs[run].clone()]
            .iter()
            .flat_map(|draw| draw.indices.clone())
            .map(|i| built.geometry.vertices[built.geometry.indices[i as usize] as usize].position)
            .collect()
    }

    fn square(x: f32, size: f32) -> VectorGroup {
        let mut builder = Path::builder();
        builder.begin(point(x, 0.0));
        builder.line_to(point(x + size, 0.0));
        builder.line_to(point(x + size, size));
        builder.line_to(point(x, size));
        builder.close();
        VectorGroup::fill(
            builder.build(),
            Color::rgb(10, 20, 30),
            lyon::tessellation::FillRule::EvenOdd,
        )
    }

    /// A many-sided shape, so its mesh is bigger than a square's.
    fn blob(x: f32, sides: usize) -> VectorGroup {
        let mut builder = Path::builder();
        for k in 0..sides {
            let angle = k as f32 / sides as f32 * std::f32::consts::TAU;
            let p = point(x + 10.0 * angle.cos(), 10.0 * angle.sin());
            if k == 0 {
                builder.begin(p);
            } else {
                builder.line_to(p);
            }
        }
        builder.close();
        VectorGroup::fill(
            builder.build(),
            Color::rgb(1, 2, 3),
            lyon::tessellation::FillRule::EvenOdd,
        )
    }

    fn meshes_of(groups: &[VectorGroup]) -> Vec<Option<Arc<GroupMesh>>> {
        let mut tessellators = Tessellators::default();
        groups
            .iter()
            .map(|group| Some(Arc::new(mesh_group(group, &mut tessellators))))
            .collect()
    }

    fn check_every_run(
        resident: &Resident,
        arrays: &Arrays,
        meshes: &[Option<Arc<GroupMesh>>],
        cuts: &[usize],
    ) {
        assert_eq!(resident.runs.len(), cuts.len() + 1);
        for run in 0..=cuts.len() {
            assert_eq!(
                drawn(resident, arrays, run),
                merged(meshes, cuts, run),
                "run {run}"
            );
        }
    }

    #[test]
    fn the_allocator_reuses_and_merges_freed_ranges() {
        let mut allocator = Allocator::new(9);
        let a = allocator.allocate(3).unwrap();
        let b = allocator.allocate(3).unwrap();
        let c = allocator.allocate(3).unwrap();
        assert_eq!((a.clone(), b.clone(), c.clone()), (0..3, 3..6, 6..9));
        assert_eq!(allocator.allocate(1), None, "full");
        allocator.release(a);
        allocator.release(c);
        assert_eq!(allocator.allocate(4), None, "the holes are apart");
        allocator.release(b);
        assert_eq!(allocator.free.len(), 1, "all merged back");
        assert_eq!(allocator.free[0], 0..9);
        assert_eq!(allocator.allocate(9), Some(0..9));
    }

    #[test]
    fn a_first_update_draws_what_merge_draws() {
        let groups: Vec<VectorGroup> = (0..12).map(|i| square(i as f32 * 20.0, 10.0)).collect();
        let meshes = meshes_of(&groups);
        let cuts = [3, 3, 8];
        let mut resident = Resident::new(u64::MAX);
        let mut arrays = Arrays::default();
        resident.update(&meshes, &cuts, &mut arrays);
        check_every_run(&resident, &arrays, &meshes, &cuts);
        // Laid out in order on one page, so neighbours merge into as few draws as `merge`.
        assert_eq!(resident.draws.len(), merge(&meshes, &cuts).draws.len());
    }

    #[test]
    fn an_edit_writes_only_the_changed_mesh_and_keeps_stacking_order() {
        let groups: Vec<VectorGroup> = (0..20).map(|i| square(i as f32 * 20.0, 10.0)).collect();
        let mut meshes = meshes_of(&groups);
        let cuts = [10];
        let mut resident = Resident::new(u64::MAX);
        let mut arrays = Arrays::default();
        resident.update(&meshes, &cuts, &mut arrays);
        // Group 4 is edited into a bigger shape, which no longer fits its old place.
        let edited = meshes_of(&[blob(80.0, 40)]).remove(0);
        let size = edited.as_ref().unwrap().vertices.len();
        meshes[4] = edited;
        let written = resident.update(&meshes, &cuts, &mut arrays);
        assert_eq!(written, size, "only the edited mesh is written");
        assert_eq!(arrays.resets, 1, "the spare room took it");
        check_every_run(&resident, &arrays, &meshes, &cuts);
        // The edited group now sits apart from its neighbours, so its draw splits from theirs.
        assert!(resident.draws.len() > merge(&meshes, &cuts).draws.len());
        // Nothing changed: nothing is written.
        assert_eq!(resident.update(&meshes, &cuts, &mut arrays), 0);
        check_every_run(&resident, &arrays, &meshes, &cuts);
    }

    #[test]
    fn reordering_groups_moves_draws_not_bytes() {
        let groups: Vec<VectorGroup> = (0..6).map(|i| square(i as f32 * 20.0, 10.0)).collect();
        let mut meshes = meshes_of(&groups);
        let mut resident = Resident::new(u64::MAX);
        let mut arrays = Arrays::default();
        resident.update(&meshes, &[], &mut arrays);
        // A Layer moved up: the same meshes in another stacking order.
        meshes.swap(1, 4);
        assert_eq!(resident.update(&meshes, &[], &mut arrays), 0);
        check_every_run(&resident, &arrays, &meshes, &[]);
    }

    #[test]
    fn running_out_of_room_lays_everything_out_again() {
        let groups: Vec<VectorGroup> = (0..4).map(|i| square(i as f32 * 20.0, 10.0)).collect();
        let mut meshes = meshes_of(&groups);
        let mut resident = Resident::new(u64::MAX);
        let mut arrays = Arrays::default();
        resident.update(&meshes, &[], &mut arrays);
        // Far more than the spare room: every group becomes a big blob.
        let blobs: Vec<VectorGroup> = (0..4).map(|i| blob(i as f32 * 30.0, 4000)).collect();
        meshes = meshes_of(&blobs);
        let total: usize = meshes.iter().flatten().map(|m| m.vertices.len()).sum();
        let written = resident.update(&meshes, &[], &mut arrays);
        assert_eq!(arrays.resets, 2);
        assert_eq!(written, total);
        check_every_run(&resident, &arrays, &meshes, &[]);
        assert_eq!(
            resident.draws.len(),
            merge(&meshes, &[]).draws.len(),
            "laid out in order again, so it merges as before"
        );
    }

    #[test]
    fn pages_stay_under_the_buffer_limit() {
        let groups: Vec<VectorGroup> = (0..30).map(|i| blob(i as f32 * 30.0, 64)).collect();
        let meshes = meshes_of(&groups);
        let biggest = meshes
            .iter()
            .flatten()
            .map(|m| m.indices.len() * 4)
            .max()
            .unwrap();
        // Room for a few meshes per buffer, but above the smallest page.
        let limit = (biggest * 8).max(MIN_PAGE.1 as usize * 4) as u64;
        let mut resident = Resident::new(limit);
        let mut arrays = Arrays::default();
        resident.update(&meshes, &[7], &mut arrays);
        assert!(resident.page_count() > 1);
        for (vertices, indices) in &arrays.pages {
            assert!(vertices.len() as u64 * 36 <= limit);
            assert!(indices.len() as u64 * 4 <= limit);
        }
        check_every_run(&resident, &arrays, &meshes, &[7]);
    }

    #[test]
    fn a_mesh_too_big_for_any_buffer_is_left_out() {
        let meshes = meshes_of(&[square(0.0, 10.0), blob(30.0, 4000), square(60.0, 10.0)]);
        let mut resident = Resident::new(1024 * 36);
        let mut arrays = Arrays::default();
        resident.update(&meshes, &[], &mut arrays);
        let small: Vec<_> = vec![meshes[0].clone(), None, meshes[2].clone()];
        assert_eq!(drawn(&resident, &arrays, 0), merged(&small, &[], 0));
    }

    #[test]
    fn removing_most_of_the_content_gives_memory_back() {
        let groups: Vec<VectorGroup> = (0..40).map(|i| blob(i as f32 * 30.0, 400)).collect();
        let meshes = meshes_of(&groups);
        let mut resident = Resident::new(u64::MAX);
        let mut arrays = Arrays::default();
        resident.update(&meshes, &[], &mut arrays);
        let before = arrays.pages[0].0.len();
        let few = meshes[..2].to_vec();
        resident.update(&few, &[], &mut arrays);
        assert_eq!(arrays.resets, 2);
        assert!(arrays.pages[0].0.len() < before / 4);
        check_every_run(&resident, &arrays, &few, &[]);
    }
}
