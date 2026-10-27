//! The state of closing a window and quitting the app, and of the unsaved-changes question that
//! comes first.

/// What a close in progress ends in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Leaving {
    /// The app exits.
    Quit,
    /// The window closes and the app keeps running.
    Window,
}

/// What happens to a tab once the user has decided about its unsaved changes.
#[derive(Clone)]
pub(crate) enum Pending {
    /// Close the tab.
    CloseTab,
    /// Quit the app, which asks about every tab with unsaved changes in turn.
    Quit,
}

/// What a quit needs: whether one was asked for, and whether closing the window leaves the app
/// running.
pub(super) struct QuitState {
    /// Quit was asked for (⌘Q) and is handled next frame.
    requested: bool,
    /// Closing the window only hides it and leaves the app running (macOS); elsewhere closing
    /// the window quits.
    close_hides_window: bool,
}

impl QuitState {
    pub(super) fn new(close_hides_window: bool) -> Self {
        Self {
            requested: false,
            close_hides_window,
        }
    }

    /// Asks to quit, to be handled next frame.
    pub(super) fn request(&mut self) {
        self.requested = true;
    }

    /// Takes the quit request, if any, so it is handled once.
    pub(super) fn take_request(&mut self) -> bool {
        std::mem::take(&mut self.requested)
    }

    /// Drops a quit request that has not been handled.
    pub(super) fn cancel(&mut self) {
        self.requested = false;
    }

    /// Whether closing the window leaves the app running.
    pub(super) fn close_hides_window(&self) -> bool {
        self.close_hides_window
    }

    pub(super) fn set_close_hides_window(&mut self, hides: bool) {
        self.close_hides_window = hides;
    }
}

/// The window's close in progress.
pub(super) struct CloseFlow {
    /// A tab's unsaved-changes question was answered Save for a Map never saved: what to do
    /// once the Save As dialog saves it. Closing that dialog instead cancels.
    awaiting_save: Option<(usize, Pending)>,
    /// Set once the user has chosen to close despite unsaved changes.
    allowed: bool,
    /// What the close or quit in progress is for.
    leaving: Leaving,
    /// Closing this window was asked for, as its close button does.
    requested: bool,
}

impl CloseFlow {
    /// A flow with nothing in progress, for a window that leaves as `leaving` by default.
    pub(super) fn new(leaving: Leaving) -> Self {
        Self {
            awaiting_save: None,
            allowed: false,
            leaving,
            requested: false,
        }
    }

    /// Asks to close the window, as its close button does.
    pub(super) fn request(&mut self) {
        self.requested = true;
    }

    /// Takes the window's close request, if any, so it is handled once.
    pub(super) fn take_request(&mut self) -> bool {
        std::mem::take(&mut self.requested)
    }

    /// Whether the user chose to close despite unsaved changes.
    pub(super) fn is_allowed(&self) -> bool {
        self.allowed
    }

    /// Lets the close go through.
    pub(super) fn allow(&mut self) {
        self.allowed = true;
    }

    /// What the close in progress ends in.
    pub(super) fn leaving(&self) -> Leaving {
        self.leaving
    }

    pub(super) fn set_leaving(&mut self, leaving: Leaving) {
        self.leaving = leaving;
    }

    /// Whether a Save As is awaited before `pending` can go on.
    pub(super) fn is_awaiting_save(&self) -> bool {
        self.awaiting_save.is_some()
    }

    /// Holds `pending` for tab `index` until the Save As dialog saves it.
    pub(super) fn await_save(&mut self, index: usize, pending: Pending) {
        self.awaiting_save = Some((index, pending));
    }

    /// Takes what waited for the Save As dialog.
    pub(super) fn take_awaiting_save(&mut self) -> Option<(usize, Pending)> {
        self.awaiting_save.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_request_is_taken_once() {
        let mut flow = CloseFlow::new(Leaving::Quit);
        assert!(!flow.take_request());
        flow.request();
        assert!(flow.take_request());
        assert!(!flow.take_request());
    }

    #[test]
    fn quit_request_is_taken_once_or_cancelled() {
        let mut quit = QuitState::new(false);
        quit.request();
        assert!(quit.take_request());
        assert!(!quit.take_request());
        quit.request();
        quit.cancel();
        assert!(!quit.take_request());
    }

    #[test]
    fn close_hides_window_can_be_switched() {
        let mut quit = QuitState::new(true);
        assert!(quit.close_hides_window());
        quit.set_close_hides_window(false);
        assert!(!quit.close_hides_window());
    }

    #[test]
    fn allow_and_leaving_are_kept() {
        let mut flow = CloseFlow::new(Leaving::Window);
        assert!(!flow.is_allowed());
        assert_eq!(flow.leaving(), Leaving::Window);
        flow.set_leaving(Leaving::Quit);
        flow.allow();
        assert!(flow.is_allowed());
        assert_eq!(flow.leaving(), Leaving::Quit);
    }

    #[test]
    fn awaiting_save_holds_the_tab_and_what_follows() {
        let mut flow = CloseFlow::new(Leaving::Quit);
        assert!(!flow.is_awaiting_save());
        flow.await_save(2, Pending::CloseTab);
        assert!(flow.is_awaiting_save());
        assert!(matches!(
            flow.take_awaiting_save(),
            Some((2, Pending::CloseTab))
        ));
        assert!(!flow.is_awaiting_save());
        assert!(flow.take_awaiting_save().is_none());
    }
}
