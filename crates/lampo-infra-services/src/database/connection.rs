//! The command lines of the MySQL clients for one endpoint, authenticated with the instance's
//! stored root password when there is one: its option file lives as long as the [`Clients`], so
//! keep them until every command built from them has finished.

use std::path::PathBuf;

use lampo_domain::mysql::DatabaseName;
use lampo_domain::mysql::credentials::ROOT_ACCOUNT;
use lampo_domain::ports::{CommandSpec, MysqlEndpoint, MysqlRootPasswordReader};
use lampo_domain::{PortError, PortResult};

use crate::client_option_file::ClientOptionFile;
use crate::mysql::client_args;

/// What `mysqldump` adds to export a database whole and consistent (InnoDB snapshot, stored
/// programs, binary columns as hex, no GTID statements a restore would choke on).
const DUMP_OPTIONS: [&str; 6] = ["--single-transaction", "--routines", "--triggers", "--events", "--hex-blob", "--set-gtid-purged=OFF"];
/// The largest packet a restore accepts (the server caps it at its own `max_allowed_packet`).
const RESTORE_MAX_ALLOWED_PACKET: &str = "--max-allowed-packet=1073741824";

pub(super) struct Clients<'a> {
    pub(super) endpoint: &'a MysqlEndpoint,
    option_file: Option<ClientOptionFile>,
}

impl<'a> Clients<'a> {
    /// The stored root password applies to the root account only.
    pub(super) fn of(endpoint: &'a MysqlEndpoint, passwords: &dyn MysqlRootPasswordReader) -> PortResult<Self> {
        let option_file = if endpoint.user == ROOT_ACCOUNT { ClientOptionFile::for_instance(passwords, &endpoint.series.service_id())? } else { None };
        Ok(Self { endpoint, option_file })
    }

    fn program(&self, exe: &str) -> PortResult<PathBuf> {
        let path = self.endpoint.client_dir.join(exe);
        if path.is_file() { Ok(path) } else { Err(PortError::not_found("executable", path.display().to_string())) }
    }

    fn base_args(&self) -> Vec<String> {
        let mut args = client_args(self.endpoint.port, &self.endpoint.user, self.option_file.as_ref());
        args.push("--default-character-set=utf8mb4".into());
        args
    }

    pub(super) fn mysql_spec(&self, extra: &[String]) -> PortResult<CommandSpec> {
        let mut args = self.base_args();
        args.extend_from_slice(extra);
        Ok(CommandSpec { program: self.program("mysql.exe")?, args, ..Default::default() })
    }

    pub(super) fn dump_spec(&self, name: &DatabaseName) -> PortResult<CommandSpec> {
        let mut args = self.base_args();
        args.extend(DUMP_OPTIONS.map(String::from));
        args.push(name.as_str().to_string());
        Ok(CommandSpec { program: self.program("mysqldump.exe")?, args, ..Default::default() })
    }

    pub(super) fn restore_spec(&self, name: &DatabaseName) -> PortResult<CommandSpec> {
        self.mysql_spec(&[RESTORE_MAX_ALLOWED_PACKET.into(), format!("--database={}", name.as_str())])
    }
}
