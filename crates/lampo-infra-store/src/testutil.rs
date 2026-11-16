//! Shared fixtures for unit tests.

use lampo_domain::address::SiteAddress;
use lampo_domain::hostname::Tld;
use lampo_domain::mysql::{DatabaseName, MysqlSeries};
use lampo_domain::paths::LampoPaths;
use lampo_domain::php::PhpMinor;
use lampo_domain::preset::{PresetContext, builtin};
use lampo_domain::project::{DisplayName, Project, ProjectId, ProjectRoot, TlsMode};
use lampo_domain::site::WorktreeSlug;
use lampo_domain::tcp_port::Port;
use lampo_domain::worktree::{WorktreeDatabase, WorktreeEnvironment};

pub fn paths(dir: &tempfile::TempDir) -> LampoPaths {
    LampoPaths::new(dir.path().join("config"), dir.path().join("data"))
}

pub fn project(id: &str, root: &str) -> Project {
    let ctx = PresetContext { latest_php: PhpMinor::new(8, 4).unwrap(), default_mysql: MysqlSeries::DEFAULT };
    let preset = builtin::all().into_iter().find(|p| p.id().as_str() == "laravel").unwrap().build(&ctx);
    let mut p = Project::new(
        ProjectId::new(id).unwrap(),
        DisplayName::new(format!("Project {id}")).unwrap(),
        ProjectRoot::new(root).unwrap(),
        SiteAddress::defaults_for(id, &Tld::default()).unwrap(),
        preset.environment.clone(),
    )
    .unwrap();
    p.set_database_name(DatabaseName::new(id.replace('-', "_")).unwrap());
    p.tls = TlsMode::LocalCa;
    p.active = true;
    p.origin_preset = Some(preset.id);
    p
}

pub fn worktree(parent: &str, slug: &str, root: &str, port: u16) -> WorktreeEnvironment {
    WorktreeEnvironment {
        parent: ProjectId::new(parent).unwrap(),
        slug: WorktreeSlug::new(slug).unwrap(),
        root: ProjectRoot::new(root).unwrap(),
        branch: Some(format!("feature/{slug}")),
        loopback_port: Port::of(port),
        database: WorktreeDatabase::Own { name: DatabaseName::new(format!("{parent}_{slug}").replace('-', "_")).unwrap(), cloned_from: Some(DatabaseName::new("shop").unwrap()) },
        created_at_unix_ms: 1_700_000_000_000,
    }
}
