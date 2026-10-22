//! The system's own Open, Save and folder dialogs (NSOpenPanel and NSSavePanel on macOS).
//!
//! A dialog runs on a thread of its own so the egui frame keeps running while it is open.
//! The result comes back through a channel that the app polls once a frame; `None` means the
//! user cancelled.

use std::any::Any;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

use eframe::egui;
use rfd::FileDialog;

use crate::document;

/// What a dialog shows, run on a thread of its own.
type Show = Box<dyn FnOnce() -> Option<PathBuf> + Send>;

/// Puts a dialog on the screen and sends its answer. What it returns is kept until the dialog
/// is answered; `None` means the dialog could not open.
pub type Launch = fn(&egui::Context, Sender<Option<PathBuf>>, Show) -> Option<Box<dyn Any>>;

/// A native dialog that is open, or has just been answered.
pub struct Pick {
    answer: Option<Receiver<Option<PathBuf>>>,
    /// Whatever the launcher needs kept alive while the dialog is open.
    keep: Option<Box<dyn Any>>,
    launch: Launch,
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

/// The launcher for the real thing: the system's dialog, on a thread of its own. The window
/// repaints when it answers.
pub fn launch_native(
    ctx: &egui::Context,
    sender: Sender<Option<PathBuf>>,
    show: Show,
) -> Option<Box<dyn Any>> {
    let ctx = ctx.clone();
    thread::Builder::new()
        .name("native-dialog".into())
        .spawn(move || {
            // The receiver is gone only when the app is, with nobody left to answer.
            let _ = sender.send(show());
            ctx.request_repaint();
        })
        .ok()?;
    Some(Box::new(()))
}

/// A launcher for tests without a window to put a system dialog on: the dialog counts as open
/// and never answers. The sender is kept so it doesn't read as cancelled.
pub fn launch_headless(
    _ctx: &egui::Context,
    sender: Sender<Option<PathBuf>>,
    _show: Show,
) -> Option<Box<dyn Any>> {
    Some(Box::new(sender))
}

impl Pick {
    /// A picker that shows its dialogs through `launch`.
    pub fn new(launch: Launch) -> Self {
        Self {
            answer: None,
            keep: None,
            launch,
        }
    }

    /// Whether a dialog is showing.
    pub fn is_open(&self) -> bool {
        self.answer.is_some()
    }

    /// Runs `show` through the launcher unless a dialog is already open.
    fn start(
        &mut self,
        ctx: &egui::Context,
        show: impl FnOnce() -> Option<PathBuf> + Send + 'static,
    ) {
        if self.is_open() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        if let Some(keep) = (self.launch)(ctx, sender, Box::new(show)) {
            self.keep = Some(keep);
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
                self.keep = None;
                Polled::Chosen(path)
            }
            Ok(None) | Err(TryRecvError::Disconnected) => {
                self.answer = None;
                self.keep = None;
                Polled::Cancelled
            }
            Err(TryRecvError::Empty) => Polled::Open,
        }
    }

    #[cfg(test)]
    fn with_answer(answer: Receiver<Option<PathBuf>>) -> Self {
        Self {
            answer: Some(answer),
            ..Self::new(launch_headless)
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
        assert_eq!(Pick::new(launch_headless).poll(), Polled::Idle);
        assert!(!Pick::new(launch_headless).is_open());
    }

    #[test]
    fn a_headless_dialog_counts_as_open_and_never_answers() {
        let mut pick = Pick::new(launch_headless);
        pick.folder(&egui::Context::default(), "Choose", None);
        assert!(pick.is_open());
        assert_eq!(pick.poll(), Polled::Open);
    }

    #[test]
    fn a_launcher_that_fails_leaves_no_dialog_open() {
        let mut pick = Pick::new(|_, _, _| None);
        pick.folder(&egui::Context::default(), "Choose", None);
        assert!(!pick.is_open());
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
