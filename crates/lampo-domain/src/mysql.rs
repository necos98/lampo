//! MySQL value objects. One MySQL instance runs per series and is shared by every project using it.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::tcp_port::Port;
use crate::version::{PackageVersion, VersionSeries};

pub mod credentials;

/// A MySQL `<major>.<minor>` series (`8.4`, `5.7`). Each series has its own data directory and port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct MysqlSeries {
    major: u8,
    minor: u8,
}

impl MysqlSeries {
    /// Default instance: always running while the app runs, port 3306.
    pub const DEFAULT: MysqlSeries = MysqlSeries { major: 8, minor: 4 };
    /// Legacy instance: started on demand, port 3307.
    pub const LEGACY_5_7: MysqlSeries = MysqlSeries { major: 5, minor: 7 };

    pub fn new(major: u8, minor: u8) -> Result<Self, DomainError> {
        let ok = match major {
            5 => minor == 7,
            8.. => minor <= 99,
            _ => false,
        };
        if ok { Ok(Self { major, minor }) } else { Err(DomainError::InvalidMysqlSeries(format!("{major}.{minor}"))) }
    }

    pub fn parse(input: &str) -> Result<Self, DomainError> {
        let err = || DomainError::InvalidMysqlSeries(input.to_string());
        let (a, b) = input.trim().split_once('.').ok_or_else(err)?;
        let major = a.parse::<u8>().map_err(|_| err())?;
        let minor = b.parse::<u8>().map_err(|_| err())?;
        Self::new(major, minor).map_err(|_| err())
    }

    pub fn major(self) -> u8 {
        self.major
    }

    pub fn minor(self) -> u8 {
        self.minor
    }

    /// The MySQL service instance of this series (`mysql-8.4`), which databases live on.
    pub fn service_id(self) -> crate::service::ServiceId {
        crate::service::ServiceId::new(crate::service::ServiceKind::MYSQL, Some(self.as_series()))
    }

    pub fn as_series(self) -> VersionSeries {
        VersionSeries { major: self.major as u32, minor: self.minor as u32 }
    }

    pub fn contains(self, version: &PackageVersion) -> bool {
        version.series() == self.as_series()
    }

    /// MySQL 5.7 does not support some options used by the low-RAM config of 8.x
    /// (e.g. `mysqlx=OFF` is 8.0+). The config renderer branches on this.
    pub fn is_legacy(self) -> bool {
        self.major < 8
    }
}

impl fmt::Display for MysqlSeries {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

impl FromStr for MysqlSeries {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl TryFrom<String> for MysqlSeries {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(&s)
    }
}

impl TryFrom<VersionSeries> for MysqlSeries {
    type Error = DomainError;
    fn try_from(v: VersionSeries) -> Result<Self, Self::Error> {
        let bad = || DomainError::InvalidMysqlSeries(v.to_string());
        Self::new(u8::try_from(v.major).map_err(|_| bad())?, u8::try_from(v.minor).map_err(|_| bad())?)
    }
}

impl From<MysqlSeries> for String {
    fn from(v: MysqlSeries) -> Self {
        v.to_string()
    }
}

/// Schemas MySQL keeps for itself on every instance. An instance is shared by all the projects
/// of its series, so none of them may be a project's database or the target of an operation.
pub const SYSTEM_SCHEMAS: [&str; 4] = ["information_schema", "mysql", "performance_schema", "sys"];

/// Whether `name` is one of the [`SYSTEM_SCHEMAS`]. Case-insensitive, like MySQL on Windows.
pub fn is_system_schema(name: &str) -> bool {
    let name = name.trim();
    SYSTEM_SCHEMAS.iter().any(|schema| schema.eq_ignore_ascii_case(name))
}

/// A MySQL schema name of a project: never one of the [`SYSTEM_SCHEMAS`], so creating, dropping,
/// importing into or cloning a `DatabaseName` cannot damage the shared instance.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DatabaseName(String);

impl DatabaseName {
    pub fn new(name: impl AsRef<str>) -> Result<Self, DomainError> {
        let n = name.as_ref().trim();
        let well_formed = !n.is_empty() && n.len() <= 64 && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
        if !well_formed {
            Err(DomainError::InvalidDatabaseName(name.as_ref().to_string()))
        } else if is_system_schema(n) {
            Err(DomainError::ReservedDatabaseName(n.to_string()))
        } else {
            Ok(Self(n.to_string()))
        }
    }

    /// Derives a schema name from a project slug (`my-shop` → `my_shop`).
    pub fn from_slug(slug: &str) -> Result<Self, DomainError> {
        Self::new(slug.replace('-', "_"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DatabaseName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for DatabaseName {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<DatabaseName> for String {
    fn from(v: DatabaseName) -> Self {
        v.0
    }
}

/// Connection details shown to the user for a project. Lampo never writes them to `.env`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseConnection {
    pub series: MysqlSeries,
    pub host: String,
    pub port: Port,
    pub user: String,
    pub password: String,
    pub database: DatabaseName,
}

impl DatabaseConnection {
    /// Instances are initialised with `--initialize-insecure`: user `root`, empty password.
    pub fn local(series: MysqlSeries, port: Port, database: DatabaseName) -> Self {
        Self { series, host: "127.0.0.1".into(), port, user: credentials::ROOT_ACCOUNT.into(), password: String::new(), database }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_series() {
        assert_eq!(MysqlSeries::parse("8.4").unwrap(), MysqlSeries::DEFAULT);
        assert_eq!(MysqlSeries::parse("5.7").unwrap(), MysqlSeries::LEGACY_5_7);
        assert!(MysqlSeries::parse("8.0").is_ok());
        assert!(MysqlSeries::parse("5.6").is_err());
        assert!(MysqlSeries::parse("8").is_err());
        assert!(MysqlSeries::LEGACY_5_7.is_legacy());
    }

    #[test]
    fn database_names() {
        assert_eq!(DatabaseName::from_slug("my-shop").unwrap().as_str(), "my_shop");
        assert!(DatabaseName::new("drop table;").is_err());
        assert!(DatabaseName::new("").is_err());
    }

    /// Dropping or importing into a system schema breaks the instance shared by every project.
    #[test]
    fn system_schemas_are_refused_whatever_their_case() {
        for name in ["information_schema", "mysql", "performance_schema", "sys", "MySQL", "SYS", " Performance_Schema "] {
            let err = DatabaseName::new(name).unwrap_err();
            assert_eq!(err, DomainError::ReservedDatabaseName(name.trim().to_string()), "{name}");
            assert!(err.to_string().contains("system schema"), "{err}");
        }
        assert!(DatabaseName::from_slug("sys").is_err());
        assert!(serde_json::from_str::<DatabaseName>(r#""mysql""#).is_err());
        for name in ["mysql_app", "my_sys", "sys1", "shop"] {
            assert!(DatabaseName::new(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn recognises_system_schemas() {
        assert!(is_system_schema("Information_Schema"));
        assert!(!is_system_schema("shop"));
    }
}
