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

## Language and design

- Use the terms in `CONTEXT.md` (Map, Object, Layer, Landmass, Stamp...) in code, docs, UI text and commit messages. Do not use the `_Avoid_` alternatives listed there.
- Read the relevant entry in `docs/adr/` before changing architecture. Record a new architectural decision as a new ADR instead of quietly departing from an old one.

## Rules

- Edits to a Map go through a `Command`, which compiles to `Op`s (ADR 0004). Never mutate a Map directly from UI or render code.
- Every change to `crates/core` comes with tests.
- Avoid `unwrap()` and `expect()` outside tests; return or handle the error.
- Keep changes focused: no unrelated refactors or dependency changes.
- Match the surrounding code's style, naming and comment density.
