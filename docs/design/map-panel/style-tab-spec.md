# Map panel, Style tab: spec for the egui build

Status: proposed. Issue #565, item 4. Reference: `prototype-style-tab.html`, **variant A**
(open it with `?variant=A`).

## Why earlier builds did not look like the prototype

The first prototype was drawn in its own CSS sizes and colours, not the app's. Fields were 32 tall
(the app's are 22), body text was 14 (the app's is 12), the label column was 112 wide (the app's is
80), the gutter was 16 (the app's is 8), and the colours were close but not the palette. Building
those numbers in egui gives a different-looking panel, and building the app's numbers gives a
denser one than the first prototype showed. The prototype has been redrawn with the app's tokens, so
**what it shows now is what egui should produce**. Where this spec and the prototype disagree, fix
the prototype.

Units: 1 egui point = 1 CSS px at 100% in the prototype. Every number below is a point.

## Tokens, in one place

All values are existing constants in `apps/starscript/src/chrome/theme.rs`. Use the constant, never
the number, and never a literal colour.

| Use | Token | Value |
| --- | --- | --- |
| Panel title | `theme::TITLE` | 15 |
| Tab names, section headings | `theme::HEADING` | 13 (headings `.strong()`) |
| Labels, values | `theme::BODY` | 12 |
| Group captions, hex text, card names | `theme::CAPTION` | 11 |
| Number box text | `theme::MONO` | 11 |
| Row gap | `ui.spacing().item_spacing.y` | 6 |
| Gap in a wrapped row | `item_spacing.x` | 8 |
| Spacing steps | `theme::SPACING[1]`, `[2]`, `[3]` | 4, 6, 8 |
| Field height | `metrics::FIELD[0]` | 22 |
| Panel header | `metrics::PANEL_HEADER` | 34 |
| Field radius | `theme::RADIUS_FIELD` | 4 |
| Card, button radius | `theme::RADIUS_BUTTON` | 6 |
| Chip, swatch radius | `theme::RADIUS_CHIP` | 3 |
| Panel fill | `palette.bg_panel` | #1B1E23 |
| Field fill | `palette.bg_field` | #111317 |
| Dividers, field border | `palette.border_subtle` | #272B32 |
| Primary text | `palette.text_primary` | #ECE6D8 |
| Labels | `palette.text_secondary` | #B4AE9F |
| Captions | `palette.text_tertiary` | #9D9788 |
| Selected card ring, tab underline | `palette.gold` | #D4A84A |

## Layout, top to bottom

Content gutter is whatever the panel's own frame gives (about 8). Nothing in this tab adds its own
left or right margin.

1. **Header.** `shell::icon_header(ui, Icon::Map, palette.text_primary, "Map", |_| {})` in
   `app/panels.rs`. Height 34, icon 16, title 15.
2. **Tab bar.** `prop_widgets::tab_bar`, unchanged: 30 tall, names at 13, gold 2 px underline.
3. **Section "Background".** `divider_section`, unchanged: 6 above, heading 13 strong, a 1 px
   `border_subtle` line, 6 below.
   - **Caption "Template"**, then the template cards.
   - **Caption "Water"**, then one row: label **Colour**, a swatch.
   - **Caption "Texture"**, then **Strength | Scale** inline on one row, then **Light** and **Dark** as
     two separate rows (one swatch each).
4. **Section "Roughness".** The same `divider_section`, with 14 above it (the 6 the section adds
   plus 8 of its own) so the two sections read as separate. Rows: **Amount | Scale** inline, then
   **Seed** with the dice button.

### Group caption (new helper)

`fn group_caption(ui, text)` in `panels/properties/mod.rs`, next to the existing `caption`:

- 8 above (`SPACING[3]`), 4 below (`SPACING[1]`).
- `theme::CAPTION`, `palette.text_tertiary`, **sentence case, no letter-spacing, not uppercase**.
  egui cannot track letters, and the prototype no longer does.
- Painted text, like `named_above`, so it adds no accessible node.

### The label column (the main cause of misalignment)

`prop_widgets::label_width` returns `max(text width + 8, LABEL_MIN)`. A label wider than 72 (for
example "Texture strength", about 85) widens **that row's** label column, so its field starts
further right than the rows around it. That is the ragged left edge in the screenshot.

- Make the label column a fixed width: `LABEL_COLUMN = 80.0`. `label_width` returns it.
- A label that does not fit is cut with an ellipsis and its full text shows as a tooltip.
- Rename the visible labels so none is cut: **Colour**, **Light**, **Dark**; the numeric fields carry
  **Strength**, **Scale**, **Amount**, **Scale** and **Seed** as in-box prefixes. The group caption supplies the context ("Water", "Texture").
- The accessible names stay "Water colour", "Texture strength", "Texture scale", "Texture light" and
  "Texture dark", so assistive tools and the existing tests still find them. Add a `display`
  label to `swatch` and `ScrubSpec` (`.shown_as("Strength")`) rather than changing the name.
- This fixes every panel that uses `field_row`, `swatch` and `scrub_field`, not only the Map panel.
  Check the Properties panel for any label that now truncates.

### Template cards

- `look_card` stays as it is: 84 × 48, radius 6, caption 11 in the bottom left, a 2 px `gold` ring
  when it is the Map's current look, else 1 px `border_subtle` (`border_strong` on hover).
- Laid out with `horizontal_wrapped`, `item_spacing = (8, 8)`. Three fit a 320 panel
  (3 × 84 + 2 × 8 = 268) and they wrap to two per row below about 190 of content width.
- **Drop the "Template: Ocean" row** above the cards. The ring says which card is the current look.

### Rows

Every row is `[80 label column][8 gap][control that fills the rest]`, `metrics::FIELD[0]` (22) tall.

- **Inline pairs** (Strength | Scale, Amount | Scale): `prop_widgets::pair`, two equal columns with
  the 8 pt gap, no label column. Each field carries its name **inside the box at the left**, in
  `text_tertiary` 12, and the value right-aligned in mono 11, as W / H do on the Canvas tab. This
  needs one addition to `ScrubSpec`: `.prefix("Strength")`, drawn inside the box (the existing
  `Handle` is a 22 pt icon-sized slot and is too small for a word). The box still scrubs and types as
  before; the prefix is painted, so the field keeps its accessible name. Seed is the same kind of
  field (prefix "Seed"), **half the row wide like Amount**, with the 24 × 22 dice button in the
  other half, left-aligned against the field with an 8 gap. So Roughness is two rows on the same
  two-column grid: Amount | Scale, then Seed | dice, and has no label column.
- **Number fields** (`scrub_field`): fill the width to the right of the label column, so every
  right edge lines up with the panel's content edge. The `u` unit sits inside the box at the right.
- **Colour swatch**: a 44 × 22 chip (1 px `border_subtle`, radius 3) followed by the hex in
  caption 11, `text_secondary`. No box around the hex; this is the existing `swatch` widget.
- **Light and Dark**: two separate rows, each a label and one swatch. They are not paired, so a
  narrow panel never has to hide a hex value.
- **Seed**: a number field half the row wide (the left column of `pair`, as Amount), then a 24 × 22
  dice button (`Icon::ReRoll`, 16 pt, radius 6) at the start of the right column. The dice is an icon
  button, not a field.

## Behaviour that changes

- The "Template" row is gone. Today it names the template the Map was made from, and the cards
  highlight only when the look equals the current background. Once a field is edited, no card is
  highlighted and the original template name is no longer shown anywhere. **Decision needed:** accept
  that, or show "Made from Ocean" as a caption under the cards (primary board only).
- No Command changes. Every edit still goes through `self.live.apply(...)` and
  `Command::SetBackground` / `SetBoardBackground`.

## Narrow panels

The Map panel defaults to 320 (`MAP_PANEL_DEFAULT`) and can be dragged to 480. Down to the smallest
width it can take, nothing in this tab may be cut off:

- Label column stays 80; fields shrink (a number field does not go under 60).
- Template cards wrap.
- Nothing is wider than `ui.available_width()`; use `set_max_width` the way `field_row` does.

## Checking the build against the prototype

Do this before calling it done, in the running app and in the prototype at the same width:

1. Set the panel to 320 and 240. Screenshot both.
2. Open `prototype-style-tab.html?variant=A`, set the width slider to the same value, and screenshot.
3. Compare, in this order: left edge of every label, left edge of every field, right edge of every
   field, row height, gaps between rows and groups, caption size. They must match to the point.
4. Add a test in `panels/properties/tests.rs` that lays the tab out at several widths with the
   egui test harness and asserts that no widget's rect is wider than the panel and that every row's
   field starts at the same x.

Nothing here can be checked in the cloud container: it has no GPU and cannot open a window, so
steps 1 to 3 need a machine with a display.

## Not in this spec

- The Canvas and Shores tabs (they should adopt the same label column and rows; see #565).
- The Properties panel's sections.
- Light theme values: use the palette, and the light theme follows.

## Shores tab (proposal, variant not yet chosen)

Reference: `prototype-shores-tab.html` (`?variant=A` or `?variant=B`). Same tokens and the same label
column as above. The code is `shores_tab` in `panels/properties/map.rs` and `shore_fields` in
`panels/properties/object_style.rs`. **`shore_fields` is shared with the Landmass and Lake
properties**, so everything below also changes those panels; check them.

What is wrong now:

- The heading says "Landmass shore" or "Lake shore" and the only way to switch is two bare icons at
  its right, which do not say which is which.
- Labels run long ("Line width", "Line colour", "Shade width", "Shade colour"), so the label column
  widens on those rows and the colour chips start further right than the number fields.
- The group captions ("Outline", "Ripples", "Inner shade") have no space above them, so the groups
  run together.
- Ripples is five rows (Count, Spacing, Line width, Line colour, Fade) and the tab is one long list.
- A colour with transparency (Line colour) shows a checkerboard chip with no sign of its opacity.

Changes in both variants:

- Heading **"Shore"** (`divider_section`), then a full-width **Landmass | Lake** switch under it:
  `prop_widgets::segmented`, 22 tall, with the tool icons (`LandBrush`, `LakeBrush`, 14 pt) before
  the names. Replaces the icon-only `icon_switch` in the header. The two kinds keep their own values.
- Group captions use `group_caption` (8 above, 4 below, caption 11, `text_tertiary`).
- Labels shrink to what the caption already says: **Width**, **Colour**, **Count**, **Spacing**,
  **Fade**. Accessible names stay "Outline width", "Outline colour", "Ripple count", "Ripple spacing",
  "Ripple line width", "Ripple line colour", "Ripple fade", "Shade width", "Shade colour", through the
  `shown_as` display label above.
- Swatches are the existing 44 × 22 chip with the hex beside it. When the colour is translucent, show
  its opacity after the hex in the same text ("#FFFFFF 60%").

Variant A (rows, like the Style tab): the rows above in one label column, 11 rows. Consistent with
the Style tab; longest.

Variant B (paired fields): `prop_widgets::pair` with `named_above`, the field's name above it in
caption 11. Outline: Width, Colour. Ripples: Count, Spacing; Width, Fade; Colour. Inner shade:
Width, Colour. 6 rows; fits without scrolling. A colour that does not fit beside its partner at a
narrow width drops to its own row.

Variant C (inline pairs, the Style tab's rule): every number is a field with its name inside the
box, two to a row, and a lone number takes half a row; every colour is a label row with the 80 pt
column. Outline: Width (half), Colour. Ripples: Count | Spacing, Width | Fade, Colour. Inner shade:
Width (half), Colour. That is 7 rows against A's 11, and it follows the Style tab exactly (Strength |
Scale, Amount | Scale, Seed at half width, colours as label rows), so a narrow panel never has to
hide a hex. It uses the same `.prefix(...)` addition to `ScrubSpec` as the Style tab.

**Chosen: C.**

## Canvas tab

Reference: `prototype-canvas-tab.html`. Same tokens, group captions and 80 pt label column as the
Style tab. The code is `canvas_fields` and `anchor_grid` in `panels/properties/map.rs`, and the
Canvas section in `map_fields`. `canvas_fields` is also used for Art boards (without the Scale
group), so check that panel.

### Why it is cut off now

After the W | H row, `canvas_fields` calls `ui.columns(2, ...)` and puts **Scale** in the left
column and **Anchor** in the right. Each is the 64 pt `row` helper: a 64 pt label plus its content.
At 320 each column is about 146, so:

- the Scale text ("0.25 mi per unit", about 80) needs 64 + 80 and runs into the Anchor label;
- the Anchor grid needs 64 + 3 × 18 + 2 × 2, plus the item spacing, and the third column is cut off.

At narrower widths it only gets worse. Two things sit side by side that cannot both shrink.

### Layout

Heading "Canvas" (`divider_section`), then:

1. **Caption "Size"**, then one row: `[W field][24 chain][H field]`. This is the existing row, which
   already works: 22 tall, `item_spacing.x = 2`, each field half of the room left after the 24 pt
   chain, the "W" and "H" shown as in-box prefixes. Keep it. The chain is `Icon::Link` (gold) /
   `Icon::Unlink`.
2. **Caption "Scale"**, then a row: label **Distance** (80 pt label column), the value
   ("0.25 mi per unit") in `text_primary` 12, which wraps if it must. Left out for an Art board,
   which has no Scale.
3. **Caption "Resize from"**, then a row: label **Anchor** (80 pt column), and in the control column
   the 3 × 3 grid, then a caption line under it that names the point that stays put ("Top left stays
   put when the size changes", `text_tertiary` 11, wrapping, 4 above).

Nothing is side by side except W | H and the chain, which already shrink. The Scale and Anchor rows
use the same label column as the rest of the panel, so they line up with the other tabs.

### Anchor grid

- Buttons **22 × 22** (they are 18 × 18 now), 2 pt gap, so the grid is 70 × 70. Radius
  `RADIUS_FIELD`.
- Off: `bg_raised` fill, `border_subtle`. On: `gold_subtle` fill, `gold_ring` outline, and a gold dot
  (the existing "●"). Hover: `hover_fill`.
- Each button keeps its name from `anchor_name` for the tooltip and the accessible name, and is a
  `RadioButton` in a group named "Anchor".
- The grid is fixed-size, so the control column must be at least 70 wide. The label column is 80 and
  the gutter 8, so that holds down to a panel width of about 180.

### Also

- The W and H labels in the fields stay. Their accessible names stay "Canvas width" and "Canvas
  height" ("Board width" and "Board height" for an Art board).
- No Command changes: resizing still goes through `resize(...)` and `self.live.apply(...)` as one
  gesture, with the anchor held for the whole drag.

## Tool panel: the mode control

Reference: `prototype-tool-modes.html` (the "After" panel, with "Now" beside it). The code is
`prop_widgets::mode_buttons`, called from `chrome/tool_section.rs` (about line 216) for the Land
Brush (Paint, Paint Lasso, Erase), the Brush, and the other tools with modes.

Now: three icon-only buttons, 40 × 32, with a tooltip each. They are the only control in the panels
that shows its options as unlabelled icons, and they do not look like the Landmass | Lake switch on
the Shores tab.

After: the same labelled switch as the Shores tab (`prop_widgets::segmented`, extended with an icon
before each name):

- Full width, one segment per mode, equal widths, **22 tall** (`metrics::FIELD[0]`), no gap between
  segments, `RADIUS_FIELD` on the outer corners only, 1 px `border_subtle` between segments.
- Each segment is the 14 pt icon, 6 gap, then the name at `BODY` 12. Content centred. A segment that
  is too narrow for icon and name (below about 70 each) drops the name and keeps the tooltip.
- Off: `bg_field` fill, `text_secondary`. Hover: `hover_fill`, `text_primary`. On: `bg_raised` fill,
  `text_primary`, a 1 px `gold` ring.
- The **erasing** mode, when on, is ringed and tinted in `palette.danger` (text and icon too), as
  `mode_buttons` does today. Its `danger` flag moves into the new option type.
- Each segment is a `RadioButton` named for the mode; the switch is a group named "Mode".
- Below it: the hint ("Hold Option to get the other mode."), `CAPTION` 11, `text_tertiary`, 8 above.
  Then the tool's sections as before.
- The mode chip at the right of the header ("Paint", "Erase") is unchanged.

One widget now serves both places: the Shore switch is this control with two segments, so
`icon_switch` goes away once it is replaced.
