use std::path::Path;

use lampo_domain::ports::ProjectTransfer;
use lampo_domain::project::Project;
use lampo_domain::{PortError, PortResult};
use toml::Value;

use crate::atomic::{SCHEMA_KEY, SCHEMA_VERSION, read_table, to_table, write_table};

const KIND_KEY: &str = "kind";
const KIND_VALUE: &str = "lampo-project";

/// Export file = project TOML with `kind = "lampo-project"` and `schema` headers;
/// `active` is reset to `false` on import.
#[derive(Debug, Default, Clone, Copy)]
pub struct TomlProjectTransfer;

impl ProjectTransfer for TomlProjectTransfer {
    fn export_to(&self, project: &Project, file: &Path) -> PortResult<()> {
        let mut table = to_table(project, &file.display().to_string())?;
        table.insert(KIND_KEY.into(), Value::String(KIND_VALUE.into()));
        table.insert(SCHEMA_KEY.into(), Value::Integer(SCHEMA_VERSION));
        write_table(file, &table)
    }

    fn import_from(&self, file: &Path) -> PortResult<Project> {
        let name = file.display().to_string();
        let mut table = read_table(file)?.ok_or_else(|| PortError::not_found("export file", &name))?;
        match table.remove(KIND_KEY) {
            Some(Value::String(k)) if k == KIND_VALUE => {}
            _ => return Err(PortError::invalid_data(&name, "not a Lampo project export (missing `kind = \"lampo-project\"`)")),
        }
        match table.remove(SCHEMA_KEY) {
            Some(Value::Integer(n)) if n <= SCHEMA_VERSION => {}
            Some(Value::Integer(n)) => return Err(PortError::invalid_data(&name, format!("export schema {n} is newer than this Lampo supports ({SCHEMA_VERSION})"))),
            _ => return Err(PortError::invalid_data(&name, "missing `schema` header")),
        }
        let parsed: Project = Value::Table(table).try_into().map_err(|e| PortError::invalid_data(&name, e))?;
        // Re-run the aggregate's own validation (address list invariants), then reset runtime state.
        let mut project = Project::new(parsed.id, parsed.name, parsed.root, parsed.addresses, parsed.environment)?;
        project.database = parsed.database;
        project.tls = parsed.tls;
        project.origin_preset = parsed.origin_preset;
        project.active = false;
        Ok(project)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::project;

    #[test]
    fn export_import_roundtrip_resets_active() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("shop.lampo.toml");
        let p = project("shop", "C:\\sites\\shop");
        assert!(p.active);
        TomlProjectTransfer.export_to(&p, &file).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("kind = \"lampo-project\"") && text.contains("schema = 2"), "{text}");
        let back = TomlProjectTransfer.import_from(&file).unwrap();
        let mut expected = p;
        expected.active = false;
        assert_eq!(back, expected);
    }

    #[test]
    fn rejects_foreign_newer_and_malformed_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.toml");
        std::fs::write(&file, "id = \"shop\"\n").unwrap();
        assert!(TomlProjectTransfer.import_from(&file).unwrap_err().to_string().contains("not a Lampo project export"));
        std::fs::write(&file, "kind = \"lampo-project\"\nschema = 99\n").unwrap();
        assert!(TomlProjectTransfer.import_from(&file).unwrap_err().to_string().contains("newer"));
        std::fs::write(&file, "kind = \"lampo-project\"\nschema = 1\nid = \"Bad Id\"\n").unwrap();
        assert!(TomlProjectTransfer.import_from(&file).unwrap_err().to_string().contains("x.toml"));
        std::fs::write(&file, "[[[").unwrap();
        assert!(TomlProjectTransfer.import_from(&file).is_err());
        assert!(matches!(TomlProjectTransfer.import_from(&dir.path().join("none.toml")), Err(PortError::NotFound { .. })));
    }
}
