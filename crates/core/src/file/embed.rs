//! The embedded Assets and fonts of a Map file (format 1.1).
//!
//! - `blobs (hash, data)`: each distinct file once, keyed by the 32-byte SHA-256 of its bytes.
//! - `embedded (kind, key, props)`: one row per embedded `asset` (keyed `pack-id/asset-id`)
//!   and `font` (keyed by file name), with its record as JSON.
//!
//! Both tables are created by the first save of this version, so a format 1.0 file gains them
//! when it is next saved.

use std::collections::{BTreeMap, HashSet};

use rusqlite::{Connection, OptionalExtension, Transaction};

use crate::asset::AssetRef;
use crate::embedded::{Blob, ContentHash, Embedded};

use super::FileError;

pub(super) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS blobs (
        hash BLOB PRIMARY KEY NOT NULL,
        data BLOB NOT NULL
    );
    CREATE TABLE IF NOT EXISTS embedded (
        kind TEXT NOT NULL,
        key TEXT NOT NULL,
        props TEXT NOT NULL,
        PRIMARY KEY (kind, key)
    );
";

const ASSET: &str = "asset";
const FONT: &str = "font";

/// What the file holds of the embedded state, to tell what changed since.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub(super) struct SavedEmbedded {
    blobs: HashSet<ContentHash>,
    /// Record JSON by (kind, key).
    records: BTreeMap<(&'static str, String), String>,
}

/// Writes the blobs and records that changed since `saved`, and returns the new saved state.
pub(super) fn write(
    tx: &Transaction<'_>,
    embedded: &Embedded,
    saved: &SavedEmbedded,
) -> Result<SavedEmbedded, FileError> {
    tx.execute_batch(SCHEMA)?;
    let mut next = SavedEmbedded::default();

    let mut insert =
        tx.prepare_cached("INSERT OR REPLACE INTO blobs (hash, data) VALUES (?1, ?2)")?;
    for (hash, blob) in &embedded.blobs {
        if !saved.blobs.contains(hash) {
            insert.execute(rusqlite::params![hash.as_bytes().as_slice(), &**blob])?;
        }
        next.blobs.insert(*hash);
    }
    let mut delete = tx.prepare_cached("DELETE FROM blobs WHERE hash = ?1")?;
    for hash in saved.blobs.difference(&next.blobs) {
        delete.execute([hash.as_bytes().as_slice()])?;
    }

    let records = embedded
        .assets
        .iter()
        .map(|(reference, record)| (ASSET, reference.to_string(), json(record)))
        .chain(
            embedded
                .fonts
                .iter()
                .map(|(file, record)| (FONT, file.clone(), json(record))),
        );
    let mut upsert = tx.prepare_cached(
        "INSERT INTO embedded (kind, key, props) VALUES (?1, ?2, ?3)
         ON CONFLICT (kind, key) DO UPDATE SET props = excluded.props",
    )?;
    for (kind, key, props) in records {
        if saved.records.get(&(kind, key.clone())) != Some(&props) {
            upsert.execute(rusqlite::params![kind, key, props])?;
        }
        next.records.insert((kind, key), props);
    }
    let mut delete = tx.prepare_cached("DELETE FROM embedded WHERE kind = ?1 AND key = ?2")?;
    for (kind, key) in saved.records.keys() {
        if !next.records.contains_key(&(*kind, key.clone())) {
            delete.execute(rusqlite::params![kind, key])?;
        }
    }
    Ok(next)
}

#[allow(clippy::expect_used, reason = "an embedded record serialises")]
fn json<T: serde::Serialize>(record: &T) -> String {
    serde_json::to_string(record).expect("an embedded record serialises")
}

/// Reads the embedded state of a file, which is empty for a file saved before format 1.1.
pub(super) fn read(conn: &Connection) -> Result<(Embedded, SavedEmbedded), FileError> {
    let mut embedded = Embedded::default();
    let mut saved = SavedEmbedded::default();
    let has_tables: bool = conn
        .query_row(
            "SELECT COUNT(*) = 2 FROM sqlite_master
             WHERE type = 'table' AND name IN ('blobs', 'embedded')",
            [],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(false);
    if !has_tables {
        return Ok((embedded, saved));
    }

    let mut statement = conn.prepare("SELECT hash, data FROM blobs")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let hash = <[u8; 32]>::try_from(row.get_ref(0)?.as_blob()?)
            .map(ContentHash::from_bytes)
            .map_err(|_| FileError::Damaged("an embedded file's hash is not 32 bytes".into()))?;
        let data = Blob::new(row.get_ref(1)?.as_blob()?);
        embedded.blobs.insert(hash, data);
        saved.blobs.insert(hash);
    }

    let mut statement = conn.prepare("SELECT kind, key, props FROM embedded")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let kind = row.get_ref(0)?.as_str()?;
        let key: String = row.get(1)?;
        let props: String = row.get(2)?;
        let damaged = |err: serde_json::Error| {
            FileError::Damaged(format!("the embedded {kind} {key}: {err}"))
        };
        let kind = match kind {
            ASSET => {
                let reference: AssetRef = key.parse().map_err(|err| {
                    FileError::Damaged(format!("an embedded Asset reference: {err}"))
                })?;
                let record = serde_json::from_str(&props).map_err(damaged)?;
                embedded.assets.insert(reference, record);
                ASSET
            }
            FONT => {
                let record = serde_json::from_str(&props).map_err(damaged)?;
                embedded.fonts.insert(key.clone(), record);
                FONT
            }
            // A kind a newer minor version added is left alone in the file.
            _ => continue,
        };
        saved.records.insert((kind, key), props);
    }
    if let Some(hash) = embedded.first_missing_blob() {
        return Err(FileError::Damaged(format!(
            "an embedded record names the file {hash}, which is missing"
        )));
    }
    Ok((embedded, saved))
}
