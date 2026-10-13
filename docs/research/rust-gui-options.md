# Research: Rust GUI options for a Figma-style wgpu editor

Resolves issue #3 (parent map #2). Researched 2026-10-13.

## Question

Which Rust UI approaches can deliver a Figma-style editor (dockable panels, layer tree with drag-and-drop, property inspector, custom GPU canvas, selection/transform handles) on Windows, macOS and Linux? Candidates: egui (+eframe), iced, Slint, Xilem/Masonry, GPUI, custom wgpu UI.

## Method and caveats

- Primary sources only: each project's README, CHANGELOG and source files on `raw.githubusercontent.com`, plus crates.io metadata (version, date, dependencies). Source URLs are listed per section.
- Only `raw.githubusercontent.com` and `crates.io` were reachable from the research sandbox. docs.rs, project doc sites and the GitHub API were blocked, so API-level claims come from source files and changelogs. Items marked **unverified** need a hands-on check.
- No code was run. Nothing here was benchmarked. Text-quality and performance statements are what the projects say about themselves or what their changelogs show.
- Crate versions below are as of 2026-10-13 (crates.io).

## Summary table

| | egui + eframe | iced | Slint | Xilem / Masonry | GPUI | Custom wgpu UI |
|---|---|---|---|---|---|---|
| Latest release | 0.36.2 (2026-09-08) | 0.14.0 (2025-12-07) | 1.18.1 (2026-09-21) | 0.4.0 (2025-10-29) | 0.2.2 (2025-10-22) | n/a |
| Self-described status | WIP, breaking changes each release | "experimental software" | stable-numbered (1.x), commercial backing | "experimental" | pre-1.0, breaking changes | n/a |
| Licence | MIT OR Apache-2.0 | MIT | Royalty-free OR GPLv3 OR commercial | Apache-2.0 | Apache-2.0 | yours |
| wgpu version used | 30 | 27 (wgpu is at 30) | optional, via renderer/feature | via Vello | own backends | any |
| Custom wgpu canvas | `egui_wgpu::Callback` | `shader` widget / `Primitive` | rendering notifier + wgpu texture import | `Canvas` widget; custom widget | custom element; backend not wgpu on macOS/Windows | native |
| Tree / virtual list | no tree widget in core; `egui_tiles`, `egui_dock` third-party | `table`, `grid` widgets; no tree seen | `ListView`, `StandardTableView`; no tree widget seen | `virtual_scroll`, `collapse_panel`, `split` widgets | list element designed for large lists; no tree widget | build it |
| Accessibility | AccessKit, on by default in eframe | none found in changelog | built-in a11y properties/roles | AccessKit built in | AccessKit crates in platform backends | build it (AccessKit) |

## egui + eframe

- **Maturity.** README says egui "does not claim to have reached all these goals yet", "lacks many features" and its interfaces are changing; new releases contain breaking changes. Release cadence is high: 0.34.3 (2026-05-27), 0.35.0 (2026-06-25), 0.36.0 (2026-08-05), 0.36.2 (2026-09-08). Unreleased 0.37.0 is already in the changelog. (README, crates.io)
- **Licence.** MIT OR Apache-2.0. (README)
- **Custom wgpu canvas.** Best-documented path of all candidates. `egui-wgpu` exposes `Callback` and `CallbackTrait` with `prepare`, `finish_prepare` and `paint` hooks so your own pipelines draw inside a region of the egui layout. eframe 0.36.2 and egui-wgpu 0.36.2 depend on `wgpu ^30`, the current wgpu major. (`crates/egui-wgpu/src/renderer.rs`, crates.io deps)
- **Text.** Font rendering moved from `ab_glyph` to `skrifa` + `vello_cpu` with font hinting for sharper text. Unreleased 0.37 adds system-font discovery via `fontique`, colour emoji and any-script glyph support. In the shipped 0.36 line non-Latin text still needs you to register fonts. (CHANGELOG, README)
- **Tree / docking.** Core egui has panels, collapsing headers, scroll areas and drag-and-drop primitives but no docking or tree widget. Third-party crates exist: `egui_dock` 0.21.1 (docking, ~4.8M downloads) and `egui_tiles` 0.17.1 (tiling with drag and drop, ~2.4M downloads). A layer tree with drag-and-drop must be assembled by hand. (crates.io; README)
- **Performance caveat.** Immediate mode re-runs layout each frame; README warns that very large scroll areas can be slow and that first-frame sizing can flicker. A layer tree must virtualise itself by hand. (README)
- **Theming.** Programmatic `Style`/`Visuals`; README says styling is less powerful than CSS. 0.37 adds an experimental `StyleProvider` theme plugin system behind the `experimental` feature. Pixel-identical custom look on all OSes is natural, since egui draws everything itself ("native-looking interfaces are a non-goal"). (README, CHANGELOG)
- **Accessibility.** AccessKit is always a dependency and enabled by default in eframe. README says native APIs are implemented on Windows and macOS; Linux is not listed, so treat Linux a11y as **unverified**. Recent changelog entries keep extending it (panel resize handles, hiding invisible widgets). (README, CHANGELOG)
- **Testing.** `egui_kittest` drives UIs through the AccessKit tree. (CHANGELOG)
- **Fit for the map editor.** Very fast to get a three-panel layout with wgpu canvas working. The cost is hand-built Figma-style pieces (layer tree DnD, transform handles) and constant upgrade churn.

Sources: https://raw.githubusercontent.com/emilk/egui/main/CHANGELOG.md, https://github.com/emilk/egui (README), https://raw.githubusercontent.com/emilk/egui/main/crates/egui-wgpu/src/renderer.rs, https://crates.io/api/v1/crates/egui

## iced

- **Maturity.** README: "Iced is currently experimental software." Last crates.io release is 0.14.0 on 2025-12-07, about ten months before this research, with prior releases 0.13.x in Sept 2024. Slow, large releases. (README, crates.io)
- **Licence.** MIT. (README)
- **wgpu.** `iced_wgpu` 0.14.0 depends on `wgpu ^27`, three majors behind the current wgpu 30. Sharing a wgpu device or texture types with other crates requires matching versions. (crates.io deps)
- **Custom wgpu canvas.** `shader` widget (added 0.12) with `iced_wgpu::primitive::{Primitive, Pipeline}` traits (`prepare`, `render`/`draw`); 0.14 added a `shader::Pipeline` trait for resource management. A 2D `canvas` widget with a geometry cache also exists. (CHANGELOG, `wgpu/src/primitive.rs`)
- **Text.** `cosmic-text` 0.15 (shaping, fallback, bidi). 0.14 added input-method support, `Auto` shaping strategy and justified alignment. (CHANGELOG, crates.io deps)
- **Widgets.** 0.14 added `table`, `grid`, `sensor`, `float`, `pin`; "primitive culling in column and row" helps long lists. No tree view or docking in the core changelog; docking exists as `pane_grid` (earlier releases, not re-verified here). Layer-tree DnD would be custom. (CHANGELOG)
- **Theming.** Palette-based built-in themes plus per-widget style functions; system theme reactions in 0.14. (CHANGELOG)
- **Accessibility.** No accessibility or AccessKit entries found anywhere in the changelog. Treat as absent. (CHANGELOG)
- **Tooling.** Hot reloading, time-travel debugging, headless and end-to-end testing arrived in 0.14. (CHANGELOG)
- **Fit.** Pleasant Elm-style architecture that maps well to a command API, but wgpu lag and no accessibility are the notable costs.

Sources: https://github.com/iced-rs/iced (README), https://raw.githubusercontent.com/iced-rs/iced/master/CHANGELOG.md, https://raw.githubusercontent.com/iced-rs/iced/master/wgpu/src/primitive.rs, https://crates.io/api/v1/crates/iced

## Slint

- **Maturity.** 1.x, rapid releases: 1.16.1 (2026-04-23), 1.17.0 (2026-06-24), 1.18.0 (2026-09-16), 1.18.1 (2026-09-21). 24k+ stars. Commercial company behind it. (crates.io, repo page)
- **Licence.** Choose one of: Royalty-free licence (proprietary desktop/mobile/web apps at no cost, embedded excluded), GPLv3, or paid commercial. Docs and examples are MIT. The royalty-free licence terms should be read in full before committing a closed-source product. (`LICENSE.md`, README)
- **UI model.** Declarative `.slint` DSL compiled at build time, not Rust code. Unlike the other candidates it is its own language; the document-model/command API stays in Rust and is bound via properties and callbacks. (README, docs)
- **Renderers.** winit backend with FemtoVG (OpenGL, or `renderer-femtovg-wgpu` over Metal/Vulkan/D3D), Skia (OpenGL/Metal/Vulkan/D3D, heavy footprint), software, and an experimental Vello renderer (never auto-selected, needs compute shaders). Docs themselves note FemtoVG "text and path rendering quality sometimes sub-optimal". (`backends_and_renderers.mdx`)
- **Custom wgpu canvas.** Changelog shows wgpu texture interop: `unstable-wgpu-30` feature, `slint::wgpu_30` module, `BackendSelector::require_wgpu_30()` and `Image::to_wgpu_30_texture()`; rendering notifiers also exist. The pattern is render your map with your own wgpu pipeline into a texture, then show it as a Slint `Image`; all of it is marked unstable. How well pointer events and transform handles layer on top: **unverified**. (CHANGELOG)
- **Widgets.** `ListView` (virtualised), `StandardTableView`, popups. No `TreeView` found in the changelog or widget history; a layer tree would be built from a flattened model. **Unverified**, check the std-widgets reference. (CHANGELOG)
- **Theming.** The DSL lets you define fully custom components; built-in widget style set not verified from primary sources here (**unverified**). Qt style exists as a separate backend/renderer. (backends doc)
- **Accessibility.** Built into the language (`accessible-role`, `accessible-label`, etc.); 1.18 work fixed text-input exposure and added roles. Best of the group apart from Xilem on first-class support. (CHANGELOG)
- **Fit.** Most commercially supported, but the DSL, the wgpu interop marked unstable and no tree widget are the risks for a Figma-style canvas editor.

Sources: https://raw.githubusercontent.com/slint-ui/slint/master/LICENSE.md, https://raw.githubusercontent.com/slint-ui/slint/master/CHANGELOG.md, https://raw.githubusercontent.com/slint-ui/slint/master/docs/astro/src/content/docs/guide/backends-and-renderers/backends_and_renderers.mdx, https://crates.io/api/v1/crates/slint

## Xilem / Masonry

- **Maturity.** README: "An experimental Rust native UI framework." crates.io 0.4.0 published 2025-10-29 (the `main` branch is well past that and already moved from `vello` directly to an `imaging` abstraction, so the published crates are stale compared to `main`). 5.6k stars, Linebender community project. (README, crates.io)
- **Licence.** Apache-2.0 (some example assets have separate licences). (README)
- **Architecture.** Masonry is a retained widget tree with centralised focus, pointer and accessibility handling and a test harness; Xilem is a reactive view layer on top. You can use Masonry directly with your own architecture. (Masonry README)
- **Rendering and text.** Vello (compute-based 2D, targets wgpu) and Parley/Fontique for text; windows through winit. Vello 0.11.0 (2026-10-02) depends on `wgpu ^30`, so Vello itself tracks the current wgpu even though the published Masonry 0.4.0 pins `vello ^0.6`. (Masonry README, crates.io deps)
- **Custom wgpu canvas.** Masonry has a `canvas` widget; embedding arbitrary wgpu pipelines is described only as "can be used with any windowing library which allows the window content to be rendered using wgpu". How to share the device/render pass with your own pipeline: **unverified**.
- **Widgets.** `main` has `virtual_scroll`, `split`, `collapse_panel`, `disclosure_button`, `portal`, `grid`, `zstack`, `text_input` and more. No tree view or docking. (`masonry/src/widgets/mod.rs`)
- **Accessibility.** AccessKit integrated by design. (Masonry README)
- **Fit.** Architecturally the most aligned with a custom Vello/wgpu canvas, but explicitly experimental, slow tagged releases, thin widget set.

Sources: https://github.com/linebender/xilem, https://raw.githubusercontent.com/linebender/xilem/main/masonry/README.md, https://raw.githubusercontent.com/linebender/xilem/main/masonry/src/widgets/mod.rs, https://crates.io/api/v1/crates/masonry

## GPUI

- **Maturity.** README: "still pre-1.0. There will often be breaking changes between versions", developed for the Zed editor. Published `gpui` 0.2.2 is from 2025-10-22; `main` has since split into `gpui_platform`, `gpui_windows`, `gpui_macos`, `gpui_linux`, `gpui_wgpu` crates, so published and `main` differ materially. (README, crates.io, Cargo files)
- **Licence.** Apache-2.0 (crate metadata). (crates.io)
- **Rendering.** Per-platform: Metal on macOS, Win32/DirectWrite on Windows, `gpui_wgpu` on Linux (Wayland and X11 features). Not a single shared wgpu path. The older published 0.2.2 used `blade-graphics` on Linux. A custom wgpu canvas in the same device is therefore not a natural fit; **unverified** for current `main`. (README, `gpui_platform/Cargo.toml`, `gpui_linux/Cargo.toml`, crates.io deps)
- **Model.** Entity/view/element registers with a Tailwind-style styling API; low-level `Element` trait supports custom layout and "efficient views into large lists". (README)
- **Accessibility.** README lists an Accessibility topic; platform crates depend on `accesskit` (and `accesskit_unix` on Linux). Maturity **unverified**. (README, Cargo files)
- **Docs.** README states the best way to learn is reading Zed source. (README)
- **Fit.** Strong performance pedigree for editors, but biggest ecosystem and wgpu-integration risk and an unstable API.

Sources: https://raw.githubusercontent.com/zed-industries/zed/main/crates/gpui/README.md, https://raw.githubusercontent.com/zed-industries/zed/main/crates/gpui_platform/Cargo.toml, https://raw.githubusercontent.com/zed-industries/zed/main/crates/gpui_linux/Cargo.toml, https://crates.io/api/v1/crates/gpui

## Custom wgpu UI

- **What it means.** Own widget layer on `wgpu` 30 + `winit` 0.30, optionally using building blocks that are individually maintained: Vello 0.11 (2D), Parley/Fontique or `cosmic-text` (text), `taffy` (layout), AccessKit 0.25.1 (accessibility). Masonry itself is built from such parts. (crates.io)
- **Pros.** Total control over Figma-style handles, snapping, tree DnD, docking and identical visuals on every OS; one wgpu device shared with the map renderer.
- **Cons.** Highest cost: text input/IME, focus, accessibility, scrolling and hit-testing must all be built or assembled. Every candidate above has already paid some of that cost.
- **Fit.** Only justified if the off-the-shelf options cannot meet the UX; consider as the fallback after a spike.

## Cross-cutting observations

- **wgpu version alignment:** egui 0.36.2 and Vello 0.11 use wgpu 30, iced 0.14 uses wgpu 27. The map renderer's wgpu version should match the UI framework's.
- **AccessKit** is the shared accessibility layer in egui, Masonry and GPUI; Slint has its own.
- **None of the options ship a Figma-style layer tree with drag-and-drop or transform handles**; it will be application code in all cases. Docking is available off the shelf only for egui (`egui_dock`, `egui_tiles`).

## Suggested next steps (for the main session)

1. Shortlist egui and Slint (and Xilem as the architectural-fit wildcard) for a time-boxed prototype, per the `prototype` skill: three-panel layout, wgpu canvas, layer tree with DnD of ~10k rows, a transform-handle overlay.
2. Verify the **unverified** items above hands-on: Slint wgpu-texture pointer/handle overlay, Xilem custom wgpu pipeline sharing, GPUI `main` wgpu integration, egui Linux accessibility.
3. Read the Slint Royalty-free licence in full before choosing it.
