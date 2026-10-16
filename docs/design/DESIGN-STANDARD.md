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

### Tab bar alignment

The macOS window buttons (close, minimize, zoom) are **vertically centered** in the tab bar, on one shared center line with the tab labels, icons, close buttons and the `+` button. Tabs are short: the tab bar is 38 px (the native title-bar row height) unless the buttons can't be centered at that height, so the buttons never sit above the middle of the tabs. The strip stays a window drag region.

### Pop-out windows

Tab dragging works like Chrome. A tab can be popped out into its own window (drag it out of the tab strip, or right-click ▸ Move Tab to New Window) and popped back in by dragging it onto another window's tab strip, where an insertion gap shows and the tab snaps in on release (or right-click ▸ Move Tab to Window ▸). A window left with no tabs closes. Every window is a full editor with its own title bar, tab strip, panels and status bar; the native menu bar acts on the focused window. A Map is open in only one place at a time. Assets and layers can be dragged from one window to another (drop lands where the mouse is, with a clear drop highlight on the target window). Windows share one app process (shared settings, recent maps and Asset packs) but keep their own panel sizes, zoom, selection and undo history.

### Closing windows and quitting (macOS)

On macOS, closing a window (red button) never quits the app, as in Photoshop. The app stays in the Dock with the menu bar active; only ⌘Q / StarScript ▸ Quit (or Dock ▸ Quit) quits. Closing a window with unsaved Maps shows the Save / Don't Save / Cancel prompt first. Clicking the Dock icon, or launching the app again, with no window open opens a new window on the Home tab. With no window open, New Map, Open… and Open Recent stay enabled. Windows and Linux are unchanged: closing the last window quits.

### Art boards

The last dock button (crop icon) is the **Art board tool**: it selects a board, shows resize handles, and resizes or moves it (with a live size HUD and exact size in Properties). Export Region is no longer a dock tool; it is the "Draw region on Canvas" action in the Export dialog.

A Map can have several art boards in one shared workspace. One board is the **primary board** (marked with a badge and label) and is the only one exported; other boards are staging areas for assets and work in progress and are never included in the export. Objects can be moved or copied between boards. Objects can be dragged off a board onto the surrounding pasteboard and stay visible and editable there; they are never clipped, and **canvas size never affects layer position**: resizing a board or the Canvas, from any edge, never moves, clips or deletes objects (they overflow onto the pasteboard instead, and come back if the board grows again). Only the part of the primary board's area is exported. The name "board" is provisional until the glossary decision in #239 is made.

### New tab

The **+** button (and ⌘T) opens a blank **New Tab** start page, not the New Map dialog. It is a quick "recent" screen: the **last 6** Maps as thumbnail cards ordered by **most recently updated**, a **Create new map** card, and an **Open a file** button for the file system. Picking any of these opens or creates the Map in that tab, and the tab then takes the Map's name. It has no side rail, search or sorting; the full list lives on the Home tab. A blank New Tab has nothing to save, so closing it shows no Save prompt. With no recent Maps, show only Create new map and Open a file.

### Map cards (Home tab and New Tab)

Cards follow Figma's dashboard file card: a rounded, bordered card with a large **thumbnail** across the top, a divider, then the Map **name** (bold) and **"Edited <relative time>"** (dim) in the footer. No icons on the card face: no file-type icon beside the name and no avatar. Extra details (pixel size, "Open in tab") go on hover or in the card menu, not on the card face. Each card has a right-click menu and a hover "…" button with **Delete** and **Show in Finder** (per platform: File Explorer / File Manager). Delete always asks for confirmation in a dialog that names the Map, says it moves to the Trash, defaults to Cancel, and notes if the Map is open in a tab. A Map whose file is missing shows a "missing" state with Show in Finder disabled and a "Remove from list" action.

### Asset packs page (Home tab)

The Home tab's Library section has an **Asset packs** page. It lists every installed pack (name, preview thumbnails, author, licence, Asset count, built-in or user-added); opening a pack shows its Assets in a searchable thumbnail grid. **Add pack…** installs a pack through the native file picker, with progress and a clear error on failure. User-installed packs can be deleted from the card menu after a confirmation dialog that names the pack and says if maps use it. **Packs embedded in the app cannot be deleted**: they carry a "Built in" badge and the Delete action is hidden or disabled with a tooltip explaining why.

### Asset thumbnails

Every Asset thumbnail (Asset picker, Asset packs page, anywhere Assets are listed) sits on a **white** background so the Asset is easy to see. If the Asset itself is white or very light, use an **off-white** background instead. The background is chosen from the Asset's pixels and cached with the thumbnail, and both colours are named tokens, not literals. Gold selection rings and badges must stay readable on top of it. Placed Stamps on the Canvas are unaffected.

### Templates (New Map)

The New Map dialog has a **template picker**: thumbnail cards with a name (same card style as map cards, without the delete and Show in Finder actions) and a live preview. Each template sets the background look (procedural ocean or flat colour), land fill, shore styles and starter Label styles; everything can be changed afterward. The reorderable template stack (backdrop, land, assets, labels) still applies on top of the chosen template. Built-in templates cannot be deleted.

### Tool dock

Floating, bottom-centre of the canvas area. Groups: Select V, Pan H │ Land Brush B, Lasso L, Label T │ Stamp S, Stamp Eraser E │ Art board (crop icon; shortcut to be decided, see #240). Active tool = gold-subtle fill + gold inset ring. Tooltip above the button after 500 ms: name + shortcut, on **one line** (no wrapping).

### Zoom control

Bottom-right: a single dark rounded pill (subtle border, elevation e2) in three sections separated by thin vertical dividers: `−` | percentage | `+`. The percentage is plain text in the middle section, with no raised inner button and no extra icon. The pill keeps a fixed width so it doesn't change size as the percentage changes. Its dropdown offers an editable % field, Zoom to Fit, Zoom to Selection, and the preset levels. ⌘+ (also ⌘=) and ⌘− zoom the **Canvas only**; they never scale the UI.

### Alerts (toasts)

Toasts appear at the **top** of the window, top-centre, and slide in from the top edge; multiple toasts stack downward. They never cover the tool dock or zoom control.

## Layers panel

The Layers panel is where every Layer action happens. There is no separate layer-properties menu.

- **Per-row actions:** lock, delete, move (drag to reorder), rename (double-click, inline; Enter commits, Esc cancels, empty falls back to the default name), visibility; the same actions are in the row's context menu.
- **Groups:** Layers nest; a Group is a nested Layer. Groups are collapsible, can be dragged into and out of, and can be renamed, locked, hidden, deleted and reordered like any Layer. Locking or hiding a Group applies to everything inside it.
- **Linking:** layers can be linked so a move or transform of one applies to the others. Linked rows show a link glyph. Linking is separate from grouping.
- **Row states:** selected (gold-subtle fill + inset ring), hover, hidden (slashed eye + italic dim name), locked (filled padlock), locked by parent (dashed padlock, dim name, not toggleable), renaming (inline field), drag-reorder (gold line between rows), drag-reparent (gold ring around the target row).
- **Multi-select:** drag a marquee on the Canvas, or ⌘-click / Shift-click rows. Canvas and panel selection stay in sync. Right-clicking a multi-selection (Canvas or panel) opens a context menu with Copy, Cut, Group, Paste and Delete, matching the menu-bar shortcuts. Layers can be copied and pasted between tabs, keeping structure, names, visibility and lock state; each paste and each cut is one undo step. Pasted layers land **exactly at the mouse cursor** (the centre of their combined bounds under the cursor, converted through the target tab's zoom and pan), falling back to the centre of the visible Canvas when the cursor isn't over it.
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
| Zoom control is one pill with `−` / percentage / `+` in three sections with dividers, no icon | Mockup showed a raised percentage button with a trailing icon | #211 | Open |
| Text gets Kerning, Tracking, Line height and Arch | Handoff had Tracking and Line height only | #213 | Open |
| Landmass Roughness stays smooth to drag at high values | — | #220 | Open |
| Map cards are Figma-style (thumbnail, name, edited time) with Delete (confirmed) and Show in Finder | Handoff cards also showed size and an "Open in tab" chip on the face | #228 | Open |
| Home tab gets an Asset packs page: browse, add via GUI, delete user packs; built-in packs can't be deleted | Handoff listed Asset packs in the Library rail without a page spec | #229 | Open |
| Asset thumbnails use a white background, off-white for white Assets | Handoff used the dark panel as the thumbnail background | #231 | Open |
| Tabs are shorter so the macOS window buttons are vertically centered, like Figma's | Handoff gave the bar height but not the button alignment | #232 | Open |
| Multi-select with context menu (copy, cut, group) and copy/paste between tabs | Handoff had no selection context menu or cross-tab paste | #235 | Open |
| Tabs can be popped out into separate windows; Assets and layers can be dragged between windows | Handoff had a single window with tabs | #236 | Open |
| macOS: closing the window doesn't quit the app; ⌘Q quits; Dock click reopens | Handoff didn't specify close vs quit | #238 | Open |
| The crop button is the Art board tool (select, resize, move boards); Export Region moves into the Export dialog | Handoff: the dock's last tool was Export Region (X) | #240 | Open |
| A Map can have multiple art boards; only the primary board is exported | Handoff had one Canvas per Map | #239 | Open |
| Multiple map templates (background looks) with a template picker in New Map | Handoff had one starter template and a template stack only | #241 | Open |
| New tab opens a start page (last 6 Maps by most recently updated, Create new map, Open a file) instead of the New Map dialog | Handoff: `+` = New Map | #223 | Open |
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
