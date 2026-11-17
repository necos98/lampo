//! Worktree environments: per-git-worktree copies of a registered project's site, so several
//! coding agents can each hit their own checkout over HTTP in parallel.
//!
//! * Derived from a parent project and keyed by the worktree folder.
//! * Inherit the parent's whole [`ProjectEnvironment`] (PHP, extensions, php.ini, public dir,
//!   Apache directives, MySQL series, Redis flag); document root = worktree + public dir.
//! * Addressed by a dedicated, persisted loopback port (canonical URL) plus a
//!   `<slug>.<project>.localhost` alias on the same vhost. Never touch the hosts file → no UAC.
//! * Always served while they exist (independent of the parent's active flag).
//! * Stored in the app config next to the parent project, never in the repository.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::address::SiteAddress;
use crate::claude_code::fnv32;
use crate::error::DomainError;
use crate::hostname::Hostname;
use crate::mysql::{DatabaseConnection, DatabaseName};
use crate::project::{Project, ProjectEnvironment, ProjectId, ProjectRoot};
use crate::settings::Settings;
use crate::site::{SiteId, WorktreeSlug};
use crate::slug::MAX_SLUG_LEN;
use crate::tcp_port::Port;

/// How a worktree environment gets its database, chosen at creation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case")]
pub enum DatabaseMode {
    /// Use the parent's database(s) directly.
    Shared,
    /// A new, empty database owned by the environment.
    Fresh,
    /// A new database owned by the environment, cloned (dump + restore on the same MySQL
    /// instance) from `source`, one of the parent instance's databases.
    Clone { source: DatabaseName },
}

/// The database actually assigned to an environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum WorktreeDatabase {
    /// Same database as the parent (`None` if the parent has no MySQL).
    Shared,
    /// Owned by the environment; dropped on `down` only with an explicit flag.
    Own { name: DatabaseName, cloned_from: Option<DatabaseName> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeEnvironment {
    pub parent: crate::project::ProjectId,
    pub slug: WorktreeSlug,
    /// The worktree folder: the environment's identity on disk.
    pub root: ProjectRoot,
    /// Branch checked out when the environment was created (informational).
    pub branch: Option<String>,
    /// Persisted, so the URL stays stable across restarts.
    pub loopback_port: Port,
    pub database: WorktreeDatabase,
    /// Set by the caller (the domain has no clock).
    pub created_at_unix_ms: u64,
}

/// What a new environment must not collide with: every registered site (folders, `.localhost`
/// aliases, databases) and the databases already present on the parent's MySQL instance.
#[derive(Debug, Clone, Copy, Default)]
pub struct Occupancy<'a> {
    pub projects: &'a [Project],
    /// Every environment, of every project.
    pub worktrees: &'a [WorktreeEnvironment],
    /// Every database found on the parent's MySQL instance, including those no site owns any
    /// more (a `down` without `--drop-database` keeps its database): never to be adopted.
    pub server_databases: &'a [String],
}

impl Occupancy<'_> {
    /// `slug` is free under `parent`: no sibling has it and no project serves its alias.
    fn slug_is_free(&self, parent: &Project, slug: &str) -> bool {
        let alias = localhost_alias(slug, &parent.id);
        let sibling = self.worktrees.iter().any(|w| w.parent == parent.id && w.slug.as_str() == slug);
        let served = self.projects.iter().flat_map(|p| p.addresses()).filter_map(SiteAddress::host).any(|h| h.as_str() == alias);
        !sibling && !served
    }

    /// MySQL compares schema names case-insensitively on Windows.
    fn database_is_taken(&self, name: &str) -> bool {
        let registered = self.projects.iter().filter_map(Project::database).chain(self.worktrees.iter().filter_map(WorktreeEnvironment::owned_database)).map(DatabaseName::as_str);
        registered.chain(self.server_databases.iter().map(String::as_str)).any(|taken| taken.eq_ignore_ascii_case(name))
    }
}

impl WorktreeEnvironment {
    /// Builds a new environment for `root` under `parent`, with a slug, an alias and (for
    /// `Fresh`/`Clone`) a database that no other site in `taken` uses.
    pub fn derive(
        parent: &Project,
        taken: &Occupancy<'_>,
        root: ProjectRoot,
        branch: Option<String>,
        loopback_port: Port,
        mode: &DatabaseMode,
        created_at_unix_ms: u64,
    ) -> Result<Self, DomainError> {
        if root == parent.root {
            return Err(DomainError::InvalidWorktree("the worktree folder is the project folder itself".into()));
        }
        if let Some(other) = taken.projects.iter().find(|p| p.root == root) {
            return Err(DomainError::DuplicateProjectRoot { root: root.to_string(), other: other.id.to_string() });
        }
        if let Some(other) = taken.worktrees.iter().find(|w| w.root == root) {
            return Err(DomainError::InvalidWorktree(format!("an environment already exists for this folder (`{}`)", other.site_id())));
        }
        let label = branch.clone().unwrap_or_else(|| root.as_str().rsplit('\\').next().unwrap_or("worktree").to_string());
        let slug = unique_slug(WorktreeSlug::from_label(&label), |s| taken.slug_is_free(parent, s));
        let database = match mode {
            DatabaseMode::Shared => WorktreeDatabase::Shared,
            DatabaseMode::Fresh | DatabaseMode::Clone { .. } if parent.environment.mysql().is_none() => {
                return Err(DomainError::InvalidWorktree("the parent project does not use MySQL".into()));
            }
            DatabaseMode::Fresh => WorktreeDatabase::Own { name: own_database_name(parent, &slug, taken)?, cloned_from: None },
            DatabaseMode::Clone { source } => WorktreeDatabase::Own { name: own_database_name(parent, &slug, taken)?, cloned_from: Some(source.clone()) },
        };
        Ok(Self { parent: parent.id.clone(), slug, root, branch, loopback_port, database, created_at_unix_ms })
    }

    pub fn site_id(&self) -> SiteId {
        SiteId::Worktree { parent: self.parent.clone(), slug: self.slug.clone() }
    }

    /// `<slug>.<project>.localhost`.
    pub fn localhost_alias(&self) -> Hostname {
        Hostname::new(localhost_alias(self.slug.as_str(), &self.parent)).expect("slugs always form a valid hostname")
    }

    /// Canonical address first (loopback port), then the `.localhost` alias.
    pub fn addresses(&self) -> Vec<SiteAddress> {
        vec![SiteAddress::LoopbackPort(self.loopback_port), SiteAddress::Localhost(self.localhost_alias())]
    }

    /// Canonical URL (`http://127.0.0.1:8101`).
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.loopback_port)
    }

    /// The environment inherited from the parent (the parent's current settings, no snapshot).
    pub fn environment<'a>(&self, parent: &'a Project) -> &'a ProjectEnvironment {
        &parent.environment
    }

    pub fn document_root(&self, parent: &Project) -> PathBuf {
        let mut p = PathBuf::from(self.root.as_str());
        for seg in parent.environment.public_dir.as_str().split('/').filter(|s| !s.is_empty()) {
            p.push(seg);
        }
        p
    }

    /// The database the environment's code should use.
    pub fn database_name<'a>(&'a self, parent: &'a Project) -> Option<&'a DatabaseName> {
        match &self.database {
            WorktreeDatabase::Shared => parent.database(),
            WorktreeDatabase::Own { name, .. } => Some(name),
        }
    }

    pub fn database_connection(&self, parent: &Project, settings: &Settings) -> Result<Option<DatabaseConnection>, DomainError> {
        let (Some(series), Some(db)) = (parent.environment.mysql(), self.database_name(parent).cloned()) else { return Ok(None) };
        let port = settings.mysql_port(series).ok_or_else(|| DomainError::UnknownMysqlSeries(series.to_string()))?;
        Ok(Some(DatabaseConnection::local(series, port, db, &parent.id, parent.mysql_credentials)))
    }

    /// Stale: its folder is gone (`folder_exists` answers for [`Self::root`]) or its parent
    /// project no longer exists (`parent_registered` answers for [`Self::parent`]: a project file
    /// that does not load still exists, its project is hidden, not gone). A stale environment
    /// cannot be used; `prune` removes it. The one definition shared by `prune`, `doctor` and the
    /// listings.
    pub fn is_stale(&self, parent_registered: impl FnOnce(&ProjectId) -> bool, folder_exists: impl FnOnce(&ProjectRoot) -> bool) -> bool {
        !parent_registered(&self.parent) || !folder_exists(&self.root)
    }

    /// The database to drop when the environment is removed with `drop_database`.
    pub fn owned_database(&self) -> Option<&DatabaseName> {
        match &self.database {
            WorktreeDatabase::Own { name, .. } => Some(name),
            WorktreeDatabase::Shared => None,
        }
    }
}

/// The `.localhost` alias of the environment `slug` of `parent`.
fn localhost_alias(slug: &str, parent: &ProjectId) -> String {
    format!("{slug}.{parent}.localhost")
}

/// `base`, else `base-2`, `base-3`, … : the first one `is_free` accepts.
fn unique_slug(base: WorktreeSlug, is_free: impl Fn(&str) -> bool) -> WorktreeSlug {
    if is_free(base.as_str()) {
        return base;
    }
    (2u32..)
        .map(|n| {
            let suffix = format!("-{n}");
            let mut stem = base.as_str().to_string();
            stem.truncate(MAX_SLUG_LEN - suffix.len());
            WorktreeSlug::new(format!("{}{suffix}", stem.trim_end_matches('-'))).expect("valid slug")
        })
        .find(|s| is_free(s.as_str()))
        .expect("unbounded")
}

/// MySQL's limit on a schema name.
const MAX_DATABASE_NAME_LEN: usize = 64;
/// `_` + the 8 hex digits of the hash that keeps a shortened name distinct.
const HASH_SUFFIX_LEN: usize = 9;

/// `<parent db>_wt_<slug>`, else `<parent db>_wt_<slug>_2`, `_3`, … : the first name no other
/// site and no database of the instance uses. A name over MySQL's limit is shortened to
/// [`within_database_name_limit`]. Names that fit and are free are the plain form, so existing
/// environments keep theirs.
fn own_database_name(parent: &Project, slug: &WorktreeSlug, taken: &Occupancy<'_>) -> Result<DatabaseName, DomainError> {
    let base = parent.database().map(|d| d.to_string()).unwrap_or_else(|| parent.id.as_str().replace('-', "_"));
    let preferred = format!("{base}_wt_{}", slug.as_str().replace('-', "_"));
    let numbered = (2u32..).map(|n| format!("{preferred}_{n}"));
    let name = std::iter::once(preferred.clone()).chain(numbered).map(|n| within_database_name_limit(&n)).find(|n| !taken.database_is_taken(n)).expect("unbounded");
    DatabaseName::new(name)
}

/// `name` unchanged when it fits MySQL's limit; otherwise its head followed by `_<fnv32 of the
/// whole name>`, so two long names sharing their first 64 characters stay distinct.
fn within_database_name_limit(name: &str) -> String {
    if name.len() <= MAX_DATABASE_NAME_LEN {
        return name.to_string();
    }
    let head: String = name.chars().take(MAX_DATABASE_NAME_LEN - HASH_SUFFIX_LEN).collect();
    format!("{}_{:08x}", head.trim_end_matches('_'), fnv32(name))
}

/// Every loopback port already assigned to a project address or a worktree environment.
pub fn used_loopback_ports(projects: &[Project], worktrees: &[WorktreeEnvironment]) -> BTreeSet<Port> {
    projects.iter().flat_map(|p| p.addresses().iter().filter_map(SiteAddress::loopback)).chain(worktrees.iter().map(|w| w.loopback_port)).collect()
}

/// Lowest port of the configured range that is not assigned and not a service port.
/// `PortProbe` still has to confirm the OS port is free.
pub fn allocate_loopback_port(used: &BTreeSet<Port>, settings: &Settings) -> Result<Port, DomainError> {
    let range = settings.loopback_ports;
    range.ports().find(|p| !used.contains(p) && settings.service_on_port(*p).is_none()).ok_or(DomainError::NoFreeLoopbackPort { start: range.start().get(), end: range.end().get() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mysql::MysqlSeries;
    use crate::project::tests::project;
    use crate::tcp_port::PortRange;

    fn root(s: &str) -> ProjectRoot {
        ProjectRoot::new(s).unwrap()
    }

    #[test]
    fn derive_uses_branch_and_dedupes_slugs() {
        let parent = project("shop", "C:\\sites\\shop");
        let a = WorktreeEnvironment::derive(&parent, &Occupancy::default(), root("D:\\wt\\a"), Some("feature/login".into()), Port::of(8100), &DatabaseMode::Shared, 1).unwrap();
        assert_eq!(a.site_id().to_string(), "shop@feature-login");
        assert_eq!(a.localhost_alias().as_str(), "feature-login.shop.localhost");
        assert_eq!(a.url(), "http://127.0.0.1:8100");
        assert!(a.addresses().iter().all(|x| !x.needs_hosts_entry()));
        let taken = Occupancy { worktrees: std::slice::from_ref(&a), ..Default::default() };
        let b = WorktreeEnvironment::derive(&parent, &taken, root("D:\\wt\\b"), Some("feature/login".into()), Port::of(8101), &DatabaseMode::Shared, 1).unwrap();
        assert_eq!(b.slug.as_str(), "feature-login-2");
        let c = WorktreeEnvironment::derive(&parent, &Occupancy::default(), root("D:\\wt\\no-branch"), None, Port::of(8102), &DatabaseMode::Shared, 1).unwrap();
        assert_eq!(c.slug.as_str(), "no-branch");
    }

    #[test]
    fn derive_rejects_bad_folders() {
        let parent = project("shop", "C:\\sites\\shop");
        assert!(WorktreeEnvironment::derive(&parent, &Occupancy::default(), root("C:\\Sites\\Shop"), None, Port::of(8100), &DatabaseMode::Shared, 1).is_err());
        let a = WorktreeEnvironment::derive(&parent, &Occupancy::default(), root("D:\\wt\\a"), None, Port::of(8100), &DatabaseMode::Shared, 1).unwrap();
        assert!(WorktreeEnvironment::derive(&parent, &Occupancy { worktrees: &[a], ..Default::default() }, root("d:\\WT\\a"), None, Port::of(8101), &DatabaseMode::Shared, 1).is_err());
    }

    #[test]
    fn database_modes() {
        let parent = project("my-shop", "C:\\sites\\shop");
        let shared = WorktreeEnvironment::derive(&parent, &Occupancy::default(), root("D:\\wt\\a"), Some("x".into()), Port::of(8100), &DatabaseMode::Shared, 1).unwrap();
        assert_eq!(shared.database_name(&parent).unwrap().as_str(), "my_shop");
        assert!(shared.owned_database().is_none());

        let mode = DatabaseMode::Clone { source: DatabaseName::new("my_shop").unwrap() };
        let cloned = WorktreeEnvironment::derive(&parent, &Occupancy::default(), root("D:\\wt\\b"), Some("fix-1".into()), Port::of(8101), &mode, 1).unwrap();
        assert_eq!(cloned.database_name(&parent).unwrap().as_str(), "my_shop_wt_fix_1");
        let conn = cloned.database_connection(&parent, &Settings::default()).unwrap().unwrap();
        assert_eq!((conn.port, conn.database.as_str()), (Port::of(3306), "my_shop_wt_fix_1"));

        let mut no_db = parent.clone();
        no_db.environment.set_mysql(None);
        assert!(WorktreeEnvironment::derive(&no_db, &Occupancy::default(), root("D:\\wt\\c"), None, Port::of(8102), &DatabaseMode::Fresh, 1).is_err());
        let _ = MysqlSeries::DEFAULT;
    }

    fn fresh(parent: &Project, taken: &Occupancy<'_>, folder: &str, branch: &str) -> WorktreeEnvironment {
        WorktreeEnvironment::derive(parent, taken, root(folder), Some(branch.into()), Port::of(8100), &DatabaseMode::Fresh, 1).unwrap()
    }

    fn own_name(env: &WorktreeEnvironment) -> &str {
        env.owned_database().unwrap().as_str()
    }

    #[test]
    fn long_database_names_keep_a_hash_of_the_whole_name() {
        let parent = project("shop", r"C:\sites\shop");
        let stem = "a".repeat(60);
        let first = fresh(&parent, &Occupancy::default(), r"D:\wt\one", &format!("{stem}-one"));
        let second = fresh(&parent, &Occupancy::default(), r"D:\wt\two", &format!("{stem}-two"));
        let (a, b) = (own_name(&first), own_name(&second));
        assert_eq!((a.len(), b.len()), (MAX_DATABASE_NAME_LEN, MAX_DATABASE_NAME_LEN));
        assert_ne!(a, b, "same first 64 characters, different slugs");
        assert!(a.starts_with("shop_wt_aaaa"), "{a}");
        assert_eq!(a, fresh(&parent, &Occupancy::default(), r"D:\wt\three", &format!("{stem}-one")).owned_database().unwrap().as_str(), "stable");
        // `shop_wt_` + 46 letters puts a separator right at the cut: it is not doubled.
        let on_separator = fresh(&parent, &Occupancy::default(), r"D:\wt\four", &format!("{}-{}", "a".repeat(46), "b".repeat(30)));
        let name = own_name(&on_separator);
        assert!(!name.contains("__") && name.len() == MAX_DATABASE_NAME_LEN - 1, "{name}");

        let fits = fresh(&parent, &Occupancy::default(), r"D:\wt\short", "feature/login");
        assert_eq!(own_name(&fits), "shop_wt_feature_login", "names within the limit keep the plain form");
    }

    #[test]
    fn own_databases_skip_names_used_by_any_site_or_found_on_the_instance() {
        let parent = project("shop", r"C:\sites\shop");
        let mut blog = project("blog", r"C:\sites\blog");
        blog.set_database_name(DatabaseName::new("SHOP_WT_FIX").unwrap());
        let projects = [parent.clone(), blog];
        // An environment of another project whose own database happens to bear the name.
        let mut claimed = fresh(&project("api", r"C:\sites\api"), &Occupancy::default(), r"D:\wt\y", "y");
        claimed.database = WorktreeDatabase::Own { name: DatabaseName::new("shop_wt_fix_2").unwrap(), cloned_from: None };
        let worktrees = [claimed];
        let server = ["shop_wt_fix_3".to_string()];
        let taken = Occupancy { projects: &projects, worktrees: &worktrees, server_databases: &server };
        let env = fresh(&parent, &taken, r"D:\wt\fix", "fix");
        assert_eq!(own_name(&env), "shop_wt_fix_4", "a project's, an environment's (case-insensitively) and a leftover database are skipped");
    }

    #[test]
    fn derive_refuses_a_registered_project_folder_and_any_environment_folder() {
        let parent = project("shop", r"C:\sites\shop");
        let projects = [parent.clone(), project("login", r"D:\wt\login")];
        let taken = Occupancy { projects: &projects, ..Default::default() };
        let err = WorktreeEnvironment::derive(&parent, &taken, root(r"d:\WT\login"), None, Port::of(8100), &DatabaseMode::Shared, 1).unwrap_err();
        assert_eq!(err, DomainError::DuplicateProjectRoot { root: r"D:\WT\login".into(), other: "login".into() });

        let other_parent = project("blog", r"C:\sites\blog");
        let theirs = WorktreeEnvironment::derive(&other_parent, &Occupancy::default(), root(r"D:\wt\x"), None, Port::of(8100), &DatabaseMode::Shared, 1).unwrap();
        let taken = Occupancy { worktrees: std::slice::from_ref(&theirs), ..Default::default() };
        assert!(matches!(WorktreeEnvironment::derive(&parent, &taken, root(r"D:\wt\x"), None, Port::of(8101), &DatabaseMode::Shared, 1), Err(DomainError::InvalidWorktree(_))));
    }

    #[test]
    fn slugs_skip_aliases_served_by_a_project() {
        let parent = project("shop", r"C:\sites\shop");
        let mut blog = project("blog", r"C:\sites\blog");
        blog.set_addresses(vec![SiteAddress::Localhost(Hostname::new("fix.shop.localhost").unwrap())]).unwrap();
        let projects = [parent.clone(), blog];
        let taken = Occupancy { projects: &projects, ..Default::default() };
        let env = WorktreeEnvironment::derive(&parent, &taken, root(r"D:\wt\fix"), Some("fix".into()), Port::of(8100), &DatabaseMode::Shared, 1).unwrap();
        assert_eq!(env.localhost_alias().as_str(), "fix-2.shop.localhost");
    }

    #[test]
    fn stale_means_folder_gone_or_parent_gone() {
        let parent = project("shop", "C:\\sites\\shop");
        let w = WorktreeEnvironment::derive(&parent, &Occupancy::default(), root("D:\\wt\\a"), None, Port::of(8100), &DatabaseMode::Shared, 1).unwrap();
        let registered = |id: &ProjectId| *id == parent.id;
        assert!(!w.is_stale(registered, |_| true));
        assert!(w.is_stale(registered, |r| r.as_str() != "D:\\wt\\a"));
        assert!(w.is_stale(|_| false, |_| true), "orphans are stale");
    }

    #[test]
    fn document_root_follows_parent_public_dir() {
        let mut parent = project("shop", "C:\\sites\\shop");
        parent.environment.public_dir = crate::project::PublicDir::new("public").unwrap();
        let w = WorktreeEnvironment::derive(&parent, &Occupancy::default(), root("D:\\wt\\a"), None, Port::of(8100), &DatabaseMode::Shared, 1).unwrap();
        assert_eq!(w.document_root(&parent), PathBuf::from("D:\\wt\\a").join("public"));
    }

    #[test]
    fn port_allocation_skips_used_and_service_ports() {
        let s = Settings { loopback_ports: PortRange::new(3305, 3308).unwrap(), ..Default::default() };
        let used: BTreeSet<Port> = [Port::of(3305)].into();
        // 3306 and 3307 are MySQL ports.
        assert_eq!(allocate_loopback_port(&used, &s).unwrap(), Port::of(3308));
        let used: BTreeSet<Port> = [Port::of(3305), Port::of(3308)].into();
        assert!(allocate_loopback_port(&used, &s).is_err());
    }
}
