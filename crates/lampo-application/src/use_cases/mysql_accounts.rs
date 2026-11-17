//! The MySQL accounts of the local instances (see [`lampo_domain::mysql::accounts`]): each
//! running instance is brought to its [`InstanceAccounts`] plan by [`ApplyEnvironment`] (after a
//! start, and whenever the projects change the plan), and the connection details shown to the
//! user carry the password of the account they name.
//!
//! Passwords are generated here, stored before they are applied (so a crash in between is
//! repaired by the next provisioning: `secure_root` finds root still without a password and
//! applies the stored one) and never logged.
//!
//! [`ApplyEnvironment`]: super::environment::ApplyEnvironment

use std::sync::Arc;

use lampo_domain::mysql::DatabaseConnection;
use lampo_domain::mysql::accounts::InstanceAccounts;
use lampo_domain::mysql::credentials::MysqlPassword;
use lampo_domain::ports::{
    MysqlAccountAdmin, MysqlEndpoint, MysqlPasswordGenerator, MysqlProjectPasswordReader, MysqlProjectPasswordWriter, MysqlRootPasswordReader, MysqlRootPasswordWriter, ProjectAccountGrant,
    RootPasswordChange,
};
use lampo_domain::project::ProjectId;
use lampo_domain::service::ServiceId;

use crate::error::AppResult;

/// Fills in the password of a connection shown to the user (`ProjectDto::database`, `lampo agent
/// context`): the project account's, or root's when one is stored.
pub trait ConnectionPasswords: Send + Sync {
    fn reveal(&self, connection: &mut DatabaseConnection);
}

/// Shows no password (tests, views that never display one).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoConnectionPasswords;

impl ConnectionPasswords for NoConnectionPasswords {
    fn reveal(&self, _connection: &mut DatabaseConnection) {}
}

pub struct MysqlPasswordsDeps {
    pub root: Arc<dyn MysqlRootPasswordReader>,
    pub root_writer: Arc<dyn MysqlRootPasswordWriter>,
    pub projects: Arc<dyn MysqlProjectPasswordReader>,
    pub project_writer: Arc<dyn MysqlProjectPasswordWriter>,
    pub generator: Arc<dyn MysqlPasswordGenerator>,
}

/// The stored passwords of root and of the project accounts, each generated the first time it is
/// needed. Works without the app (the CLI shows connection details too).
pub struct MysqlPasswords {
    deps: MysqlPasswordsDeps,
}

impl MysqlPasswords {
    pub fn new(deps: MysqlPasswordsDeps) -> Self {
        Self { deps }
    }

    /// Over one `store` keeping both kinds of password (`DpapiMysqlPasswords` in production).
    pub fn with_store<S>(store: Arc<S>, generator: Arc<dyn MysqlPasswordGenerator>) -> Self
    where
        S: MysqlRootPasswordReader + MysqlRootPasswordWriter + MysqlProjectPasswordReader + MysqlProjectPasswordWriter + 'static,
    {
        Self::new(MysqlPasswordsDeps { root: store.clone(), root_writer: store.clone(), projects: store.clone(), project_writer: store, generator })
    }

    /// The password of `project`'s dedicated account, generated and stored the first time (two
    /// first uses racing agree on one).
    pub fn project_password(&self, project: &ProjectId) -> AppResult<MysqlPassword> {
        if let Some(stored) = self.deps.projects.project_password(project)? {
            return Ok(stored);
        }
        let candidate = self.deps.generator.generate()?;
        Ok(self.deps.project_writer.save_project_password_if_absent(project, &candidate)?)
    }

    /// Root's stored password, or a new one stored now (before the server gets it).
    fn root_password_or_new(&self, instance: &ServiceId) -> AppResult<MysqlPassword> {
        if let Some(stored) = self.deps.root.root_password(instance)? {
            return Ok(stored);
        }
        let password = self.deps.generator.generate()?;
        self.deps.root_writer.save_root_password(instance, &password)?;
        Ok(password)
    }
}

/// A password that cannot be read is left empty (logged): the listing still works.
impl ConnectionPasswords for MysqlPasswords {
    fn reveal(&self, connection: &mut DatabaseConnection) {
        let password = match &connection.project_account {
            Some(project) => self.project_password(project).map(Some),
            None => self.deps.root.root_password(&connection.series.service_id()).map_err(Into::into),
        };
        match password {
            Ok(password) => connection.password = password.map(|p| p.expose().to_string()).unwrap_or_default(),
            Err(e) => tracing::warn!(user = %connection.user, "the MySQL password to show cannot be read: {}", e.message),
        }
    }
}

/// What [`MysqlAccounts::provision`] did, for the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Provisioned {
    /// Root got its password now (the migration of an instance created without one).
    pub root_secured: bool,
    /// Root was switched to `mysql_native_password` now, for a PHP before 7.4 using it.
    pub root_on_native_password: bool,
    pub project_accounts: usize,
}

/// Provisioning of the accounts on a running instance.
pub struct MysqlAccounts {
    admin: Arc<dyn MysqlAccountAdmin>,
    passwords: Arc<MysqlPasswords>,
}

impl MysqlAccounts {
    pub fn new(admin: Arc<dyn MysqlAccountAdmin>, passwords: Arc<MysqlPasswords>) -> Self {
        Self { admin, passwords }
    }

    /// Brings the instance at `endpoint` (running) to `plan`: root's password first when the plan
    /// allows it (the project accounts are then managed as root with it), else root on
    /// `mysql_native_password` when the plan needs it, then the project accounts.
    pub async fn provision(&self, endpoint: &MysqlEndpoint, plan: &InstanceAccounts) -> AppResult<Provisioned> {
        let mut done = Provisioned { project_accounts: plan.accounts.len(), ..Default::default() };
        if plan.secure_root {
            let password = self.passwords.root_password_or_new(&endpoint.series.service_id())?;
            done.root_secured = self.admin.secure_root(endpoint, &password).await? == RootPasswordChange::Applied;
        }
        if plan.native_root {
            done.root_on_native_password = self.admin.use_native_password_for_root(endpoint).await?;
        }
        let grants = plan
            .accounts
            .iter()
            .map(|a| Ok(ProjectAccountGrant { user: a.user.clone(), password: self.passwords.project_password(&a.project)?, databases: a.databases.clone(), auth: a.auth }))
            .collect::<AppResult<Vec<_>>>()?;
        self.admin.sync_project_accounts(endpoint, &grants).await?;
        Ok(done)
    }
}
