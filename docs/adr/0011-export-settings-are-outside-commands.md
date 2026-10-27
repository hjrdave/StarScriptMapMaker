# Export settings are written outside Commands

Status: accepted

ADR 0004 says every change to a Map is a Command. `Map.export_settings`, the last PNG export size, region and options, is the one exception. It is a preference remembered with the Map, not Map content: nobody expects Undo to rewind an export dialog, and a drag of the export region per frame would flood History. Making it a Command (`SetExportSettings` compiling to an Op) would cost a new Op with an inverse and journal encoding, a file and journal compatibility check, and coalescing for the region drag, for edits users would not want to undo. We chose to keep direct writes and bound the exception instead.

## Rules

- **One field.** `Map.export_settings` is the only Map field written outside a Command. `Map.canvas` and everything else stay under ADR 0004 and change only through Commands.
- **Two writers.** Only the export dialog and the export-region tool write it. Render code never does; it reads the settings it is given.
- **Edits mark the Tab unsaved.** Changing export settings sets the Tab's unsaved flag, so closing offers to save and the next save writes them. Today the settings are only compared at save time (`saved_export`), so a change on its own can be lost when the Tab closes.
- **Not undoable, not journaled.** These edits create no Undo step and no Recovery journal entry. After a crash the settings come back as of the last save.
- **Enforcement.** When the Command rule is enforced by the compiler (issue 374), `export_settings` keeps its own setter next to the `Map` getters; it is not behind the `test-support` feature.
