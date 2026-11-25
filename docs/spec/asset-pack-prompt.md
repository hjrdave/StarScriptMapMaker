# Spec: making an Asset pack for StarScript Map Maker

Give this file to the tool that makes the pack. The output must follow it exactly, or the app
won't load the pack. The format is defined in `crates/core/src/asset/mod.rs`. A complete example
is `assets/packs/default/pack.toml`.

## What to make

A folder of map Stamps for drawing fantasy maps: mountains, hills, trees, towns, castles, ruins
and other features, drawn in one style. The folder holds a `pack.toml` manifest and the image
files.

```
<pack-id>/
  pack.toml
  art/            one image per Asset (SVG preferred, or PNG)
  textures/       optional: seamless PNG textures for Landmass fills
```

Leave out the `silhouettes/` folder. It is generated afterwards (see the end of this file).

## pack.toml

```toml
[pack]
id = "my-pack"                 # required
name = "My Pack"               # required, shown in the app
licence = "CC0-1.0"            # required, the default licence for every Asset
credits = "Art by …"           # required, shown wherever the pack is credited
licence_url = "https://…"      # optional
attribution_required = false   # optional; true means exports must credit the pack
author = "…"                   # optional default for every Asset
source_url = "https://…"       # optional default for every Asset
modified = "2026-11-25"        # optional default for every Asset

[[asset]]                      # one table per Stamp
id = "mountain-1"              # required, unique among the pack's Assets
name = "Mountain"              # required, shown in the Asset browser
category = "terrain"           # required
tags = ["peak", "rocky"]       # optional, used by search
file = "art/mountain-1.svg"    # required, relative to the pack folder
anchor = [0.5, 0.92]           # required, see "Anchor" below
width = 64.0                   # required, default width in Canvas units
variant_group = "mountains"    # optional, see "Variants" below

[[texture]]                    # optional, one table per texture
id = "parchment"               # required, unique among the pack's textures
name = "Parchment"             # required
file = "textures/parchment.png"  # required, a seamless PNG
tags = ["paper"]               # optional
```

An Asset or texture may also set its own `author`, `source_url`, `licence` and `modified`. When
it doesn't, the `[pack]` values apply.

## Rules the app checks

- **Ids** (the pack's, and every Asset's and texture's) start with a lowercase letter or digit,
  then use only lowercase letters, digits, `-` and `_`. No spaces, no capitals.
- **Unique ids:** no two Assets share an id, and no two textures do. An Asset and a texture may
  share one.
- **Pack id:** must differ from every installed pack's. `default` and `textures` are taken.
- **Files:** every `file` path must exist in the folder.
- **Errors:** a missing or wrongly typed required field stops the pack from loading.

## Art

- **Style:** ink line art in one consistent style. Dark outlines with little or no fill, on a
  **transparent background**. The app draws a fill shape in the land colour under each Stamp, so
  overlapping Stamps hide the ones behind them. A white or coloured background breaks this.
- **Closed outlines:** the outer outline of every drawing must be closed, with no gaps. The
  fill shape is made by filling from the outside edge, and a gap lets the fill leak in.
- **One object per image,** cropped tight. No shadows, text, labels, frames or ground patches.
- **View:** the same view for every image. Classic fantasy maps draw features from the side or
  at a slight angle (three-quarter view), not from straight above.
- **Format:** SVG is preferred, because it stays sharp at any zoom and in large exports. A PNG is
  drawn at its own size, so make each PNG at least 512 px on its longer side.
- **Size limit:** no image over 4096 px on its longer side. Some machines can only draw 2048 px,
  so 2048 is the safest limit.
- **Textures:** PNG only, and seamless: the left edge must match the right and the top must match
  the bottom. 512 × 512 or 1024 × 1024. Keep them subtle, because they are blended over a fill
  colour.

## Anchor

`anchor = [x, y]` is the point where the object touches the ground, as fractions of the image's
width and height. `[0, 0]` is the top-left corner and `[1, 1]` the bottom-right. For most Stamps
it is the middle of the base, around `[0.5, 0.9]`, which is what the built-in pack uses for
every Stamp. A flat, round thing seen from above (a well, a
pond) uses its centre, `[0.5, 0.5]`. The app stacks overlapping Stamps by this point, so it must
be accurate. It also rotates and scales around it.

## Width

`width` is the default width in Canvas units. The height follows the image's proportions. The
built-in pack uses `64` for almost every Stamp, trees and houses included, and `128` for wide ones
(large castles, lakes). Use `64` as the normal size, `128` for wide or large Stamps, and `32` for
small props if the art is drawn small. Users can resize every Stamp after placing it.

## Categories

Use the built-in pack's categories where they fit, so the Asset browser filter stays tidy:

- `terrain`: mountains, hills, rocks, dunes
- `forests`: trees, bushes, palms
- `settlements`: houses, towns, wells, tents
- `fortifications`: castles, towers, walls, gates
- `features`: bridges, ruins, banners, ships, compass roses and other landmarks

New category names work too. Use lowercase.

## Variants

Make 3–5 variants of things that repeat on a map (trees, mountains, hills, houses). Give the
variants of one thing the same `variant_group`, a different `id` each (`tree-oak-1`,
`tree-oak-2`, …) and the same `name`.

## Licence

Set `licence` and `credits` to match the terms of the tool that made the art. If the pack is ever
to ship with the app, it must be CC0 or the owner's own art (ADR 0003).

## After the pack is made

From the repository root, generate the Silhouettes (the fill shapes under each Stamp):

```bash
cargo run -p ssmap-render --bin silhouettes -- path/to/<pack-id>
```

The tool writes `silhouettes/<asset-id>.png` for each Asset and adds the `silhouette` line to
`pack.toml`. Then open the app's packs page and choose **Add pack**, picking the pack **folder**
(zip files aren't supported yet).
