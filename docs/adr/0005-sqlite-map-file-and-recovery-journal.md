# The Map file is a self-contained SQLite database with a recovery journal

Status: accepted (amends the Serialization row of ADR 0001)

A Map is saved as one `.ssmap` file that is a SQLite database (`rusqlite`, bundled SQLite). We chose it over a zip container because Maps can hold 100k Stamps and about 1M coastline vertices, and SQLite gives **incremental, transactional saves**: only changed Objects are written, and a crash mid-save cannot corrupt the file. A zip would rewrite everything on every save and need its own temp-file-and-rename crash safety. A folder bundle was rejected because it is awkward to share on Windows and Linux. The file stays open and inspectable with any SQLite tool.

## Layout

- Each Object is a row: id (UUIDv7), parent, order, kind, properties as **JSON**, and geometry as a separate **binary blob** of little-endian `f64` pairs. JSON is readable and tolerant of added fields; binary keeps polygon-heavy Maps compact.
- **Every Asset and font the Map uses is embedded**, de-duplicated by content hash, with its manifest entry, credits and licence. The `pack-id/asset-id` reference is kept too. A Map renders identically even if a pack is updated, removed or never installed, and an "Update from pack" Command can re-pull newer art.
- The metadata table holds `format_version`, the save generation id, the last export settings, and a 512px preview PNG refreshed on every save.

## Saving and recovery

- Save is explicit, and Save As writes a full copy that becomes the open Map. Export PNG never modifies the Map file but stores its settings in it.
- Between saves, the Ops (ADR 0004) of every committed Undo step are appended to a sidecar `<map>.recovery` **Recovery journal** within about a second, and deleted on clean close or Save.
- Each save stamps a random **save generation id**, and the journal records the generation it was written against. On reopen, a matching journal is offered for replay. A mismatched one (the file was replaced, synced or edited elsewhere) is offered only as a separate recovered copy, never applied to the wrong base.
- We chose this over "always saved" (as in Figma) because undo history is not saved, so "revert to saved" must stay meaningful.

## Compatibility

- `format_version` is major.minor. A minor bump means only optional fields were added; a major bump means anything else.
- Newer app, older file: step-by-step migrations run on open, after writing `<map>.v<old>.bak`.
- Older app, newer minor: the file opens, and unknown JSON fields are preserved through a serde catch-all, so round-tripping loses nothing.
- Older app, newer major: it refuses with a clear message.

## Large Maps

- v1 loads the whole Map into memory. Derived data (Roughness outlines, tessellation, rasterised SVGs) is not stored; it is rebuilt progressively after open, with the view usable before it finishes.
- Spec targets at 100k Stamps and about 1M vertices: save under 200 ms, open-to-interactive under 3 s.
- Caching derived data in the file can be added later as a minor-version change if opening proves slow. Lazy regional streaming is not planned.
