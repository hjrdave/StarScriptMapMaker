//! The Recovery journal (ADR 0005): the Ops of every committed Undo step since the last save,
//! appended to a sidecar file so that work survives a crash.
//!
//! The file is JSON lines. The first line is a header naming what the Ops apply to: a Map
//! file at a save generation, or for a Map never saved, a snapshot of where it started. Each
//! later line is a JSON array of Ops, appended and synced as one write. A crash mid-append
//! leaves at most a torn last line, which reading drops.
//!
//! The writer holds an exclusive lock on the file while the Map is open, so another window
//! never mistakes a live journal for a crashed one.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::map::{Map, MapView};
use crate::op::{Op, OpError};

/// The `journal` field of every header, to tell a Recovery journal from any other file.
const MAGIC: &str = "ssmap-recovery";
/// The journal format this version writes and reads.
const JOURNAL_VERSION: u32 = 1;

/// What a journal's Ops apply to.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "base", rename_all = "snake_case")]
pub enum JournalBase {
    /// The Map file beside the journal, as of the save that stamped `generation`.
    File { generation: Uuid },
    /// A Map never saved, as it was when the editor opened it.
    Unsaved { map: Box<MapView> },
}

#[derive(Serialize, Deserialize)]
struct Header {
    journal: String,
    version: u32,
    #[serde(flatten)]
    base: JournalBase,
}

#[derive(Debug, Error)]
pub enum JournalError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("the Recovery journal is in use by another window")]
    InUse,
    #[error("this is not a Recovery journal")]
    NotAJournal,
    #[error("the Recovery journal was written by a newer version (journal format {0})")]
    NewerVersion(u32),
}

/// The path of the Recovery journal of the Map file at `map_path`: `<map>.recovery`.
pub fn recovery_path(map_path: &Path) -> PathBuf {
    let mut name = map_path.as_os_str().to_os_string();
    name.push(".recovery");
    name.into()
}

/// An open Recovery journal being written.
#[derive(Debug)]
pub struct Journal {
    path: PathBuf,
    /// Holds the exclusive lock.
    file: File,
}

impl Journal {
    /// Starts a journal at `path` holding `ops`, replacing any journal there that is not in
    /// use. It is written beside `path` and renamed over it, so a crash never leaves a
    /// journal without its header.
    pub fn create(path: &Path, base: &JournalBase, ops: &[Op]) -> Result<Self, JournalError> {
        if let Some(existing) = open_existing(path)? {
            lock_shared(&existing)?;
        }
        let mut name = path
            .file_name()
            .ok_or_else(|| std::io::Error::other("the path has no file name"))?
            .to_os_string();
        name.push(".new");
        let temp = path.with_file_name(name);
        let mut file = File::create(&temp)?;
        lock_exclusive(&file)?;
        let header = Header {
            journal: MAGIC.into(),
            version: JOURNAL_VERSION,
            base: base.clone(),
        };
        let mut text = serde_json::to_string(&header).expect("a header serialises");
        text.push('\n');
        if !ops.is_empty() {
            text.push_str(&ops_line(ops));
        }
        let written = file
            .write_all(text.as_bytes())
            .and_then(|()| file.sync_data())
            .and_then(|()| std::fs::rename(&temp, path));
        if let Err(err) = written {
            drop(file);
            let _ = std::fs::remove_file(&temp);
            return Err(err.into());
        }
        Ok(Self {
            path: path.to_path_buf(),
            file,
        })
    }

    /// Appends `ops` as one line and syncs it to disk.
    pub fn append(&mut self, ops: &[Op]) -> Result<(), JournalError> {
        if ops.is_empty() {
            return Ok(());
        }
        self.file.write_all(ops_line(ops).as_bytes())?;
        self.file.sync_data()?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Deletes the journal, as on Save or a clean close.
    pub fn delete(self) -> std::io::Result<()> {
        let Self { path, file } = self;
        drop(file);
        remove_if_present(&path)
    }
}

fn ops_line(ops: &[Op]) -> String {
    let mut line = serde_json::to_string(ops).expect("Ops serialise");
    line.push('\n');
    line
}

/// A journal read back after a crash.
#[derive(Clone, PartialEq, Debug)]
pub struct Recovered {
    pub base: JournalBase,
    /// Every Op that reached the disk, in the order they were applied.
    pub ops: Vec<Op>,
    /// Whether the last line was torn by a crash mid-append and dropped.
    pub torn: bool,
}

impl Recovered {
    /// Reads the journal at `path`, or `None` if there is none. Fails with
    /// [`JournalError::InUse`] while another window writes it.
    pub fn read(path: &Path) -> Result<Option<Self>, JournalError> {
        let Some(file) = open_existing(path)? else {
            return Ok(None);
        };
        lock_shared(&file)?;
        let mut lines = BufReader::new(&file).lines();
        let header = lines.next().transpose()?.ok_or(JournalError::NotAJournal)?;
        let header: Header =
            serde_json::from_str(&header).map_err(|_| JournalError::NotAJournal)?;
        if header.journal != MAGIC {
            return Err(JournalError::NotAJournal);
        }
        if header.version > JOURNAL_VERSION {
            return Err(JournalError::NewerVersion(header.version));
        }
        let mut ops = Vec::new();
        let mut torn = false;
        for line in lines {
            // A line that doesn't parse can only be the torn last append; nothing follows it.
            match line.map(|line| serde_json::from_str::<Vec<Op>>(&line)) {
                Ok(Ok(batch)) => ops.extend(batch),
                _ => {
                    torn = true;
                    break;
                }
            }
        }
        Ok(Some(Self {
            base: header.base,
            ops,
            torn,
        }))
    }

    /// Whether the Ops apply to the Map file whose last save stamped `generation`.
    pub fn matches(&self, generation: Uuid) -> bool {
        self.base == JournalBase::File { generation }
    }

    /// Applies every Op to `map`, all or nothing. On error the Map is unchanged.
    pub fn replay(self, map: &mut Map) -> Result<(), OpError> {
        let mut inverses = Vec::with_capacity(self.ops.len());
        for op in self.ops {
            match map.apply(op) {
                Ok(inverse) => inverses.push(inverse),
                Err(err) => {
                    map.revert(inverses);
                    return Err(err);
                }
            }
        }
        Ok(())
    }

    /// Applies every Op that still applies to `map`, which may not be the Map the journal was
    /// written against, and returns how many were skipped. For a recovered copy.
    pub fn replay_lenient(self, map: &mut Map) -> usize {
        self.ops
            .into_iter()
            .filter(|op| map.apply(op.clone()).is_err())
            .count()
    }
}

/// Deletes the journal at `path`, if there is one.
pub fn remove_journal(path: &Path) -> std::io::Result<()> {
    remove_if_present(path)
}

fn open_existing(path: &Path) -> std::io::Result<Option<File>> {
    match OpenOptions::new().read(true).open(path) {
        Ok(file) => Ok(Some(file)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

fn lock_shared(file: &File) -> Result<(), JournalError> {
    match file.try_lock_shared() {
        Ok(()) => Ok(()),
        Err(TryLockError::WouldBlock) => Err(JournalError::InUse),
        Err(TryLockError::Error(err)) => Err(err.into()),
    }
}

fn lock_exclusive(file: &File) -> Result<(), JournalError> {
    match file.try_lock() {
        Ok(()) => Ok(()),
        Err(TryLockError::WouldBlock) => Err(JournalError::InUse),
        Err(TryLockError::Error(err)) => Err(err.into()),
    }
}

fn remove_if_present(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}
