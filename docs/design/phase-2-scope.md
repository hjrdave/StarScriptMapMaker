# Phase 2: UI redesign scope

Phase 2 rebuilds the editor's look and layout from the design handoff in
[`ui-handoff/`](ui-handoff/HANDOFF.md), and adds two features: **multiple Map tabs** and a
**Home tab**.

Read `ui-handoff/HANDOFF.md` first. It is the source of truth when it disagrees with a
mockup.

**To see a screen, open its PNG in [`ui-handoff/screens/`](ui-handoff/screens/)**: every
mockup is rendered there under the same name (`mockups/Start.html` → `screens/Start.png`).
Use the HTML in `ui-handoff/mockups/` for exact colours, sizes and copy. It is a design
reference, not code to port.

The PNGs are rendered by `render/render-mockups.js`. Re-run it if a mockup changes:
`node docs/design/render/render-mockups.js` (needs Playwright and Chromium). Two bugs in the
mockup source are corrected only in the renders (see `render/shim.js`): the Properties tool
section was squashed by the dock-button style, and Label and NodeEdit had an empty spacer
pushing the window down.

## The rule for this phase

The mockups show some controls the app does not have yet. **Restyle and rearrange what the
app already does. Do not build a feature just because a mockup shows it.** If a mockup shows
a control for something that doesn't exist, leave the control out. The one exception is
multiple tabs and the Home tab, which the author asked for.

Small affordances that only expose behaviour the app already has are fine, for example a
Paint / Erase toggle for the Land Brush (erase exists behind the modifier key), or zoom
presets in the zoom control (the view already zooms).

## In scope

- Theme and tokens (dark and light), the UI font, and the icon atlas.
- Multiple Map tabs in the title bar, plus the Home tab.
- The window shell: three columns, collapsible Layers, status bar, responsive rules.
- Native menu bar on macOS, holding the commands the app already has.
- The Layers panel's row states and indicators for features that exist.
- The Properties panel: section framework, scrub fields, swatches, inherited and overridden
  states, and per-tool sections for tools with settings that exist.
- Tool dock, zoom control and the existing canvas overlays restyled.
- Toasts for the existing notices.
- Restyling the dialogs that exist: Unsaved changes, Recovery, New Map, Export PNG, Export
  progress, Label styles, Credits.

## Out of scope (shown in the mockups, not built in Phase 2)

Each of these would be its own issue if the author wants it later.

- Land Brush tips (Round, Rough, Crag, Shoal, Fjord, Soft), Smoothing and "Paint into".
- Scatter Variant modes Cycle and Fixed (the app has a Random variant on/off).
- Landmass Name field and the Transform section (Landmasses have no names).
- Stamp vertical flip (the app flips horizontally only).
- **Mixed** values and editing several Objects' fields at once. Multi-selection keeps its
  summary and gains the existing Combine actions (Union, Subtract, Split apart).
- "Update style" on a Label (Reset overrides exists and stays).
- Group command (⌘G). New Layer exists, and Layers already nest by drag.
- Summary rows for large Stamp sets ("Pine cluster × 822").
- Undo History window, Cut / Copy / Paste.
- Cursor HUD during gestures.
- Frame time in the status bar (the `--frame-time` benchmark is separate).
- Home tab Folders (scanning folders for Map files), an Asset packs browser page and
  "What's new".
- Recovery dialog previews and the list of recovered Commands.
- New Map presets, orientation, aspect lock, template stack, Label style set and live
  preview. The dialog keeps its current fields.
- Export progress "Show in Finder".
- Restoring the open tabs at the next launch.

## Decisions made for this phase

- **Scatter Brush shortcut** moves from C to ⇧S, as in the handoff.
- **Nothing selected** shows the active Layer (as in the handoff), followed by a collapsed
  **Map** section with today's Map properties (Canvas, Scale and Map defaults), so nothing
  that exists loses its place.
- **The app launches on the Home tab** unless a Map file was passed on the command line or a
  crash left something to recover.
- **macOS first.** On Windows and Linux the window keeps its native decorations, and the tab
  strip and an egui menu bar sit at the top of the window instead of in the title bar.
