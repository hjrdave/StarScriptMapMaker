# Rust stack and monorepo layout for v1

Status: accepted. The renderer-swap part (`VectorRenderer`) is superseded by [ADR 0009](0009-lyon-is-the-only-vector-renderer.md).

The v1 desktop app is a Cargo workspace at the repo root, split into `crates/core`, `crates/render` and `apps/starscript`. `core` holds the Map model, Commands, geometry and serialization, and must never depend on egui or wgpu, so the model stays UI-free and drivable by a future agent. `render` draws the Canvas and PNG exports through wgpu without the UI. `app` is the egui/eframe binary. The UI is egui on eframe, accepted after the editor-shell prototype showed custom wgpu passes, handles and nested layer-tree drag-and-drop all work without third-party crates. Vector layers render with lyon tessellation and 4x MSAA behind a `VectorRenderer` trait, not `vello_gpu`, even though the GPU research made `vello_gpu` the lead: `vello_gpu` panics on mask layers, and v1 needs Clip-to-landmass (a stencil pass in lyon). It also pins its own wgpu version, which would have to match egui-wgpu's. A later benchmark can swap renderers behind the trait.

## Stack

| Concern | Choice | Crate |
| --- | --- | --- |
| UI and windowing | egui on eframe (winit + egui-wgpu underneath); drop to raw winit only if eframe gets in the way | `app` |
| Vector rendering | lyon tessellation, 4x MSAA (ADR 0009); Clip via stencil | `render` |
| Placed assets | Instanced quads from paged atlases; page size from queried wgpu limits (2048 floor); allocator hidden behind the atlas module, `etagere` by default | `render` |
| Geometry | kurbo (`f64` Bézier paths, hit tests); converted to `f32` only in `render`. Landmass and Lake outlines are polygons instead, per ADR 0002 | `core` |
| Spatial index | rstar, for picking and culling | `core` |
| Label text | cosmic-text shaping; glyphs drawn as vector outlines through the same renderer, so halos are strokes and screen matches export | `render` (shaping data may live in `core`) |
| Images | `image` with only the PNG feature (asset decode, PNG export); SVG Assets via resvg/usvg, per ADR 0003 | `render` |
| Serialization | `serde` derives on every `core` type; Map file encoding per ADR 0005 | `core` |
| Persistence | `rusqlite` with bundled SQLite, per ADR 0005 | `core` (an `io` crate if it grows) |

## Repo layout

```
Cargo.toml            # workspace root
rust-toolchain.toml   # pinned stable; this is also the MSRV
deny.toml             # cargo-deny licence policy
crates/{core,fonts,render}   # shared libraries
apps/starscript/      # the desktop app (egui/eframe binary)
apps/website/         # reserved for the docs and download site, own toolchain
apps/cli/             # reserved for a CLI / agent interface
assets/               # default packs and fonts, shared by app and website
docs/                 # ADRs and research
prototypes/           # throwaway code, never a workspace member
```

## Policies

- Edition 2024; one pinned stable toolchain across Windows, macOS and Linux CI.
- CI fails if `core` gains an egui or wgpu dependency.
- `cargo deny` allows MIT, Apache-2.0, BSD-2/3, ISC, Zlib, Unicode-3.0, CC0 and MPL-2.0, and denies GPL, LGPL and AGPL, because the app may be sold.

## Considered options

- **`vello_gpu` first**: rejected for v1 (mask-layer panics, wgpu version coupling). Revisit after measuring.
- **Slint, Xilem**: not prototyped; egui was good enough, with polish deferred.
- **glyphon for labels**: easier to start with, but halos and future curved text are harder on a texture atlas.
- **Rust workspace under `app/`**: rejected; a root workspace keeps plain `cargo` commands working, and the website keeps its own toolchain in `website/`.
