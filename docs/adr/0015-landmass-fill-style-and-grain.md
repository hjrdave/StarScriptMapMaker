# A Landmass has a Fill style, and grain is worked out by the GPU

Status: accepted. Issue #605 (Landmass fill styles) asks for more than a flat colour. This record covers the first two styles, **Flat** and **Grain**. Mottled colour, texture images and painting colour onto the land are later work and are not decided here.

## Decision

- **`Landmass.fill_style`** is a `FillStyle` in `crates/core`: `Flat` (the default) or `Grain { amount, size }`. It is separate from `fill`, the colour, so a style can be switched without losing the colour. Lakes and Rivers have no Fill style.
- **File format.** The field is left out of the file while it is Flat, and a file without it reads as Flat. This adds an optional field, so it is a **minor** format change (format 1.13): an older app opens the Map and keeps the field without reading it (`extra`), and shows the Landmass flat. No migration and no `.bak`.
- **Edits go through `Command::SetFillStyle`**, which compiles to `Property::FillStyle` (ADR 0004). It refuses Lakes, Rivers, locked Objects and values out of range (`amount` 0 to 1, `size` 0.25 to 16 Canvas units).
- **Grain is not stored as pixels and is not geometry.** `crates/render` draws it in the fill shader. Each vertex of a grained fill carries the amount and size. The fragment shader works out smooth noise from the sample's Canvas position, as fine flecks of `size` blended with softer, wider ones about three times that size, and scales the fill's brightness by it (by up to 50% at an amount of 1). Smooth noise instead of one hash per cell, because hard-edged cells look like TV static.
- **Grain is placed by Canvas position**, so it stays put under pan and zoom, and it is the same in every export tile (ADR 0007). It has no seed: the pattern depends only on position.
- **Grain fades out as its specks get smaller than a pixel**, so a zoomed-out view shows the flat colour and not shimmer. The fade starts at about two pixels per speck and is gone at three-quarters of a pixel.
- Only the fill carries grain. A Landmass's outline, ripples and shading do not.

## Consequences

- The vertex grows from 24 to 32 bytes. Every vector fill and stroke pays for it, so a Map with a lot of geometry uses a third more vertex memory. Check this against the benches.
- Golden images do not cover grain yet, because there is no GPU in the cloud session. Render one on a machine with a GPU, compare it by eye, then add a golden image.
- A later style (mottled, texture) can add variants to `FillStyle` without touching the ones here.
