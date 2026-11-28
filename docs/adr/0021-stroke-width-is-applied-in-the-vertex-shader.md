# Stroke width is applied in the vertex shader

Status: proposed (builds on ADR 0009 and ADR 0019). Issue #692. Waiting for the owner.

## Context

Dragging a Shore slider changes the width of every Landmass outline on each frame. On the full-scale generated Map (20 Landmasses, 50k outline points each, about 3 M outline vertices), #701 made tessellation parallel, but each frame still tessellates, uploads and copies every outline again. Measured with `--frame-time` on an M1 Pro (Metal), release build with `--features profiling`, times in ms (from [#692](https://github.com/hjrdave/StarScriptMapMaker/issues/692#issuecomment-7240843690)):

| drag | frame p50 | frame p95 | tessellation | upload | draw |
| --- | --- | --- | --- | --- | --- |
| outline width only | 85–87 | 200–490 | 41–42 | 24 | 11 |
| `slider-shore` (outline width, ripple spacing, shading width) | 97–103 | 133–168 | 34–43 | 37–38 | 13 |

The budget is 16.6 ms at p95 (#245). ADR 0019 already makes an edit upload only the meshes that changed. The trouble is that a width change really does change every outline mesh, because lyon bakes the width into each vertex: `position = position_on_path + normal × width / 2` (`StrokeVertex::position` in lyon_tessellation 1.0).

Facts any option has to keep:
- **lyon's joins depend on the width.** It folds inner joins and subdivides round joins according to the width. #701 tested moving a mesh's vertices to a new width. Narrowing matches a fresh tessellation (20 → 10: worst vertex outside the stroke 1.06, the same as fresh). Widening does not: 2 → 4 puts inner-join vertices 4.04 units outside, against 0.07 for a fresh tessellation.
- **Merged draws** (ADR 0019): neighbouring groups in a page share one draw. A per-draw value would split those draws. On Maps with many Labels and Rivers, that is the draw count ADR 0019 chose D to avoid.
- **Export** (ADR 0007) draws the same content as the editor, tile by tile, so whatever gives the width has to be ready there too.
- **The vertex is 36 bytes** (`Vertex` in `vector/chunk.rs`): position, colour, and three 4-byte attributes (`look`, `tint`, `texture`) that only fills use. A stroke vertex leaves those 12 bytes at zero.

## Options

### A. Rescale the cached mesh while the slider is held, tessellate on release

While a slider is held, an Object whose groups differ only in stroke widths gets its last mesh moved to the new widths on the CPU (`on_path + normal × width / 2`). When the slider is let go, the next frame tessellates exactly those Objects once. This was prototyped on branch `claude/issue-692-rescale` and measured:

| outline width only | frame p50 | frame p95 | tessellation | upload | draw | release frame |
| --- | --- | --- | --- | --- | --- | --- |
| today | 85–87 | 200–490 | 41–42 | 24 | 11 | – |
| A | 54–56 | 60–61 | 13 | 21 | 11–12 | 41 |

- **Removes the tessellation** (41 → 13 ms) and its long tail.
- **Leaves the upload and the GPU copy** (about 21 + 12 ms). Moved vertices are still new bytes. Frames stay around 55 ms, about 18 fps.
- **Shows a wrong shape while held:** widening puts sharp inner joins up to a few units outside the stroke. The owner accepted that for a preview.
- **Costs** 16 bytes a vertex of CPU memory for the outlines being dragged, a preview flag on `TessellationCache`, and the app has to say when a slider is held and let go. Recording the extra data on every tessellation, not only while held, cost 10–17 ms at p50 on every outline edit. The prototype records it only while held.

### B. Stroke width in the vertex shader

A stroke vertex carries its place on the path and its normal instead of its final position. The vertex shader computes `position = on_path + normal × width / 2`, with the width read from a small storage buffer of widths, indexed per vertex.

- **Tessellate at the widest width in a range, draw narrower.** Narrowing is exact (see Context), so a mesh tessellated at width `W` draws correctly at any width from `W / k` to `W`. A change within that range writes one 4-byte width and nothing else. Leaving the range tessellates the outline again at the new range's top, which costs the same as one frame today. `k` is chosen by a test that compares narrowed meshes with fresh ones; #701's numbers support `k = 2`.
- **No bigger vertex.** A stroke's `look` holds the normal as two f16 values, and its `tint` holds the index of its width. Both are zero today. A flag in `texture`'s unused fourth byte tells the shader the vertex is a stroke. Fills don't change.
- **Merging is kept.** The width is looked up per vertex, so neighbouring groups with different widths still share one draw.
- **Upper bound, measured:** a build whose held frames keep the old outline mesh as it is (what B gives while the width stays in range) runs the outline-width drag at **p50 27 ms, p95 29–30 ms**: upload 0.5–0.6 ms, draw 4.6–4.7 ms. On `slider-shore`, which also changes ripple spacing and shading width (their paths change, so they re-tessellate), it gives p50 37–38 ms.
- **Costs:** a stroke variant of the vector shader's vertex stage, the width buffer and its slots (kept by `Resident` next to the mesh ranges), and tessellating outlines wider than drawn. That gives about 10–35% more vertices for those outlines, from round joins subdivided for the wider width. Export writes the same width buffer before drawing its tiles.
- **Precision:** an f16 normal has about 3 significant digits, so at a width of 20 a vertex can move about 0.02 Canvas units. That is under the flattening tolerance (`TOLERANCE` = 0.05). The golden images may shift by a fraction of a pixel and have to be looked at on a software GPU.

### C. Throttle the slider, or use a coarser tolerance while dragging

These were in #692. Both change what a drag shows, and neither removes the upload: a throttled drag still uploads 3 M vertices on each frame it draws, and a coarser tolerance only shrinks the meshes. Not measured.

## Recommendation

**B.** A does not make the drag smooth. It removes the tessellation, but the upload and the GPU copy it leaves (about 33 ms) are what B removes, and with B nothing has to be re-tessellated while the width stays in range. A also shows a wrong shape while held, and needs the app to tell the renderer when a slider is held and let go. B needs neither.

B alone does not meet the budget either (27 ms against 16.6 ms on the outline-width drag). What remains after it:
- **The scene update, about 9 ms:** `MapScene::content` builds the groups again for a Shore change.
- **About 12 ms in `resolve`:** each changed group is compared with the cached one and copied into the cache. With B the stroke width leaves the group's identity (the width is drawn, not tessellated), so a width change within range is not a change to the group at all, and this cost should mostly go. That is to be measured once B is built.

Those are the next steps for #692 after B.

## The Land Eraser

`erase` spends about 14 ms of its 46 ms p95 re-tessellating the erased Landmass ([#696](https://github.com/hjrdave/StarScriptMapMaker/issues/696#issuecomment-7240458995)). **Neither A nor B helps it.** The eraser changes the outline's path, not its width, so the mesh has to be tessellated again whatever applies the width. What would help is tessellating only the part of the coast that changed, by splitting a long outline into pieces that are meshed and cached separately. That is a separate decision, outside this ADR.

## Measuring success

On the full-scale Map, an outline-width drag (`slider-shore` with only `outline.width` changing; a script for it is added with B): upload under 1 ms and draw under 5 ms on the held frames, and no tessellation while the width stays in range. The golden images and the tiled-export tests must not change beyond what a look on a software GPU accepts.

## Consequences

- A stroke width change within range costs a 4-byte write, not a tessellation and an upload.
- Strokes and fills take different vertex paths in one shader, and `look` and `tint` mean different things for each. The packing comment in `fill.wgsl` and in `vector/chunk.rs` has to say so.
- Outline meshes are 10–35% bigger on the GPU and in the cache.
- Leaving a width range costs one frame like today's. A drag that crosses several ranges has one such frame per range.
- The rescale prototype (A) stays on its branch and is not merged.
