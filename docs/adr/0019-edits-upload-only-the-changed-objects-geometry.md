# Edits upload only the changed Objects' geometry

Status: proposed (builds on ADR 0009; touches ADR 0007). Issue #662, items (a) and (b).

## Context

On the full-scale generated Map (100k Stamps, 20 Landmasses, 220 vector groups), every editing frame takes 100–340 ms on an M1 Pro, against #245's budget of 16.6 ms. Profiling in [#662](https://github.com/hjrdave/StarScriptMapMaker/issues/662#issuecomment-7194796442) shows where an edit's frame goes:

- `CanvasRenderer::render` (`crates/render/src/canvas.rs`) calls `self.content.set_runs(...)` whenever the groups are not the very same ones (`!same_groups`), which is every edit.
- `set_runs` (`vector/renderer.rs`) calls `build` → `merge` (`vector/chunk.rs`). That copies every Object's cached mesh into one new vertex and index array and rebases every index: about 167 MB of vertices (4.6 M × 36 B) and 56 MB of indices. **About 23 ms.**
- It then makes two new buffers with `create_buffer_init`. wgpu-core zero-fills a 223 MB staging buffer and copies into it. **About 40 ms**, the "upload" stage.
- `unmap` queues a 223 MB GPU copy from staging into the real buffers, and the copy runs in the next `submit`. **About 25 ms**, measured as "draw". When panning, draw costs 1.4 ms on the same Map. Forcing `set_runs` with unchanged content on every pan frame brings draw to 25–28 ms, so the cost comes from the copy, not from the draw calls.

So an edit to one Landmass costs about 90 ms to move 223 MB that hasn't changed. The edited Object's own mesh is a few MB. Tessellation (items (c) and (e) of #662) is handled separately.

The renderer already has a second path. Since #142, `Chunked` (`vector/chunk.rs`) splits the groups into chunks of 64 (`CHUNK_GROUPS`), each with its own buffers (`ChunkMesh`), and rebuilds the stale ones within a 4 ms budget (`APPLY_BUDGET`). It is used only while background results land (`building || self.carry`). Edits never take it, and `Chunked::new` marks every chunk stale anyway, keeping old chunks only if their group range is the same.

Some facts every option has to keep:
- **Stacking order is the order of draw calls,** not the order of bytes in a buffer. `draw_draws` issues draws in list order, so geometry can live anywhere as long as the draw list follows the stacking order.
- **Runs** (`cuts`) separate Stamps, Clips, paint and board placement. Something else draws between runs, so no draw may span a cut.
- **Merged draws.** `merge` joins neighbouring groups into one draw (up to `MERGE_LIMIT` = 30,000 indices) unless a group may overlap itself (`once`, drawn twice through the stencil) or differs in `effect`. That keeps the draw count down on Maps with many small Objects such as Labels and Rivers. On this Map, 220 groups give about 220 draws, so merging saves little here, but it does on other Maps.
- **Tiled export** (ADR 0007) waits until everything is ready (`wait_until_ready`), then draws the same content once per tile. Any option works for export as long as the full content and draw order are what it draws. Export doesn't edit, so it never takes the partial path.
- **Device limits.** Since #673, buffer sizes must respect the device's `max_buffer_size` (`GpuCapabilities` now reports device limits). The app's device is opened with wgpu's default limits, whose `max_buffer_size` is 256 MiB, so one merged vertex buffer of 167 MB on this Map is already two-thirds of the limit. A Map about 1.5 times bigger would exceed it, whatever the GPU.

## Options

### A. Per-Object buffers

Each Object's groups get their own vertex and index buffers, made when its mesh changes. An edit uploads only that Object.

- **Upload:** only the edited Object, a few MB.
- **Draw calls:** at least one per Object. No merging across Objects, and a `set_vertex_buffer`/`set_index_buffer` pair per Object. Fine at 20 Landmasses, costly on a Map with thousands of Labels or Rivers.
- **Memory:** total as now, plus per-buffer overhead. No big transient copy.
- **Stacking order:** kept by drawing Objects in group order. An Object whose groups are not next to each other (the cache keeps them whole; see `resolve_chunk`) needs several draws.
- **Export:** unchanged.
- **Max buffer size:** no single large buffer, so this goes away.

### B. Use the existing chunks for edits

Let edits take the `Chunked` path: identify chunks by their content (the Objects and versions they hold), not by their group range, and mark stale only the chunks that hold a changed Object.

- **Upload:** the chunks holding the edited Object. A chunk is 64 groups, and groups are very uneven in size. On this Map, a chunk holding a large Landmass is still tens of MB (223 MB × 64 / 220 ≈ 65 MB on average). That is better than now, but still well over budget.
- **Draw calls:** as now plus one buffer bind per chunk; merging works within a chunk.
- **Memory:** as now.
- **Stacking order:** kept; chunks are built "giving the same triangles in the same order as merging everything at once", and there is already a test for that.
- **Shifting boundaries:** adding or removing one group moves every later chunk edge (chunks are cut every 64 groups by index), so a Shore change that adds rings can stale every chunk after it. Content-addressed chunks fix that, but then chunk sizes drift and need re-splitting.
- **Cost:** lowest; most of the code exists.

### C. One persistent buffer, rewritten from the first change

Keep one merged buffer, made with spare room. On an edit, `queue.write_buffer` from the first changed byte to the end.

- **Upload:** everything after the edited Object, so on average half the Map. Editing the bottom Layer still writes everything.
- **Stacking order and draws:** as now.
- **Memory:** as now plus the spare room.
- **Verdict:** this is simple, but it only halves the cost on average, and the worst case stays the same.

### D. Persistent buffers with sub-allocation, draw list in stacking order

Keep one large vertex buffer and one index buffer (or a few, to stay under `max_buffer_size`). Each Object's meshes get a range from a 1D allocator. An edit frees the Object's old range, allocates a new one, and fills it with `queue.write_buffer`. The draw list is rebuilt on the CPU in stacking order, pointing at the ranges.

- **Upload:** only the edited Object's bytes. `write_buffer` stages only what it writes, and there is no zero-fill of the whole buffer.
- **No index rebasing:** each group's indices stay relative to its own first vertex, and the draw passes it as `base_vertex` in `draw_indexed`. That removes the rebasing loop in `merge` (about 300 of the profiled samples).
- **No 223 MB CPU copy:** the cache already holds every mesh, and nothing needs one merged `Vec`.
- **Draw calls:** neighbouring groups can still merge when their ranges are next to each other in the buffer and in stacking order. After an edit, only the draws around the edited Object split, which costs a few extra draws until the next compaction.
- **Memory:** total plus spare room (say 25%) and holes left by freed ranges. Compaction rewrites the buffer in stacking order when holes pass a threshold, or when the buffer must grow. Growing makes a new buffer and copies the old one on the GPU (`copy_buffer_to_buffer`, with no zero-fill), so it's rare and cheap compared with now.
- **Stacking order:** it lives in the draw list, not in buffer order, so editing never moves other Objects.
- **Export:** it draws from the same buffers. Compacting before an export would give the fewest draws, though exports don't need it.
- **Cost:** highest. It needs an allocator (the `offset-allocator` crate or our own, about 200 lines), compaction, growth, and tests that the draw list matches what `merge` gives today.

## Recommendation

**D**, built in two steps:

1. **Content first:** `LyonRenderer::set_runs` for `self.content` uses the sub-allocated buffers. `masks` (Clip stencil) and `paint` keep `set_runs` for now. They cost nothing on this Map (no Clips, no paint), and they can move later the same way.
2. **Replace `Chunked`:** apply background results through the same path, since landing a background result is just an edit to the Objects it finished. Then remove `Chunked`, so there is one path to keep correct.

Why not B: it reuses the most code, but its upload is the size of a chunk, not of an Object, and on Maps with a few large Landmasses that stays over budget. Why not A: it does what D does for uploads, but the draw count grows with the Object count, which is a problem on Maps with many Labels and Rivers. Why not C: it only halves the cost.

D also stops one buffer from growing with the whole Map, so that the 256 MiB `max_buffer_size` of the app's device is no longer a ceiling on Map size (the same kind of device-limit mismatch as #670).

## Measuring success

The editing scripts of `--frame-time` on the full-scale Map (#245) on the M1 Pro: the upload stage below 2 ms and draw back near pan-zoom's 1.4 ms. Tessellation (#662 (c) and (e)) is separate and is the next limit. The tiled-export tests (`crates/render/tests/tiled_export.rs`) and the golden images must not change.

## Consequences

- An edit's upload cost follows the size of what changed, not the size of the Map.
- The draw list has to be kept in stacking order separately from the buffer layout. A test compares it with what today's `merge` gives for the same groups.
- Memory use is slightly higher at peak (spare room and holes), and there's no transient 223 MB copy on each edit.
- `Chunked` goes away in step 2. Until then, there are two paths.
