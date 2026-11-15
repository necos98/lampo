//! `%APPDATA%\Lampo\projects\<id>.toml`. The TOML schema is the serde form of
//! `lampo_domain::project::Project` plus a `schema` header for migrations.
//!
//! A file that does not load (malformed, or edited into values the domain rejects) is listed as
//! unreadable with the loopback ports it names, and copied aside before a save replaces it.

use lampo_domain::PortResult;
use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::{ProjectListing, ProjectRepository, UnreadableProject};
use lampo_domain::project::{Project, ProjectId};

use crate::atomic::{TomlScan, read_toml, remove_file_if_exists, scan_toml, write_toml};
use crate::unreadable::loopback_ports_in;

pub struct TomlProjectRepository {
    paths: LampoPaths,
}

impl TomlProjectRepository {
    pub fn new(paths: LampoPaths) -> Self {
        Self { paths }
    }
}

impl ProjectRepository for TomlProjectRepository {
    fn scan(&self) -> PortResult<ProjectListing> {
        let TomlScan { loaded: mut projects, failed } = scan_toml::<Project>(&self.paths.projects_dir())?;
        projects.sort_by(|a, b| a.id.cmp(&b.id));
        let unreadable = failed.into_iter().map(|(file, error)| UnreadableProject { loopback_ports: loopback_ports_in(&file), reason: error.to_string(), file }).collect();
        Ok(ProjectListing { projects, unreadable })
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

    /// Rewrites the saved file of `p` through `edit`, as a user editing it by hand would.
    fn edit_file(dir: &tempfile::TempDir, p: &Project, edit: impl FnOnce(&mut toml::Table)) -> std::path::PathBuf {
        let file = paths(dir).project_file(&p.id);
        let mut table = crate::atomic::read_table(&file).unwrap().unwrap();
        edit(&mut table);
        crate::atomic::write_table(&file, &table).unwrap();
        file
    }

    /// A file edited into values the domain rejects does not load (no aggregate that panics
    /// later): `scan` reports it, `list` leaves it out, `get` names it.
    #[test]
    fn files_breaking_the_project_invariants_are_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let repo = TomlProjectRepository::new(paths(&dir));
        let (shop, blog, api) = (project("shop", r"C:\sites\shop"), project("blog", r"C:\sites\blog"), project("api", r"C:\sites\api"));
        for p in [&shop, &blog, &api] {
            repo.save(p).unwrap();
        }
        edit_file(&dir, &shop, |t| {
            t.insert("addresses".into(), toml::Value::Array(Vec::new()));
        });
        edit_file(&dir, &blog, |t| {
            let services = t["environment"].as_table_mut().unwrap()["services"].as_table_mut().unwrap();
            assert!(services.remove("mysql").is_some());
            assert!(t.contains_key("database"));
        });

        let listing = repo.scan().unwrap();
        assert_eq!(listing.projects, std::slice::from_ref(&api));
        let unreadable: Vec<_> = listing.unreadable.iter().map(|u| (u.file.file_name().unwrap().to_string_lossy().into_owned(), u.reason.clone())).collect();
        assert_eq!(unreadable.len(), 2, "{unreadable:?}");
        assert_eq!(unreadable[0].0, "blog.toml");
        assert!(unreadable[0].1.contains("blog.toml") && unreadable[0].1.contains("needs MySQL"), "{unreadable:?}");
        assert_eq!(unreadable[1].0, "shop.toml");
        assert!(unreadable[1].1.contains("at least one address"), "{unreadable:?}");
        assert_eq!(repo.list().unwrap(), [api]);
        assert!(repo.get(&shop.id).unwrap_err().to_string().contains("shop.toml"));
        assert!(repo.get(&blog.id).unwrap_err().to_string().contains("blog.toml"));
    }

    /// A corrupt file keeps its loopback ports reserved, and a save over it (a new project with
    /// the same id) keeps a copy of the user's content first.
    #[test]
    fn corrupt_files_keep_their_ports_and_are_backed_up_before_a_save_replaces_them() {
        let dir = tempfile::tempdir().unwrap();
        let repo = TomlProjectRepository::new(paths(&dir));
        let shop = project("shop", r"C:\sites\shop");
        repo.save(&shop).unwrap();
        let file = paths(&dir).project_file(&shop.id);
        let corrupt = "id = \"shop\"\n[[addresses]]\nkind = \"loopback-port\"\nport = 8107\n[[[\n";
        std::fs::write(&file, corrupt).unwrap();

        let listing = repo.scan().unwrap();
        assert!(listing.projects.is_empty());
        assert_eq!(listing.unreadable.len(), 1);
        assert_eq!(listing.unreadable[0].file, file);
        assert_eq!(listing.unreadable[0].loopback_ports.iter().copied().collect::<Vec<_>>(), [8107]);

        repo.save(&shop).unwrap();
        assert_eq!(repo.get(&shop.id).unwrap().unwrap(), shop);
        let copies: Vec<_> = std::fs::read_dir(paths(&dir).projects_dir())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("shop.toml.corrupt-"))
            .collect();
        assert_eq!(copies.len(), 1, "{copies:?}");
        assert_eq!(std::fs::read_to_string(&copies[0]).unwrap(), corrupt);
        assert_eq!(repo.scan().unwrap(), ProjectListing { projects: vec![shop], unreadable: Vec::new() }, "the copy is not listed");
    }
}
