# PNG export renders tiles and streams them into the encoder

Status: accepted

Exports must reach print-poster sizes on GPUs whose texture limit may be only 2048px. `render` therefore draws the export offscreen in **tiles** sized from the device limit. Each tile is rendered with an overlap margin wide enough for halos, outlines and ripples, then cropped. Rows are streamed straight into the PNG encoder, so even a 32,768 × 32,768 export never exists in memory as one image. A golden-image test guards against seams at tile edges. Export waits until derived data (Roughness outlines, rasterised SVGs) is fully rebuilt, so a freshly opened Map never exports half-drawn.

## Export settings

- **Size:** a scale of the Canvas's nominal 1x pixel size (1x, 2x, 4x, custom), a target width with locked aspect, or a print helper (inches × DPI). The cap is 32,768px per side, with a warning above 16k.
- **Region:** the whole Canvas (default) or a custom rectangle drawn on it.
- **Content:** hidden Layers are excluded, locked Layers are included, and editor visuals are never included. An optional transparent background skips the Canvas background fill.
- **File:** 8-bit sRGB RGBA, with pHYs DPI when a print DPI was chosen and the app name and version in a text chunk.
- **Credits:** when credit-required Assets are used, credits are written by default to `<name>.credits.txt` and to a PNG text chunk, alongside the existing copy-or-add-as-Label notice (ADR 0003).
- **Stored in the Map** (ADR 0005): scale or size, region, transparency and the credits options, plus the file name only, never an absolute path.

## Considered options

- **Background export from a Map snapshot** while editing continues: rejected for v1. Export shares the GPU with the editor, so v1 shows a modal progress dialog with Cancel.
- **One full-size render target**: rejected; it cannot exceed the device limit and would need gigabytes of memory at poster sizes.
