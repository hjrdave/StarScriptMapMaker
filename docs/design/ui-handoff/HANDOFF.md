# StarScript Map Maker — UI redesign handoff

Implementation brief for the egui (Rust) desktop app. macOS first, cross-platform later.
Everything here reflects the **final** decisions; earlier iterations were removed.

## What's in this folder

| Path | What it is |
|---|---|
| `HANDOFF.md` | This brief — layout, behaviour, shortcuts, build order. **Source of truth when it disagrees with a mockup.** |
| `tokens.json` | Colour (dark + light), type, spacing, radii, elevation tokens. |
| `theme.rs` | Starter egui theme built from the tokens (adjust field names to your egui version). |
| `mockups/*.html` | Static reference screens (1440×900 window). Read them for exact colours, sizes and copy; they are design mockups, not code to port. They need the design tool's runtime to render, so treat them as annotated source. |
| `screens/*.png` | **Look at these.** A rendered PNG of every mockup, same name (`mockups/Main.html` → `screens/Main.png`). Added to the repo after the handoff, by `docs/design/render/render-mockups.js`. |

Mockup index:

| File | Screen |
|---|---|
| `Foundations.html` | Tokens, type scale, spacing, radii, elevation, icon set (32 icons as SVG paths), component states |
| `Components.html` | Tool buttons, tool dock, title-bar tabs, layer row states, numeric field, slider, colour picker, asset thumbnails, section headers, dialog, toasts, Stamp inspector, multi-selection inspector |
| `Main.html` | Editor — Land Brush active, zoom dropdown open |
| `Scatter.html` | Editor — Scatter Brush with the Asset picker in Properties |
| `Label.html` | Editor — in-canvas Label editing, font picker open, Layers collapsed to rail |
| `NodeEdit.html` | Editor — node editing, macOS Edit menu open, undo history |
| `Start.html` | Home tab (map browser) |
| `NewMap.html`, `Export.html`, `ExportProgress.html`, `Recovery.html`, `LabelStyles.html` | Dialogs / windows |
| `Responsive.html` | Breakpoints, collapse rules, tab rules, 1×/2× icon pipeline |

## Domain vocabulary (use in all UI copy)

Map, Canvas, Object, Layer, Group (nested Layer), Landmass, Coastline, Lake, Asset, Asset pack, Stamp, Label, Label style, Roughness, Shore style.
Never: project, element, symbol, folder (except real filesystem folders on the Home tab).

## Window anatomy (top → bottom)

```
┌───────────────────────────────────────────────────────────────────────────┐
│ ● ● ●  [⌂] [Isles of Valdren •] [Coast of Brannoch] [Hollowfen …] [+]     │ 30  title bar = tab bar
├──────────┬───────────────────────────────────────────────┬────────────────┤
│ Layers   │                  Canvas                       │ Properties     │
│ 232      │                                               │ 288            │
│ (→ 40    │                                               │                │
│  rail)   │         ┌───────── tool dock ─────────┐  [− 84% +]           │
├──────────┴───────────────────────────────────────────────┴────────────────┤
│ Zoom 84% · X 3,118 Y 1,886 px · 779 × 471 mi · Selection … · Frame 4.1 ms │ 24  status bar
└───────────────────────────────────────────────────────────────────────────┘
```

### Title bar with tabs (30 px)
- Native traffic lights in the left 84 px; tabs start right after. On macOS use a transparent/full-size-content title bar so egui draws the tabs in the title-bar area (eframe `ViewportBuilder`: `with_fullsize_content_view(true)`, `with_title_shown(false)`, `with_titlebar_shown(false)` / transparent titlebar — exact API depends on eframe version). The tab strip must also act as the window drag region.
- No window title text: the active tab identifies the file.
- **Home tab**: 44 px, house icon, always first, can't be closed.
- **Map tabs**: 220 px, flat, 1 px left divider `#2A2E35`. Text ellipsises.
  - Inactive: text `#9D9788`, transparent.
  - Hover: bg `#1D2025`, shows close × (16 px hit area).
  - Active: bg `#23272E`, text `#ECE6D8` 500 weight, 2 px gold `#D4A84A` bottom inset line.
  - Unsaved: 7 px gold dot where the × sits (× replaces it on hover).
- **+** (40 px) = New Tab (⌘T): opens a blank tab with a start page (New Map card, recent Maps, Open…). A Map opened or created there takes that tab. New Map is ⌘N.
- Overflow: tabs shrink 220 → 120 pt, then the strip scrolls with chevrons.
- Opening a Map that's already open focuses its tab. Closing a dirty tab shows the Save prompt (Components.html → Dialog).

### Left: Layers (always)
- 232 px, resizable 200–360. Header: "Layers", New Layer, Group (⌘G). Collapse (⌘⌥1) is a double chevron at the end of the bottom bar (the footer), mirrored to point right when collapsed.
- Collapsed = 40 px rail, icons only: Layers icon and vertical selection path, with the same double-chevron control (now expand) pinned at the bottom (see `Label.html`).
- Tree rows 26 px: chevron, kind icon (Layer/Group, Landmass, Lake, Stamp, Label), name, trailing indicators.
  - Indicators: Clip-to-Landmass (blue ↳ glyph), Depth-sort (↕), opacity % when ≠ 100, Active Layer (gold target icon = where new Objects go), eye / lock on hover or when non-default.
  - States: selected (gold-subtle fill + inset ring), hover, hidden (slashed eye + italic dim name), locked (filled padlock), **locked by parent** (dashed padlock, dim name — not toggleable), renaming (inline field), drag-reorder (gold line between rows), drag-reparent (gold ring around target row).
  - Large Stamp sets: collapse into **summary rows** grouped by Asset ("Pine cluster × 822") plus a "List all 1,496 Stamps…" row. Virtualise the list.
- Footer: Object and Layer counts.

### Right: Properties (always)
Stacks two things:
1. **Active tool section** (tinted bg `#1F1D17`, tool icon + shortcut in header) — only for tools with settings:
   - Land Brush: brush-tip row (Round, Rough, Crag, Shoal, Fjord, Soft), Mode (Paint / Erase ⌥), Size, Smoothing, Paint into (Landmass dropdown incl. New Landmass / New Lake).
   - Scatter Brush (and Stamp): **Asset picker** — search, category dropdown, 4-column thumbnail grid (variant count badge, gold dot = attribution needed), picked Asset's pack · author · licence + attribution notice, then Place on (Land / Water / Any), Density, Spacing, Size jitter, Rotation jitter, Random flip, Variant (Random / Cycle / Fixed).
   - Label: handled by the Label section itself (style dropdown + overrides).
   - Select, Pan, Lasso, Stamp Eraser, Export Region: no tool section.
2. **Selection section(s)** — breadcrumb (`Layer › Object`), then collapsible sections. When nothing is selected, show the **active Layer** (Opacity, Clip to Landmass, Depth-sort).
   - Landmass: Name, Fill, Roughness (Amount, Scale, Seed + Re-roll), Shore style (Outline, Ripples, Inner shade — each inherited from Map default or overridden), Transform (collapsed), Combine (Union / Subtract / Split apart; Union/Subtract need 2+ Landmasses).
   - Label: Text (multi-line), Label style + override count + Reset all / Update style / Manage Label styles…, Font, Size, Tracking, Line height, All caps, Align, Rotation, Fill, Opacity, Halo.
   - Stamp: Scale, Rotation, Flip H/V, Tint, Silhouette fill, Opacity, Shuffle variant (Components.html).
   - Nodes (node-edit mode): count, X/Y (Mixed), Delete, Simplify, display toggles.
   - Multi-selection: shared fields show **Mixed**; scrubbing a Mixed field offsets all relatively, typing sets all.

### Canvas overlays
- **Tool dock**: floating, bottom-centre of the canvas area (never over a panel), 40 px from bottom. Groups: Select V, Pan H │ Land Brush B, Lasso L, Label T │ Stamp S, Scatter Brush ⇧S, Stamp Eraser E │ Export Region X. Active = gold-subtle fill + gold inset ring. Tooltip above after 500 ms: name + shortcut.
- **Zoom control**: bottom-right: − | `84%` | +. The dropdown: editable % field, Zoom to Fit (⇧1), Zoom to Selection (⇧2), 25 / 50 / 100 (⇧0) / 200 / 400 / 800 %.
- **Cursor HUD**: small pill near the cursor during a gesture ("Paint · 48 px → Valdren mainland", "Scatter · Pine cluster · Land only · +212 this stroke").
- Selection = blue `#4FA3E0` bounds + white handles. Brush cursor = white ring over a dark ring. In-progress stroke = gold dashed outline.
- Node editing: blue banner top-centre ("Editing points · 3 of 142 selected · Cancel / Done"), square nodes (selected = filled blue), box-select marquee, + ghost on hovered edge for "double-click to add".

### Status bar (24 px)
Zoom %, cursor X/Y in px plus real-world units, selection summary (or Active Layer), frame time (right).

## Native menu bar (no in-window toolbar buttons)
Undo, zoom and export live in the macOS menu bar (use the `muda` crate or equivalent):
- **File**: New Map ⌘N, New Tab ⌘T, Open… ⌘O, Open Recent ▸, Close Tab ⌘W, Save ⌘S, Save As… ⇧⌘S, Export PNG… ⌘E.
- **Edit**: Undo *<last Command name>* ⌘Z, Redo *<name>* ⇧⌘Z, Undo History… ⌥⌘Z, Cut/Copy/Paste, Delete, Select All ⌘A, Deselect ⇧⌘A. In node mode the items switch to Delete Nodes / Select All Nodes / Finish Node Editing ⏎ / Cancel Gesture Esc (see `NodeEdit.html`).
- **View**: Zoom In ⌘+, Zoom Out ⌘−, Zoom to Fit ⇧1, Zoom to Selection ⇧2, Actual Size ⇧0, Toggle Layers ⌘⌥1, Toggle Properties ⌘⌥2, Hide Panels ⇥, Home ⌘1, Tab 2–9 ⌘2–⌘9.
- **Map**, **Layer**, **Object**, **Window**, **Help** (Credits, Check for Updates).

## Behaviour rules
- One gesture = one undo step; the Edit menu shows the Command name. Esc cancels the gesture in progress.
- Inherited vs overridden is never colour-only: inherited = italic + link glyph; overridden = gold dot + gold label + reset arrow.
- Locked-by-parent is shown with a dashed padlock and cannot be toggled on the child.
- Clip-to-Landmass hides Stamps over water; it never deletes them.
- Assets needing attribution are flagged in the picker and collected into a credits sidecar on export.
- Panels: user-collapsed state persists and wins over auto-collapse.

## Home tab
Left rail (232): All maps / Recent / Open in tabs (counts), Folders (+ Add folder…), Library (Asset packs, Credits & licences), version + What's new.
Main: "All maps" title, search, sort (Last edited ▾), Open…; grid of 4 columns: first card = New Map (dashed), then maps with 512 px previews shown at ~2×, name, edited time, pixel size, "Open in tab" chip for open Maps.
Bottom banner (first launch only): opt-in automatic update checks (Not now / Check automatically).

## Dialogs
- **New Map** — name, preset, W × H with aspect lock, orientation, real-world scale (1 px = n units, Miles/Km/Leagues → computed coverage), template stack (reorderable checklist), Label style set, live preview.
- **Export PNG** — Whole Canvas / Custom region (+ Draw region on Canvas), size by Scale / Target width / Inches × DPI, output size + memory estimate, warning > 16,384 px, hard cap 32,768 px (disable larger options), transparent background, credits sidecar notice, file name + folder.
- **Export progress** — modal with tile progress, time left, Cancel; finished state with Show in Finder / Re-export / Done.
- **Recover unsaved changes** — Last saved vs Recovered previews (changed areas outlined), list of recovered Commands, opens as a copy; Discard / Open Last Saved / Recover Edits.
- **Unsaved changes** — Don't Save / Cancel / Save (Components.html).
- **Label styles** — style list with live previews + editor (Name, Font, Weight, Size, Tracking, Line height, All caps, Fill, Opacity, Halo, Align, Rotation); Delete disabled while in use.

## Responsive
Min window 1024 × 640. ≥1360 full; 1180–1359 Layers narrows to 200; 1024–1179 Layers becomes the 40 px rail (overlay on click); at minimum both panels are rails and Properties opens as a floating inspector. Canvas keeps its zoom centre when panels change. Status bar drops frame time, then real-world coords, below 1180.

## Icons
16 pt grid, 1.5 pt stroke, round caps/joins, drawn in the current text colour. SVG paths are in `Foundations.html` (`icons` array) and inline in every mockup. Pipeline: rasterise with `resvg` at build time into 1× (16 px) and 2× (32 px) atlases; pick by `pixels_per_point()`; tint at draw time. Hand-hint 1× variants where 1.5 pt blurs.

## Fonts
- UI: system (SF Pro on macOS). Fallback Inter/Noto Sans on other platforms.
- Bundled map Label fonts (SIL OFL): Cinzel, IM Fell Double Pica, Almendra, Uncial Antiqua, MedievalSharp. Cinzel also for the brand wordmark only.

## Accessibility
Text ≥ 11 px; contrast ≥ 4.5:1 (ratios in `Foundations.html`); every state has a non-colour cue; all icon-only buttons need accessible labels (see `aria-label`s in mockups for wording).

## Suggested build order
1. Theme + tokens (`theme.rs`), fonts, icon atlas.
2. Window shell: title-bar tabs, three-column layout, status bar, collapsible Layers.
3. Native menus + shortcuts + undo naming.
4. Layers tree (states, drag/drop, summary rows, virtualisation).
5. Properties framework: section headers, numeric scrub fields, sliders, swatches, inherited/overridden/mixed states.
6. Tool dock + per-tool Properties sections; canvas overlays (HUD, selection, zoom control).
7. Home tab, then dialogs.

## Known gaps in the mockups
- Screens are fixed at 1440 × 900; use `Responsive.html` for other sizes.
- Light theme is defined in tokens and sampled in `Foundations.html` only.
- Stamp tool (single placement), Lasso and Export Region have no dedicated screen; follow the patterns above.
