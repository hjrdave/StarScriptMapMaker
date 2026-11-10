# Colour painting uses Map-level vector strokes

Status: accepted. Issue #616 asks for a brush that paints colour over a Map, with an eraser. This record covers the model, Commands and file format, which issue #630 builds in `crates/core`. Drawing and the tool follow in two later parts of #616.

## Decision

- **Vector strokes.** Paint is stored as a list of **paint strokes**, not pixels. Each is a path of points in Canvas units, a colour, a size (brush diameter in Canvas units), a softness (0 to 1) and an opacity (0 to 1). This keeps the Map resolution independent and editable, as ADR 0001 intends, and keeps the file small (ADR 0005).
- **Map-level, in drawing order.** Strokes belong to the Map, not to a Landmass, Layer or Object, and draw first to last. They are not part of the node tree, so Lock, Link and Selection do not apply to them. Strokes are clipped to land when drawn, so a Landmass that moves leaves its paint behind. Paint follows the Landmass's Roughness when drawn. Both belong to the drawing part, not to `core`.
- **Eraser.** An eraser is a stroke with `erase: true`. It removes paint under it that was drawn before it, and its colour is ignored. Because it is in the same ordered list, paint added after an eraser is not affected, and undo needs nothing special. The list can grow with erasing; deleting strokes is how a user shrinks it.
- **Commands and Ops (ADR 0004).**
  - `AddPaintStroke` adds one stroke, paint or eraser. One press-drag-release is one stroke, so one Undo step, named "Paint" or, for an eraser, "Erase Paint".
  - `DeletePaintStrokes { ids }` and `ClearPaint` remove strokes. Their Ops restore each stroke at its old index on undo.
  - Two Ops: `InsertStroke { index, stroke }` and `RemoveStroke { id }`, each the inverse of the other. The `Changes` log records them as `other`, so derived data rebuilds.
- **Simplification.** A stroke's path is simplified with Ramer-Douglas-Peucker when the Command compiles, to within `STROKE_TOLERANCE` (0.5 Canvas units). It keeps the first and last point, so a dot or a closed loop survives. The Command takes at most `MAX_RAW_STROKE_POINTS` (100 000) points, as a live drag can produce many.
- **Validation.** A stored stroke has 1 to `MAX_STROKE_POINTS` (5 000) finite points, a size above 0 and at most `MAX_STROKE_SIZE` (2 000), and a softness and opacity from 0 to 1. A Map has at most `MAX_PAINT_STROKES` (20 000) strokes. Commands refuse a stroke that breaks these, and opening a file whose strokes do is an error (a damaged file), not a silent repair.
- **File format.** Strokes are stored in the `strokes` meta key as JSON (ADR 0005), left out when there are none. A file with paint is format 1.19, an optional addition, so older files open with no paint and an older app keeps the key it does not read. `MapView` has the same optional list.
- **Export (ADR 0007).** Paint is part of the picture, so export draws it, in the same pass as the Map, in tiles. Nothing for export is built in `core`.
- **Not in this version.** No palette and no recent colours, and a stroke cannot be edited after it is added, only deleted.

## Consequences

- A stroke is data that grows with the drag, so a Map with much painting is bigger than one without. The caps and simplification bound this; check the save and open targets of ADR 0005 on a heavily painted Map once drawing exists.
- Moving a Landmass leaves its paint where it was, which matches the decision above but may surprise users. The tool should say so.
- Erasing adds a stroke instead of cutting existing ones, so deleted paint still costs space until the strokes are deleted. A later change can flatten fully covered strokes.
