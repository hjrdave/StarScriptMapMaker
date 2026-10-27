# StarScript Map Maker

Native desktop app for drawing fantasy maps, written in Rust. Start with `CONTEXT.md`.

## Where a change goes

| Change | Crate |
| --- | --- |
| Map model, Commands/Ops, geometry, file format (SQLite, ADR 0005) | `crates/core` (`ssmap-core`) |
| wgpu drawing, golden-image tests, render benches | `crates/render` |
| Bundled fonts (ADR 0006) | `crates/fonts` |
| egui UI, tools, panels, packaging, app-level tests | `apps/starscript` |

- `crates/core` must never depend on egui or wgpu (ADR 0001). `scripts/check-core-deps.sh` enforces it.
- `apps/cli` and `apps/website` are placeholders (README only), not workspace members.
- Built-in Asset packs live in `assets/`. Packaging notes are in `docs/packaging.md`.

### Inside `apps/starscript/src`

Modules are grouped by feature. `lib.rs` declares the groups and re-exports each module at the
crate root (`pub use chrome::theme;`), so `crate::theme` and `ssmap_app::theme` stay the same
wherever the file lives. A new module goes in its group's `mod.rs` and gets a re-export in `lib.rs`.

- `app/`: `MapMakerApp`, the eframe app that ties the groups together
- `workspace/`: Map documents and app state: `document`, `editor`, `shell`, `home`, `tools`,
  `clipboard`, `credits`, `recent`, `file_actions`, `update`
- `canvas/`: the Map view: `canvas_view`, `canvas_controls`, `art_board`, `overlay`, `handles`,
  `zoom`, `tessellation`, `frame_time`
- `panels/`: side panels: `layer_tree`, `properties`, `prop_widgets`, `asset_browser`, `label_styles`
- `chrome/`: window chrome: `menu`, `native_menu`, `tab`, `tab_strip`, `tool_dock`,
  `tool_section`, `toast`, `theme`, `icons`
- `dialogs/`: `dialog`, `native_dialog`, `paste_dialog`, `new_map`, `recovery`
- `export/`: `export_dialog`, `export_region`, `export_run`
- `packs/`: `pack_install`, `pack_page`

## Before you call work done

Run, in this order:

```
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
./scripts/check-core-deps.sh
cargo test --workspace --locked
```

CI (`.github/workflows/ci.yml`) runs the same checks. Do not edit `.github/`.

- Golden images are `crates/render/tests/golden/*.png`. They are compared only on a software GPU (lavapipe, WARP); with a hardware adapter only, the tests are smoke tests. Rewrite them with `SSMAP_GOLDEN_UPDATE=1 cargo test -p ssmap-render --test golden`. Update them only for an intended visual change, and look at the changed PNGs first.
- `insta` snapshots are in `crates/core/tests/snapshots/`. After an intended change, run `cargo insta review` (needs `cargo-insta`), or run the tests with `INSTA_UPDATE=always` and check the `.snap` diff.
- Benches and the frame-time check read `SSMAP_BENCH_*` env vars; see `crates/*/benches/` and `apps/starscript/tests/frame_time.rs`.

### In a cloud session

`.claude/hooks/session-start.sh` installs the pinned toolchain and fetches crates. All four checks above run there. The container has no GPU, so some tests pass without checking anything:

- The render tests that need an adapter return early and pass: `crates/render/tests/golden.rs` (it prints `skipping render test`), `tiled_export.rs` and `membership.rs`. Golden images are not compared.
- `apps/starscript/tests/frame_time.rs` prints `skipping:` and passes.
- macOS packaging and anything that opens a window can't be checked.

Say which of these a change touches, so they get checked on a machine with a GPU.

## Language and design

- Use the terms in `CONTEXT.md` (Map, Object, Layer, Landmass, Stamp...) in code, docs, UI text and commit messages. Do not use the `_Avoid_` alternatives listed there.
- Read the relevant entry in `docs/adr/` before changing architecture. Record a new architectural decision as a new ADR instead of quietly departing from an old one.

## Rules

- Edits to a Map go through a `Command`, which compiles to `Op`s (ADR 0004). Never mutate a Map directly from UI or render code.
- Every change to `crates/core` comes with tests.
- Avoid `unwrap()` and `expect()` outside tests; return or handle the error.
- Keep changes focused: no unrelated refactors or dependency changes.
- Match the surrounding code's style, naming and comment density.

## Lint policy

`[workspace.lints.clippy]` in `Cargo.toml` warns on `unwrap_used`, `expect_used`, `too_many_lines` and `cognitive_complexity`; CI's `-D warnings` makes them errors. Thresholds and the test exemptions are in `clippy.toml`.

- Existing hits carry `#[allow(..., reason = "...")]`, many at file level. Remove an allow when you clean up the code under it; don't add one to new code.
- Integration tests and benches may panic, so their files allow the two lints at the top.

## Conventions

These apply to new and touched code. Do not rewrite existing code only to fit them.

- **Modules:** a module with submodules is `foo/mod.rs`, never `foo.rs` next to `foo/`. A module without submodules is a single `foo.rs`.
- **Visibility:** private by default, then `pub(crate)`. Use `pub` only when another crate needs it.
- **Errors:** library crates (`core`, `render`, `fonts`) return `thiserror` error types. No `unwrap()`/`expect()` outside tests unless a comment explains why it can't fail.
- **Tests:** unit tests go in `#[cfg(test)] mod tests` at the bottom of the file. Tests through the public API go in the crate's `tests/` folder.
- **Dependency direction:** `apps/starscript` → `render` → `core` (and `fonts`). Never the reverse. `core` never depends on egui or wgpu (ADR 0001).
- **Naming:** follow the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/naming.html) (`as_`/`to_`/`into_` conversions, no `get_` prefix on plain getters), and use the domain terms in `CONTEXT.md`.

For anything not covered, see the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) and matklad's [Large Rust Workspaces](https://matklad.github.io/2021/08/22/large-rust-workspaces.html).
