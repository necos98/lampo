use std::path::Path;

use lampo_domain::ports::ProjectTransfer;
use lampo_domain::project::{ApacheDirectives, Project};
use lampo_domain::project_file::ProjectFile;
use lampo_domain::project_import::{DroppedOnImport, ImportedProject, is_untrusted_php_ini_key};
use lampo_domain::{PortError, PortResult};
use toml::{Table, Value};

use crate::atomic::{SCHEMA_KEY, SCHEMA_VERSION, read_table, to_table, write_table};

const KIND_KEY: &str = "kind";
const KIND_VALUE: &str = "lampo-project";
const ENVIRONMENT_KEY: &str = "environment";
const DIRECTIVES_KEY: &str = "apache_directives";
const PHP_INI_KEY: &str = "php_ini";
const ENV_VARS_KEY: &str = "env_vars";

/// Export file = project TOML with `kind = "lampo-project"` and `schema` headers.
///
/// An export may come from someone else, so the import leaves out (and reports) what would run
/// code here (`project_import`: Apache directives, environment variables, some php.ini keys),
/// resets `active` and `tls`, and keeps the database name only when the project uses MySQL.
#[derive(Debug, Default, Clone, Copy)]
pub struct TomlProjectTransfer;

impl ProjectTransfer for TomlProjectTransfer {
    fn export_to(&self, project: &Project, file: &Path) -> PortResult<()> {
        let mut table = to_table(project, &file.display().to_string())?;
        table.insert(KIND_KEY.into(), Value::String(KIND_VALUE.into()));
        table.insert(SCHEMA_KEY.into(), Value::Integer(SCHEMA_VERSION));
        write_table(file, &table)
    }

    fn import_from(&self, file: &Path) -> PortResult<ImportedProject> {
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
        let dropped = drop_untrusted(&mut table, &name)?;
        let parsed: ProjectFile = Value::Table(table).try_into().map_err(|e| PortError::invalid_data(&name, e))?;
        // Re-run the aggregate's own validation (address list invariants), then reset runtime state.
        let mut project = Project::new(parsed.id, parsed.name, parsed.root, parsed.addresses, parsed.environment)?;
        if let Some(db) = parsed.database {
            project.set_database_name(db);
        }
        project.origin_preset = parsed.origin_preset;
        project.active = false;
        Ok(ImportedProject { project, dropped })
    }
}

/// Removes from the raw export what an import never brings along, before it is parsed (an
/// `extension` php.ini key could not even be represented).
fn drop_untrusted(table: &mut Table, name: &str) -> PortResult<DroppedOnImport> {
    let Some(Value::Table(environment)) = table.get_mut(ENVIRONMENT_KEY) else { return Ok(DroppedOnImport::default()) };
    let apache_directives = match environment.remove(DIRECTIVES_KEY) {
        None => None,
        Some(Value::String(text)) => {
            let directives = ApacheDirectives::try_from(text).map_err(|e| PortError::invalid_data(name, e))?;
            (!directives.is_empty()).then(|| directives.into())
        }
        Some(_) => return Err(PortError::invalid_data(name, "`environment.apache_directives` must be a string")),
    };
    let php_ini_keys = match environment.get_mut(PHP_INI_KEY) {
        Some(Value::Table(ini)) => {
            let untrusted: Vec<String> = ini.keys().filter(|k| is_untrusted_php_ini_key(k)).cloned().collect();
            for key in &untrusted {
                ini.remove(key);
            }
            untrusted
        }
        _ => Vec::new(),
    };
    let env_vars = match environment.remove(ENV_VARS_KEY) {
        None => Vec::new(),
        Some(Value::Table(vars)) => vars.into_iter().map(|(name, _)| name).collect(),
        Some(_) => return Err(PortError::invalid_data(name, "`environment.env_vars` must be a table")),
    };
    Ok(DroppedOnImport { apache_directives, php_ini_keys, env_vars })
}

#[cfg(test)]
mod tests {
    use lampo_domain::project::TlsMode;

    use super::*;
    use crate::testutil::project;

    fn exported(dir: &tempfile::TempDir, p: &Project) -> (std::path::PathBuf, Table) {
        let file = dir.path().join("shop.lampo.toml");
        TomlProjectTransfer.export_to(p, &file).unwrap();
        let table = read_table(&file).unwrap().unwrap();
        (file, table)
    }

    #[test]
    fn export_import_roundtrip_resets_active_and_tls() {
        let dir = tempfile::tempdir().unwrap();
        let p = project("shop", "C:\\sites\\shop");
        assert!(p.active && p.tls == TlsMode::LocalCa);
        let (file, _) = exported(&dir, &p);
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("kind = \"lampo-project\"") && text.contains("schema = 2"), "{text}");
        let back = TomlProjectTransfer.import_from(&file).unwrap();
        let mut expected = p;
        expected.active = false;
        expected.tls = TlsMode::Disabled;
        assert_eq!(back, ImportedProject { project: expected, dropped: DroppedOnImport::default() });
    }

    /// A shared export must not run code here: its Apache directives, its environment variables
    /// and the php.ini keys that load or run files are left out and reported, everything else
    /// is kept.
    #[test]
    fn a_malicious_export_loses_its_directives_variables_and_untrusted_php_ini_keys() {
        let dir = tempfile::tempdir().unwrap();
        let (file, mut table) = exported(&dir, &project("shop", "C:\\sites\\shop"));
        let env = table.get_mut(ENVIRONMENT_KEY).and_then(Value::as_table_mut).unwrap();
        let directives = "CustomLog \"|C:/Windows/System32/cmd.exe /c calc\" common\nInclude C:/evil.conf";
        env.insert(DIRECTIVES_KEY.into(), Value::String(directives.into()));
        let ini = env.get_mut(PHP_INI_KEY).and_then(Value::as_table_mut).unwrap();
        for (key, value) in [("auto_prepend_file", "\\\\evil\\share\\x.php"), ("Extension", "C:/evil.dll"), (" zend_extension", "x"), ("include_path", "C:/"), ("memory_limit", "512M")] {
            ini.insert(key.into(), Value::String(value.into()));
        }
        let vars = [("COMSPEC", "C:/evil.exe"), ("APP_KEY", "secret")].map(|(k, v)| (k.to_string(), Value::String(v.into())));
        env.insert(ENV_VARS_KEY.into(), Value::Table(vars.into_iter().collect()));
        table.insert("tls".into(), Value::String("local-ca".into()));
        write_table(&file, &table).unwrap();

        let ImportedProject { project, dropped } = TomlProjectTransfer.import_from(&file).unwrap();
        assert_eq!(dropped.env_vars, ["APP_KEY", "COMSPEC"], "names only");
        assert!(project.environment.env_vars.is_empty());
        assert_eq!(dropped.apache_directives.as_deref(), Some(directives));
        let mut keys = dropped.php_ini_keys.clone();
        keys.sort();
        assert_eq!(keys, [" zend_extension", "Extension", "auto_prepend_file", "include_path"]);
        assert!(project.environment.apache_directives.is_empty());
        let ini: Vec<&str> = project.environment.php_ini.keys().map(|k| k.as_str()).collect();
        assert!(ini.contains(&"memory_limit") && !ini.contains(&"auto_prepend_file") && !ini.contains(&"include_path"), "{ini:?}");
        assert_eq!(project.tls, TlsMode::Disabled);
        assert!(!project.active);
    }

    #[test]
    fn a_database_without_mysql_is_not_imported() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = project("shop", "C:\\sites\\shop");
        let mut env = p.environment.clone();
        env.set_mysql(None);
        p.set_environment(env).unwrap();
        let (file, mut table) = exported(&dir, &p);
        table.insert("database".into(), Value::String("orphan".into()));
        write_table(&file, &table).unwrap();
        assert!(TomlProjectTransfer.import_from(&file).unwrap().project.database().is_none());

        // With MySQL and no name in the file, the default one is used.
        let (file, mut table) = exported(&dir, &project("my-shop", "C:\\sites\\shop"));
        assert!(table.remove("database").is_some());
        write_table(&file, &table).unwrap();
        assert_eq!(TomlProjectTransfer.import_from(&file).unwrap().project.database().unwrap().as_str(), "my_shop");
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
        std::fs::write(&file, "kind = \"lampo-project\"\nschema = 2\n[environment]\napache_directives = 1\n").unwrap();
        assert!(TomlProjectTransfer.import_from(&file).unwrap_err().to_string().contains("must be a string"));
        std::fs::write(&file, "kind = \"lampo-project\"\nschema = 2\n[environment]\nenv_vars = \"x\"\n").unwrap();
        assert!(TomlProjectTransfer.import_from(&file).unwrap_err().to_string().contains("must be a table"));
        std::fs::write(&file, "[[[").unwrap();
        assert!(TomlProjectTransfer.import_from(&file).is_err());
        assert!(matches!(TomlProjectTransfer.import_from(&dir.path().join("none.toml")), Err(PortError::NotFound { .. })));
    }
}
