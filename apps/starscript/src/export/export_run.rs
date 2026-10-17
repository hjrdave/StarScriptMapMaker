//! A PNG export running behind a modal progress dialog. The app steps it once per egui frame;
//! this module holds the stepping logic, which needs neither a window nor a GPU, and the
//! dialog that shows it.

use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use ssmap_core::{ExportSettings, ExportSize, credits_file_path};
use ssmap_render::{ExportJob, ExportStatus, ExportStep};

use crate::{dialog, theme};

/// What the app shows for an export that is still going.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Progress {
    /// Derived data is rebuilding; tiles render once it has.
    Waiting,
    /// `done` of `total` tiles are rendered.
    Rendering { done: u32, total: u32 },
}

/// What one frame of an export came to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    Running(Progress),
    /// The PNG is in place at the run's destination.
    Finished(ExportSize),
    Failed(String),
}

/// A job that can be stopped. [`ExportJob`] in the app; a stand-in in tests.
pub trait Stoppable {
    /// Stops the job, leaving no partial file.
    fn cancel(self);
}

impl Stoppable for ExportJob {
    fn cancel(self) {
        ExportJob::cancel(self);
    }
}

/// One export in progress.
pub struct ExportRun<J = ExportJob> {
    job: Option<J>,
    dest: PathBuf,
    progress: Progress,
    /// The text for `<name>.credits.txt`, written once the PNG is in place.
    credits_file: Option<String>,
    /// When the first tile began rendering, for the time left.
    rendering_since: Option<Instant>,
}

impl<J: Stoppable> ExportRun<J> {
    pub fn new(job: J, dest: PathBuf, credits_file: Option<String>) -> Self {
        Self {
            job: Some(job),
            dest,
            progress: Progress::Waiting,
            credits_file,
            rendering_since: None,
        }
    }

    /// The time the rest of the tiles should take, once enough have rendered to tell.
    pub fn time_left(&self) -> Option<Duration> {
        match (self.progress, self.rendering_since) {
            (Progress::Rendering { done, total }, Some(since)) => {
                estimate_time_left(since.elapsed(), done, total)
            }
            _ => None,
        }
    }

    pub fn dest(&self) -> &Path {
        &self.dest
    }

    pub fn progress(&self) -> Progress {
        self.progress
    }

    /// Whether the run has ended, by finishing, failing or being cancelled.
    pub fn is_over(&self) -> bool {
        self.job.is_none()
    }

    /// Advances the export by one `step` of its job. Call once per frame while the run is
    /// not over. Once it finishes, the credits file is written beside the PNG.
    pub fn frame<E: Display>(
        &mut self,
        step: impl FnOnce(&mut J) -> Result<ExportStep, E>,
    ) -> Outcome {
        let Some(job) = self.job.as_mut() else {
            return Outcome::Failed("the export is not running".into());
        };
        match step(job) {
            Ok(ExportStep::Working(status)) => {
                self.progress = match status {
                    ExportStatus::WaitingForDerivedData => Progress::Waiting,
                    ExportStatus::Rendering { done, total } => Progress::Rendering { done, total },
                };
                if matches!(self.progress, Progress::Rendering { .. })
                    && self.rendering_since.is_none()
                {
                    self.rendering_since = Some(Instant::now());
                }
                Outcome::Running(self.progress)
            }
            Ok(ExportStep::Finished(size)) => {
                self.job = None;
                match self.write_credits_file() {
                    Ok(()) => Outcome::Finished(size),
                    Err(err) => Outcome::Failed(format!(
                        "the PNG was saved, but the credits file was not: {err}"
                    )),
                }
            }
            Err(err) => {
                self.job = None;
                Outcome::Failed(err.to_string())
            }
        }
    }

    /// Stops the export. No PNG, and no credits file, is written.
    pub fn cancel(&mut self) {
        if let Some(job) = self.job.take() {
            job.cancel();
        }
    }

    fn write_credits_file(&self) -> std::io::Result<()> {
        match &self.credits_file {
            Some(text) => std::fs::write(credits_file_path(&self.dest), text),
            None => Ok(()),
        }
    }
}

/// The PNG to write for `settings` in `directory`. `None` before the settings have a file
/// name, when there is nothing to re-export to.
pub fn export_path(settings: &ExportSettings, directory: &Path) -> Option<PathBuf> {
    let name = settings
        .file_name
        .as_deref()
        .filter(|name| !name.is_empty())?;
    let mut name = name.to_owned();
    if !name.to_ascii_lowercase().ends_with(".png") {
        name.push_str(".png");
    }
    Some(directory.join(name))
}

/// How long the rest of the tiles will take, going by the `elapsed` time the first `done`
/// took. `None` before a tile is done.
pub fn estimate_time_left(elapsed: Duration, done: u32, total: u32) -> Option<Duration> {
    if done == 0 || done > total {
        return None;
    }
    Some(elapsed.mul_f64(f64::from(total - done) / f64::from(done)))
}

/// "12 s left", "2 min 5 s left".
pub fn time_left_text(left: Duration) -> String {
    let seconds = left.as_secs_f64().ceil() as u64;
    if seconds < 60 {
        format!("{seconds} s left")
    } else {
        format!("{} min {} s left", seconds / 60, seconds % 60)
    }
}

/// The modal progress dialog. Returns true when Cancel is pressed (or Esc).
pub fn show_progress(
    ctx: &egui::Context,
    dest: &Path,
    progress: Progress,
    time_left: Option<Duration>,
) -> bool {
    let mut cancel = false;
    let shown = dialog::show(ctx, "export_progress", "Exporting PNG", 380.0, |ui| {
        let p = theme::palette(ui.ctx());
        ui.label(egui::RichText::new(dest.display().to_string()).color(p.text_secondary));
        ui.add_space(8.0);
        match progress {
            Progress::Waiting => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Waiting for rebuild");
                });
            }
            Progress::Rendering { done, total } => {
                let fraction = if total == 0 {
                    0.0
                } else {
                    done as f32 / total as f32
                };
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .fill(p.gold)
                        .text(format!("{done} of {total} tiles")),
                );
                if let Some(left) = time_left {
                    ui.label(egui::RichText::new(time_left_text(left)).color(p.text_secondary));
                }
            }
        }
        dialog::buttons(ui, |ui| {
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
    });
    cancel || shown.cancelled
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    struct Fake {
        cancelled: Rc<Cell<bool>>,
    }

    impl Stoppable for Fake {
        fn cancel(self) {
            self.cancelled.set(true);
        }
    }

    fn run(credits: Option<&str>, dest: &Path) -> (ExportRun<Fake>, Rc<Cell<bool>>) {
        let cancelled = Rc::new(Cell::new(false));
        let fake = Fake {
            cancelled: cancelled.clone(),
        };
        (
            ExportRun::new(fake, dest.to_owned(), credits.map(str::to_owned)),
            cancelled,
        )
    }

    fn size() -> ExportSize {
        ExportSize {
            width: 10,
            height: 10,
            dpi: None,
            warn_large: false,
        }
    }

    fn work(status: ExportStatus) -> Result<ExportStep, String> {
        Ok(ExportStep::Working(status))
    }

    #[test]
    fn a_rebuild_shows_waiting_then_tiles_then_finishes() {
        let (mut run, _) = run(None, Path::new("/x/a.png"));
        assert_eq!(run.progress(), Progress::Waiting);
        let waiting = run.frame(|_| work(ExportStatus::WaitingForDerivedData));
        assert_eq!(waiting, Outcome::Running(Progress::Waiting));
        let first = run.frame(|_| work(ExportStatus::Rendering { done: 0, total: 2 }));
        assert_eq!(
            first,
            Outcome::Running(Progress::Rendering { done: 0, total: 2 })
        );
        let second = run.frame(|_| work(ExportStatus::Rendering { done: 1, total: 2 }));
        assert_eq!(
            second,
            Outcome::Running(Progress::Rendering { done: 1, total: 2 })
        );
        assert!(!run.is_over());
        let done = run.frame(|_| Ok::<_, String>(ExportStep::Finished(size())));
        assert_eq!(done, Outcome::Finished(size()));
        assert!(run.is_over());
    }

    #[test]
    fn each_frame_takes_exactly_one_step() {
        let (mut run, _) = run(None, Path::new("/x/a.png"));
        let steps = Cell::new(0);
        run.frame(|_| {
            steps.set(steps.get() + 1);
            work(ExportStatus::WaitingForDerivedData)
        });
        assert_eq!(steps.get(), 1);
    }

    #[test]
    fn cancel_stops_the_job_and_ends_the_run() {
        let (mut run, cancelled) = run(Some("credits\n"), Path::new("/x/a.png"));
        run.frame(|_| work(ExportStatus::Rendering { done: 1, total: 4 }));
        run.cancel();
        assert!(cancelled.get());
        assert!(run.is_over());
        // Nothing steps a cancelled run.
        let outcome = run.frame(|_| work(ExportStatus::WaitingForDerivedData));
        assert!(matches!(outcome, Outcome::Failed(_)));
    }

    #[test]
    fn an_error_ends_the_run_without_cancelling() {
        let (mut run, cancelled) = run(None, Path::new("/x/a.png"));
        let outcome = run.frame(|_| Err::<ExportStep, _>("disk full"));
        assert_eq!(outcome, Outcome::Failed("disk full".into()));
        assert!(run.is_over());
        assert!(!cancelled.get());
    }

    #[test]
    fn finishing_writes_the_credits_file_but_cancelling_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("world.png");
        let (mut finished, _) = run(Some("Sword by Lorc\n"), &dest);
        finished.frame(|_| Ok::<_, String>(ExportStep::Finished(size())));
        let text = std::fs::read_to_string(dir.path().join("world.credits.txt")).unwrap();
        assert_eq!(text, "Sword by Lorc\n");

        let other = dir.path().join("other.png");
        let (mut cancelled, _) = run(Some("Sword by Lorc\n"), &other);
        cancelled.cancel();
        assert!(!dir.path().join("other.credits.txt").exists());
    }

    #[test]
    fn re_export_uses_the_stored_file_name_and_folder() {
        let settings = ExportSettings {
            file_name: Some("world.png".into()),
            ..ExportSettings::default()
        };
        assert_eq!(
            export_path(&settings, Path::new("/maps")),
            Some(PathBuf::from("/maps/world.png"))
        );
        let bare = ExportSettings {
            file_name: Some("world".into()),
            ..ExportSettings::default()
        };
        assert_eq!(
            export_path(&bare, Path::new("/maps")),
            Some(PathBuf::from("/maps/world.png"))
        );
        assert_eq!(
            export_path(&ExportSettings::default(), Path::new("/maps")),
            None
        );
    }
}
