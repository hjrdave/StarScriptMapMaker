//! What Copy or Cut last took, kept apart from the rest of the Editor.

use crate::clipboard::Clipboard;

/// The Editor's clipboard: what Paste reads back, and the same as text until the app mirrors
/// it to the system clipboard.
#[derive(Default)]
pub(super) struct ClipboardSlot {
    clipboard: Option<Clipboard>,
    text: Option<String>,
}

impl ClipboardSlot {
    pub(super) fn has_contents(&self) -> bool {
        self.clipboard.is_some()
    }

    pub(super) fn contents(&self) -> Option<&Clipboard> {
        self.clipboard.as_ref()
    }

    /// Takes `clipboard` as a Copy or Cut in this Map: it is also queued as text for the system
    /// clipboard.
    pub(super) fn copied(&mut self, clipboard: Clipboard) {
        self.text = Some(clipboard.to_text());
        self.clipboard = Some(clipboard);
    }

    /// Takes `clipboard` from another tab or Map, without mirroring it to the system.
    pub(super) fn set(&mut self, clipboard: Clipboard) {
        self.clipboard = Some(clipboard);
    }

    pub(super) fn take_text(&mut self) -> Option<String> {
        self.text.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> Clipboard {
        Clipboard::new(Vec::new(), None)
    }

    #[test]
    fn a_copy_queues_its_text_once_and_a_set_does_not() {
        let mut slot = ClipboardSlot::default();
        assert!(!slot.has_contents());
        slot.set(empty());
        assert!(slot.has_contents());
        assert!(slot.take_text().is_none());
        slot.copied(empty());
        assert!(slot.take_text().is_some());
        assert!(slot.take_text().is_none());
        assert!(slot.contents().is_some());
    }
}
