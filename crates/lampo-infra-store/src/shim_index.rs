//! The shim index file (`%LOCALAPPDATA%\Lampo\bin\shim-index.json`). JSON (not TOML) because
//! it is read on every `php`/`composer` call and serde_json is the fastest parser at hand.
//!
//! Writes are atomic (temp file + rename), so a reader sees either the previous or the next
//! complete index. Reads are a single `read` of the whole file followed by one parse.

use std::path::PathBuf;
use std::time::Duration;

use lampo_domain::ports::{ShimIndexReader, ShimIndexWriter};
use lampo_domain::shim::{SHIM_INDEX_VERSION, ShimIndex};
use lampo_domain::{PortError, PortResult};

use crate::atomic::write_atomic;

pub struct JsonShimIndex {
    file: PathBuf,
}

impl JsonShimIndex {
    /// `file` = `LampoPaths::shim_index_file()`.
    pub fn new(file: PathBuf) -> Self {
        Self { file }
    }

    /// Reads the file. On Windows the writer's rename-over can be observed for a moment as
    /// "not found" or "access denied", so both are retried briefly before being believed (a
    /// genuinely missing file costs ~10 ms once, on the very first run).
    fn read_bytes(&self) -> std::io::Result<Option<Vec<u8>>> {
        use std::io::ErrorKind::{NotFound, PermissionDenied};
        let mut attempt = 0u32;
        loop {
            match std::fs::read(&self.file) {
                Ok(b) => return Ok(Some(b)),
                Err(e) if attempt < 6 && matches!(e.kind(), NotFound | PermissionDenied) => {
                    attempt += 1;
                    std::thread::sleep(Duration::from_millis(2 * u64::from(attempt)));
                }
                Err(e) if e.kind() == NotFound => return Ok(None),
                Err(e) => return Err(e),
            }
        }
    }
}

impl ShimIndexReader for JsonShimIndex {
    /// A missing file is an empty index; a different `version` is `InvalidData`.
    fn read(&self) -> PortResult<ShimIndex> {
        let name = self.file.display().to_string();
        let Some(bytes) = self.read_bytes().map_err(|e| PortError::io(format!("reading {name}"), e))? else {
            return Ok(ShimIndex::default());
        };
        let mut raw: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| PortError::invalid_data(&name, e))?;
        if raw.get("version").and_then(serde_json::Value::as_u64) == Some(1) {
            migrate_v1(&mut raw);
        }
        let index: ShimIndex = serde_json::from_value(raw).map_err(|e| PortError::invalid_data(&name, e))?;
        if index.version != SHIM_INDEX_VERSION {
            return Err(PortError::invalid_data(&name, format!("shim index version {} is not supported (expected {SHIM_INDEX_VERSION}); open Lampo to regenerate it", index.version)));
        }
        Ok(index)
    }
}

/// Version 1 → 2, in place: `php_exe` was implied (`<php_dir>\php.exe` in the v1 layout) and
/// `mysql`/`redis` were fixed fields; v2 has the resolved executable and a `services` map. The
/// app rewrites the file in the v2 shape on its next sync; until then shims keep working.
fn migrate_v1(raw: &mut serde_json::Value) {
    use serde_json::{Map, Value, json};
    fn target(t: &mut Value) {
        let Some(t) = t.as_object_mut() else { return };
        if let Some(dir) = t.get("php_dir").and_then(Value::as_str) {
            let exe = std::path::Path::new(dir).join("php.exe").display().to_string();
            t.entry("php_exe").or_insert(Value::String(exe));
        }
        let mut services = Map::new();
        if let Some(m) = t.remove("mysql").filter(|m| m.is_object()) {
            let series = m.get("series").and_then(Value::as_str).unwrap_or("8.4");
            services.insert(
                "mysql".into(),
                json!({ "service": format!("mysql-{series}"), "client_dir": m["client_dir"], "host": m["host"], "port": m["port"], "user": m["user"], "database": m["database"], "env_prefix": "DB" }),
            );
        }
        if let Some(r) = t.remove("redis").filter(|r| r.is_object()) {
            let dir = r.get("cli").and_then(Value::as_str).and_then(|c| std::path::Path::new(c).parent()).map(|d| d.display().to_string()).unwrap_or_default();
            services.insert("redis".into(), json!({ "service": "redis", "client_dir": dir, "host": r["host"], "port": r["port"], "env_prefix": "REDIS" }));
        }
        t.insert("services".into(), Value::Object(services));
    }
    if let Some(d) = raw.get_mut("default") {
        target(d);
    }
    if let Some(entries) = raw.get_mut("projects").and_then(Value::as_array_mut) {
        for e in entries {
            if let Some(t) = e.get_mut("target") {
                target(t);
            }
        }
    }
    raw["version"] = Value::from(SHIM_INDEX_VERSION);
}

impl ShimIndexWriter for JsonShimIndex {
    fn write(&self, index: &ShimIndex) -> PortResult<()> {
        let bytes = serde_json::to_vec(index).map_err(|e| PortError::invalid_data(self.file.display().to_string(), e))?;
        write_atomic(&self.file, &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lampo_domain::php::{ComposerChannel, PhpMinor};
    use lampo_domain::project::ProjectRoot;
    use lampo_domain::shim::{ShimEntry, ShimTarget};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn index(n: usize) -> ShimIndex {
        let php = PhpMinor::new(8, 3).unwrap();
        let target = ShimTarget {
            php,
            php_dir: PathBuf::from(r"C:\php\8.3"),
            php_exe: PathBuf::from(r"C:\php\8.3\php.exe"),
            phprc_dir: None,
            composer_phar: None,
            composer_channel: ComposerChannel::for_php(php),
            services: Default::default(),
        };
        let projects =
            (0..n).map(|i| ShimEntry { site: format!("site-{i}").parse().unwrap(), root: ProjectRoot::new(format!("C:\\sites\\site-{i}")).unwrap(), target: target.clone() }).collect();
        ShimIndex { default: Some(target), projects, ..Default::default() }
    }

    #[test]
    fn missing_file_is_empty_index_and_roundtrip_works() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonShimIndex::new(dir.path().join("bin").join("shim-index.json"));
        assert_eq!(store.read().unwrap(), ShimIndex::default());
        let idx = index(3);
        store.write(&idx).unwrap();
        assert_eq!(store.read().unwrap(), idx);
    }

    /// H2: a version-1 index (fixed `mysql`/`redis` fields, implied `php.exe`) still serves the
    /// shims until the app rewrites it.
    #[test]
    fn version_1_files_are_migrated_on_read() {
        use lampo_domain::service::ServiceKind;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("shim-index.json");
        let v1 = r#"{"version":1,
            "default":{"php":"8.5","php_dir":"C:\\p\\php-8.5","phprc_dir":"C:\\g\\_default","composer_phar":null,"composer_channel":"stable",
                "mysql":{"series":"8.4","client_dir":"C:\\p\\mysql-8.4\\bin","host":"127.0.0.1","port":3306,"user":"root","database":null},"redis":null},
            "projects":[{"site":"beta","root":"C:\\sites\\beta","target":{"php":"7.4","php_dir":"C:\\p\\php-7.4","phprc_dir":null,"composer_phar":null,"composer_channel":"stable",
                "mysql":{"series":"8.4","client_dir":"C:\\p\\mysql-8.4\\bin","host":"127.0.0.1","port":3306,"user":"root","database":"beta"},
                "redis":{"cli":"C:\\p\\redis-8.10\\redis-cli.exe","host":"127.0.0.1","port":6379}}}]}"#;
        std::fs::write(&file, v1).unwrap();
        let index = JsonShimIndex::new(file).read().unwrap();
        assert_eq!(index.version, SHIM_INDEX_VERSION);
        let d = index.default.unwrap();
        assert_eq!(d.php_exe, PathBuf::from(r"C:\p\php-8.5").join("php.exe"));
        assert_eq!(d.services[&ServiceKind::MYSQL].service.to_string(), "mysql-8.4");
        assert!(!d.services.contains_key(&ServiceKind::REDIS));
        let beta = &index.projects[0].target;
        assert_eq!(beta.services[&ServiceKind::MYSQL].database.as_deref(), Some("beta"));
        assert_eq!(beta.services[&ServiceKind::MYSQL].env_prefix, "DB");
        assert_eq!(beta.services[&ServiceKind::REDIS].client_dir, PathBuf::from(r"C:\p\redis-8.10"));
        assert_eq!(beta.services[&ServiceKind::REDIS].port, 6379);
    }

    #[test]
    fn version_mismatch_and_garbage_are_invalid_data_naming_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("shim-index.json");
        let store = JsonShimIndex::new(file.clone());
        std::fs::write(&file, r#"{"version":99,"default":null,"projects":[]}"#).unwrap();
        let err = store.read().unwrap_err();
        assert!(matches!(err, PortError::InvalidData { .. }) && err.to_string().contains("shim-index.json"), "{err}");
        std::fs::write(&file, "{not json").unwrap();
        assert!(matches!(store.read(), Err(PortError::InvalidData { .. })));
    }

    #[test]
    fn readers_never_observe_a_partial_file_while_it_is_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("shim-index.json");
        let store = Arc::new(JsonShimIndex::new(file));
        store.write(&index(1)).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let readers: Vec<_> = (0..4)
            .map(|_| {
                let (store, stop) = (store.clone(), stop.clone());
                std::thread::spawn(move || {
                    let mut reads = 0u32;
                    while !stop.load(Ordering::Relaxed) {
                        let idx = store.read().expect("reader observed a partial or missing file");
                        assert!(!idx.projects.is_empty());
                        reads += 1;
                    }
                    reads
                })
            })
            .collect();
        for i in 0..150 {
            store.write(&index(1 + (i % 40))).unwrap();
        }
        stop.store(true, Ordering::Relaxed);
        for r in readers {
            assert!(r.join().unwrap() > 0);
        }
    }
}
