Default Asset packs and bundled fonts, shared by the app and the website.

- `packs/<pack>/`: one folder per built-in Asset pack, each with a `pack.toml` manifest. The
  format is documented in `crates/core/src/asset/mod.rs`, and a small example pack lives in
  `crates/core/tests/packs/test-pack/`.

Installed builds ship this folder read-only beside the app. `ssmap_core::asset::assets_dir`
is the one place that resolves it; set `SSMAP_ASSETS_DIR` to point the app elsewhere.

`packs/textures/` holds seamless textures for Fill styles. Its PNGs are generated from noise, not
drawn or downloaded. Regenerate them from the repository root with:

```
cargo run -p ssmap-render --bin textures
```

(edit `crates/render/src/bin/textures.rs` to change one, and `packs/textures/pack.toml` to add one).

Generate Silhouettes for a pack's outline art with:

```
cargo run -p ssmap-render --bin silhouettes -- assets/packs/<pack>
```
