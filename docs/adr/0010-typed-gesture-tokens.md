# 0010: Gestures are opened with a typed token

Status: accepted. Refs #396, #415.

## Context

A gesture is a run of Commands that makes one Undo step: a drag, a brush stroke, typing into a
new Label. The Editor opened one with `begin_gesture()` and closed it with `end_gesture()` or
`cancel_gesture()`, tracking it in a `gesture_open` flag next to History's own open
transaction. Two things could go wrong:

- the flag and History's transaction could disagree, and the code papered over that with
  `expect`s;
- any caller could end or cancel a gesture it never opened, and a late call from an earlier
  drag could close the next one.

## Decision

`begin_gesture` returns a `GestureId`. It is non-`Clone`, non-`Copy`, `#[must_use]`, and holds a
generation counter. `gesture`, `gesture_replace`, `end_gesture` and `cancel_gesture` take it;
the last two take it by value. Its `Drop` only `debug_assert!`s that it was consumed.

- A stale or foreign token (generation mismatch) makes a call a no-op that returns `false`. It
  never panics.
- `cancel_open_gesture()` is for Escape, which has no token. It rolls back whatever gesture is
  open and so makes the token stale.
- `in_gesture(label, |g| …)` is for code that fits in one call: it commits on `Ok` and rolls
  back on `Err`. It does not use `catch_unwind`; a panic is a bug, not a rollback.
- The Editor's state is one private `Option<OpenGesture { id, selection }>`. It changes only
  together with History's open transaction, in two private helpers (`open_gesture`,
  `close_gesture`), so the two cannot disagree.
- `History::begin` returns a `Result`. The Editor shows a refusal in the status bar.
- `gesture_open()` stays for UI gating.

## Why not a closure or a borrowing guard

A closure-scoped gesture (`editor.gesture(|g| …)`) needs the whole gesture to happen inside one
call. egui drags span many frames: the gesture begins on one `update`, receives a Command each
frame, and ends on a later one, with the Editor handed out fresh each frame. A closure cannot
return to egui between frames.

A borrowing RAII guard (`let g = editor.gesture(); … drop(g)`) holds `&mut Editor` for its
whole life. The tool would have to keep it across frames, in a struct that outlives the
borrow, and the rest of the frame (picking, the tree, the property panel) needs the Editor too.
A guard that owns nothing and only proves the gesture is open, checked by generation, can be
stored in tool state without borrowing.

`in_gesture` is kept for the cases that do fit in one call.

## Rollout

1. Add the typed API next to the old one. Rust has no overloading, so the typed methods
   temporarily carried a suffix while the old ones kept their names.
2. Move callers over.
3. Remove the old API and drop the suffix. Done: `begin_gesture`, `gesture`, `gesture_replace`,
   `end_gesture` and `cancel_gesture` all take or return a `GestureId`, and no token-less form
   remains. `gesture_open()` stays for UI gating.
