//! The MySQL accounts of the local instances (see [`crate::mysql::accounts`]): the stored
//! passwords of the projects' dedicated accounts, new random passwords, and the statements that
//! give root its password and keep each project account's grants as planned. Implemented by
//! `lampo-infra-services` (DPAPI files, the system RNG, `MysqlCliAdmin`).

use std::collections::BTreeSet;

use async_trait::async_trait;

use crate::error::PortResult;
use crate::mysql::DatabaseName;
use crate::mysql::accounts::MysqlUserName;
use crate::mysql::credentials::MysqlPassword;
use crate::ports::services::MysqlEndpoint;
use crate::project::ProjectId;

/// The stored password of a project's dedicated account. Synchronous: one small file, read by
/// the shims too.
pub trait MysqlProjectPasswordReader: Send + Sync {
    /// `None` when none is stored yet.
    fn project_password(&self, project: &ProjectId) -> PortResult<Option<MysqlPassword>>;
}

/// Stores the password of a project's dedicated account, once.
pub trait MysqlProjectPasswordWriter: Send + Sync {
    /// Stores `password` unless one is stored already, atomically, and returns the stored one: two
    /// first uses racing agree on one password.
    fn save_project_password_if_absent(&self, project: &ProjectId, password: &MysqlPassword) -> PortResult<MysqlPassword>;
}

/// New account passwords from a cryptographically secure source.
pub trait MysqlPasswordGenerator: Send + Sync {
    fn generate(&self) -> PortResult<MysqlPassword>;
}

/// What [`MysqlAccountAdmin::secure_root`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootPasswordChange {
    /// Root already authenticates with the password.
    AlreadySet,
    /// Root had no password and now has this one.
    Applied,
}

/// A project account as the server must have it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectAccountGrant {
    pub user: MysqlUserName,
    pub password: MysqlPassword,
    /// Every privilege on these databases and none elsewhere.
    pub databases: BTreeSet<DatabaseName>,
}

/// Account administration on a running instance, as its root. Passwords travel to the server in
/// statements the client reads from a file only the current user can open, never on a command
/// line.
#[async_trait]
pub trait MysqlAccountAdmin: Send + Sync {
    /// Gives every `root` account of the instance `password`, unless root already accepts it.
    /// Root without a password is the only state it changes: when root accepts neither, the
    /// error says so and nothing is changed.
    async fn secure_root(&self, endpoint: &MysqlEndpoint, password: &MysqlPassword) -> PortResult<RootPasswordChange>;

    /// Makes the instance's project accounts exactly `accounts` (connecting as root with its
    /// stored password, if any): each created or given its password, its privileges replaced by
    /// all privileges on its databases; the accounts named like project accounts and not listed
    /// are dropped.
    async fn sync_project_accounts(&self, endpoint: &MysqlEndpoint, accounts: &[ProjectAccountGrant]) -> PortResult<()>;
}
