# A Landmass has a Fill style, and the GPU works out grain and mottle

Status: accepted. Issue #605 (Landmass fill styles) asks for more than a flat colour. This record covers **grain** (fine speckle) and **mottle** (large soft patches of a second colour), which can be used alone or together. Texture images and painting colour onto the land are later work and are not decided here.

## Decision

- **`Landmass.fill_style`** is a `FillStyle` in `crates/core`: an optional `grain` and an optional `mottle`. With neither, the fill is flat. It is separate from `fill`, the colour, so a style can be switched without losing the colour. Lakes and Rivers have no Fill style.
  - `Grain { amount, size }`: how much each speck lightens or darkens the fill (0 to 1), and how wide one speck is (0.25 to 16 Canvas units).
  - `Mottle { color, scale, contrast, seed }`: the patch colour (its alpha is ignored), how wide a patch is (10 to 5000 Canvas units), how far the patches go towards their colour (0 to 1), and a seed that picks which patches there are.
- **File format.** The field is left out of the file while the fill is flat, and each part is left out while it is off. A file without it reads as flat. This only adds optional fields, so it is a **minor** format change (format 1.13): an older app opens the Map and keeps the field without reading it (`extra`), and shows the Landmass flat. No migration and no `.bak`.
- **Edits go through `Command::SetFillStyle`**, which compiles to `Property::FillStyle` (ADR 0004). It sets the whole style at once. It refuses Lakes, Rivers, locked Objects and values out of range.
- **Neither is stored as pixels or geometry.** `crates/render` draws them in the fill shader from the sample's Canvas position:
  - **Grain** is smooth noise: fine flecks of `size` blended with softer, wider ones about three times that size. It scales the fill's brightness (by up to 50% at an amount of 1). Smooth noise, not one hash per cell, because hard-edged cells look like TV static.
  - **Mottle** is two sizes of smooth noise, shifted by the seed and pushed through a smoothstep, so there are clear patches with soft edges. Where the noise is high, the fill colour blends towards the patch colour by up to `contrast`. Mottle is applied first and grain on top.
- **Packing.** Each vertex of a styled fill carries the settings as two 4-byte attributes, so the vertex stays 32 bytes. `look` is the grain amount, grain size, mottle contrast and mottle scale; `tint` is the patch colour (sRGB) and a byte made from the seed. Sizes and scales are on a log scale in one byte each, which is within about 3% of the value. Because the seed is one byte, two seeds can give the same patches; the byte is spread out so nearby seeds do not look alike. `pack_style` in `vector/chunk.rs` and `fill.wgsl` must agree.
- **Both are placed by Canvas position**, so they stay put under pan and zoom, and they are the same in every export tile (ADR 0007).
- **Grain fades out as its specks get smaller than a pixel**, so a zoomed-out view shows the flat colour and not shimmer. The fade starts at about two pixels per speck and is gone at three-quarters of a pixel. Mottle needs no fade, because it is smooth and 10 units or wider.
- Only the fill carries a style. A Landmass's outline, ripples and shading do not.

## Consequences

- The vertex grows from 24 to 32 bytes. Every vector fill and stroke pays for it, so a Map with a lot of geometry uses a third more vertex memory. Check this against the benches.
- Golden images do not cover grain or mottle yet, because there is no GPU in the cloud session. Render one on a machine with a GPU, compare it by eye, then add a golden image.
- Lakes have no Fill style. A Map-wide default was added afterwards (see below).
- A later style (texture image, painted colour) can add a field to `FillStyle` without touching the ones here.

## Amendment: a Map-wide default and an optional override (format 1.15)

Like Shore styles (ADR 0016), a Fill style can be set for the whole Map and overridden per Landmass.

- **`Map.defaults.fill_style`** is a `FillStyle`, flat by default and left out of the file while flat. New Landmasses start from it by having no override.
- **`Landmass.fill_style`** is now `Option<FillStyle>`: `None` means "use the Map default". `Some(FillStyle::FLAT)` is a deliberate flat override, which differs from `None` once the default is not flat. `Map::fill_style(id)` gives the style an Object is drawn with, and `ObjectKind::fill_style()` gives only the override.
- **File format.** Still only an optional field in the Map defaults, so it is a **minor** change (format 1.15) and needs no migration or `.bak`. A file from 1.13 or 1.14 reads as a flat Map default, and a Landmass with a stored `fill_style` keeps it as its override, so every existing Map looks the same. A Landmass without one reads as `None`, which is flat under the flat default.
- **Commands.** `Command::SetFillStyle` takes `Option<FillStyle>` (`None` clears the override), and `Command::SetDefaultFillStyle` sets the Map default. Both refuse values out of range. `Property::FillStyle` carries the `Option`.
- **UI.** The Map's Style tab has a "Landmass fill" section with the same Grain and Mottled fields. A Landmass shows the style it is drawn with, any edit makes it an override, and a "Use Map default" button clears the override.

## Amendment: texture images from Asset packs (format 1.18)

Issue #615 asks for a texture image under the fill colour. This is part 1 (core only); drawing and UI follow.

- **Textures are Assets in packs.** A pack's `pack.toml` can hold `[[texture]]` tables: `id`, `name`, `file` (a PNG, relative to the pack folder, meant to tile seamlessly), optional `tags`, and the credit fields (`author`, `source_url`, `licence`, `modified`) that fall back to the `[pack]` values like an Asset's. Texture ids are separate from Asset ids. The same code reads and validates them, and a missing file is reported the same way (`texture <id>`, field `file`). Users can include their own textures by putting them in their own packs. `Pack::textures` and `Pack::texture(id)` expose them.
- **`FillStyle.texture`** is an optional `FillTexture { asset, amount, scale }`: the texture's `pack-id/texture-id` (an `AssetRef`), how strongly it blends over the fill colour (0 to 1; it does not replace the colour), and how wide one tile is in Canvas units (8 to 5000). `FillStyle::problem` checks the ranges, so `SetFillStyle` and `SetDefaultFillStyle` refuse bad values with no new Command. A style with a texture is not flat.
- **`FillStyle` is no longer `Copy`**, because `AssetRef` holds strings. Callers clone it.
- **File format.** Only an optional field in a Fill style (in a Landmass's override and in the Map defaults), left out while there is no texture. A **minor** bump (format 1.18), no migration or `.bak`. Older files have no texture and look the same. An older app reads the style without the texture and drops the field the next time that style is saved.
- **A texture that is not installed** is not a validation error: the reference is only an id, and a Map can be opened without the pack. The renderer and UI (parts 2 and 3) decide what to show then.
- **Not decided here:** how the image is sampled and blended in `fill.wgsl`, and the picker.
