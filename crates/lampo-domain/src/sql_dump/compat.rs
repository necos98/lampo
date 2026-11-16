//! Importing a dump made by another server (MySQL 8.0, MariaDB) into a newer MySQL: what has to
//! change in the importing session and in the stream for it to load.
//!
//! - MySQL 8.4 refuses foreign keys on non-unique or partial keys
//!   (`restrict_fk_on_non_standard_key`, `ON` by default), which 8.0 and MariaDB accept.
//! - MariaDB's default `utf8mb4` collation is `utf8mb4_general_ci`, MySQL 8's is
//!   `utf8mb4_0900_ai_ci`: tables declared with `DEFAULT CHARSET=utf8mb4` and no collation would
//!   change collation and views comparing them with string literals would mix collations.
//! - Views, triggers and routines name a `DEFINER` account that rarely exists on this machine.
//! - Recent MariaDB dumps start with a sandbox-mode comment MySQL's client rejects.

use std::fmt;

use super::rewrite::{DumpRewrite, MARIADB_SANDBOX_LINE};
use crate::mysql::MysqlSeries;

/// Header lines of a dump inspected to recognise the server that made it.
const HEADER_LINES: usize = 40;
/// The `utf8mb4` collation MariaDB (and MySQL 5.7) use by default.
const MARIADB_UTF8MB4_COLLATION: &str = "utf8mb4_general_ci";
/// First series with `restrict_fk_on_non_standard_key`.
const RESTRICT_FK_SINCE: (u8, u8) = (8, 4);
/// First series with `default_collation_for_utf8mb4`.
const DEFAULT_COLLATION_VARIABLE_SINCE: (u8, u8) = (8, 0);

/// The server a dump comes from, for the fixes [`DumpCompat`] applies while importing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DumpCompat {
    /// MySQL 8.0: non-standard foreign keys, `DEFINER` clauses.
    Mysql80,
    /// MariaDB: as MySQL 8.0, plus the sandbox line and `utf8mb4_general_ci` as the default.
    Mariadb,
}

/// A session system variable set before the dump runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionSetting {
    pub variable: &'static str,
    pub value: &'static str,
}

impl DumpCompat {
    /// The option value (`--compat mysql80`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mysql80 => "mysql80",
            Self::Mariadb => "mariadb",
        }
    }

    /// The rewrites applied to the stream.
    pub fn rewrite(self) -> DumpRewrite {
        DumpRewrite { drop_sandbox_line: self == Self::Mariadb, strip_definers: true }
    }

    /// Session variables for importing into `target`, in the order they must be set (only those
    /// `target` knows: setting an unknown variable fails the connection).
    pub fn session_settings(self, target: MysqlSeries) -> Vec<SessionSetting> {
        let series = (target.major(), target.minor());
        let mut settings = Vec::new();
        if series >= RESTRICT_FK_SINCE {
            settings.push(SessionSetting { variable: "restrict_fk_on_non_standard_key", value: "OFF" });
        }
        if self == Self::Mariadb && series >= DEFAULT_COLLATION_VARIABLE_SINCE {
            // The default collation of `CHARSET utf8mb4` without `COLLATE` (tables, `SET NAMES`),
            // then the connection's own collation, which the client negotiated before.
            settings.push(SessionSetting { variable: "default_collation_for_utf8mb4", value: MARIADB_UTF8MB4_COLLATION });
            settings.push(SessionSetting { variable: "collation_connection", value: MARIADB_UTF8MB4_COLLATION });
        }
        settings
    }

    /// One `SET SESSION …` statement with every [`Self::session_settings`], `None` if there are
    /// none.
    pub fn init_statement(self, target: MysqlSeries) -> Option<String> {
        let settings = self.session_settings(target);
        (!settings.is_empty()).then(|| {
            let assignments: Vec<String> = settings.iter().map(|s| format!("SESSION {}={}", s.variable, s.value)).collect();
            format!("SET {}", assignments.join(", "))
        })
    }

    /// The collation of the target database when the import creates it.
    pub fn database_collation(self) -> Option<&'static str> {
        (self == Self::Mariadb).then_some(MARIADB_UTF8MB4_COLLATION)
    }
}

impl fmt::Display for DumpCompat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A [`DumpCompat`] the header of a dump calls for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatSuggestion {
    pub compat: DumpCompat,
    /// The `-- Server version` of the header, when it has one.
    pub server_version: Option<String>,
}

/// The [`DumpCompat`] to suggest for a dump whose (decompressed) start is `head`, imported into
/// `target`: MariaDB dumps always, MySQL 8.0 dumps into 8.4 or later. Only the header comments
/// mysqldump / mariadb-dump write are read; the dump is never assumed to be anything else.
pub fn suggest_compat(head: &[u8], target: MysqlSeries) -> Option<CompatSuggestion> {
    let text = String::from_utf8_lossy(head);
    let header: Vec<&str> = text.lines().take(HEADER_LINES).collect();
    let server_version = header.iter().find_map(|line| line.strip_prefix("-- Server version")).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let mariadb = head.starts_with(MARIADB_SANDBOX_LINE)
        || header.iter().any(|line| line.starts_with("-- MariaDB dump"))
        || server_version.as_deref().is_some_and(|v| v.to_ascii_lowercase().contains("mariadb"));
    let compat = if mariadb {
        DumpCompat::Mariadb
    } else if server_version.as_deref().is_some_and(is_mysql_8_0) && (target.major(), target.minor()) >= RESTRICT_FK_SINCE {
        DumpCompat::Mysql80
    } else {
        return None;
    };
    Some(CompatSuggestion { compat, server_version })
}

fn is_mysql_8_0(version: &str) -> bool {
    version.strip_prefix("8.0").is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', '-', ' ']))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(s: &str) -> MysqlSeries {
        MysqlSeries::parse(s).unwrap()
    }

    #[test]
    fn session_settings_depend_on_the_dump_and_the_target_series() {
        assert_eq!(DumpCompat::Mysql80.init_statement(series("8.4")).as_deref(), Some("SET SESSION restrict_fk_on_non_standard_key=OFF"));
        assert_eq!(
            DumpCompat::Mariadb.init_statement(series("8.4")).as_deref(),
            Some("SET SESSION restrict_fk_on_non_standard_key=OFF, SESSION default_collation_for_utf8mb4=utf8mb4_general_ci, SESSION collation_connection=utf8mb4_general_ci")
        );
        assert_eq!(
            DumpCompat::Mariadb.init_statement(series("8.0")).as_deref(),
            Some("SET SESSION default_collation_for_utf8mb4=utf8mb4_general_ci, SESSION collation_connection=utf8mb4_general_ci"),
            "8.0 has no restrict_fk_on_non_standard_key"
        );
        assert_eq!(DumpCompat::Mysql80.init_statement(series("8.0")), None);
        assert_eq!(DumpCompat::Mariadb.init_statement(series("5.7")), None, "5.7 knows neither variable");
        assert_eq!(DumpCompat::Mysql80.init_statement(series("9.1")).as_deref(), Some("SET SESSION restrict_fk_on_non_standard_key=OFF"));
    }

    #[test]
    fn rewrites_and_database_collation() {
        assert_eq!(DumpCompat::Mariadb.rewrite(), DumpRewrite { drop_sandbox_line: true, strip_definers: true });
        assert_eq!(DumpCompat::Mysql80.rewrite(), DumpRewrite { drop_sandbox_line: false, strip_definers: true });
        assert_eq!(DumpCompat::Mariadb.database_collation(), Some("utf8mb4_general_ci"));
        assert_eq!(DumpCompat::Mysql80.database_collation(), None);
    }

    #[test]
    fn mariadb_dumps_are_recognised_from_their_header() {
        let sandbox = b"/*M!999999\\- enable the sandbox mode */\n-- MariaDB dump 10.19-11.4.2-MariaDB, for Win64 (AMD64)\n--\n-- Host: localhost    Database: shop\n-- Server version\t11.4.2-MariaDB\n";
        let found = suggest_compat(sandbox, series("8.4")).unwrap();
        assert_eq!((found.compat, found.server_version.as_deref()), (DumpCompat::Mariadb, Some("11.4.2-MariaDB")));
        // mysqldump of MySQL run against a MariaDB server.
        let mysqldump = b"-- MySQL dump 10.13  Distrib 8.0.36, for Win64 (x86_64)\n--\n-- Server version\t10.6.12-MariaDB-log\n";
        assert_eq!(suggest_compat(mysqldump, series("5.7")).map(|s| s.compat), Some(DumpCompat::Mariadb));
        assert_eq!(suggest_compat(b"-- MariaDB dump 10.19\n", series("8.4")).map(|s| (s.compat, s.server_version)), Some((DumpCompat::Mariadb, None)));
    }

    #[test]
    fn mysql_8_0_dumps_are_suggested_only_for_newer_targets() {
        let dump = b"-- MySQL dump 10.13  Distrib 8.0.36, for Win64 (x86_64)\n--\n-- Host: 127.0.0.1    Database: shop\n-- ------------------------------------------------------\n-- Server version\t8.0.36\r\n";
        assert_eq!(suggest_compat(dump, series("8.4")).map(|s| (s.compat, s.server_version)), Some((DumpCompat::Mysql80, Some("8.0.36".into()))));
        assert_eq!(suggest_compat(dump, series("8.0")), None);
        for other in ["8.4.3", "5.7.44-log", "8.01", ""] {
            let dump = format!("-- MySQL dump 10.13\n-- Server version\t{other}\n");
            assert_eq!(suggest_compat(dump.as_bytes(), series("8.4")), None, "{other}");
        }
        let late = format!("{}-- Server version\t8.0.36\n", "-- filler\n".repeat(HEADER_LINES));
        assert_eq!(suggest_compat(late.as_bytes(), series("8.4")), None, "only the header counts");
        assert_eq!(suggest_compat(b"INSERT INTO t VALUES ('-- MariaDB dump');\n", series("8.4")), None);
    }
}
