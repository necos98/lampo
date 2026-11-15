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
use crate::error::DomainError;
use crate::hostname::Hostname;
use crate::mysql::{DatabaseConnection, DatabaseName};
use crate::project::{Project, ProjectEnvironment, ProjectRoot};
use crate::settings::Settings;
use crate::site::{SiteId, WorktreeSlug};

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
    pub loopback_port: u16,
    pub database: WorktreeDatabase,
    /// Set by the caller (the domain has no clock).
    pub created_at_unix_ms: u64,
}

impl WorktreeEnvironment {
    /// Builds a new environment for `root` under `parent`.
    /// `existing` are the parent's current environments (slug de-duplication).
    pub fn derive(
        parent: &Project,
        existing: &[WorktreeEnvironment],
        root: ProjectRoot,
        branch: Option<String>,
        loopback_port: u16,
        mode: &DatabaseMode,
        created_at_unix_ms: u64,
    ) -> Result<Self, DomainError> {
        if root == parent.root {
            return Err(DomainError::InvalidWorktree("the worktree folder is the project folder itself".into()));
        }
        if let Some(other) = existing.iter().find(|e| e.root == root) {
            return Err(DomainError::InvalidWorktree(format!("an environment already exists for this folder (`{}`)", other.site_id())));
        }
        let label = branch.clone().unwrap_or_else(|| root.as_str().rsplit('\\').next().unwrap_or("worktree").to_string());
        let slug = unique_slug(existing, WorktreeSlug::from_label(&label));
        let database = match mode {
            DatabaseMode::Shared => WorktreeDatabase::Shared,
            DatabaseMode::Fresh | DatabaseMode::Clone { .. } if parent.environment.mysql().is_none() => {
                return Err(DomainError::InvalidWorktree("the parent project does not use MySQL".into()));
            }
            DatabaseMode::Fresh => WorktreeDatabase::Own { name: own_database_name(parent, &slug)?, cloned_from: None },
            DatabaseMode::Clone { source } => WorktreeDatabase::Own { name: own_database_name(parent, &slug)?, cloned_from: Some(source.clone()) },
        };
        Ok(Self { parent: parent.id.clone(), slug, root, branch, loopback_port, database, created_at_unix_ms })
    }

    pub fn site_id(&self) -> SiteId {
        SiteId::Worktree { parent: self.parent.clone(), slug: self.slug.clone() }
    }

    /// `<slug>.<project>.localhost`.
    pub fn localhost_alias(&self) -> Hostname {
        Hostname::new(format!("{}.{}.localhost", self.slug, self.parent)).expect("slugs always form a valid hostname")
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
            WorktreeDatabase::Shared => parent.database.as_ref(),
            WorktreeDatabase::Own { name, .. } => Some(name),
        }
    }

    pub fn database_connection(&self, parent: &Project, settings: &Settings) -> Result<Option<DatabaseConnection>, DomainError> {
        let (Some(series), Some(db)) = (parent.environment.mysql(), self.database_name(parent).cloned()) else { return Ok(None) };
        let port = settings.mysql_port(series).ok_or_else(|| DomainError::UnknownMysqlSeries(series.to_string()))?;
        Ok(Some(DatabaseConnection::local(series, port, db)))
    }

    /// Stale: its folder is gone (`folder_exists` answers for [`Self::root`]) or its parent
    /// project no longer exists. A stale environment cannot be used; `prune` removes it. The one
    /// definition shared by `prune`, `doctor` and the listings.
    pub fn is_stale(&self, projects: &[Project], folder_exists: impl FnOnce(&ProjectRoot) -> bool) -> bool {
        !projects.iter().any(|p| p.id == self.parent) || !folder_exists(&self.root)
    }

    /// The database to drop when the environment is removed with `drop_database`.
    pub fn owned_database(&self) -> Option<&DatabaseName> {
        match &self.database {
            WorktreeDatabase::Own { name, .. } => Some(name),
            WorktreeDatabase::Shared => None,
        }
    }
}

fn unique_slug(existing: &[WorktreeEnvironment], base: WorktreeSlug) -> WorktreeSlug {
    let taken = |s: &str| existing.iter().any(|e| e.slug.as_str() == s);
    if !taken(base.as_str()) {
        return base;
    }
    (2u32..)
        .map(|n| {
            let suffix = format!("-{n}");
            let mut stem = base.as_str().to_string();
            stem.truncate(63 - suffix.len());
            WorktreeSlug::new(format!("{}{suffix}", stem.trim_end_matches('-'))).expect("valid slug")
        })
        .find(|s| !taken(s.as_str()))
        .expect("unbounded")
}

/// `<parent db>_wt_<slug>`, truncated to MySQL's 64-character limit.
pub fn own_database_name(parent: &Project, slug: &WorktreeSlug) -> Result<DatabaseName, DomainError> {
    let base = parent.database.as_ref().map(|d| d.to_string()).unwrap_or_else(|| parent.id.as_str().replace('-', "_"));
    let mut name = format!("{base}_wt_{}", slug.as_str().replace('-', "_"));
    name.truncate(64);
    DatabaseName::new(name)
}

/// Inclusive TCP port range for dedicated loopback listeners (worktree environments and
/// projects with a `LoopbackPort` address).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortRange {
    pub start: u16,
    pub end: u16,
}

impl PortRange {
    pub fn new(start: u16, end: u16) -> Result<Self, DomainError> {
        if start < 1024 || end < start {
            return Err(DomainError::InvalidSettings(format!("invalid port range {start}-{end}")));
        }
        Ok(Self { start, end })
    }

    pub fn contains(&self, port: u16) -> bool {
        (self.start..=self.end).contains(&port)
    }
}

impl Default for PortRange {
    fn default() -> Self {
        Self { start: crate::address::LOOPBACK_PORT_RANGE_START, end: 8199 }
    }
}

/// Every loopback port already assigned to a project address or a worktree environment.
pub fn used_loopback_ports(projects: &[Project], worktrees: &[WorktreeEnvironment]) -> BTreeSet<u16> {
    projects.iter().flat_map(|p| p.addresses.iter().filter_map(SiteAddress::loopback)).chain(worktrees.iter().map(|w| w.loopback_port)).collect()
}

/// Lowest port of the configured range that is not assigned and not a service port.
/// `PortProbe` still has to confirm the OS port is free.
pub fn allocate_loopback_port(used: &BTreeSet<u16>, settings: &Settings) -> Result<u16, DomainError> {
    (settings.loopback_ports.start..=settings.loopback_ports.end)
        .find(|p| !used.contains(p) && settings.service_on_port(*p).is_none())
        .ok_or(DomainError::NoFreeLoopbackPort { start: settings.loopback_ports.start, end: settings.loopback_ports.end })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mysql::MysqlSeries;
    use crate::project::tests::project;

    fn root(s: &str) -> ProjectRoot {
        ProjectRoot::new(s).unwrap()
    }

    #[test]
    fn derive_uses_branch_and_dedupes_slugs() {
        let parent = project("shop", "C:\\sites\\shop");
        let a = WorktreeEnvironment::derive(&parent, &[], root("D:\\wt\\a"), Some("feature/login".into()), 8100, &DatabaseMode::Shared, 1).unwrap();
        assert_eq!(a.site_id().to_string(), "shop@feature-login");
        assert_eq!(a.localhost_alias().as_str(), "feature-login.shop.localhost");
        assert_eq!(a.url(), "http://127.0.0.1:8100");
        assert!(a.addresses().iter().all(|x| !x.needs_hosts_entry()));
        let b = WorktreeEnvironment::derive(&parent, std::slice::from_ref(&a), root("D:\\wt\\b"), Some("feature/login".into()), 8101, &DatabaseMode::Shared, 1).unwrap();
        assert_eq!(b.slug.as_str(), "feature-login-2");
        let c = WorktreeEnvironment::derive(&parent, &[], root("D:\\wt\\no-branch"), None, 8102, &DatabaseMode::Shared, 1).unwrap();
        assert_eq!(c.slug.as_str(), "no-branch");
    }

    #[test]
    fn derive_rejects_bad_folders() {
        let parent = project("shop", "C:\\sites\\shop");
        assert!(WorktreeEnvironment::derive(&parent, &[], root("C:\\Sites\\Shop"), None, 8100, &DatabaseMode::Shared, 1).is_err());
        let a = WorktreeEnvironment::derive(&parent, &[], root("D:\\wt\\a"), None, 8100, &DatabaseMode::Shared, 1).unwrap();
        assert!(WorktreeEnvironment::derive(&parent, &[a], root("d:\\WT\\a"), None, 8101, &DatabaseMode::Shared, 1).is_err());
    }

    #[test]
    fn database_modes() {
        let parent = project("my-shop", "C:\\sites\\shop");
        let shared = WorktreeEnvironment::derive(&parent, &[], root("D:\\wt\\a"), Some("x".into()), 8100, &DatabaseMode::Shared, 1).unwrap();
        assert_eq!(shared.database_name(&parent).unwrap().as_str(), "my_shop");
        assert!(shared.owned_database().is_none());

        let mode = DatabaseMode::Clone { source: DatabaseName::new("my_shop").unwrap() };
        let cloned = WorktreeEnvironment::derive(&parent, &[], root("D:\\wt\\b"), Some("fix-1".into()), 8101, &mode, 1).unwrap();
        assert_eq!(cloned.database_name(&parent).unwrap().as_str(), "my_shop_wt_fix_1");
        let conn = cloned.database_connection(&parent, &Settings::default()).unwrap().unwrap();
        assert_eq!((conn.port, conn.database.as_str()), (3306, "my_shop_wt_fix_1"));

        let mut no_db = parent.clone();
        no_db.environment.set_mysql(None);
        assert!(WorktreeEnvironment::derive(&no_db, &[], root("D:\\wt\\c"), None, 8102, &DatabaseMode::Fresh, 1).is_err());
        let _ = MysqlSeries::DEFAULT;
    }

    #[test]
    fn stale_means_folder_gone_or_parent_gone() {
        let parent = project("shop", "C:\\sites\\shop");
        let w = WorktreeEnvironment::derive(&parent, &[], root("D:\\wt\\a"), None, 8100, &DatabaseMode::Shared, 1).unwrap();
        let projects = std::slice::from_ref(&parent);
        assert!(!w.is_stale(projects, |_| true));
        assert!(w.is_stale(projects, |r| r.as_str() != "D:\\wt\\a"));
        assert!(w.is_stale(&[], |_| true), "orphans are stale");
    }

    #[test]
    fn document_root_follows_parent_public_dir() {
        let mut parent = project("shop", "C:\\sites\\shop");
        parent.environment.public_dir = crate::project::PublicDir::new("public").unwrap();
        let w = WorktreeEnvironment::derive(&parent, &[], root("D:\\wt\\a"), None, 8100, &DatabaseMode::Shared, 1).unwrap();
        assert_eq!(w.document_root(&parent), PathBuf::from("D:\\wt\\a").join("public"));
    }

    #[test]
    fn port_allocation_skips_used_and_service_ports() {
        let s = Settings { loopback_ports: PortRange::new(3305, 3308).unwrap(), ..Default::default() };
        let used: BTreeSet<u16> = [3305].into();
        // 3306 and 3307 are MySQL ports.
        assert_eq!(allocate_loopback_port(&used, &s).unwrap(), 3308);
        let used: BTreeSet<u16> = [3305, 3308].into();
        assert!(allocate_loopback_port(&used, &s).is_err());
    }
}
