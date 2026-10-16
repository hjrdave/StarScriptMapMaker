//! Adding a pack folder in the background: the copy runs on a thread of its own so the window
//! keeps drawing, and the Asset packs page shows its progress and then its result.
//!
//! The thread works on a copy of the library; the app reloads the packs from disk once the
//! install has succeeded, which is what the picker and open Maps see (via the sprite store).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use eframe::egui;
use ssmap_core::asset::{InstallError, InstallProgress, Pack, PackLibrary, PackRoots};

/// What the Asset packs page says about the last Add pack.
#[derive(Clone, PartialEq, Debug)]
pub enum Status {
    /// Copying `name`; `progress` is empty until the files have been counted.
    Installing {
        name: String,
        progress: Option<InstallProgress>,
    },
    /// The pack was added.
    Done(String),
    /// The pack was not added; the reason.
    Failed(String),
}

enum Message {
    Progress(InstallProgress),
    Finished(Result<Pack, InstallError>),
}

/// An install in progress, or the message about the last one.
#[derive(Default)]
pub struct PackInstall {
    job: Option<Receiver<Message>>,
    status: Option<Status>,
}

impl PackInstall {
    /// Whether a pack is being copied.
    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    pub fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }

    /// Clears a finished install's message.
    pub fn dismiss(&mut self) {
        if !self.is_running() {
            self.status = None;
        }
    }

    /// Starts installing the folder `source`, unless an install is already running. The window
    /// repaints as the copy advances.
    pub fn start(
        &mut self,
        ctx: &egui::Context,
        source: PathBuf,
        library: Arc<PackLibrary>,
        roots: PackRoots,
    ) {
        if self.is_running() {
            return;
        }
        let name = folder_name(&source);
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        let spawned = thread::Builder::new()
            .name("pack-install".into())
            .spawn(move || {
                let mut library = (*library).clone();
                let result = library.install_pack(&source, &roots, |progress| {
                    let _ = sender.send(Message::Progress(progress));
                    ctx.request_repaint();
                });
                let _ = sender.send(Message::Finished(result));
                ctx.request_repaint();
            });
        if spawned.is_ok() {
            self.job = Some(receiver);
            self.status = Some(Status::Installing {
                name,
                progress: None,
            });
        } else {
            self.status = Some(Status::Failed(
                "Couldn’t add the pack: the installer could not start".into(),
            ));
        }
    }

    /// Takes in what the install has reported. Returns the pack once it has been installed.
    pub fn poll(&mut self) -> Option<Pack> {
        let job = self.job.as_ref()?;
        let mut installed = None;
        loop {
            match job.try_recv() {
                Ok(Message::Progress(now)) => {
                    if let Some(Status::Installing { progress, .. }) = &mut self.status {
                        *progress = Some(now);
                    }
                }
                Ok(Message::Finished(result)) => {
                    self.job = None;
                    self.status = Some(match result {
                        Ok(pack) => {
                            let status = Status::Done(format!(
                                "Added “{}” with {}",
                                pack.name,
                                match pack.asset_count() {
                                    1 => "1 Asset".to_owned(),
                                    n => format!("{n} Assets"),
                                }
                            ));
                            installed = Some(pack);
                            status
                        }
                        Err(err) => Status::Failed(format!("Couldn’t add the pack: {err}")),
                    });
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // The thread ended without an answer.
                    self.job = None;
                    self.status = Some(Status::Failed(
                        "Couldn’t add the pack: the installer stopped unexpectedly".into(),
                    ));
                    break;
                }
            }
        }
        installed
    }
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

/// How far a running install has got, 0 to 1; `None` while the files are not counted yet.
pub fn progress_fraction(progress: Option<InstallProgress>) -> Option<f32> {
    let progress = progress?;
    (progress.total > 0).then(|| (progress.copied as f32 / progress.total as f32).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fraction_is_empty_until_files_are_counted() {
        assert_eq!(progress_fraction(None), None);
        let none = InstallProgress {
            copied: 0,
            total: 0,
        };
        assert_eq!(progress_fraction(Some(none)), None);
        let half = InstallProgress {
            copied: 5,
            total: 10,
        };
        assert_eq!(progress_fraction(Some(half)), Some(0.5));
    }
}
