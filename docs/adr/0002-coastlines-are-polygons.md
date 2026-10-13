# Coastlines are stored as polygons, with generated Roughness

Status: accepted (amends the Geometry row of ADR 0001)

Landmass and Lake outlines are stored as `f64` polygon rings (outer boundaries and holes), not as kurbo Bézier paths. Every v1 land operation is a polygon operation: brush add and erase, lasso, union, subtract and split, ripple and outline offsets, and Clip. The mature Rust engine for those, i_overlay (releasing since 2023, the backend of geo), is polygon-native. The Bézier boolean options were explicitly early-stage when we decided: linesweeper called itself "early beta" and i_curve was two months old. Béziers would mainly have bought smooth curve handles for a pen tool, which v1 defers.

To keep stored outlines light and editable, brush strokes are simplified on commit (geo RDP, tolerance tied to brush size and zoom). Every stored vertex is an editable node. The jagged look of fantasy coasts comes from **Roughness**: fractal detail that `core` derives deterministically from the stored outline plus amount, scale and seed, and caches. Rendering, Clip, picking and export all use that one roughened outline, so what you see is what clips, what you click and what exports.

## Considered options

- **Bézier storage** with linesweeper or i_curve booleans: rejected for v1 because the boolean layer is too young, and kurbo has no direct fitter from brush points to Béziers.
- **Hybrid** (store Béziers, flatten for booleans, refit afterwards): rejected because each edit loses a little accuracy and the losses add up.
- **Roughness computed on the GPU in `render`**: rejected because Clip and picking would need a CPU approximation that could disagree with the screen.

## Consequences

- kurbo stays for curves elsewhere (glyph outlines; later the pen tool and curved text). A future pen tool needs either Bézier rings alongside polygons or flattening on commit.
- Outward shore effects (ripple lines) from all Landmasses in a Layer draw beneath every Landmass fill in that Layer. This is a deliberate, narrow exception to "one ordered tree decides stacking".
