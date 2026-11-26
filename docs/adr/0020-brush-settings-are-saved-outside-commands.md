# Brush settings are saved with the Map, outside Commands

Status: accepted (amends ADR 0011; builds on ADR 0004 and ADR 0005)

Each mode of the Land Brush, the Lake Brush and the Paint tool keeps its own settings (issue 645): Paint and Erase each have a size, the Land Brush an Irregularity too, and the Paint tool a softness and opacity, with the colour on its Paint mode. Users expect them back when they open the Map again, and each open Map has its own. Like the export settings, they are a preference kept with the Map, not Map content. Nobody expects Undo to rewind a slider, and a slider drag would put one step per frame in History. ADR 0011 allowed only `export_settings` to be written outside a Command; this ADR adds a second field under the same rules.

## Decision

- **Where.** `Map.brush_settings` (`BrushSettings` in `crates/core`, with no egui types) holds every mode's settings. The file stores it as JSON under the `brush_settings` meta key, from format 1.21. It is left out of the view while it equals the defaults. An older file, or one that leaves fields out, gets the defaults for what is missing, and fields from a newer version ride along in `extra`.
- **Defaults.** A new Map starts with the defaults. A template made from a Map does not keep them.
- **One copy.** The Editor reads and writes the settings on its Map (`Editor::brush_settings_mut` and the per-mode accessors). It keeps no copy of its own, so open and save need no extra step. The mode choices (which mode is the default, Brush or Lasso) stay in the Editor and are not saved.
- **Writers.** Only the tool settings in Properties and the `[` / `]` size shortcuts write them. Render code never does.
- **Edits mark the Tab unsaved.** `Document` compares the settings with those last saved or opened, as it does for the export settings. Setting a value back to the saved one clears the mark.
- **Not undoable, not journaled.** These edits create no Undo step and no Recovery journal entry. After a crash the settings come back as of the last save.

## Consequences

- ADR 0011's "One field" rule now reads "two fields": `export_settings` and `brush_settings`. Everything else stays under ADR 0004.
- When the Command rule is enforced by the compiler (issue 374), `brush_settings` keeps its own setter next to `export_settings`.
