//! Writes the Recovery journal of the open Map (ADR 0005), and finds journals left by a
//! crash.
//!
//! Ops reach the journal within [`FLUSH_DELAY`] of being applied. A Map file's journal sits
//! beside it as `<map>.recovery`; a Map never saved journals under the platform data
//! directory instead, starting from a snapshot of the Map at its first flush.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "existing invariant panics; replace with error handling over time (#351)"
)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use ssmap_core::file::{Journal, JournalBase, JournalError, Recovered, recovery_path};
use ssmap_core::{Map, Op};
use uuid::Uuid;

use crate::document::Document;

/// How long applied Ops may wait before they are written: "within about a second".
pub(crate) const FLUSH_DELAY: Duration = Duration::from_millis(500);

/// The extension of journal files.
const EXTENSION: &str = "recovery";

pub struct Recovery {
    /// Where journals of Maps never saved go; `None` turns journaling them off.
    unsaved_dir: Option<PathBuf>,
    journal: Option<Journal>,
    /// The journal path to use for a Map never saved, once chosen.
    unsaved_path: Option<PathBuf>,
    /// Ops not yet written, and when the oldest of them was applied.
    pending: Vec<Op>,
    since: Option<Instant>,
    /// Writing failed; it is retried only after the next save, open or reset.
    failed: bool,
}

/// A journal left behind by a crash.
pub struct Found {
    pub path: PathBuf,
    pub recovered: Recovered,
}

impl Recovery {
    pub fn new(unsaved_dir: Option<PathBuf>) -> Self {
        Self {
            unsaved_dir,
            journal: None,
            unsaved_path: None,
            pending: Vec::new(),
            since: None,
            failed: false,
        }
    }

    /// The platform's directory for journals of Maps never saved.
    pub fn default_unsaved_dir() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "StarScript Map Maker")
            .map(|dirs| dirs.data_dir().join("Recovery"))
    }

    /// Queues Ops just applied to the Map.
    pub fn record(&mut self, ops: Vec<Op>, now: Instant) {
        if ops.is_empty() || self.failed {
            return;
        }
        self.since.get_or_insert(now);
        self.pending.extend(ops);
    }

    /// How long until queued Ops are due, or `None` if nothing is queued.
    pub fn due_in(&self, now: Instant) -> Option<Duration> {
        self.since
            .map(|since| FLUSH_DELAY.saturating_sub(now.duration_since(since)))
    }

    /// Whether Ops are queued but not yet written.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Writes queued Ops if they are due. `map` must already include them.
    pub fn flush_if_due(
        &mut self,
        now: Instant,
        document: &Document,
        map: &Map,
    ) -> Result<(), JournalError> {
        if self.due_in(now).is_some_and(|wait| wait.is_zero()) {
            self.flush(document, map)?;
        }
        Ok(())
    }

    /// Writes queued Ops now, starting the journal if this is the first write. A failure
    /// stops journaling until the next save, open or reset.
    pub fn flush(&mut self, document: &Document, map: &Map) -> Result<(), JournalError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let ops = std::mem::take(&mut self.pending);
        self.since = None;
        let written = match &mut self.journal {
            Some(journal) => journal.append(&ops),
            None => self.start(document, map, &ops),
        };
        if written.is_err() {
            self.failed = true;
        }
        written
    }

    fn start(&mut self, document: &Document, map: &Map, ops: &[Op]) -> Result<(), JournalError> {
        let journal = match (document.path(), document.save_generation()) {
            (Some(path), Some(generation)) => {
                Journal::create(&recovery_path(path), &JournalBase::File { generation }, ops)?
            }
            _ => {
                // A Map never saved starts from what it is now, which already has `ops`.
                let Some(path) = self.unsaved_path() else {
                    return Ok(());
                };
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                let base = JournalBase::Unsaved {
                    map: Box::new(map.view()),
                };
                Journal::create(&path, &base, &[])?
            }
        };
        self.journal = Some(journal);
        Ok(())
    }

    /// Starts a journal at once holding `ops`, replacing the one they were recovered from:
    /// after replaying a Map file's journal, or recovering a Map never saved (which starts
    /// from `map` as it is, ops and all).
    pub fn resume(
        &mut self,
        document: &Document,
        map: &Map,
        found: &Path,
        ops: &[Op],
    ) -> Result<(), JournalError> {
        let deleted = self.reset();
        if document.path().is_none() {
            self.unsaved_path = Some(found.to_path_buf());
        }
        let started = self.start(document, map, ops);
        self.failed = started.is_err();
        started.and(deleted.map_err(JournalError::from))
    }

    /// Forgets queued Ops and deletes the journal, as on Save, open, or discarding changes.
    /// Everything is forgotten even when the delete fails; the journal left behind is offered
    /// again on the next launch, so the caller should say so.
    pub fn reset(&mut self) -> std::io::Result<()> {
        self.pending.clear();
        self.since = None;
        self.failed = false;
        let deleted = match self.journal.take() {
            Some(journal) => journal.delete(),
            None => Ok(()),
        };
        self.unsaved_path = None;
        deleted
    }

    pub fn journal_path(&self) -> Option<&Path> {
        self.journal.as_ref().map(Journal::path)
    }

    fn unsaved_path(&mut self) -> Option<PathBuf> {
        let dir = self.unsaved_dir.as_ref()?;
        Some(
            self.unsaved_path
                .get_or_insert_with(|| dir.join(format!("{}.{EXTENSION}", Uuid::now_v7())))
                .clone(),
        )
    }

    /// The journal beside the Map file at `map_path`, if a crash left one. A journal in use
    /// by another window is not offered.
    pub fn find_beside(map_path: &Path) -> Result<Option<Found>, JournalError> {
        let path = recovery_path(map_path);
        match Recovered::read(&path) {
            Ok(Some(recovered)) => Ok(Some(Found { path, recovered })),
            Ok(None) | Err(JournalError::InUse) => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// Journals of Maps never saved that a crash left behind, newest first. Unreadable ones
    /// and those in use by another window are left alone.
    pub fn find_unsaved(&self) -> Vec<Found> {
        let Some(entries) = self
            .unsaved_dir
            .as_ref()
            .and_then(|dir| std::fs::read_dir(dir).ok())
        else {
            return Vec::new();
        };
        let mut found: Vec<(SystemTime, Found)> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == EXTENSION))
            .filter_map(|path| {
                let recovered = Recovered::read(&path).ok()??;
                if !matches!(recovered.base, JournalBase::Unsaved { .. }) {
                    return None;
                }
                let modified = path
                    .metadata()
                    .and_then(|meta| meta.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                Some((modified, Found { path, recovered }))
            })
            .collect();
        found.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
        found.into_iter().map(|(_, found)| found).collect()
    }
}

/// A free path for a recovered copy of the Map file at `path`: `<name> (recovered).ssmap`,
/// numbered if that exists.
pub(crate) fn recovered_copy_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let extension = crate::document::EXTENSION;
    (1..)
        .map(|n| {
            let suffix = if n == 1 {
                String::new()
            } else {
                format!(" {n}")
            };
            path.with_file_name(format!("{stem} (recovered{suffix}).{extension}"))
        })
        .find(|candidate| !candidate.exists())
        .expect("some numbered name is free")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovered_copies_take_the_first_free_name() {
        let dir = tempfile::tempdir().unwrap();
        let map = dir.path().join("Coast.ssmap");
        let first = recovered_copy_path(&map);
        assert_eq!(first, dir.path().join("Coast (recovered).ssmap"));
        std::fs::write(&first, "").unwrap();
        assert_eq!(
            recovered_copy_path(&map),
            dir.path().join("Coast (recovered 2).ssmap")
        );
    }

    #[test]
    fn ops_are_due_after_the_flush_delay() {
        let mut recovery = Recovery::new(None);
        let start = Instant::now();
        assert_eq!(recovery.due_in(start), None);
        recovery.record(Vec::new(), start);
        assert_eq!(recovery.due_in(start), None, "nothing to write");
        recovery.record(
            vec![Op::Remove {
                id: ssmap_core::Id::new(),
            }],
            start,
        );
        assert_eq!(recovery.due_in(start), Some(FLUSH_DELAY));
        assert_eq!(
            recovery.due_in(start + FLUSH_DELAY * 2),
            Some(Duration::ZERO)
        );
        assert!(recovery.reset().is_ok());
        assert!(!recovery.has_pending());
    }

    #[test]
    fn reset_returns_the_delete_error_and_forgets_everything() {
        let dir = tempfile::tempdir().unwrap();
        let mut recovery = Recovery::new(Some(dir.path().to_path_buf()));
        let op = Op::Remove {
            id: ssmap_core::Id::new(),
        };
        recovery.record(vec![op], Instant::now());
        let map = Map::new(ssmap_core::Canvas::new(
            100,
            100,
            ssmap_core::Scale::new(1.0, ssmap_core::DistanceUnit::Miles),
        ));
        recovery.flush(&Document::recovered(), &map).unwrap();
        let journal = recovery.journal_path().unwrap().to_path_buf();
        // A directory where the journal was can't be removed as a file.
        std::fs::remove_file(&journal).unwrap();
        std::fs::create_dir(&journal).unwrap();

        assert!(recovery.reset().is_err());
        assert!(recovery.journal_path().is_none());
        assert!(!recovery.has_pending());
    }
}
