//! Atomic file writes (temp file in the same directory + rename) and the TOML helpers shared by
//! all repositories.
//!
//! Every config file Lampo writes carries a `schema = <SCHEMA_VERSION>` header. When a file is rewritten, keys
//! present on disk that this version does not know (written by a newer Lampo) are carried over
//! instead of being dropped. A file of a newer schema is read-only: it loads, with a warning, but
//! is never rewritten, so its header never goes back to an older schema.

use std::path::{Path, PathBuf};

use lampo_domain::{PortError, PortResult};
pub use lampo_infra_fs::write_atomic;
use lampo_infra_fs::{TEMP_PREFIX, corrupt_backup_path};
use toml::{Table, Value};

/// Current schema number written into every TOML file. 2: services are data
/// (`environment.services`, `settings.services`); schema-1 files are migrated on read.
pub const SCHEMA_VERSION: i64 = 2;
/// Key of the schema header.
pub const SCHEMA_KEY: &str = "schema";
/// Schema of the files written before the header existed.
const LEGACY_SCHEMA_VERSION: i64 = 1;

/// Reads and parses a TOML file as a raw table; `Ok(None)` if it does not exist.
pub fn read_table(path: &Path) -> PortResult<Option<Table>> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(PortError::io(format!("reading {}", path.display()), e)),
    };
    let text = std::str::from_utf8(&bytes).map_err(|e| PortError::invalid_data(path.display().to_string(), format!("not valid UTF-8: {e}")))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    text.parse::<Table>().map(Some).map_err(|e| PortError::invalid_data(path.display().to_string(), e))
}

/// Reads a TOML file into `T`; `Ok(None)` if the file does not exist. Unknown keys are ignored.
/// A file of a newer schema loads too, with a warning that it is read-only (see [`write_toml`]).
pub fn read_toml<T: serde::de::DeserializeOwned>(path: &Path) -> PortResult<Option<T>> {
    let Some(table) = read_table(path)? else { return Ok(None) };
    if let Some(schema) = newer_schema(&table) {
        tracing::warn!("{}: {}", path.display(), read_only_reason(schema));
    }
    Value::Table(table).try_into::<T>().map(Some).map_err(|e| PortError::invalid_data(path.display().to_string(), e))
}

/// Serialises `value` to a TOML table (without any header).
pub fn to_table<T: serde::Serialize>(value: &T, source_name: &str) -> PortResult<Table> {
    match Value::try_from(value) {
        Ok(Value::Table(t)) => Ok(t),
        Ok(_) => Err(PortError::invalid_data(source_name, "value is not a TOML table")),
        Err(e) => Err(PortError::invalid_data(source_name, e)),
    }
}

/// The schema of a raw file; a file without the header predates it.
fn schema_of(table: &Table) -> i64 {
    table.get(SCHEMA_KEY).and_then(Value::as_integer).unwrap_or(LEGACY_SCHEMA_VERSION)
}

/// The schema of a file written by a newer Lampo than this one, if it is one.
fn newer_schema(table: &Table) -> Option<i64> {
    Some(schema_of(table)).filter(|&schema| schema > SCHEMA_VERSION)
}

fn read_only_reason(schema: i64) -> String {
    format!("schema {schema} is newer than this Lampo supports ({SCHEMA_VERSION}): the file is read-only until Lampo is updated")
}

/// Writes `table` as pretty TOML, atomically.
pub fn write_table(path: &Path, table: &Table) -> PortResult<()> {
    let text = toml::to_string_pretty(table).map_err(|e| PortError::invalid_data(path.display().to_string(), e))?;
    write_atomic(path, text.as_bytes())
}

/// Writes `value` with the `schema` header, keeping keys of the existing file that this version
/// of the type does not know. Atomic.
///
/// An existing file that does not load as `T` (corrupt, or edited into an invalid state) is
/// copied to `<file>.corrupt-<unix ms>` first: the caller asked to save, but the user's
/// hand-made content must not vanish with it. A file that cannot even be read is an error.
///
/// The schema on disk never goes down: a file of a newer schema is refused with
/// [`PortError::InvalidData`] and left untouched, so what ends up on disk is always
/// `max(existing schema, SCHEMA_VERSION)`.
pub fn write_toml<T: serde::Serialize + serde::de::DeserializeOwned>(path: &Path, value: &T) -> PortResult<()> {
    let name = path.display().to_string();
    let mut table = to_table(value, &name)?;
    table.insert(SCHEMA_KEY.to_string(), Value::Integer(SCHEMA_VERSION));
    match inspect_existing::<T>(path)? {
        Existing::Missing => {}
        Existing::Newer(schema) => return Err(PortError::invalid_data(name, read_only_reason(schema))),
        Existing::Unloadable => back_up_corrupt(path)?,
        // Only a file of this schema can hold keys worth keeping (newer ones are never rewritten):
        // the unknown keys of an older file are retired ones that were migrated on read
        // (`mysql`/`redis` → `services`), and keeping them would resurrect the old values on the
        // next read.
        Existing::Loadable { raw, known, schema } => {
            if schema == SCHEMA_VERSION {
                carry_unknown(&raw, &known, &mut table);
            }
        }
    }
    write_table(path, &table)
}

/// What [`write_toml`] finds at the path it is about to replace.
enum Existing {
    Missing,
    /// Written by a newer Lampo (with this schema): read-only for this version, loadable or not.
    Newer(i64),
    /// Malformed TOML, or values `T` rejects.
    Unloadable,
    /// The raw file, its round-trip through `T` (what this version understands) and its schema,
    /// at most [`SCHEMA_VERSION`].
    Loadable {
        raw: Value,
        known: Value,
        schema: i64,
    },
}

fn inspect_existing<T: serde::Serialize + serde::de::DeserializeOwned>(path: &Path) -> PortResult<Existing> {
    let table = match read_table(path) {
        Ok(None) => return Ok(Existing::Missing),
        Ok(Some(table)) => table,
        Err(PortError::InvalidData { .. }) => return Ok(Existing::Unloadable),
        Err(e) => return Err(e),
    };
    if let Some(schema) = newer_schema(&table) {
        return Ok(Existing::Newer(schema));
    }
    let schema = schema_of(&table);
    let raw = Value::Table(table);
    let Ok(parsed) = raw.clone().try_into::<T>() else { return Ok(Existing::Unloadable) };
    let known = Value::try_from(&parsed).map_err(|e| PortError::invalid_data(path.display().to_string(), e))?;
    Ok(Existing::Loadable { raw, known, schema })
}

/// Copies `path` next to itself as `<name>.corrupt-<unix ms>`.
fn back_up_corrupt(path: &Path) -> PortResult<()> {
    let backup = corrupt_backup_path(path);
    std::fs::copy(path, &backup).map_err(|e| PortError::io(format!("backing up the unreadable {} to {}", path.display(), backup.display()), e))?;
    tracing::warn!("{} did not load: kept a copy as {} before replacing it", path.display(), backup.display());
    Ok(())
}

/// Copies into `out` every key of `raw` that the type did not recognise (absent from `known`, the
/// round-trip of `raw` through the type), recursing into nested tables.
fn carry_unknown(raw: &Value, known: &Value, out: &mut Table) {
    let (Value::Table(raw), Value::Table(known)) = (raw, known) else { return };
    for (key, raw_value) in raw {
        if key == SCHEMA_KEY {
            continue;
        }
        match known.get(key) {
            None => {
                out.entry(key.clone()).or_insert_with(|| raw_value.clone());
            }
            Some(known_value) => {
                if let Some(Value::Table(inner)) = out.get_mut(key) {
                    carry_unknown(raw_value, known_value, inner);
                }
            }
        }
    }
}

/// Lists the regular `*.toml` files directly inside `dir` (missing dir = empty), sorted by name.
/// Temp files of in-flight atomic writes never match.
pub fn list_toml_files(dir: &Path) -> PortResult<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(PortError::io(format!("listing {}", dir.display()), e)),
    };
    let mut files: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("toml")))
        .filter(|p| !p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(TEMP_PREFIX)))
        .collect();
    files.sort();
    Ok(files)
}

/// Removes a file; a file that is already gone is success.
pub fn remove_file_if_exists(path: &Path) -> PortResult<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(PortError::io(format!("deleting {}", path.display()), e)),
    }
}

/// Every TOML file of a directory, read as `T`.
pub struct TomlScan<T> {
    pub loaded: Vec<T>,
    /// The files that did not load, with the reason (sorted by name). Logged as warnings.
    pub failed: Vec<(PathBuf, PortError)>,
}

/// Reads every TOML file in `dir` as `T`; the ones that fail are reported, not fatal.
pub fn scan_toml<T: serde::de::DeserializeOwned>(dir: &Path) -> PortResult<TomlScan<T>> {
    let mut scan = TomlScan { loaded: Vec::new(), failed: Vec::new() };
    for file in list_toml_files(dir)? {
        match read_toml::<T>(&file) {
            Ok(Some(v)) => scan.loaded.push(v),
            // Removed between the listing and the read.
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("skipping unreadable file: {e}");
                scan.failed.push((file, e));
            }
        }
    }
    Ok(scan)
}

/// Reads every TOML file in `dir` as `T`, skipping (and logging) the ones that fail to parse.
pub fn read_all_toml<T: serde::de::DeserializeOwned>(dir: &Path) -> PortResult<Vec<T>> {
    scan_toml(dir).map(|scan| scan.loaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lampo_infra_fs::CORRUPT_BACKUP_MARKER;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Sample {
        name: String,
        #[serde(default)]
        inner: Inner,
        #[serde(default)]
        opt: Option<u8>,
    }
    #[derive(Debug, Serialize, Deserialize, PartialEq, Default)]
    struct Inner {
        a: u8,
    }

    #[test]
    fn missing_file_is_none_and_malformed_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.toml");
        assert!(read_toml::<Sample>(&file).unwrap().is_none());
        std::fs::write(&file, "name = [").unwrap();
        let err = read_toml::<Sample>(&file).unwrap_err();
        assert!(err.to_string().contains("x.toml"), "{err}");
        std::fs::write(&file, "other = 1").unwrap();
        assert!(read_toml::<Sample>(&file).unwrap_err().to_string().contains("x.toml"));
    }

    #[test]
    fn roundtrip_writes_schema_and_preserves_unknown_keys_at_any_depth() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.toml");
        std::fs::write(&file, "schema = 2\nname = \"old\"\nfuture = \"keep\"\n[inner]\na = 1\nnewer = 5\n").unwrap();
        write_toml(&file, &Sample { name: "new".into(), inner: Inner { a: 7 }, opt: None }).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("schema = 2"), "{text}");
        assert!(text.contains("future = \"keep\""), "{text}");
        assert!(text.contains("newer = 5"), "{text}");
        let back: Sample = read_toml(&file).unwrap().unwrap();
        assert_eq!(back, Sample { name: "new".into(), inner: Inner { a: 7 }, opt: None });
    }

    fn backups(dir: &Path) -> Vec<(String, String)> {
        let mut found: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.file_name().unwrap().to_string_lossy().contains(CORRUPT_BACKUP_MARKER))
            .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap()))
            .collect();
        found.sort();
        found
    }

    /// A save replaces a corrupt file, after keeping a copy of it that no listing reads back.
    #[test]
    fn corrupt_existing_file_is_backed_up_then_replaced_on_save() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.toml");
        std::fs::write(&file, "garbage [[[").unwrap();
        write_toml(&file, &Sample { name: "n".into(), inner: Inner::default(), opt: Some(1) }).unwrap();
        assert!(read_toml::<Sample>(&file).unwrap().is_some());
        let found = backups(dir.path());
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].0.starts_with("s.toml.corrupt-"), "{found:?}");
        assert_eq!(found[0].1, "garbage [[[");
        assert_eq!(list_toml_files(dir.path()).unwrap(), std::slice::from_ref(&file), "the copy is not listed as data");
        // The file is valid now: the next save keeps no further copy.
        write_toml(&file, &Sample { name: "m".into(), inner: Inner::default(), opt: None }).unwrap();
        assert_eq!(backups(dir.path()).len(), 1);
    }

    /// Valid TOML the type rejects is just as unloadable: it is backed up too.
    #[test]
    fn a_file_the_type_rejects_is_backed_up_on_save() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.toml");
        std::fs::write(&file, "schema = 2\nname = 5\n").unwrap();
        write_toml(&file, &Sample { name: "n".into(), inner: Inner::default(), opt: None }).unwrap();
        assert_eq!(backups(dir.path()).iter().map(|(_, text)| text.as_str()).collect::<Vec<_>>(), ["schema = 2\nname = 5\n"]);
    }

    /// A file written by a newer Lampo still loads: this version shows it, read-only.
    #[test]
    fn a_newer_schema_file_is_still_read() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.toml");
        std::fs::write(&file, "schema = 3\nname = \"n\"\nfuture = 1\n").unwrap();
        assert_eq!(read_toml::<Sample>(&file).unwrap(), Some(Sample { name: "n".into(), inner: Inner::default(), opt: None }));
    }

    /// Saving over a file of a newer schema would relabel it as this schema, and the newer Lampo
    /// would then migrate its own data again as if it were old: the write is refused and the
    /// file stays exactly as it was, whether or not this version can load it.
    #[test]
    fn writing_over_a_newer_schema_file_is_refused_and_leaves_it_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.toml");
        for original in ["schema = 3\nname = \"n\"\nfuture = 1\n", "schema = 3\nname = 5\n"] {
            std::fs::write(&file, original).unwrap();
            let err = write_toml(&file, &Sample { name: "new".into(), inner: Inner::default(), opt: None }).unwrap_err();
            let message = err.to_string();
            assert!(message.contains("s.toml") && message.contains("schema 3") && message.contains("read-only"), "{message}");
            assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
            assert!(backups(dir.path()).is_empty(), "a refused write keeps no backup");
        }
    }

    #[test]
    fn a_scan_reports_the_files_that_do_not_load() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.toml"), "name = \"a\"").unwrap();
        std::fs::write(dir.path().join("b.toml"), "name = [").unwrap();
        std::fs::write(dir.path().join("c.toml"), "name = 1").unwrap();
        let scan = scan_toml::<Sample>(dir.path()).unwrap();
        assert_eq!(scan.loaded.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["a"]);
        let failed: Vec<_> = scan.failed.iter().map(|(p, e)| (p.file_name().unwrap().to_string_lossy().into_owned(), e.to_string())).collect();
        assert_eq!(failed.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["b.toml", "c.toml"]);
        assert!(failed.iter().all(|(n, e)| e.contains(n.as_str())), "{failed:?}");
    }

    #[test]
    fn listing_ignores_temp_and_foreign_files() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["b.toml", "a.toml", ".lampo-xyz.tmp", "c.txt"] {
            std::fs::write(dir.path().join(n), "").unwrap();
        }
        std::fs::create_dir(dir.path().join("d.toml")).unwrap();
        let names: Vec<_> = list_toml_files(dir.path()).unwrap().iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["a.toml", "b.toml"]);
        assert!(list_toml_files(&dir.path().join("nope")).unwrap().is_empty());
    }
}
