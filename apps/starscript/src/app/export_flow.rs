//! A window's PNG export: the settings dialog and the run in progress.

use eframe::egui;
use ssmap_core::{Canvas, ExportSettings};
use std::path::PathBuf;

use crate::export_dialog::{DialogAction, ExportDialog};
use crate::export_run::ExportRun;

/// The export dialog and the export it started, if still running.
#[derive(Default)]
pub(super) struct ExportFlow {
    dialog: ExportDialog,
    /// The PNG export in progress, stepped once per frame behind a modal dialog.
    run: Option<ExportRun>,
}

impl ExportFlow {
    /// Whether the dialog or a run is on screen.
    pub(super) fn is_busy(&self) -> bool {
        self.dialog.open || self.run.is_some()
    }

    /// Whether an export is running. Only one runs at a time.
    pub(super) fn is_running(&self) -> bool {
        self.run.is_some()
    }

    /// Opens the dialog on a copy of `settings`, writing into `directory` unless another is typed.
    pub(super) fn open_dialog(&mut self, settings: &ExportSettings, directory: Option<PathBuf>) {
        self.dialog.open_with(settings, directory);
    }

    /// Shows the dialog if it is open and returns what its buttons asked for.
    pub(super) fn show_dialog(
        &mut self,
        ctx: &egui::Context,
        canvas: &Canvas,
    ) -> Option<DialogAction> {
        self.dialog.show(ctx, canvas)
    }

    /// The folder typed in the dialog; empty if none was.
    pub(super) fn typed_directory(&self) -> PathBuf {
        PathBuf::from(self.dialog.directory.trim())
    }

    /// Starts showing `run`.
    pub(super) fn start(&mut self, run: ExportRun) {
        self.run = Some(run);
    }

    /// The run in progress, to step it.
    pub(super) fn run_mut(&mut self) -> Option<&mut ExportRun> {
        self.run.as_mut()
    }

    /// Forgets the run, finished, failed or cancelled.
    pub(super) fn end(&mut self) {
        self.run = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_idle() {
        let flow = ExportFlow::default();
        assert!(!flow.is_busy());
        assert!(!flow.is_running());
    }

    #[test]
    fn open_dialog_is_busy_but_not_running() {
        let mut flow = ExportFlow::default();
        flow.open_dialog(&ExportSettings::default(), None);
        assert!(flow.is_busy());
        assert!(!flow.is_running());
    }

    #[test]
    fn typed_directory_is_trimmed() {
        let mut flow = ExportFlow::default();
        flow.open_dialog(&ExportSettings::default(), Some(PathBuf::from("  /maps ")));
        assert_eq!(flow.typed_directory(), PathBuf::from("/maps"));
        // Opening again without a folder leaves the last one.
        flow.open_dialog(&ExportSettings::default(), None);
        assert_eq!(flow.typed_directory(), PathBuf::from("/maps"));
        assert_eq!(ExportFlow::default().typed_directory(), PathBuf::new());
    }
}
