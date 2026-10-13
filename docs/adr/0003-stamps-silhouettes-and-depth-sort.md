# Stamps: dual-format art, Silhouettes and Depth sort

Status: accepted (amends the Images row of ADR 0001)

An Asset's art is either SVG, rasterised into the atlas at the zoom or export scale needed (resvg/usvg in `render`), or PNG at its native resolution. The default CC0 base, the Kenney Cartography Pack, ships PNGs of only 64–128px, which would blur on print-size exports, and its ink line art is naturally vector. Its stamps are also outline-only, so overlapping Stamps would show each other's lines. Each Asset therefore has a **Silhouette** (generated when the pack is built by filling from the outer edge, or hand-authored). The renderer fills the Silhouette with a fill colour beneath the line art, defaulting to the Map's land colour and overridable per Stamp. The Silhouette also gives pixel-accurate picking.

Overlapping terrain must draw back to front, so a Layer can switch on **Depth sort**. The Stamps directly inside it then draw by Anchor height, the layer tree lists them in that order, and manual reordering inside that Layer is disabled. This is the second deliberate, narrow exception to "one ordered tree decides stacking" (the first is ripple lines under land, ADR 0002). It is confined to Layers where the user switched it on, and the tree still shows the true draw order.

## Pack format

An Asset pack is a folder with a hand-editable `pack.toml` and image files. Per Asset, the manifest holds:
- id, name, category, tags, file
- Anchor
- default size in Canvas units
- optional Silhouette file
- Variant group
- `author`, `source_url`, `licence`, `modified`

The pack also carries its own licence and credits text, and Assets are referenced as `pack-id/asset-id`. Stamps store the concrete Asset they got, never a variant group, so editing a pack never reshuffles existing Maps. v1 loads the built-in packs from the repo's `assets/` plus an unsupported per-user packs folder.

## Considered options

- **PNG only, large sources**: rejected; the existing CC0 base would blur, and ink art loses nothing as vector.
- **Require opaque art**: rejected; it means hand-editing every line-art pack.
- **Strict tree order with y-sorted insertion**: rejected; moving a Stamp later would leave it drawn out of depth order.
- **Map-wide y-sort**: rejected; it would override the layer tree everywhere.

## Consequences

- Default packs are CC0 or the author's own art only. game-icons.net (CC BY 3.0) ships as an optional pack flagged as requiring attribution. When a Map uses any such Asset, export shows its credit lines from the manifests and offers to copy them or add them as a Label.
- Rotation and scale pivot on the Anchor.
