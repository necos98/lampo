//! Which MySQL account a project's code connects as, and the accounts each local instance needs.
//!
//! A project either keeps the instance's administrative account ([`MysqlCredentials::Root`], the
//! transition for projects created before dedicated accounts, whose `.env` uses `root` without a
//! password) or gets an account of its own ([`MysqlCredentials::Project`]): `lampo_<id>`, with
//! every privilege on its database and on the own databases of its worktree environments, and
//! nothing else. The root of an instance gets a password only once no project on it uses root
//! ([`InstanceAccounts::secure_root`]): a password would lock their `.env` out.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::claude_code::fnv32;
use crate::mysql::{DatabaseName, MysqlSeries};
use crate::project::{Project, ProjectId};
use crate::worktree::WorktreeEnvironment;

/// How a project's code authenticates to its MySQL instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MysqlCredentials {
    /// The instance's `root` (transition): what project files written before dedicated accounts
    /// read as, so their `.env` with `root` and no password keeps working.
    #[default]
    Root,
    /// A dedicated account ([`MysqlUserName::for_project`]) granted only the project's databases.
    Project,
}

impl MysqlCredentials {
    /// What a new project gets: a dedicated account when the setting asks for it
    /// (`Settings::mysql_project_credentials`, on by default).
    pub fn for_new_project(dedicated: bool) -> Self {
        if dedicated { Self::Project } else { Self::Root }
    }
}

/// Prefix of the accounts Lampo creates for projects: a `lampo_` account no project owns any
/// more is dropped when the instance's accounts are reconciled.
pub const PROJECT_ACCOUNT_PREFIX: &str = "lampo_";
/// The longest user name every supported series accepts (MySQL 5.7: 32 characters).
pub const MAX_USER_NAME_LEN: usize = 32;
/// `_` + 8 hexadecimal digits of [`fnv32`].
const HASH_SUFFIX_LEN: usize = 9;

/// The user name of a project's dedicated account: `lampo_<id>` with `-` as `_`, so it is a
/// plain identifier; when that is longer than [`MAX_USER_NAME_LEN`], its head followed by
/// `_<fnv32 of the id>`, so two long ids sharing their first characters stay distinct. Always
/// `[a-z0-9_]` (a project id is `[a-z0-9-]`), so it needs no quoting beyond SQL's.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MysqlUserName(String);

impl MysqlUserName {
    pub fn for_project(project: &ProjectId) -> Self {
        let name = format!("{PROJECT_ACCOUNT_PREFIX}{}", project.as_str().replace('-', "_"));
        if name.len() <= MAX_USER_NAME_LEN {
            return Self(name);
        }
        let head: String = name.chars().take(MAX_USER_NAME_LEN - HASH_SUFFIX_LEN).collect();
        Self(format!("{}_{:08x}", head.trim_end_matches('_'), fnv32(project.as_str())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `user` is named like an account Lampo creates for projects.
    pub fn is_project_account(user: &str) -> bool {
        user.starts_with(PROJECT_ACCOUNT_PREFIX)
    }
}

impl fmt::Display for MysqlUserName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The dedicated account of one project on its instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectAccount {
    pub project: ProjectId,
    pub user: MysqlUserName,
    /// The project's database and the own databases of its worktree environments: the only
    /// ones the account may use (a database that does not exist yet can be created by it).
    pub databases: BTreeSet<DatabaseName>,
}

/// The accounts one instance needs for the registered projects.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InstanceAccounts {
    /// One per project on the instance with [`MysqlCredentials::Project`], ordered by project.
    pub accounts: Vec<ProjectAccount>,
    /// `true` when no project on the instance uses [`MysqlCredentials::Root`]: root may get a
    /// password. Once set it is never removed (a project switched back to root then uses it).
    pub secure_root: bool,
}

impl InstanceAccounts {
    /// The accounts of the `series` instance for `projects` (active or not: an inactive
    /// project's `.env` still names its account) and their worktree environments.
    pub fn plan(series: MysqlSeries, projects: &[Project], worktrees: &[WorktreeEnvironment]) -> Self {
        let on_instance = projects.iter().filter(|p| p.environment.mysql() == Some(series));
        let mut plan = Self { accounts: Vec::new(), secure_root: true };
        for project in on_instance {
            match project.mysql_credentials {
                MysqlCredentials::Root => plan.secure_root = false,
                MysqlCredentials::Project => {
                    let own = worktrees.iter().filter(|w| w.parent == project.id).filter_map(WorktreeEnvironment::owned_database);
                    let databases = project.database().into_iter().chain(own).cloned().collect();
                    plan.accounts.push(ProjectAccount { project: project.id.clone(), user: MysqlUserName::for_project(&project.id), databases });
                }
            }
        }
        plan.accounts.sort_by(|a, b| a.project.cmp(&b.project));
        plan
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::tests::project;
    use crate::site::WorktreeSlug;
    use crate::tcp_port::Port;
    use crate::worktree::WorktreeDatabase;

    fn id(s: &str) -> ProjectId {
        ProjectId::new(s).unwrap()
    }

    fn dedicated(name: &str) -> Project {
        let mut p = project(name, &format!(r"C:\sites\{name}"));
        p.mysql_credentials = MysqlCredentials::Project;
        p
    }

    fn worktree(parent: &str, slug: &str, database: WorktreeDatabase) -> WorktreeEnvironment {
        WorktreeEnvironment {
            parent: id(parent),
            slug: WorktreeSlug::new(slug).unwrap(),
            root: crate::project::ProjectRoot::new(format!(r"C:\wt\{parent}-{slug}")).unwrap(),
            branch: None,
            loopback_port: Port::of(8101),
            database,
            created_at_unix_ms: 0,
        }
    }

    fn db(name: &str) -> DatabaseName {
        DatabaseName::new(name).unwrap()
    }

    #[test]
    fn user_names_are_plain_identifiers_within_the_limit() {
        assert_eq!(MysqlUserName::for_project(&id("my-shop")).as_str(), "lampo_my_shop");
        let long = MysqlUserName::for_project(&id("a-very-long-project-name-for-the-test"));
        let other = MysqlUserName::for_project(&id("a-very-long-project-name-for-the-best"));
        assert_eq!(long.as_str().len(), MAX_USER_NAME_LEN, "{long}");
        assert!(long.as_str().starts_with("lampo_a_very_long_pr"), "{long}");
        assert_ne!(long, other, "a shared head does not collide");
        assert!(MysqlUserName::is_project_account(long.as_str()));
        assert!(!MysqlUserName::is_project_account("root"));
    }

    #[test]
    fn new_projects_follow_the_setting_and_old_files_read_as_root() {
        assert_eq!(MysqlCredentials::for_new_project(true), MysqlCredentials::Project);
        assert_eq!(MysqlCredentials::for_new_project(false), MysqlCredentials::Root);
        assert_eq!(MysqlCredentials::default(), MysqlCredentials::Root);
        assert_eq!(serde_json::to_string(&MysqlCredentials::Project).unwrap(), r#""project""#);
    }

    #[test]
    fn dedicated_accounts_get_their_database_and_their_worktrees_own_ones() {
        let projects = [dedicated("shop"), dedicated("blog")];
        let worktrees = [
            worktree("shop", "fix", WorktreeDatabase::Own { name: db("shop_wt_fix"), cloned_from: None }),
            worktree("shop", "shared", WorktreeDatabase::Shared),
            worktree("blog", "x", WorktreeDatabase::Own { name: db("blog_wt_x"), cloned_from: Some(db("blog")) }),
        ];
        let plan = InstanceAccounts::plan(MysqlSeries::DEFAULT, &projects, &worktrees);
        assert!(plan.secure_root);
        let accounts: Vec<(&str, Vec<&str>)> = plan.accounts.iter().map(|a| (a.user.as_str(), a.databases.iter().map(DatabaseName::as_str).collect())).collect();
        assert_eq!(accounts, [("lampo_blog", vec!["blog", "blog_wt_x"]), ("lampo_shop", vec!["shop", "shop_wt_fix"])]);
    }

    #[test]
    fn a_project_on_root_keeps_root_without_a_password_on_its_instance_only() {
        let mut legacy = project("legacy", r"C:\sites\legacy");
        legacy.environment.set_mysql(Some(MysqlSeries::LEGACY_5_7));
        let projects = [dedicated("shop"), legacy];
        let current = InstanceAccounts::plan(MysqlSeries::DEFAULT, &projects, &[]);
        assert!(current.secure_root);
        assert_eq!(current.accounts.len(), 1);
        let old = InstanceAccounts::plan(MysqlSeries::LEGACY_5_7, &projects, &[]);
        assert_eq!(old, InstanceAccounts { accounts: Vec::new(), secure_root: false });
    }

    #[test]
    fn an_instance_without_projects_secures_root() {
        assert_eq!(InstanceAccounts::plan(MysqlSeries::DEFAULT, &[], &[]), InstanceAccounts { accounts: Vec::new(), secure_root: true });
    }
}
