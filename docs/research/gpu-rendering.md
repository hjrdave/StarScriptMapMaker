# Research: GPU rendering for vector layers and many instanced assets

Resolves issue #4 (parent map #2). Researched 2026-10-13.

## Question

How should a Rust/wgpu map editor render editable vector coastlines, fills and textures plus tens of thousands of placed asset sprites at interactive frame rates? Options: Vello, lyon, custom; instancing and atlasing; culling/tiling; picking; pitfalls (MSAA vs analytic AA, texture limits, old/integrated GPUs).

## Method and confidence

Sources are first-party READMEs and source files fetched on 2026-10-13 (linked per claim). Pages were read through a summarising fetcher, so wording marked as quotes is as reported by it, not verified verbatim against the raw file. docs.rs, w3.org and gpuweb.github.io were unreachable from the research environment, so WebGPU-spec limits were not checked directly; wgpu-types source was used instead.

**Not covered by any primary source found: benchmarks.** No source gave frame-time numbers for Vello, lyon or instanced sprites. Anything about performance below is architectural reasoning, labelled as such. Treat "tens of thousands of sprites at interactive rates" as unproven until prototyped on target hardware.

## Findings

### 1. Vector rendering options

**Vello, three renderers now exist** ([vello README](https://github.com/linebender/vello), [ARCHITECTURE.md](https://raw.githubusercontent.com/linebender/vello/main/ARCHITECTURE.md)):

| Renderer | Requires | Maturity (per project) |
|---|---|---|
| `vello` compute-centric (now under `research/`) | Compute shaders | "remains an experimental implementation for compute-capable GPUs" |
| `vello_gpu` (renamed from `vello_hybrid`) | Vertex+fragment only; wgpu and WebGL2 backends | "slightly less mature than its CPU-only counterpart"; description: "solid GPU-accelerated 2D renderer" |
| `vello_cpu` | No GPU | "the most mature choice" |

- `vello_gpu` uses Sparse Strips: the CPU flattens paths, bins into tiles and builds sparse strips; the GPU rasterizes and composites them ([vello_gpu README](https://raw.githubusercontent.com/linebender/vello/main/vello_gpu/README.md), ARCHITECTURE.md). Its wgpu and WebGL2 backends "use vertex and fragment rendering rather than requiring compute shaders" (ARCHITECTURE.md).
- Its README lists limitations: mask layers, complex filter graphs and some non-isolated blend modes are unsupported and panic; the wgpu pipeline "isn't fully optimized"; some failures panic instead of returning errors; glyph caching is experimental. It "does not match the compute-centric `vello` renderer's raw performance on dynamic and vector-heavy workloads."
- The compute renderer's own README says it "can currently be considered in an alpha state", needs "a GPU with support for compute shaders", and lists open issues: blur/filters, "Conflations artifacts", GPU memory allocation strategy, glyph caching ([research/vello_research README](https://raw.githubusercontent.com/linebender/vello/main/research/vello_research/README.md)). Its example uses `AaConfig::Msaa16`; the page does not explain the AA methods.
- MSRV 1.89; API is moving (the `vello_hybrid` to `vello_gpu` rename already happened). Pin versions and check wgpu version compatibility per release; the READMEs do not state it.

**lyon** ([README](https://raw.githubusercontent.com/nical/lyon/master/README.md)): a path tessellation library producing vertex/index buffers for fill and stroke; "not an SVG renderer", drawing is up to the caller. "There is currently no built-in support for antialiasing in the tessellators"; README points to MSAA/TAA/FXAA. No performance numbers given.

**Custom**: no source needed to say it is the most work. Not evaluated further.

**kurbo** ([README](https://raw.githubusercontent.com/linebender/kurbo/main/README.md), [shape.rs](https://raw.githubusercontent.com/linebender/kurbo/main/kurbo/src/shape.rs)): path/curve math in `f64`; the `Shape` trait provides `winding`, `contains` (non-zero rule by default), `bounding_box`, `path_elements`, `area`. Vello's ecosystem uses it, so it is the natural editable-path data type regardless of renderer.

### 2. Instancing and atlasing for assets

- Instancing itself is a standard wgpu/graphics feature; no source here quantifies limits for sprite counts, so a per-instance vertex buffer (transform, atlas rect, tint, layer id) with one draw per atlas page is the expected design (reasoning, not sourced).
- Atlas allocation: **etagere** is "a dynamic texture atlas allocator using the shelf packing algorithm", supports deallocation, and "works very well when there is a high number of items with similar sizes" ([README](https://raw.githubusercontent.com/nical/etagere/master/README.md)). Its sibling guillotiere uses a different algorithm with different trade-offs (same README). Asset sprites of mixed sizes may pack better with guillotiere; needs a quick test.
- Alternative to one big atlas: texture arrays. `max_texture_array_layers` is 256 in all wgpu default profiles ([limits.rs](https://raw.githubusercontent.com/gfx-rs/wgpu/trunk/wgpu-types/src/limits.rs)).
- Bindless/binding-array use is not covered by sources read; do not assume it on integrated GPUs.

### 3. Pitfalls: limits and old GPUs

wgpu limits (from `limits.rs`): `max_texture_dimension_2d` is 8192 by default, **2048 in `downlevel_defaults()` and `downlevel_webgl2_defaults()`**. `max_storage_buffers_per_shader_stage`: 8 default, 4 downlevel, 0 WebGL2. `max_storage_buffer_binding_size`: 128 MiB default/downlevel, 0 WebGL2. `max_buffer_size`: 256 MiB. Compute limits are all 0 in the WebGL2 profile.

- `DownlevelFlags::COMPUTE_SHADERS` doc: "WebGL2, and GLES3.0 devices do not support compute." `VERTEX_STORAGE` (storage buffers in vertex shaders) is also a downlevel flag, so instance data should preferably go in vertex buffers, not storage buffers, if GL-class GPUs matter. `DownlevelCapabilities::is_webgpu_compliant` tells you at runtime whether the device is baseline-compliant.
- wgpu backends: Vulkan, Metal, DX12 first class; OpenGL is "Downlevel/Best Effort" (GL 3.3+ on Windows, GLES 3.0+ on Linux), and macOS GL needs ANGLE ([wgpu README](https://raw.githubusercontent.com/gfx-rs/wgpu/trunk/README.md)). Since v1 targets Windows/macOS/Linux, Vulkan/Metal/DX12 cover nearly all hardware; the GL fallback is only a safety net.
- Consequence: the compute-based Vello needs compute-capable drivers; `vello_gpu` or lyon avoid that dependency. A 2048 downlevel texture cap implies atlases must be paged and the app must query actual limits instead of assuming 8192 or 16384.

### 4. MSAA vs analytic AA

- lyon output relies on MSAA or post-process AA (lyon README). In wgpu, sample count support is per format via `TextureFormatFeatureFlags::MULTISAMPLE_X2/X4/X8/X16`; from the source, count 1 is always supported and the guaranteed-format table grants only X4 ([format.rs](https://raw.githubusercontent.com/gfx-rs/wgpu/trunk/wgpu-types/src/texture/format.rs), first 100k chars read). So 4x MSAA is the safe portable choice; 8x/16x must be feature-queried per adapter.
- Vello rasterizes coverage itself (coverage kept at path boundaries per ARCHITECTURE.md), so it does not depend on MSAA hardware. How its AA is implemented is not described in the pages read.
- MSAA cost on integrated GPUs (bandwidth) is a known concern but no primary source here quantifies it; measure.

### 5. Culling, tiling, huge canvases

No primary source read specifically addresses editor-scale culling. What the sources do establish: Sparse Strips already bins into tiles on the CPU, and wgpu caps textures at 2048 to 8192 depending on profile, so a single full-resolution raster of a huge map is not viable on all devices. Recommended design (reasoning, not sourced): keep the document as vector/instance data; keep a spatial index for visible-set queries; render only the viewport (optionally per-tile caches of rasterised vector layers for pan/zoom); export in tiles (matches the map's "tiling for huge canvases" open item).

### 6. Picking / hit-testing

- Geometric picking on the CPU is directly supported by kurbo: `bounding_box` for broad phase, `contains`/`winding` for exact tests on closed paths (shape.rs; `winding` is only meaningful for closed shapes). Open coastline strokes need a distance/stroke test; kurbo's nearest-point support was not confirmed from the pages read.
- Spatial index: `rstar` is "a flexible, n-dimensional r*-tree... suitable for use as a spatial index" ([README](https://github.com/georust/rstar)); the page did not confirm bulk-load or envelope-query APIs, so check its crate docs.
- GPU ID-buffer picking is an alternative but no source read covers it; CPU picking fits the "command API / document model independent of UI" requirement in map #2 better (reasoning).

## Recommendation (for the spec ticket, not a decision)

1. Own the document model as kurbo paths + instance records; keep the renderer behind a trait so Vello/lyon is swappable.
2. Vector layers: start with **`vello_gpu`** (no compute requirement, WebGL2/wgpu, analytic coverage) with lyon + 4x MSAA as the fallback to prototype against; avoid the compute `vello` for v1 given its alpha/research status and compute requirement. Prototype both on one integrated GPU per OS before committing.
3. Assets: instanced quads from paged atlases (etagere/guillotiere), page size from queried limits (assume 2048 floor), instance data in vertex buffers.
4. CPU spatial index (rstar) for culling and picking; kurbo for exact hit tests.
5. Query `Limits`, `DownlevelCapabilities` and per-format MSAA flags at startup.

## Open items / needs prototype

- Real frame-time numbers for N = 10k to 100k sprites and for large coastline path counts (no primary data found).
- `vello_gpu` stability, API churn and wgpu version pairing at the time of build.
- Whether `vello_gpu`'s documented unsupported features (masks, some blends) affect planned layer effects.
- Stroke hit-testing and rstar API details.
