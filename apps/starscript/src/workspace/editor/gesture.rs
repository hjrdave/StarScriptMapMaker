use super::*;

/// The next [`GestureId`] generation. Shared by all Editors so no two tokens ever match; 0 means
/// "never opened".
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

/// Proof that a gesture is open, handed out by [`Editor::begin_gesture`]. It has to be
/// given back to [`Editor::end_gesture`] or [`Editor::cancel_gesture`]. A token
/// whose gesture is over, or that another Editor issued, is stale: calls with it do nothing.
#[must_use = "a gesture has to be ended or cancelled with its GestureId"]
#[derive(Debug)]
pub struct GestureId {
    generation: u64,
    consumed: bool,
}

impl GestureId {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            consumed: false,
        }
    }

    fn consume(mut self) -> u64 {
        self.consumed = true;
        self.generation
    }
}

impl Drop for GestureId {
    fn drop(&mut self) {
        debug_assert!(
            self.consumed || std::thread::panicking(),
            "a GestureId was dropped without ending or cancelling its gesture"
        );
    }
}

/// A gesture whose History transaction is open.
pub(super) struct OpenGesture {
    id: u64,
    /// The Selection when the gesture began, restored if it is cancelled.
    selection: Selection,
}

/// What [`Editor::in_gesture`] hands its closure: the Editor and the gesture's token.
pub struct Gesture<'a> {
    pub(super) editor: &'a mut Editor,
    id: &'a GestureId,
}

impl Gesture<'_> {
    /// Runs a Command inside the gesture.
    pub fn run(&mut self, command: Command) -> bool {
        self.editor.gesture(self.id, command)
    }

    /// Replaces everything the gesture has done so far with `command`.
    pub fn replace(&mut self, command: Command) -> bool {
        self.editor.gesture_replace(self.id, command)
    }

    /// The Layer a new Object goes into, creating one inside the gesture if the Map has none.
    pub(super) fn placement_layer(&mut self, placing: Placing) -> Option<Id> {
        match self.editor.placement_layer_or_new(placing) {
            Ok(layer) => Some(layer),
            Err(create) => {
                let (id, command) = *create;
                self.run(command).then_some(id)
            }
        }
    }
}

impl Editor {
    /// Opens a gesture and returns its token. If a gesture is already open, or History
    /// refuses, the status bar says so and the token is stale.
    pub fn begin_gesture(&mut self, label: &str) -> GestureId {
        if self.gesture.is_some() {
            self.error("Another edit is still in progress");
            return GestureId::new(0);
        }
        GestureId::new(self.open_gesture(label).unwrap_or(0))
    }

    /// Runs a Command inside the gesture `id` opened. A stale `id` does nothing and returns
    /// `false`.
    pub fn gesture(&mut self, id: &GestureId, command: Command) -> bool {
        self.is_open(id) && self.execute_in_gesture(&command).is_ok()
    }

    /// Replaces everything the gesture `id` opened has done so far with `command`. A stale
    /// `id` does nothing and returns `false`.
    pub fn gesture_replace(&mut self, id: &GestureId, command: Command) -> bool {
        self.is_open(id) && self.rewind_gesture() && self.execute_in_gesture(&command).is_ok()
    }

    /// Commits the gesture `id` opened as one Undo step. Returns `false`, and changes
    /// nothing, if `id` is stale.
    pub fn end_gesture(&mut self, id: GestureId) -> bool {
        let live = self.is_open(&id);
        id.consume();
        if live {
            self.close_gesture(true);
        }
        live
    }

    /// Rolls the gesture `id` opened back, Selection included. Returns `false`, and changes
    /// nothing, if `id` is stale.
    pub fn cancel_gesture(&mut self, id: GestureId) -> bool {
        let live = self.is_open(&id);
        id.consume();
        if live {
            self.close_gesture(false);
        }
        live
    }

    /// Escape: rolls back whatever gesture is open, whoever opened it. Its token goes stale.
    pub fn cancel_open_gesture(&mut self) -> bool {
        let open = self.gesture.is_some();
        self.close_gesture(false);
        open
    }

    /// Commits whatever gesture is open, whoever opened it, as one Undo step. Its token goes
    /// stale. Returns whether one was open.
    pub fn end_open_gesture(&mut self) -> bool {
        let open = self.gesture.is_some();
        self.close_gesture(true);
        open
    }

    /// Runs `f` inside a gesture labelled `label`: commits it if `f` returns `Ok`, and rolls
    /// it back if `f` returns `Err`. `None` means the gesture could not be opened. A panic in
    /// `f` is not caught.
    pub fn in_gesture<T, E>(
        &mut self,
        label: &str,
        f: impl FnOnce(&mut Gesture<'_>) -> Result<T, E>,
    ) -> Option<Result<T, E>> {
        let id = self.begin_gesture(label);
        if !self.is_open(&id) {
            id.consume();
            return None;
        }
        let result = f(&mut Gesture {
            editor: self,
            id: &id,
        });
        if result.is_ok() {
            self.end_gesture(id);
        } else {
            self.cancel_gesture(id);
        }
        Some(result)
    }

    /// Whether `id`'s gesture is still the one open.
    pub(crate) fn is_open(&self, id: &GestureId) -> bool {
        self.gesture
            .as_ref()
            .is_some_and(|open| open.id == id.generation)
    }

    /// Opens History's transaction and the gesture together. Returns the new generation.
    pub(super) fn open_gesture(&mut self, label: &str) -> Option<u64> {
        if let Err(err) = self.history.begin(label, self.selection.clone()) {
            self.error(err.to_string());
            return None;
        }
        let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
        self.gesture = Some(OpenGesture {
            id: generation,
            selection: self.selection.clone(),
        });
        Some(generation)
    }

    /// Closes History's transaction and the gesture together, committing or rolling back. A
    /// rollback restores the Selection the gesture began with.
    pub(super) fn close_gesture(&mut self, commit: bool) {
        let Some(open) = self.gesture.take() else {
            return;
        };
        let closed = if commit {
            self.history.commit(self.selection.clone())
        } else {
            self.history.rollback(&mut self.map)
        };
        match closed {
            Ok(()) if !commit => self.restore(open.selection),
            Ok(()) => {}
            Err(err) => self.error(err.to_string()),
        }
    }

    pub(super) fn execute_in_gesture(&mut self, command: &Command) -> Result<(), ExecuteError> {
        let result = self.history.execute(&mut self.map, command);
        if let Err(err) = &result {
            self.refused(err);
        }
        result
    }

    pub(super) fn rewind_gesture(&mut self) -> bool {
        match self.history.rewind(&mut self.map) {
            Ok(()) => true,
            Err(err) => {
                self.error(err.to_string());
                false
            }
        }
    }

    pub fn gesture_open(&self) -> bool {
        self.gesture.is_some()
    }
}
