//! Parsing and validating `pack.toml`.
//!
//! The manifest is read field by field rather than through serde derives, so every problem can
//! name the file, the Asset and the field, and one pass reports all of them.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "existing invariant panics; replace with error handling over time (#351)"
)]

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;
use toml::{Table, Value};

use super::install::PACK_MANIFEST;
use super::{ArtFormat, Asset, Pack, PackSource, is_valid_id};

/// One problem with a pack manifest.
#[derive(Clone, PartialEq, Eq, Debug, Error)]
#[error(
    "{}{}{}: {message}",
    file.display(),
    asset.as_ref().map(|asset| format!(": Asset `{asset}`")).unwrap_or_default(),
    field.as_ref().map(|field| format!(": field `{field}`")).unwrap_or_default(),
)]
pub struct ManifestError {
    /// The manifest file.
    pub file: PathBuf,
    /// The Asset's id, or `#n` (counting from 1) when its id is unusable. `None` for the
    /// `[pack]` table and the file as a whole.
    pub asset: Option<String>,
    /// The field, such as `anchor` or `pack.licence`. `None` when the problem is not one field.
    pub field: Option<String>,
    pub message: String,
}

/// Every problem found in one manifest, in file order. Never empty.
#[derive(Clone, PartialEq, Eq, Debug, Error)]
#[error("{}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"))]
pub struct ManifestErrors(pub Vec<ManifestError>);

/// Reads and validates the pack in `dir`, including that every file it names exists.
pub fn read_pack(dir: &Path, source: PackSource) -> Result<Pack, ManifestErrors> {
    let file = dir.join(PACK_MANIFEST);
    let text = std::fs::read_to_string(&file).map_err(|err| {
        ManifestErrors(vec![ManifestError {
            file: file.clone(),
            asset: None,
            field: None,
            message: format!("cannot be read: {err}"),
        }])
    })?;
    let pack = parse_manifest(&file, &text, source)?;

    let mut errors = Vec::new();
    for asset in &pack.assets {
        let files = std::iter::once(("file", &asset.file))
            .chain(asset.silhouette.iter().map(|path| ("silhouette", path)));
        for (field, path) in files {
            if !dir.join(path).is_file() {
                errors.push(ManifestError {
                    file: file.clone(),
                    asset: Some(asset.id.clone()),
                    field: Some(field.into()),
                    message: format!("`{}` does not exist in the pack folder", path.display()),
                });
            }
        }
    }
    if errors.is_empty() {
        Ok(pack)
    } else {
        Err(ManifestErrors(errors))
    }
}

/// Parses and validates manifest text. `file` is the manifest's path: errors name it, and its
/// folder becomes [`Pack::dir`]. Does not touch the file system.
pub fn parse_manifest(file: &Path, text: &str, source: PackSource) -> Result<Pack, ManifestErrors> {
    let mut errors = Errors {
        file,
        list: Vec::new(),
    };
    let root: Table = match text.parse() {
        Ok(root) => root,
        Err(err) => {
            errors.push(
                None,
                None,
                format!("is not valid TOML: {}", err.to_string().trim_end()),
            );
            return Err(errors.finish());
        }
    };
    for key in root.keys() {
        if key != "pack" && key != "asset" {
            errors.push(
                None,
                Some(key),
                "unknown field; expected `[pack]` and `[[asset]]`",
            );
        }
    }

    let empty = Table::new();
    let pack_table = match root.get("pack") {
        Some(Value::Table(table)) => table,
        Some(_) => {
            errors.push(None, Some("pack"), "must be a table, written `[pack]`");
            &empty
        }
        None => {
            errors.push(
                None,
                Some("pack"),
                "is missing; every manifest needs a `[pack]` table",
            );
            &empty
        }
    };
    let section = Section {
        table: pack_table,
        asset: None,
        prefix: "pack.",
    };
    section.unknown_fields(&mut errors, PACK_FIELDS);
    let id = section.required(&mut errors, "id", Section::string);
    if let Some(id) = &id
        && !is_valid_id(id)
    {
        errors.push(None, Some("pack.id"), INVALID_ID);
    }
    let name = section.required(&mut errors, "name", Section::string);
    let licence = section.required(&mut errors, "licence", Section::string);
    let licence_url = section.optional(&mut errors, "licence_url", Section::url);
    let credits = section.required(&mut errors, "credits", Section::string);
    let attribution_required = section
        .optional(&mut errors, "attribution_required", Section::boolean)
        .unwrap_or(false);
    let default = section
        .optional(&mut errors, "default", Section::boolean)
        .unwrap_or(false);
    let defaults = Defaults {
        author: section.optional(&mut errors, "author", Section::string),
        source_url: section.optional(&mut errors, "source_url", Section::url),
        modified: section.optional(&mut errors, "modified", Section::date),
    };

    let asset_tables: Vec<(usize, &Table)> = match root.get("asset") {
        None => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| match item {
                Value::Table(table) => Some((i, table)),
                _ => {
                    errors.push(
                        Some(&format!("#{}", i + 1)),
                        None,
                        "must be a table, written `[[asset]]`",
                    );
                    None
                }
            })
            .collect(),
        Some(_) => {
            errors.push(
                None,
                Some("asset"),
                "must be a list of tables, written `[[asset]]`",
            );
            Vec::new()
        }
    };

    let mut seen = HashSet::new();
    let mut assets = Vec::with_capacity(asset_tables.len());
    for (i, table) in asset_tables {
        let asset = read_asset(
            &mut errors,
            i,
            table,
            &defaults,
            licence.as_deref(),
            &mut seen,
        );
        assets.extend(asset);
    }

    if !errors.list.is_empty() {
        return Err(errors.finish());
    }
    Ok(Pack {
        id: id.expect("no errors"),
        name: name.expect("no errors"),
        dir: file.parent().map(Path::to_path_buf).unwrap_or_default(),
        licence: licence.expect("no errors"),
        licence_url,
        credits: credits.expect("no errors"),
        attribution_required,
        default,
        source,
        assets,
    })
}

const PACK_FIELDS: &[&str] = &[
    "id",
    "name",
    "licence",
    "licence_url",
    "credits",
    "attribution_required",
    "default",
    "author",
    "source_url",
    "modified",
];

const ASSET_FIELDS: &[&str] = &[
    "id",
    "name",
    "category",
    "tags",
    "file",
    "anchor",
    "width",
    "silhouette",
    "variant_group",
    "author",
    "source_url",
    "licence",
    "modified",
];

const INVALID_ID: &str = "must start with a lowercase letter or digit, then use only lowercase letters, digits, `-` and `_`";

/// `[pack]` values that Assets inherit when they leave the field out.
struct Defaults {
    author: Option<String>,
    source_url: Option<String>,
    modified: Option<String>,
}

fn read_asset(
    errors: &mut Errors<'_>,
    index: usize,
    table: &Table,
    defaults: &Defaults,
    pack_licence: Option<&str>,
    seen: &mut HashSet<String>,
) -> Option<Asset> {
    let before = errors.list.len();
    // Name the Asset by its id when it has a usable one, else by its position.
    let id = match table.get("id") {
        Some(Value::String(id)) if is_valid_id(id) => Some(id.clone()),
        _ => None,
    };
    let position = format!("#{}", index + 1);
    let label = match &id {
        Some(id) if seen.contains(id) => format!("{id} ({position})"),
        Some(id) => id.clone(),
        None => position,
    };
    let section = Section {
        table,
        asset: Some(&label),
        prefix: "",
    };
    section.unknown_fields(errors, ASSET_FIELDS);
    match table.get("id") {
        None => section.push(errors, "id", "is missing"),
        Some(Value::String(_)) if id.is_some() => {}
        Some(Value::String(_)) => section.push(errors, "id", INVALID_ID),
        Some(_) => section.push(errors, "id", "must be a string"),
    }
    if let Some(id) = &id
        && !seen.insert(id.clone())
    {
        section.push(errors, "id", "is used by an earlier Asset in this pack");
    }

    let name = section.required(errors, "name", Section::string);
    let category = section.required(errors, "category", Section::string);
    let tags = section
        .optional(errors, "tags", Section::strings)
        .unwrap_or_default();
    let file = section.required(errors, "file", Section::art_path);
    let anchor = section.required(errors, "anchor", Section::anchor);
    let width = section.required(errors, "width", Section::positive);
    let silhouette = section.optional(errors, "silhouette", Section::art_path);
    let variant_group = section.optional(errors, "variant_group", Section::id);
    let author = section.inherited(errors, "author", Section::string, &defaults.author);
    let source_url = section
        .optional(errors, "source_url", Section::url)
        .or_else(|| defaults.source_url.clone());
    let licence = section.inherited(
        errors,
        "licence",
        Section::string,
        &pack_licence.map(str::to_owned),
    );
    let modified = section.inherited(errors, "modified", Section::date, &defaults.modified);

    if errors.list.len() > before {
        return None;
    }
    let (file, format) = file?;
    Some(Asset {
        id: id?,
        name: name?,
        category: category?,
        tags,
        file,
        format,
        anchor: anchor?,
        width: width?,
        silhouette: silhouette.map(|(path, _)| path),
        variant_group,
        author: author?,
        source_url,
        licence: licence?,
        modified: modified?,
    })
}

struct Errors<'a> {
    file: &'a Path,
    list: Vec<ManifestError>,
}

impl Errors<'_> {
    fn push(&mut self, asset: Option<&str>, field: Option<&str>, message: impl Into<String>) {
        self.list.push(ManifestError {
            file: self.file.to_path_buf(),
            asset: asset.map(str::to_owned),
            field: field.map(str::to_owned),
            message: message.into(),
        });
    }

    fn finish(self) -> ManifestErrors {
        ManifestErrors(self.list)
    }
}

/// The `[pack]` table or one `[[asset]]`.
struct Section<'a> {
    table: &'a Table,
    asset: Option<&'a str>,
    /// Prepended to field names in errors, so `[pack]` fields read `pack.licence`.
    prefix: &'static str,
}

/// Reads one field's value, or says what is wrong with it.
type Reader<T> = fn(&Value) -> Result<T, String>;

impl Section<'_> {
    fn push(&self, errors: &mut Errors<'_>, key: &str, message: impl Into<String>) {
        let field = format!("{}{key}", self.prefix);
        errors.push(self.asset, Some(&field), message);
    }

    fn unknown_fields(&self, errors: &mut Errors<'_>, known: &[&str]) {
        for key in self.table.keys() {
            if !known.contains(&key.as_str()) {
                self.push(
                    errors,
                    key,
                    format!("unknown field; known fields are {}", list(known)),
                );
            }
        }
    }

    fn optional<T>(&self, errors: &mut Errors<'_>, key: &str, read: Reader<T>) -> Option<T> {
        let value = self.table.get(key)?;
        read(value)
            .map_err(|message| self.push(errors, key, message))
            .ok()
    }

    fn required<T>(&self, errors: &mut Errors<'_>, key: &str, read: Reader<T>) -> Option<T> {
        if !self.table.contains_key(key) {
            self.push(errors, key, "is missing");
            return None;
        }
        self.optional(errors, key, read)
    }

    /// Required unless `[pack]` gives a default.
    fn inherited<T: Clone>(
        &self,
        errors: &mut Errors<'_>,
        key: &str,
        read: Reader<T>,
        default: &Option<T>,
    ) -> Option<T> {
        if self.table.contains_key(key) {
            return self.optional(errors, key, read);
        }
        if default.is_none() {
            self.push(
                errors,
                key,
                "is missing, and `[pack]` has no default for it",
            );
        }
        default.clone()
    }

    fn string(value: &Value) -> Result<String, String> {
        match value {
            Value::String(s) if s.trim().is_empty() => Err("must not be empty".into()),
            Value::String(s) => Ok(s.clone()),
            other => Err(format!("must be a string, not {}", other.type_str())),
        }
    }

    fn boolean(value: &Value) -> Result<bool, String> {
        value
            .as_bool()
            .ok_or_else(|| format!("must be `true` or `false`, not {}", value.type_str()))
    }

    fn number(value: &Value) -> Result<f64, String> {
        match value {
            Value::Float(v) if v.is_finite() => Ok(*v),
            Value::Float(_) => Err("must be a finite number".into()),
            Value::Integer(v) => Ok(*v as f64),
            other => Err(format!("must be a number, not {}", other.type_str())),
        }
    }

    fn positive(value: &Value) -> Result<f64, String> {
        let v = Self::number(value)?;
        if v > 0.0 {
            Ok(v)
        } else {
            Err(format!("must be greater than 0, not {v}"))
        }
    }

    fn strings(value: &Value) -> Result<Vec<String>, String> {
        let Value::Array(items) = value else {
            return Err(format!(
                "must be a list of strings, not {}",
                value.type_str()
            ));
        };
        items
            .iter()
            .map(|item| Self::string(item).map_err(|err| format!("every entry {err}")))
            .collect()
    }

    fn id(value: &Value) -> Result<String, String> {
        let id = Self::string(value)?;
        if is_valid_id(&id) {
            Ok(id)
        } else {
            Err(INVALID_ID.into())
        }
    }

    /// `[x, y]`, each from 0 to 1.
    fn anchor(value: &Value) -> Result<[f64; 2], String> {
        let shape = "must be `[x, y]`, each from 0 to 1 across the art's width and height";
        let Value::Array(items) = value else {
            return Err(shape.into());
        };
        let [x, y] = items.as_slice() else {
            return Err(shape.into());
        };
        let point = [Self::number(x), Self::number(y)];
        match point {
            [Ok(x), Ok(y)] if (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y) => Ok([x, y]),
            _ => Err(shape.into()),
        }
    }

    /// A relative path inside the pack folder to an SVG or PNG file.
    fn art_path(value: &Value) -> Result<(PathBuf, ArtFormat), String> {
        let text = Self::string(value)?;
        if text.contains('\\') {
            return Err(format!("`{text}` must use `/` between folders"));
        }
        let path = PathBuf::from(&text);
        if !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err(format!(
                "`{text}` must be a relative path inside the pack folder, without `..`"
            ));
        }
        let format = ArtFormat::from_path(&path)
            .ok_or_else(|| format!("`{text}` must be an `.svg` or `.png` file"))?;
        Ok((path, format))
    }

    fn url(value: &Value) -> Result<String, String> {
        let url = Self::string(value)?;
        if url.starts_with("https://") || url.starts_with("http://") {
            Ok(url)
        } else {
            Err(format!("`{url}` must start with `https://` or `http://`"))
        }
    }

    /// `YYYY-MM-DD`, as a string or a bare TOML date.
    fn date(value: &Value) -> Result<String, String> {
        let text = match value {
            Value::Datetime(datetime) if datetime.time.is_none() && datetime.offset.is_none() => {
                datetime.to_string()
            }
            other => Self::string(other)?,
        };
        let parts: Vec<&str> = text.split('-').collect();
        let valid = matches!(parts.as_slice(), [y, m, d]
            if y.len() == 4 && m.len() == 2 && d.len() == 2
                && [y, m, d].iter().all(|p| p.bytes().all(|b| b.is_ascii_digit()))
                && (1..=12).contains(&m.parse::<u32>().unwrap_or(0))
                && (1..=31).contains(&d.parse::<u32>().unwrap_or(0)));
        if valid {
            Ok(text)
        } else {
            Err(format!("`{text}` must be a date written `YYYY-MM-DD`"))
        }
    }
}

fn list(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}
