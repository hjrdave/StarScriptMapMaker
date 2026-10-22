//! The Editor's notices: pushing toasts, drawing them and running their actions.

use std::time::Instant;

use eframe::egui;

use super::Editor;
use crate::toast::{self, Toast, ToastAction, ToastKind};

impl Editor {
    /// The newest toast still showing.
    pub fn notice(&self) -> Option<&Toast> {
        self.toasts.items().last()
    }

    /// Draws this tab's toasts top-centre over `canvas`, expires them, and runs the action the
    /// user clicked. Actions the history has moved past are dropped first.
    pub fn show_toasts(&mut self, ctx: &egui::Context, canvas: egui::Rect) {
        self.drop_stale_toast_actions();
        if let Some(action) = toast::show(ctx, canvas, &mut self.toasts) {
            self.run_toast_action(action);
        }
    }

    /// Strips the actions the history has moved past from the toasts.
    pub(super) fn drop_stale_toast_actions(&mut self) {
        let (open, depth) = (self.gesture_open, self.history.undo_len());
        self.toasts.retain_actions(|action| match action {
            ToastAction::Undo { steps } => !open && depth == steps,
        });
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.toasts
            .push(text, ToastKind::Info, None, Instant::now());
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.toasts
            .push(text, ToastKind::Error, None, Instant::now());
    }

    /// Tells the user about a Command they might regret, with an Undo on the toast.
    pub(super) fn info_with_undo(&mut self, text: impl Into<String>) {
        let action = ToastAction::Undo {
            steps: self.history.undo_len(),
        };
        self.toasts
            .push(text, ToastKind::Info, Some(action), Instant::now());
    }

    /// Runs the action a toast carried, if it still applies.
    pub fn run_toast_action(&mut self, action: ToastAction) {
        match action {
            ToastAction::Undo { steps } => {
                if self.history.undo_len() == steps {
                    self.undo();
                }
            }
        }
    }
}
