//! The system's own Open, Save and folder dialogs (NSOpenPanel and NSSavePanel on macOS).
//!
//! A dialog runs on a thread of its own so the egui frame keeps running while it is open.
//! The result comes back through a channel that the app polls once a frame; `None` means the
//! user cancelled.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use eframe::egui;
use rfd::FileDialog;

use crate::document;

/// A native dialog that is open, or has just been answered.
#[derive(Default)]
pub struct Pick {
    answer: Option<Receiver<Option<PathBuf>>>,
    /// UI tests have no window to put a system dialog on: the dialog counts as open and
    /// never answers, and the sender is kept here so it doesn't read as cancelled.
    headless: Option<mpsc::Sender<Option<PathBuf>>>,
    is_headless: bool,
}

/// What a poll of a [`Pick`] found.
#[derive(Debug, PartialEq, Eq)]
pub enum Polled {
    /// No dialog is open.
    Idle,
    /// The dialog is still showing.
    Open,
    /// The user cancelled, or the dialog failed to open.
    Cancelled,
    /// The user chose this path.
    Chosen(PathBuf),
}

impl Pick {
    /// Whether a dialog is showing.
    pub fn is_open(&self) -> bool {
        self.answer.is_some()
    }

    /// A picker that never shows a system dialog, for tests without a window.
    pub fn headless() -> Self {
        Self {
            is_headless: true,
            ..Self::default()
        }
    }

    /// Runs `show` on a new thread unless a dialog is already open. The window repaints when
    /// it answers.
    fn start(
        &mut self,
        ctx: &egui::Context,
        show: impl FnOnce() -> Option<PathBuf> + Send + 'static,
    ) {
        if self.is_open() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        if self.is_headless {
            self.headless = Some(sender);
            self.answer = Some(receiver);
            return;
        }
        let ctx = ctx.clone();
        let spawned = thread::Builder::new()
            .name("native-dialog".into())
            .spawn(move || {
                let _ = sender.send(show());
                ctx.request_repaint();
            });
        if spawned.is_ok() {
            self.answer = Some(receiver);
        }
    }

    /// Asks for a Map file to open, starting in `directory`.
    pub fn open_map(&mut self, ctx: &egui::Context, directory: Option<&Path>) {
        let dialog = map_files(FileDialog::new().set_title("Open Map"), directory);
        self.start(ctx, move || dialog.pick_file());
    }

    /// Asks where to save a Map, offering `file_name` in `directory`.
    pub fn save_map(&mut self, ctx: &egui::Context, directory: Option<&Path>, file_name: &str) {
        let dialog = map_files(FileDialog::new().set_title("Save Map As"), directory)
            .set_file_name(document::with_extension(file_name.into()).to_string_lossy());
        self.start(ctx, move || dialog.save_file());
    }

    /// Asks for a folder, starting in `directory`.
    pub fn folder(&mut self, ctx: &egui::Context, title: &str, directory: Option<&Path>) {
        let mut dialog = FileDialog::new().set_title(title);
        if let Some(directory) = directory.filter(|directory| directory.is_dir()) {
            dialog = dialog.set_directory(directory);
        }
        self.start(ctx, move || dialog.pick_folder());
    }

    /// Takes the answer if the dialog has given one.
    pub fn poll(&mut self) -> Polled {
        let Some(answer) = &self.answer else {
            return Polled::Idle;
        };
        match answer.try_recv() {
            Ok(Some(path)) => {
                self.answer = None;
                Polled::Chosen(path)
            }
            Ok(None) | Err(TryRecvError::Disconnected) => {
                self.answer = None;
                Polled::Cancelled
            }
            Err(TryRecvError::Empty) => Polled::Open,
        }
    }

    #[cfg(test)]
    fn with_answer(answer: Receiver<Option<PathBuf>>) -> Self {
        Self {
            answer: Some(answer),
            ..Self::default()
        }
    }
}

/// `dialog` limited to Map files, starting in `directory` when it exists.
fn map_files(dialog: FileDialog, directory: Option<&Path>) -> FileDialog {
    let dialog = dialog.add_filter("Map files", &[document::EXTENSION]);
    match directory.filter(|directory| directory.is_dir()) {
        Some(directory) => dialog.set_directory(directory),
        None => dialog,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_without_a_dialog() {
        assert_eq!(Pick::default().poll(), Polled::Idle);
        assert!(!Pick::default().is_open());
    }

    #[test]
    fn open_until_answered_then_idle() {
        let (sender, receiver) = mpsc::channel();
        let mut pick = Pick::with_answer(receiver);
        assert!(pick.is_open());
        assert_eq!(pick.poll(), Polled::Open);
        sender.send(Some(PathBuf::from("/maps/a.ssmap"))).unwrap();
        assert_eq!(pick.poll(), Polled::Chosen("/maps/a.ssmap".into()));
        assert_eq!(pick.poll(), Polled::Idle);
    }

    #[test]
    fn cancelling_is_reported_once() {
        let (sender, receiver) = mpsc::channel();
        let mut pick = Pick::with_answer(receiver);
        sender.send(None).unwrap();
        assert_eq!(pick.poll(), Polled::Cancelled);
        assert_eq!(pick.poll(), Polled::Idle);
    }

    #[test]
    fn a_dialog_thread_that_dies_counts_as_cancelled() {
        let (sender, receiver) = mpsc::channel();
        let mut pick = Pick::with_answer(receiver);
        drop(sender);
        assert_eq!(pick.poll(), Polled::Cancelled);
    }
}
