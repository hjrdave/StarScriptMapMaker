# Rivers are paths with generated outlines

Status: accepted (amends ADR 0002 for Rivers)

ADR 0002 stores every outline as polygon rings, and the water brush made rivers as narrow Lakes. They looked like thick lines: the same width all along, no bends unless drawn by hand, and nothing to edit afterwards except outline nodes. A **River** is its own Object kind that stores only a path of points from source to mouth, each with a width, plus how much it bends (`meander`) and a `seed`. Its outline is generated from that path whenever the River is read or edited. It is never stored or edited directly. This follows the River tool in Wonderdraft: place points from source to mouth, widen the River as you go, and pick among bend variations.

## How the outline is made

`ssmap-core` (`river.rs`) smooths the path with a centripetal Catmull-Rom curve through every point, so it never loops or overshoots between points. It adds bends between points from seeded value noise that fades to nothing at every point, so the River passes through each point placed. It then offsets the banks by half the width, which tapers linearly from point to point, with a slight wobble. The source narrows to a fine point. The mouth is cut square, and the shore line and shading leave that edge out, so it opens into the water it meets. The outline is the union of the triangles between neighbouring cross-sections, so a tight bend that folds a bank over itself still makes one clean outline.

Like Roughness, the generator uses only `+ - * /` and `sqrt` and draws its randomness from an integer hash. A River therefore has the same outline on every platform, and render, picking, Clip and export all agree on it.

## Rules

- **The path is the River.** `CreateRiver` and `SetRiver` carry the River's stored fields. `MoveNodes`, `InsertNode` and `DeleteNodes` edit its points: part 0, ring 0, from source to mouth. `Translate`, `Transform` and paste move its points and scale its widths. `ReplaceGeometry`, `AddRegion`, `SubtractRegion` and `SetRoughness` refuse a River.
- **Water, like a Lake.** A River draws with its Layer's Lakes, above the land, uses the Lakes' default Shore style, and counts as water for scatter placement and Clip. It has no Roughness of its own: its bends and banks are its detail.
- **The tool ends Rivers at the shore.** The River tool cuts a path where it first leaves land (`ending_at_shore`), just past where the later bank crosses. The square mouth then covers the Lake's or the coast's shore line, and the River joins the water with no line across it.
- **A River can leave a Lake.** A path that starts in water and later reaches land has its start cut where it first reaches land (`starting_at_shore`, the mirror of `ending_at_shore`), just inside the water. The River then stores `open_source: true` (omitted when false), which cuts the source square instead of narrowing it to a point, and leaves its edge out of the shore line, as the mouth's is, so the River joins the Lake with no line across it. `River::source_mouth` is that edge, and `away_from_mouth` skips both.
- **File format 1.11.** A River's row holds its path in `props` and has no `geometry` blob. Older apps refuse the `river` kind, as they would any unknown Object kind.

## Considered options

- **Rivers as Lakes with a better brush:** rejected. A stored polygon can't keep its widths or bends editable, and every edit would lose the path.
- **Bézier storage and a variable-width stroke in `render`:** rejected for the same reason as GPU Roughness in ADR 0002. Picking, Clip and scatter would need a CPU copy that could disagree with the screen.
- **Clipping each River to land in `render`:** this would hide the part past the shore without cutting the path. It was rejected because a Clip scope turns off the scene's incremental patching, so every edit to a Map with Rivers would rebuild the whole scene.

## Consequences

- Tributaries are separate Rivers that end on another River; nothing joins them yet.
- A River placed by Commands rather than the tool keeps whatever path it was given, including any part that runs past the shore.
