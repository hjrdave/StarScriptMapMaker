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
   - **Caption "Texture"**, then rows: **Strength**, **Scale**, and **Light / dark** (two swatches
     side by side).
4. **Section "Roughness".** The same `divider_section`, with 14 above it (the 6 the section adds
   plus 8 of its own) so the two sections read as separate. Rows: **Amount**, **Scale**, **Seed**
   with the dice button.

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
- Rename the visible labels so none is cut: **Colour**, **Strength**, **Scale**, **Light / dark**,
  **Amount**, **Scale**, **Seed**. The group caption supplies the context ("Water", "Texture").
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

- **Number fields** (`scrub_field`): fill the width to the right of the label column, so every
  right edge lines up with the panel's content edge. The `u` unit sits inside the box at the right.
- **Colour swatch**: a 44 × 22 chip (1 px `border_subtle`, radius 3) followed by the hex in
  caption 11, `text_secondary`. No box around the hex; this is the existing `swatch` widget.
- **Light / dark**: one label, then two swatches in `ui.columns(2)` (`prop_widgets::pair`). Each half
  is chip plus hex. When a half is narrower than chip + 6 + hex (about 100), hide the hex text and
  keep it in the tooltip; at that width the row never overflows.
- **Seed**: a number field that fills the row minus the button, then a 24 × 22 dice button
  (`Icon::ReRoll`, 16 pt, radius 6) after an 8 gap. The dice is an icon button, not a field.

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
- Light / dark hides its hex text, as above.
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
