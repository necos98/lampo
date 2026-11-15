//! `%APPDATA%\Lampo\projects\<id>.toml`. The TOML schema is the serde form of
//! `lampo_domain::project::Project` plus a `schema` header for migrations.

use lampo_domain::PortResult;
use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::ProjectRepository;
use lampo_domain::project::{Project, ProjectId};

use crate::atomic::{read_all_toml, read_toml, remove_file_if_exists, write_toml};

pub struct TomlProjectRepository {
    paths: LampoPaths,
}

impl TomlProjectRepository {
    pub fn new(paths: LampoPaths) -> Self {
        Self { paths }
    }
}

impl ProjectRepository for TomlProjectRepository {
    fn list(&self) -> PortResult<Vec<Project>> {
        let mut projects: Vec<Project> = read_all_toml(&self.paths.projects_dir())?;
        projects.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(projects)
    }

    fn get(&self, id: &ProjectId) -> PortResult<Option<Project>> {
        read_toml(&self.paths.project_file(id))
    }

    fn save(&self, project: &Project) -> PortResult<()> {
        write_toml(&self.paths.project_file(&project.id), project)
    }

    fn delete(&self, id: &ProjectId) -> PortResult<()> {
        remove_file_if_exists(&self.paths.project_file(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{paths, project};

    #[test]
    fn save_get_roundtrip_is_lossless() {
        let dir = tempfile::tempdir().unwrap();
        let repo = TomlProjectRepository::new(paths(&dir));
        let p = project("shop", "C:\\sites\\shop");
        repo.save(&p).unwrap();
        assert_eq!(repo.get(&p.id).unwrap().unwrap(), p);
        assert!(repo.get(&ProjectId::new("nope").unwrap()).unwrap().is_none());
        let text = std::fs::read_to_string(paths(&dir).project_file(&p.id)).unwrap();
        assert!(text.contains("schema = 2"), "{text}");
    }

    #[test]
    fn list_is_sorted_and_survives_a_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let repo = TomlProjectRepository::new(paths(&dir));
        assert!(repo.list().unwrap().is_empty());
        repo.save(&project("zeta", "C:\\z")).unwrap();
        repo.save(&project("alpha", "C:\\a")).unwrap();
        std::fs::write(paths(&dir).projects_dir().join("broken.toml"), "id = [[[").unwrap();
        std::fs::create_dir(paths(&dir).projects_dir().join("alpha.worktrees")).unwrap();
        let ids: Vec<_> = repo.list().unwrap().iter().map(|p| p.id.to_string()).collect();
        assert_eq!(ids, ["alpha", "zeta"]);
        // `get` on the corrupt file reports it by name.
        let err = repo.get(&ProjectId::new("broken").unwrap()).unwrap_err();
        assert!(err.to_string().contains("broken.toml"), "{err}");
    }

    /// H2: a schema-1 project file (`mysql = "8.4"`, `redis = true`) loads into the services
    /// map, and saving it writes schema 2 without the retired keys (which would otherwise
    /// resurrect a removed service on the next read).
    #[test]
    fn schema_1_files_are_migrated_and_resaved_without_the_retired_keys() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(&dir);
        let file = p.project_file(&ProjectId::new("beta").unwrap());
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let legacy = r#"active = true
database = "beta"
id = "beta"
name = "beta"
origin_preset = "generic-php"
root = 'C:\sites\beta'
schema = 1
tls = "disabled"

[[addresses]]
host = "beta.localhost"
kind = "localhost"

[[addresses]]
kind = "loopback-port"
port = 8101

[environment]
addons = []
apache_directives = ""
extensions = ["mysqli", "pdo_mysql"]
mysql = "8.4"
php = "7.4"
public_dir = ""
redis = true

[environment.php_ini]
"#;
        std::fs::write(&file, legacy).unwrap();
        let repo = TomlProjectRepository::new(p);
        let mut beta = repo.get(&ProjectId::new("beta").unwrap()).unwrap().unwrap();
        assert_eq!(beta.environment.mysql(), Some(lampo_domain::mysql::MysqlSeries::DEFAULT));
        assert!(beta.environment.uses(&lampo_domain::service::ServiceKind::REDIS));

        beta.environment.drop_service(&lampo_domain::service::ServiceKind::REDIS);
        repo.save(&beta).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("schema = 2") && !text.contains("redis = ") && !text.contains("mysql = \""), "{text}");
        let again = repo.get(&beta.id).unwrap().unwrap();
        assert!(!again.environment.uses(&lampo_domain::service::ServiceKind::REDIS), "the removed service stays removed");
        assert_eq!(again, beta);
    }

    #[test]
    fn delete_is_idempotent_and_keeps_unknown_fields_on_resave() {
        let dir = tempfile::tempdir().unwrap();
        let repo = TomlProjectRepository::new(paths(&dir));
        let p = project("shop", "C:\\sites\\shop");
        repo.save(&p).unwrap();
        let file = paths(&dir).project_file(&p.id);
        let mut text = std::fs::read_to_string(&file).unwrap();
        text = format!("from_the_future = \"yes\"\n{text}");
        std::fs::write(&file, text).unwrap();
        repo.save(&p).unwrap();
        assert!(std::fs::read_to_string(&file).unwrap().contains("from_the_future"));
        repo.delete(&p.id).unwrap();
        repo.delete(&p.id).unwrap();
        assert!(repo.get(&p.id).unwrap().is_none());
    }
}
