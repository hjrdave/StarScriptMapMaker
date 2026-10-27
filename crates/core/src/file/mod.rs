//! The Map file: a self-contained SQLite database (ADR 0005).
//!
//! Tables:
//!
//! - `meta (key, value)`: `format_version` (`"major.minor"`), `save_generation` (a UUID), the
//!   `canvas` and the Map-wide `defaults` as JSON, and a `preview` PNG at most [`PREVIEW_SIZE`] pixels on its longer side.
//! - `nodes (id, parent, ord, kind, props, geometry)`: one row per Layer and Object. `id` and
//!   `parent` are 16-byte UUIDs, and `parent` is NULL for a top-level Layer. `ord` is the
//!   index among siblings, 0 drawing lowest. `kind` is `layer` or the Object kind. `props` is
//!   the node as JSON without its outline, and `geometry` is the outline of a Landmass or Lake
//!   as a binary blob (see `blob.rs`).
//!
//! - `blobs (hash, data)` and `embedded (kind, key, props)`: the Assets and fonts the Map
//!   embeds (see `embed.rs`). Added in format 1.1; a 1.0 file gains them on its next save.
//!   Format 1.2 adds the Label styles to the Map defaults; older files get the default style.
//!   Format 1.3 adds the `export_settings` meta key (JSON); older files get the defaults.
//!   Format 1.4 adds the optional `arch` to Labels and Label styles; older files get no arch.
//!   Format 1.5 adds the optional `background` block to the Canvas; older files get the default.
//!   Format 1.9 adds the optional `template` (the name of the template the Map was made from) to the Canvas.
//!   Format 1.6 stores Groups: Layers nested inside Layers, with `parent` pointing at the
//!   enclosing Layer. There is no compatibility with older files.
//!
//! Saves are incremental: only rows whose node changed are written, inside one SQLite
//! transaction, so a crash mid-save leaves the previous save intact. Save As and new files
//! are written beside the destination and renamed over it, for the same reason.
//!
//! Between saves, a Recovery journal beside the file records the Ops applied since (see
//! [`Journal`]).
//!
//! Rows, meta keys, columns and tables this version doesn't know are left alone, and unknown
//! JSON fields ride along in each object's [`Extra`](crate::Extra), so a file from a newer
//! minor format version round-trips without loss.

mod blob;
mod embed;
mod journal;
mod migrate;

pub use blob::BlobError;
pub use journal::{Journal, JournalBase, JournalError, Recovered, recovery_path, remove_journal};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior};
use thiserror::Error;
use uuid::Uuid;

use crate::export::ExportSettings;
use crate::id::Id;
use crate::map::{Canvas, Map, MapDefaults, MapView, Subtree};
use crate::node::{Node, Object};

/// The format this version writes. A minor bump only adds optional fields; a major bump is
/// anything else, and needs a migration.
pub const FORMAT_VERSION: FormatVersion = FormatVersion { major: 1, minor: 9 };

/// The longer side of the preview image stored in every save, in pixels.
pub const PREVIEW_SIZE: u32 = 512;

/// `PRAGMA application_id` of every Map file: "SSMP".
const APPLICATION_ID: i32 = 0x5353_4d50;

const SCHEMA: &str = "
    CREATE TABLE meta (
        key TEXT PRIMARY KEY NOT NULL,
        value
    );
    CREATE TABLE nodes (
        id BLOB PRIMARY KEY NOT NULL,
        parent BLOB,
        ord INTEGER NOT NULL,
        kind TEXT NOT NULL,
        props TEXT NOT NULL,
        geometry BLOB
    );
    CREATE INDEX nodes_by_parent ON nodes (parent, ord);
";

const UPSERT_NODE: &str = "
    INSERT INTO nodes (id, parent, ord, kind, props, geometry) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
    ON CONFLICT (id) DO UPDATE SET
        parent = excluded.parent,
        ord = excluded.ord,
        kind = excluded.kind,
        props = excluded.props,
        geometry = excluded.geometry
";

/// A Map file version, written as `major.minor`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct FormatVersion {
    /// The major version. A newer major version cannot be opened.
    pub major: u32,
    /// The minor version. Newer minor versions open, keeping what they add.
    pub minor: u32,
}

impl fmt::Display for FormatVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

impl FromStr for FormatVersion {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        let (major, minor) = s.split_once('.').ok_or(())?;
        Ok(Self {
            major: major.parse().map_err(|_| ())?,
            minor: minor.parse().map_err(|_| ())?,
        })
    }
}

/// Why a Map file could not be opened, saved or recovered.
#[derive(Debug, Error)]
pub enum FileError {
    /// Reading or writing the file failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The file is not a StarScript Map file.
    #[error("this is not a StarScript Map file")]
    NotAMapFile,
    /// The file was saved by a newer major format version.
    #[error(
        "this Map was saved by a newer StarScript Map Maker (file format {found}); this \
         version opens format {}.x and older",
        FORMAT_VERSION.major
    )]
    NewerMajor {
        /// The file's format version.
        found: FormatVersion,
    },
    /// The file's format version has no upgrade to the current one.
    #[error("this Map uses file format {found}, which this version can't upgrade")]
    NoMigration {
        /// The file's format version.
        found: FormatVersion,
    },
    /// The file's contents are not a valid Map.
    #[error("the Map file is damaged: {0}")]
    Damaged(String),
    /// A Landmass or Lake's outline could not be read.
    #[error("the Map file is damaged: the outline of {id} is unreadable")]
    Outline {
        /// The Landmass or Lake.
        id: Id,
        /// Why the outline is unreadable.
        #[source]
        source: BlobError,
    },
    /// A column holds a value of the wrong type.
    #[error("the Map file is damaged: a column holds the wrong type")]
    WrongColumnType(#[from] rusqlite::types::FromSqlError),
    /// The path is not valid Unicode.
    #[error("the file path must be valid Unicode: {0}")]
    PathNotUnicode(PathBuf),
    /// The database reported an error.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

/// What a save wrote, which shows that saves are incremental.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SaveStats {
    /// Rows written in full: new or changed nodes.
    pub written: usize,
    /// Rows whose node only changed place among its siblings.
    pub moved: usize,
    /// Rows removed because their node was deleted.
    pub deleted: usize,
}

/// The saved state of one row, to tell what changed since.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct SavedRow {
    /// [`Map::version`] when it was saved.
    version: u64,
    parent: Option<Id>,
    ord: usize,
}

/// An open Map file. It keeps the SQLite connection open and remembers what it last saved, so
/// the next save writes only what changed.
///
/// Every save must be given the Map this file was opened or last saved with, changed since
/// only by Ops: change tracking relies on that Map's node versions.
#[derive(Debug)]
pub struct MapFile {
    path: PathBuf,
    conn: Connection,
    /// The version written on save: ours, or a newer minor the file already had.
    version: FormatVersion,
    generation: Uuid,
    saved: HashMap<Id, SavedRow>,
    saved_canvas: Option<Canvas>,
    saved_defaults: Option<MapDefaults>,
    saved_export: Option<ExportSettings>,
    saved_embedded: embed::SavedEmbedded,
    saved_revision: u64,
}

impl MapFile {
    /// Saves `map` as a new file at `path`, replacing any file there.
    pub fn create(
        path: impl AsRef<Path>,
        map: &Map,
        preview: Option<&[u8]>,
    ) -> Result<(Self, SaveStats), FileError> {
        let path = path.as_ref();
        save_via_temp(path, map, preview, |temp| {
            let conn = Connection::open(temp)?;
            configure(&conn)?;
            conn.pragma_update(None, "application_id", APPLICATION_ID)?;
            conn.execute_batch(SCHEMA)?;
            Ok(Self {
                path: temp.to_path_buf(),
                conn,
                version: FORMAT_VERSION,
                generation: Uuid::nil(),
                saved: HashMap::new(),
                saved_canvas: None,
                saved_defaults: None,
                saved_export: None,
                saved_embedded: Default::default(),
                saved_revision: 0,
            })
        })
    }

    /// Opens a Map file. Files from an older major version are upgraded in place, after a
    /// copy is kept as `<name>.v<major>.<minor>.bak`.
    pub fn open(path: impl AsRef<Path>) -> Result<(Self, Map), FileError> {
        Self::open_with(path.as_ref(), migrate::MIGRATIONS)
    }

    fn open_with(path: &Path, migrations: &[migrate::Migration]) -> Result<(Self, Map), FileError> {
        let mut conn = connect(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        let found = read_version(&conn)?;
        if found.major > FORMAT_VERSION.major {
            return Err(FileError::NewerMajor { found });
        }
        if found.major < FORMAT_VERSION.major {
            drop(conn);
            migrate::run(path, found, migrations)?;
            conn = connect(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        }
        let version = if found.major == FORMAT_VERSION.major {
            found.max(FORMAT_VERSION)
        } else {
            FORMAT_VERSION
        };

        let canvas_json: String = read_meta(&conn, "canvas")?
            .ok_or_else(|| FileError::Damaged("the Canvas is missing".into()))?;
        let canvas: Canvas = serde_json::from_str(&canvas_json)
            .map_err(|err| FileError::Damaged(format!("the Canvas: {err}")))?;
        // Absent from files saved before the Map had defaults.
        let defaults: MapDefaults = match read_meta::<String>(&conn, "defaults")? {
            Some(json) => serde_json::from_str(&json)
                .map_err(|err| FileError::Damaged(format!("the Map defaults: {err}")))?,
            None => MapDefaults::default(),
        };
        // Absent from files saved before format 1.3.
        let export_settings: ExportSettings = match read_meta::<String>(&conn, "export_settings")? {
            Some(json) => serde_json::from_str(&json)
                .map_err(|err| FileError::Damaged(format!("the export settings: {err}")))?,
            None => ExportSettings::default(),
        };
        let generation = read_meta::<String>(&conn, "save_generation")?
            .and_then(|text| Uuid::parse_str(&text).ok())
            .unwrap_or_else(Uuid::nil);
        let (layers, saved) = read_nodes(&conn)?;
        let (embedded, saved_embedded) = embed::read(&conn)?;
        let map = Map::from_view(MapView {
            canvas: canvas.clone(),
            defaults: defaults.clone(),
            layers,
            embedded,
            export_settings: export_settings.clone(),
        })
        .map_err(|err| FileError::Damaged(err.to_string()))?;
        let file = Self {
            path: path.to_path_buf(),
            conn,
            version,
            generation,
            saved,
            saved_canvas: Some(canvas),
            saved_defaults: Some(defaults),
            saved_export: Some(export_settings),
            saved_embedded,
            saved_revision: map.revision(),
        };
        Ok((file, map))
    }

    /// Writes what changed in `map` since the last save, as one transaction, and stamps a new
    /// save generation. A `preview` PNG replaces the stored one; without one, the stale
    /// preview is removed.
    #[allow(
        clippy::expect_used,
        reason = "a Canvas serialises; settings serialise; defaults serialise"
    )]
    pub fn save(&mut self, map: &Map, preview: Option<&[u8]>) -> Result<SaveStats, FileError> {
        let generation = Uuid::now_v7();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (stats, saved) = write_nodes(&tx, map, &self.saved)?;
        if self.saved_canvas.as_ref() != Some(&map.canvas) {
            let canvas = serde_json::to_string(&map.canvas).expect("a Canvas serialises");
            write_meta(&tx, "canvas", &canvas)?;
        }
        if self.saved_export.as_ref() != Some(&map.export_settings) {
            let json = serde_json::to_string(&map.export_settings).expect("settings serialise");
            write_meta(&tx, "export_settings", &json)?;
        }
        if self.saved_defaults.as_ref() != Some(map.defaults()) {
            let defaults = serde_json::to_string(map.defaults()).expect("defaults serialise");
            write_meta(&tx, "defaults", &defaults)?;
        }
        let saved_embedded = embed::write(&tx, map.embedded(), &self.saved_embedded)?;
        write_meta(&tx, "format_version", self.version.to_string())?;
        write_meta(&tx, "save_generation", generation.to_string())?;
        match preview {
            Some(png) => write_meta(&tx, "preview", png)?,
            None => {
                tx.execute("DELETE FROM meta WHERE key = 'preview'", [])?;
            }
        }
        tx.commit()?;

        self.saved = saved;
        self.saved_canvas = Some(map.canvas.clone());
        self.saved_defaults = Some(map.defaults().clone());
        self.saved_export = Some(map.export_settings.clone());
        self.saved_embedded = saved_embedded;
        self.generation = generation;
        self.saved_revision = map.revision();
        Ok(stats)
    }

    /// Saves a full copy of the file with `map`'s changes at `path`, which then becomes this
    /// file. Saving as the current path is a plain save.
    pub fn save_as(
        &mut self,
        path: impl AsRef<Path>,
        map: &Map,
        preview: Option<&[u8]>,
    ) -> Result<SaveStats, FileError> {
        let path = path.as_ref();
        if same_file(path, &self.path) {
            return self.save(map, preview);
        }
        let (copy, stats) = save_via_temp(path, map, preview, |temp| {
            // A byte-level copy of the last save keeps anything this version doesn't know.
            let target = temp
                .to_str()
                .ok_or_else(|| FileError::PathNotUnicode(temp.to_path_buf()))?;
            self.conn.execute("VACUUM INTO ?1", [target])?;
            Ok(Self {
                path: temp.to_path_buf(),
                conn: connect(temp, OpenFlags::SQLITE_OPEN_READ_WRITE)?,
                version: self.version,
                generation: self.generation,
                saved: self.saved.clone(),
                saved_canvas: self.saved_canvas.clone(),
                saved_defaults: self.saved_defaults.clone(),
                saved_export: self.saved_export.clone(),
                saved_embedded: self.saved_embedded.clone(),
                saved_revision: self.saved_revision,
            })
        })?;
        *self = copy;
        Ok(stats)
    }

    /// The path of the open file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The id stamped by the last save. The Recovery journal records it.
    pub fn save_generation(&self) -> Uuid {
        self.generation
    }

    /// The format version this file is saved as.
    pub fn format_version(&self) -> FormatVersion {
        self.version
    }

    /// [`Map::revision`] at the last save or open, to tell whether the Map has unsaved changes.
    pub fn saved_revision(&self) -> u64 {
        self.saved_revision
    }
}

/// Reads the preview PNG of a Map file without opening the Map, as for a recent-files list.
pub fn read_preview(path: impl AsRef<Path>) -> Result<Option<Vec<u8>>, FileError> {
    let conn = connect(path.as_ref(), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    read_version(&conn)?;
    read_meta(&conn, "preview")
}

/// Reads the Canvas of a Map file without opening the Map, as for a Map browser's size label.
pub fn read_canvas(path: impl AsRef<Path>) -> Result<Canvas, FileError> {
    let conn = connect(path.as_ref(), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    read_version(&conn)?;
    let json: String = read_meta(&conn, "canvas")?
        .ok_or_else(|| FileError::Damaged("the Canvas is missing".into()))?;
    serde_json::from_str(&json).map_err(|err| FileError::Damaged(format!("the Canvas: {err}")))
}

/// Writes a file at a temporary path beside `path` with `prepare` and a save, then renames
/// it over `path`, so a crash never leaves a half-written file there.
fn save_via_temp(
    path: &Path,
    map: &Map,
    preview: Option<&[u8]>,
    prepare: impl FnOnce(&Path) -> Result<MapFile, FileError>,
) -> Result<(MapFile, SaveStats), FileError> {
    let mut name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("the path has no file name"))?
        .to_os_string();
    name.push(".saving");
    let temp = path.with_file_name(name);
    remove_if_present(&temp)?;

    let written = prepare(&temp).and_then(|mut file| {
        let stats = file.save(map, preview)?;
        Ok((file, stats))
    });
    let (file, stats) = match written {
        Ok(written) => written,
        Err(err) => {
            let _ = remove_if_present(&temp);
            return Err(err);
        }
    };
    // Close the connection first: Windows can't rename an open file.
    let MapFile {
        conn,
        version,
        generation,
        saved,
        saved_canvas,
        saved_defaults,
        saved_export,
        saved_embedded,
        saved_revision,
        ..
    } = file;
    conn.close().map_err(|(_, err)| err)?;
    if let Err(err) = std::fs::rename(&temp, path) {
        let _ = remove_if_present(&temp);
        return Err(err.into());
    }
    let file = MapFile {
        path: path.to_path_buf(),
        conn: connect(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?,
        version,
        generation,
        saved,
        saved_canvas,
        saved_defaults,
        saved_export,
        saved_embedded,
        saved_revision,
    };
    Ok((file, stats))
}

fn remove_if_present(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Opens an existing database file.
fn connect(path: &Path, access: OpenFlags) -> Result<Connection, FileError> {
    if !path.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{} does not exist", path.display()),
        )
        .into());
    }
    let conn = Connection::open_with_flags(path, access | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    if access.contains(OpenFlags::SQLITE_OPEN_READ_WRITE) {
        configure(&conn)?;
    }
    Ok(conn)
}

/// A rollback journal keeps the Map in one file between saves, and full syncs make each
/// committed save durable.
fn configure(conn: &Connection) -> Result<(), FileError> {
    conn.pragma_update(None, "journal_mode", "DELETE")
        .map_err(not_a_map_file)?;
    conn.pragma_update(None, "synchronous", "FULL")?;
    Ok(())
}

fn not_a_map_file(err: rusqlite::Error) -> FileError {
    match err.sqlite_error_code() {
        Some(rusqlite::ErrorCode::NotADatabase) => FileError::NotAMapFile,
        _ => FileError::Sqlite(err),
    }
}

fn read_version(conn: &Connection) -> Result<FormatVersion, FileError> {
    let id: i32 = conn
        .pragma_query_value(None, "application_id", |row| row.get(0))
        .map_err(not_a_map_file)?;
    if id != APPLICATION_ID {
        return Err(FileError::NotAMapFile);
    }
    let text: String = read_meta(conn, "format_version")?
        .ok_or_else(|| FileError::Damaged("the format version is missing".into()))?;
    text.parse()
        .map_err(|()| FileError::Damaged(format!("bad format version {text:?}")))
}

fn read_meta<T: rusqlite::types::FromSql>(
    conn: &Connection,
    key: &str,
) -> Result<Option<T>, FileError> {
    Ok(conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()?)
}

fn write_meta(
    tx: &Transaction<'_>,
    key: &str,
    value: impl rusqlite::ToSql,
) -> Result<(), FileError> {
    tx.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

/// Writes rows for new and changed nodes, moves rows whose place changed, deletes rows of
/// removed nodes, and returns the new saved state.
fn write_nodes(
    tx: &Transaction<'_>,
    map: &Map,
    saved: &HashMap<Id, SavedRow>,
) -> Result<(SaveStats, HashMap<Id, SavedRow>), FileError> {
    let mut stats = SaveStats::default();
    let mut next = HashMap::with_capacity(map.entries.len());
    let mut upsert = tx.prepare_cached(UPSERT_NODE)?;
    let mut place = tx.prepare_cached("UPDATE nodes SET parent = ?2, ord = ?3 WHERE id = ?1")?;

    let mut stack: Vec<Option<Id>> = vec![None];
    while let Some(parent) = stack.pop() {
        for (ord, &id) in map.children(parent).iter().enumerate() {
            let row = SavedRow {
                version: map.version(id).ok_or_else(|| unlisted(id))?,
                parent,
                ord,
            };
            match saved.get(&id) {
                Some(old) if *old == row => {}
                Some(old) if old.version == row.version => {
                    place.execute(rusqlite::params![
                        id_bytes(id),
                        parent.map(id_bytes),
                        ord as i64
                    ])?;
                    stats.moved += 1;
                }
                _ => {
                    let node = map.node(id).ok_or_else(|| unlisted(id))?;
                    let (kind, props, geometry) = encode_node(node);
                    upsert.execute(rusqlite::params![
                        id_bytes(id),
                        parent.map(id_bytes),
                        ord as i64,
                        kind,
                        props,
                        geometry,
                    ])?;
                    stats.written += 1;
                }
            }
            next.insert(id, row);
            if map.node(id).is_some_and(Node::is_layer) {
                stack.push(Some(id));
            }
        }
    }

    let mut delete = tx.prepare_cached("DELETE FROM nodes WHERE id = ?1")?;
    for id in saved.keys().filter(|id| !next.contains_key(id)) {
        delete.execute([id_bytes(*id)])?;
        stats.deleted += 1;
    }
    Ok((stats, next))
}

/// The error for a node its parent lists but the Map lacks, which a consistent Map never has.
fn unlisted(id: Id) -> FileError {
    FileError::Damaged(format!("node {id} is listed but missing"))
}

/// The `kind`, `props` and `geometry` columns of a node.
#[allow(clippy::expect_used, reason = "a node serialises")]
fn encode_node(node: &Node) -> (&'static str, String, Option<Vec<u8>>) {
    let to_json = |node: &Node| serde_json::to_string(node).expect("a node serialises");
    match node {
        Node::Layer(_) => ("layer", to_json(node), None),
        Node::Object(object) => match object.kind.geometry() {
            None => (object.kind.name(), to_json(node), None),
            Some(geometry) => {
                let stripped = Node::Object(Object {
                    name: object.name.clone(),
                    locked: object.locked,
                    membership: object.membership,
                    kind: object.kind.without_geometry(),
                    link: object.link,
                    extra: object.extra.clone(),
                });
                let mut props = serde_json::to_value(&stripped).expect("a node serialises");
                if let Some(kind) = props.get_mut("kind").and_then(|kind| kind.as_object_mut()) {
                    kind.remove("geometry");
                }
                (
                    object.kind.name(),
                    props.to_string(),
                    Some(blob::encode(geometry)),
                )
            }
        },
    }
}

/// Reads every row back into the top-level Layers, and the saved state of each row.
fn read_nodes(conn: &Connection) -> Result<(Vec<Subtree>, HashMap<Id, SavedRow>), FileError> {
    let mut by_parent: HashMap<Option<Id>, Vec<(i64, Subtree)>> = HashMap::new();
    let mut saved = HashMap::new();
    let mut statement = conn.prepare("SELECT id, parent, ord, kind, props, geometry FROM nodes")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let id = id_from(row.get_ref(0)?.as_blob()?)?;
        let parent = row
            .get_ref(1)?
            .as_blob_or_null()?
            .map(id_from)
            .transpose()?;
        let ord: i64 = row.get(2)?;
        let kind = row.get_ref(3)?.as_str()?;
        let props = row.get_ref(4)?.as_str()?;
        let geometry = row.get_ref(5)?.as_blob_or_null()?;
        let node = decode_node(id, kind, props, geometry)?;
        by_parent.entry(parent).or_default().push((
            ord,
            Subtree {
                id,
                node,
                children: Vec::new(),
            },
        ));
        let ord = usize::try_from(ord).unwrap_or(usize::MAX);
        let row = SavedRow {
            version: 0,
            parent,
            ord,
        };
        if saved.insert(id, row).is_some() {
            return Err(FileError::Damaged(format!("{id} appears twice")));
        }
    }

    let layers = assemble(None, &mut by_parent);
    if let Some(orphan) = by_parent.values().flatten().next() {
        return Err(FileError::Damaged(format!(
            "{} is not reachable from the top level",
            orphan.1.id
        )));
    }
    Ok((layers, saved))
}

/// Builds the children of `parent` in `ord` order, taking them out of `by_parent`. Rows left
/// over afterwards are orphans or cycles.
fn assemble(
    parent: Option<Id>,
    by_parent: &mut HashMap<Option<Id>, Vec<(i64, Subtree)>>,
) -> Vec<Subtree> {
    let Some(mut children) = by_parent.remove(&parent) else {
        return Vec::new();
    };
    children.sort_by_key(|(ord, subtree)| (*ord, subtree.id));
    children
        .into_iter()
        .map(|(_, mut subtree)| {
            subtree.children = assemble(Some(subtree.id), by_parent);
            subtree
        })
        .collect()
}

fn decode_node(
    id: Id,
    kind: &str,
    props: &str,
    geometry: Option<&[u8]>,
) -> Result<Node, FileError> {
    let damaged = |what: String| FileError::Damaged(format!("{id}: {what}"));
    let mut node: Node = serde_json::from_str(props).map_err(|err| damaged(err.to_string()))?;
    let stored_kind = match &node {
        Node::Layer(_) => "layer",
        Node::Object(object) => object.kind.name(),
    };
    if stored_kind != kind {
        return Err(damaged(format!("kind {kind} holds a {stored_kind}")));
    }
    let slot = match &mut node {
        Node::Object(object) => object.kind.geometry_mut(),
        Node::Layer(_) => None,
    };
    match (slot, geometry) {
        (Some(slot), Some(bytes)) => {
            *slot = blob::decode(bytes).map_err(|source| FileError::Outline { id, source })?;
        }
        (Some(_), None) => return Err(damaged("the outline is missing".into())),
        (None, Some(_)) => return Err(damaged(format!("a {kind} has no outline"))),
        (None, None) => {}
    }
    Ok(node)
}

fn id_bytes(id: Id) -> [u8; 16] {
    *id.as_uuid().as_bytes()
}

fn id_from(bytes: &[u8]) -> Result<Id, FileError> {
    Uuid::from_slice(bytes)
        .map(Id::from_uuid)
        .map_err(|_| FileError::Damaged("an id is not 16 bytes".into()))
}
