# The Canvas is the primary Art board

Status: accepted

A Map used to have exactly one drawing area, the Canvas. Issue 239 adds more: Art boards for staging Assets and half-finished pieces, with one of them the one Export draws. We chose to keep the Canvas as the primary board and store the others beside it, instead of turning everything into one list of equal boards.

## Rules

- **The Canvas is the primary board.** `Map.canvas` holds its size, origin, background, name and id, and also what is Map-wide: the real-world scale and the template name. Everything that reads the Canvas keeps working, and Export draws it.
- **Other boards are `ArtBoard`s** in `Map.art_boards`, lowest first. The Canvas is always the lowest board, so only the Art boards can be reordered.
- **Any board can become the Canvas.** `Op::SetPrimaryBoard` swaps the two boards' contents (id, name, size, origin, background) and re-homes the Objects: members of the old Canvas become members of the old board's id, and members of the new Canvas become Canvas members. Applying it with the ids swapped is its inverse. The scale and template stay with the Map.
- **Membership.** An Object belongs to the Canvas (`Membership::Canvas`, the default, also for files saved before Art boards), to an Art board (`Membership::Board(id)`, never the Canvas's own id), or to none (`Membership::OffCanvas`, saved as `pasteboard`). Only a drag or an explicit Command changes it; resizing or moving a board never does.
- **Deleting a board never deletes Objects.** `Command::DeleteBoard` sets its members off the boards, then removes it. The Canvas cannot be deleted. `Op::RemoveBoard` refuses a board that still has members.
- **Files.** Format 1.10 adds the `art_boards` meta key (a JSON array, absent when there are none). The Canvas JSON gains `id` and `name`, written only when they are not the defaults, so a Map with only the Canvas is saved as before. Older files load as a Map with only the Canvas.
- **Commands** act on any board by id (`ResizeBoard`, `SetBoardBackground`, `RenameBoard`); for the Canvas, resizing and the background compile to the existing `ResizeCanvas` and `SetBackground` Ops, so Recovery journals and tests written before Art boards keep working.

## Why not one list with a primary id

It would make promotion a change of one id, but every reader of the Canvas (about a hundred call sites, the file's `canvas` key, Home's size labels) would then need a derived copy kept in step, and the Canvas could be reordered above other boards. Swapping two boards' contents costs one Op and leaves the readers alone.
