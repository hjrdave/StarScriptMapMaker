# StarScript Map Maker — Design Standard

The rules every UI change must follow. Humans and bots read this before adding or changing any UI.

## How this document works

- **Sources.** Distilled from `docs/design/ui-handoff/` (`HANDOFF.md`, `tokens.json`, `mockups/`, `screens/`) and the app theme in `crates/app/src/theme.rs`. Where this document and the handoff disagree, **this document wins**: it includes decisions made after the handoff (see [Decisions since the handoff](#decisions-since-the-handoff)).
- **Look before you build.** For any screen you touch, open the matching PNG in `docs/design/ui-handoff/screens/` and the HTML in `mockups/` for exact colours, sizes and copy. New UI with no mockup follows the patterns below.
- **Never hard-code design values.** Colours come from the `Palette` in `crates/app/src/theme.rs`, never from `Visuals` and never as literals in widgets. Sizes and spacing come from the scales below.
- **Keeping it current.** When the owner asks for any design change (layout, colour, spacing, copy, icon, behaviour of a UI element), update this document **in the same change**, and update `tokens.json` / `theme.rs` if a token changes. See `CLAUDE.md`. A design change that isn't recorded here didn't happen.
- **Out-of-date mockups.** Do not edit the mockup HTML/PNGs to follow a decision. Record the decision here, under Decisions since the handoff.

## Vocabulary (all UI copy)

Use: Map, Canvas, Object, Layer, Group (nested Layer), Landmass, Coastline, Lake, Asset, Asset pack, Stamp, Label, Label style, Roughness, Shore style.
Never use: project, element, symbol, folder (except real filesystem folders on the Home tab).
See `CONTEXT.md` for definitions.

## Colour

Dark is the default; a light system theme is followed. There is no in-app theme switch. Every colour must have a dark and a light value.

| Token | Dark | Use |
|---|---|---|
| `bg.window` | `#14161A` | Window background |
| `bg.chrome` | `#17191E` | Title bar, status bar |
| `bg.panel` | `#1B1E23` | Side panels |
| `bg.raised` | `#262A31` | Raised surfaces, popovers, dock |
| `bg.field` | `#111317` | Input fields |
| `bg.canvas` | `#0F1114` | Canvas surround |
| `bg.toolSection` | `#1F1D17` | Active-tool section tint in Properties |
| `border.subtle` / `border.strong` | `#272B32` / `#3B414B` | Dividers / emphasised borders |
| `text.primary` | `#ECE6D8` | Main text |
| `text.secondary` | `#B4AE9F` | Secondary text |
| `text.tertiary` | `#9D9788` | Hints, inactive tabs |
| `accent.gold` | `#D4A84A` | Active/selected accent, overridden values |
| `accent.goldText` | `#F0CF7A` | Gold text |
| `accent.goldSubtle` | `#3A311C` | Selected fill |
| `accent.goldRing` | `#8A6C2C` | Selected inset ring |
| `overlay.select` | `#4FA3E0` | Selection bounds, node editing |
| `overlay.clip` | `#7FB6E6` | Clip-to-Landmass glyph |
| `status.warn` / `danger` / `ok` | `#F2B65A` / `#F2A093` / `#8FD49A` | Status |

Light values, tab colours and the hover fill/stroke are in `tokens.json` and `theme.rs`. Contrast must be ≥ 4.5:1 for text. Colour is never the only cue for a state (see Behaviour rules).

## Type

UI font: system (SF Pro on macOS), with Inter / Noto Sans as fallback; JetBrains Mono for numerics. Map Label fonts (Cinzel, IM Fell Double Pica, Almendra, Uncial Antiqua, MedievalSharp) are for map labels only; Cinzel is also for the brand wordmark only.

| Style | Size / line height | Weight |
|---|---|---|
| Title | 15 / 20 | 600 |
| Heading | 13 / 18 | 600 |
| Body | 12 / 16 | 400 |
| Caption | 11 / 14 | 400 (**minimum text size is 11**) |
| Mono | 11 / 15 | 400 |

## Spacing, radii, elevation

- **Spacing scale (px):** 2, 4, 6, 8, 12, 16, 20, 24, 32. Use only these.
- **Radii:** chip 3, field 4, button 6, popover 8, dock 10, dialog 12.
- **Elevation:** e0 border only; e1 (HUD, tooltip) y4 blur12 α.45; e2 (popover, menu, tool dock) y8 blur24 α.5; e3 (dialog) y24 blur60 α.6; scrim `rgba(8,9,11,0.55)`.

## Key metrics (px)

Title bar with tabs 38 · traffic-light inset 84 · Home tab 44 · Map tab 220 (min 120) · New-tab button 40 · status bar 24 · panel header 34 · layer row 26 · field height 22–26 · Layers panel 232 (200–360, rail 40) · Properties panel 288 (260–360) · tool button 32×30 · tool dock bottom offset 40 · minimum window 1024×640.

## Icons

16 pt grid, 1.5 pt stroke, round caps and joins, drawn in the current text colour; 14 px in rows. SVG paths are in `mockups/Foundations.html`. Every icon-only button needs an accessible label and a tooltip. Each tool has its own distinct icon; two tools never share one.

## Window anatomy

Title bar (tabs) → Layers panel | Canvas | Properties panel → status bar. Details, tab states and responsive rules are in `HANDOFF.md` and `mockups/Responsive.html`.

- **Layers panel** (left, always): the single place to manage Layers. See [Layers panel](#layers-panel).
- **Properties panel** (right, always): active tool section (only for tools with settings), then selection sections.
- **Floating controls** sit on the same baseline at the bottom of the canvas area (never over a panel): tool dock bottom-centre, zoom control bottom-right, both 40 px from the bottom and vertically centred on each other.
- **No in-window toolbar buttons** for undo, zoom or export: those live in the native menu bar.

### Tool dock

Floating, bottom-centre of the canvas area. Groups: Select V, Pan H │ Land Brush B, Lasso L, Label T │ Stamp S, Stamp Eraser E │ Export Region X. Active tool = gold-subtle fill + gold inset ring. Tooltip above the button after 500 ms: name + shortcut, on **one line** (no wrapping).

### Zoom control

Bottom-right: `−` | percentage | `+`. The percentage button shows the percentage only, with no extra icon. Its dropdown offers an editable % field, Zoom to Fit, Zoom to Selection, and the preset levels.

### Alerts (toasts)

Toasts appear at the **top** of the window, top-centre, and slide in from the top edge; multiple toasts stack downward. They never cover the tool dock or zoom control.

## Layers panel

The Layers panel is where every Layer action happens. There is no separate layer-properties menu.

- **Per-row actions:** lock, delete, move (drag to reorder), rename (double-click, inline; Enter commits, Esc cancels, empty falls back to the default name), visibility; the same actions are in the row's context menu.
- **Groups:** Layers nest; a Group is a nested Layer. Groups are collapsible, can be dragged into and out of, and can be renamed, locked, hidden, deleted and reordered like any Layer. Locking or hiding a Group applies to everything inside it.
- **Linking:** layers can be linked so a move or transform of one applies to the others. Linked rows show a link glyph. Linking is separate from grouping.
- **Row states:** selected (gold-subtle fill + inset ring), hover, hidden (slashed eye + italic dim name), locked (filled padlock), locked by parent (dashed padlock, dim name, not toggleable), renaming (inline field), drag-reorder (gold line between rows), drag-reparent (gold ring around the target row).
- Rows show the kind icon of the Object they hold. Large Stamp sets collapse into summary rows grouped by Asset.

## Properties panel

- **Active tool section** (tinted `bg.toolSection`, tool icon + shortcut in the header) only for tools with settings.
- **Stamp tool** (one tool for placing and scattering):
  - A **click** places one Stamp; a **drag** scatters Stamps along the path. There is no separate Scatter tool.
  - Settings: Asset picker (search, category, 4-column thumbnail grid, pack · author · licence), Place on (Land / Water / Any), Density, Spacing, Size jitter, Rotation jitter, Random flip, Variant (Random / Cycle / Fixed).
- **Selection sections** start with a breadcrumb (`Layer › Object`), then collapsible sections.
- **Landmass:** Name, Fill, Roughness (Amount, Scale, Seed, Re-roll), Shore style (Outline, Ripples, Inner shade), Transform, Combine.
- **Label / text:** Text, Label style, Font, Size, Tracking, Kerning, Line height, Arch, All caps, Align, Rotation, Fill, Opacity, Halo.
- Multi-selection: shared fields show **Mixed**; scrubbing offsets all relatively, typing sets all.

## Behaviour rules

- One gesture = one undo step; the Edit menu shows the Command name. Esc cancels the gesture in progress.
- Inherited vs overridden is never colour-only: inherited = italic + link glyph; overridden = gold dot + gold label + reset arrow.
- Locked-by-parent is shown with a dashed padlock and can't be toggled on the child.
- Clip-to-Landmass hides Stamps over water; it never deletes them.
- Every state has a non-colour cue. Text ≥ 11 px.
- User-collapsed panel state persists and wins over auto-collapse.
- Interactions stay smooth with heavy content: dragging, zooming and panning must not recompute expensive generated detail (e.g. rough Coastlines) every frame.

## Decisions since the handoff

Changes the owner has decided after the handoff. The sections above already reflect them; this list records why and where it differs from the mockups. "Open" means the app has not caught up yet.

| Decision | Differs from handoff | Issue | Status |
|---|---|---|---|
| Tool dock and zoom control share a baseline and vertical centre | Handoff gave offsets separately | #202 | Open |
| Tooltips never wrap | — | #203 | Open |
| Alerts come from the top, not the bottom | Handoff did not place toasts | #204 | Open |
| Layer management is entirely in the Layers panel (lock, delete, move, rename, groups, linking); no separate layer-properties menu | Handoff had Layer-level fields in Properties | #205, #206 | Open |
| Stamp and Land Brush have distinct, self-explanatory icons (stamp icon, land icon) | Handoff reused one icon | #208 | Open |
| A Water brush for lakes and rivers is a tool | Handoff only had Lake via the Land Brush's "Paint into" | #209 | Open |
| Zoom button shows the percentage only | Mockup showed a trailing icon | #211 | Open |
| Text gets Kerning, Tracking, Line height and Arch | Handoff had Tracking and Line height only | #213 | Open |
| Landmass Roughness stays smooth to drag at high values | — | #220 | Open |
| One Stamp tool: click places, drag scatters; no separate Scatter Brush | Handoff had Stamp and Scatter Brush as separate tools | #219 | Open |

## Checklist for any UI change

1. Opened the matching `screens/*.png` and mockup, or followed the patterns above if none exists.
2. Used `Palette` colours and the spacing, radius and elevation scales. No literals.
3. Used the domain vocabulary for all copy.
4. Icons follow the icon rules; icon-only buttons have labels and tooltips.
5. Every state has a non-colour cue; text ≥ 11 px; contrast ≥ 4.5:1 in dark and light.
6. Works at the minimum window size (1024×640) and with panels collapsed.
7. One gesture = one undo step.
8. If the owner asked for a design change, this document (and `tokens.json` / `theme.rs` if needed) is updated in the same change.
