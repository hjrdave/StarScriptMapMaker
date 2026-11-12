# A Shore style can have an outer glow and short water lines

Status: accepted. Issue #604 asks for a Shore style like the soft one Wonderdraft has: a thin dark outline, a pale tinted glow over the water and short horizontal lines beside the coast. ADR 0002 gives a Shore style an outline, ripples and inner shading, none of which draws that. This record adds two effects to the Shore style instead of a second kind of style, so the existing settings, the Map-wide defaults and the per-Object overrides all keep working.

## Decision

- **`ShoreStyle.glow`** is a `ShoreGlow { width, color }`: a tinted glow that fades outward from the coast over the water. A width of 0 draws nothing. The colour's alpha is the strength at the coast.
- **`ShoreStyle.lines`** is a `ShoreLines { spacing, length, width, color }`: rows of short horizontal lines on the water. Each row crosses the land and puts one line out from each side of every stretch of land it crosses. A length of 0 draws nothing.
  - Rows sit `spacing` apart, starting half a gap in from the bottom of the outline, so the lines move with the land.
  - Lines are long where the coast runs up and down and short or none where it runs along, which is the look in the reference.
  - A line stops short of other land. Where two coasts face each other across less than twice the length of water, each takes half, so lines never double up.
  - Only exteriors count. A hole in a Landmass is a Lake, which has its own coast.
  - The geometry is `land::shore_lines` in `crates/core`. It is pure, so it is tested without a GPU.
- **`ShoreStyle::soft_glow()`** is a preset with the look in the reference: a thin dark outline, no ripples, a little inner shade, a 26 unit glow and 44 unit lines. The Shore style fields in the Properties panel and the Map panel's Shores tab have an "Outer glow" and a "Water lines" group and a "Soft glow" button that applies the preset.
- **File format.** Both fields are left out of the file while they are at their off values, and a Shore style without them draws neither. This only adds optional fields, so it is a **minor** format change (format 1.14). Existing Maps look the same, and an older app keeps the fields it does not read (`extra`). The existing defaults and `plain()` have both off.
- **Drawing.** Both are drawn by `crates/render` with the ripples, beneath every fill of the Layer, so they never cover land. The glow is bands along rings grown outward from the coast, each fainter than the last, using the same ring store as ripples and inner shading (ADR 0002). A stroke cannot fade along its length, so each water line is drawn in three stretches, each fainter than the one before. Both count as Shore effects, which a frame drawn mid-gesture skips.
- **Only Landmasses draw the glow and the lines.** A Lake or River with a glow set does not draw it: a glow out from a lake would sit on land, and lines beside a river make no sense. The settings are still stored and shown, because Lakes share the Shore style editor.
- Validation lives in `ShoreStyle::problem`: a glow width from 0 to 500, a line length from 0 to 500, a line spacing from 1 to 500 and a line width from 0 to 50. The spacing limit and a cap of 4000 rows bound the geometry.

## Consequences

- A tall outline with fine spacing makes many short strokes, three per stretch. The row cap keeps it bounded, but check it against the benches on a big Map.
- The glow adds ten bands per Landmass that has one, each a closed ring stroke. Rings are computed on the worker like the others, so a glow does not block editing.
- Golden images do not cover the glow or the lines yet, because there is no GPU in the cloud session. Render one on a machine with a GPU, compare it by eye, then add a golden image.
- The grainy fill the reference also shows is a separate setting (ADR 0015).

## Addendum: hand-drawn water lines (issue #618)

`ShoreLines` gets three optional fields so the lines can look hand drawn, like the Wonderdraft reference. The Soft glow preset is unchanged, and no new preset or wave marks are added here.

- **`taper`** (0 to 1): how much each line thins from the coast to its far end. 0 is an even stroke, 1 tapers to a point.
- **`variation`** (0 to 1): how uneven the lines are in gaps, length and weight. At 1, up to 40% of the lines are left out, and the rest are up to 70% shorter and 60% lighter by different amounts. 0 keeps every line whole.
- **`seed`**: picks which lines are uneven and how. What happens to a line depends only on the seed and the line's position on the Canvas (rounded to a tenth of a unit), never on the viewport or tile, so Canvas, export and tiled export agree (ADR 0007). The geometry is `land::hand_drawn` in `crates/core`.
- **File format.** All three are left out of the file at 0, and a file without them reads as 0, so existing Maps look the same. Optional fields only, so a **minor** bump (format 1.16). `ShoreStyle::problem` requires taper and variation from 0 to 1.
- **Drawing.** With both at 0 the lines are strokes as before. Otherwise each stretch of the fade is one fill of thin quads (thickest at the coast), since a stroke cannot taper.
- **Properties.** The "Water lines" group has Taper and Variation (as percentages) and Seed.
- The golden image for this look is not added yet; it needs a software GPU.

## Addendum: wave marks and the Hand drawn preset (issue #619)

- **`ShoreStyle.waves`** is a `ShoreWaves { density, size, color, seed }`: small wavy marks scattered over the open water beyond the water lines. `density` is marks per 10,000 square Canvas units of water (0 to 50, 0 draws nothing) and `size` is the length of a mark (1 to 100).
- **Placement.** `land::wave_marks` in `crates/core` puts one candidate in each cell of a grid fixed to the Canvas, jittered by `seed`, so a mark depends only on the seed and its place, never on the viewport or tile (ADR 0007). A mark is kept only in open water: at least the water line length plus its own size from the coast, and at most six sizes further out. Anything inside an exterior is dropped (a hole counts as land here). At most 4000 marks and about 40000 cells, so the geometry is bounded.
- **Drawing.** One stroke per Landmass, beneath the ripples and every fill of the Layer, so marks never cover land. It is a Shore effect, so a frame drawn mid-gesture skips it. Lakes and Rivers do not draw wave marks, like the glow and water lines.
- **Hand drawn preset.** `ShoreStyle::hand_drawn()`: a 3 unit dark outline, no ripples, no shading, no glow, thick tapered uneven water lines, and wave marks off (density 0, with a matching dark colour for when they are turned on). It is listed after Soft glow, for Landmasses only. Soft glow is unchanged.
- **File format.** `waves` is left out of the file while it equals the default (density 0), and a file without it reads as the default, so existing Maps look the same. Optional field only, so a **minor** bump (format 1.17). `ShoreStyle::problem` checks the ranges above.
- **Properties.** A "Wave marks" group has Density, Size, Seed and a colour.
- The golden image `hand_drawn_shore.png` is not committed yet: it needs a software GPU. The test compares to it once the file exists.
