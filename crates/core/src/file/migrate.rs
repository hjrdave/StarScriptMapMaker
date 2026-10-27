//! Step-by-step upgrades of Map files from older major format versions (ADR 0005).

use std::path::{Path, PathBuf};

use rusqlite::{OpenFlags, Transaction, TransactionBehavior};

use super::{FORMAT_VERSION, FileError, FormatVersion, connect};

/// Upgrades a file from major version `from_major` to `from_major + 1`.
pub(super) struct Migration {
    pub(super) from_major: u32,
    pub(super) run: fn(&Transaction<'_>) -> rusqlite::Result<()>,
}

/// Format 1 is the first, so there is nothing to upgrade from yet.
pub(super) const MIGRATIONS: &[Migration] = &[];

/// Copies the file to its `.bak` path, then runs every step from `found` up to
/// [`FORMAT_VERSION`] in one transaction, so a failed upgrade leaves the file as it was.
pub(super) fn run(
    path: &Path,
    found: FormatVersion,
    migrations: &[Migration],
) -> Result<PathBuf, FileError> {
    let steps = (found.major..FORMAT_VERSION.major)
        .map(|major| migrations.iter().find(|m| m.from_major == major))
        .collect::<Option<Vec<_>>>()
        .ok_or(FileError::NoMigration { found })?;

    let backup = backup_path(path, found);
    std::fs::copy(path, &backup)?;
    let mut conn = connect(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for step in steps {
        (step.run)(&tx)?;
    }
    super::write_meta(&tx, "format_version", FORMAT_VERSION.to_string())?;
    tx.commit()?;
    Ok(backup)
}

/// `world.ssmap` at version 0.3 is kept as `world.v0.3.bak` beside it.
pub(super) fn backup_path(path: &Path, version: FormatVersion) -> PathBuf {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!("{stem}.v{version}.bak"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::{MapFile, read_meta};
    use crate::map::{Canvas, DistanceUnit, Map, Scale};

    /// A pretend format 0 that stored the Canvas under another key.
    const FROM_ZERO: Migration = Migration {
        from_major: 0,
        run: |tx| {
            tx.execute(
                "UPDATE meta SET key = 'canvas' WHERE key = 'old_canvas'",
                [],
            )?;
            Ok(())
        },
    };

    fn old_file(dir: &Path) -> PathBuf {
        let path = dir.join("world.ssmap");
        let map = Map::new(Canvas::new(
            300,
            200,
            Scale::new(2.0, DistanceUnit::Kilometres),
        ));
        MapFile::create(&path, &map, None).unwrap();
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "UPDATE meta SET value = '0.3' WHERE key = 'format_version';
             UPDATE meta SET key = 'old_canvas' WHERE key = 'canvas';",
        )
        .unwrap();
        path
    }

    #[test]
    fn upgrades_after_keeping_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = old_file(dir.path());
        let (file, map) = MapFile::open_with(&path, &[FROM_ZERO]).unwrap();
        assert_eq!(map.canvas().width, 300);
        assert_eq!(file.format_version(), FORMAT_VERSION);

        let backup = dir.path().join("world.v0.3.bak");
        let conn = connect(&backup, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let version: Option<String> = read_meta(&conn, "format_version").unwrap();
        assert_eq!(version.as_deref(), Some("0.3"));
        let current = connect(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let version: Option<String> = read_meta(&current, "format_version").unwrap();
        assert_eq!(version, Some(FORMAT_VERSION.to_string()));
    }

    #[test]
    fn refuses_without_a_migration_and_leaves_the_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = old_file(dir.path());
        let before = std::fs::read(&path).unwrap();
        let err = MapFile::open_with(&path, &[]).unwrap_err();
        assert!(matches!(err, FileError::NoMigration { .. }), "{err}");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!dir.path().join("world.v0.3.bak").exists());
    }

    #[test]
    fn a_failed_migration_changes_nothing() {
        let failing = Migration {
            from_major: 0,
            run: |tx| {
                tx.execute(
                    "UPDATE meta SET key = 'canvas' WHERE key = 'old_canvas'",
                    [],
                )?;
                tx.execute("SELECT * FROM no_such_table", [])?;
                Ok(())
            },
        };
        let dir = tempfile::tempdir().unwrap();
        let path = old_file(dir.path());
        assert!(MapFile::open_with(&path, &[failing]).is_err());
        let conn = connect(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let version: Option<String> = read_meta(&conn, "format_version").unwrap();
        assert_eq!(version.as_deref(), Some("0.3"));
    }
}
