//! The stored form of a [`Project`]: what a project TOML holds, before the aggregate's rules.
//!
//! A project file can be edited by hand, so loading one runs the same validation as creating a
//! project ([`Project::new`]): an empty or invalid address list, or a database name on a project
//! without MySQL, makes the file invalid instead of producing an aggregate that breaks its own
//! invariants later (a panic in `primary_address`).

use serde::Deserialize;

use crate::address::SiteAddress;
use crate::error::DomainError;
use crate::mysql::DatabaseName;
use crate::mysql::accounts::MysqlCredentials;
use crate::preset::PresetId;
use crate::project::{DisplayName, Project, ProjectEnvironment, ProjectId, ProjectRoot, TlsMode};

/// Unvalidated fields of a project file. Missing optional fields take their defaults.
/// `Project` deserializes through it (`TryFrom`); an import reads it directly, to apply its own
/// policy before building the project.
#[derive(Debug, Clone, Deserialize)]
pub struct ProjectFile {
    pub id: ProjectId,
    pub name: DisplayName,
    pub root: ProjectRoot,
    pub addresses: Vec<SiteAddress>,
    pub environment: ProjectEnvironment,
    #[serde(default)]
    pub database: Option<DatabaseName>,
    #[serde(default)]
    pub tls: TlsMode,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub origin_preset: Option<PresetId>,
    /// Absent in files written before dedicated accounts: root, as they were created with.
    #[serde(default)]
    pub mysql_credentials: MysqlCredentials,
}

impl TryFrom<ProjectFile> for Project {
    type Error = DomainError;

    fn try_from(f: ProjectFile) -> Result<Self, Self::Error> {
        let mut project = Project::new(f.id, f.name, f.root, f.addresses, f.environment)?;
        if let Some(database) = f.database {
            if project.environment.mysql().is_none() {
                return Err(DomainError::DatabaseWithoutMysql(database.to_string()));
            }
            project.set_database_name(database);
        }
        project.tls = f.tls;
        project.active = f.active;
        project.origin_preset = f.origin_preset;
        project.mysql_credentials = f.mysql_credentials;
        Ok(project)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::tests::project;

    fn json(p: &Project) -> serde_json::Value {
        serde_json::to_value(p).unwrap()
    }

    #[test]
    fn a_valid_file_round_trips() {
        let mut p = project("shop", "C:\\sites\\shop");
        p.active = true;
        p.tls = TlsMode::LocalCa;
        p.mysql_credentials = MysqlCredentials::Project;
        let back: Project = serde_json::from_value(json(&p)).unwrap();
        assert_eq!(back, p);
    }

    /// Projects created before dedicated accounts keep connecting as root (their `.env` says so).
    #[test]
    fn a_file_without_mysql_credentials_reads_as_root() {
        let mut p = project("shop", "C:\\sites\\shop");
        p.mysql_credentials = MysqlCredentials::Project;
        let mut raw = json(&p);
        raw.as_object_mut().unwrap().remove("mysql_credentials");
        assert_eq!(serde_json::from_value::<Project>(raw).unwrap().mysql_credentials, MysqlCredentials::Root);
    }

    #[test]
    fn an_empty_address_list_is_rejected() {
        let mut raw = json(&project("shop", "C:\\sites\\shop"));
        raw["addresses"] = serde_json::json!([]);
        let err = serde_json::from_value::<Project>(raw).unwrap_err();
        assert!(err.to_string().contains("at least one address"), "{err}");
    }

    #[test]
    fn invalid_addresses_are_rejected() {
        let mut raw = json(&project("shop", "C:\\sites\\shop"));
        let first = raw["addresses"][0].clone();
        raw["addresses"] = serde_json::json!([first.clone(), first]);
        assert!(serde_json::from_value::<Project>(raw).unwrap_err().to_string().contains("listed twice"));
    }

    #[test]
    fn a_database_without_mysql_is_rejected() {
        let mut p = project("shop", "C:\\sites\\shop");
        let mut env = p.environment.clone();
        env.set_mysql(None);
        p.set_environment(env).unwrap();
        let mut raw = json(&p);
        raw["database"] = serde_json::json!("orphan");
        let err = serde_json::from_value::<Project>(raw).unwrap_err();
        assert!(err.to_string().contains("`orphan`"), "{err}");
    }

    /// A project with MySQL but no `database` key gets the default schema, as `Project::new` does.
    #[test]
    fn a_missing_database_with_mysql_defaults_to_the_slug() {
        let mut raw = json(&project("my-shop", "C:\\sites\\shop"));
        raw.as_object_mut().unwrap().remove("database");
        let p: Project = serde_json::from_value(raw).unwrap();
        assert_eq!(p.database().map(DatabaseName::as_str), Some("my_shop"));
    }
}
