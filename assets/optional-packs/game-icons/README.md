# game-icons.net (optional pack)

[game-icons.net](https://game-icons.net) icons under
[CC BY 3.0](https://creativecommons.org/licenses/by/3.0/). The licence requires credit, so the
pack is flagged `attribution_required = true`: the credits screen lists it, and an export of a
Map that uses its icons must credit them ("Icons made by {author}. Available on
https://game-icons.net").

This folder is not under `assets/packs/`, so the app does not load it by default. To use it,
copy it into the per-user packs folder.

## Adding the icons

The art is not committed yet. Use the recoloured, transparent-background variant that
`colorize-svgs.sh` in the [game-icons/icons](https://github.com/game-icons/icons) repo
produces, put the SVGs in `art/`, and add an `[[asset]]` for each to `pack.toml` with that
icon's `author` (its folder name in the repo), a `source_url` and `modified`. Then generate
Silhouettes:

```
cargo run -p ssmap-render --bin silhouettes -- assets/optional-packs/game-icons
```
