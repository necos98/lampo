//! The `Project` aggregate and its value objects.
//!
//! A project's configuration lives in the app (one TOML per project under `%APPDATA%\Lampo`),
//! never in the project's repository. Its identity on disk is its folder ([`ProjectRoot`]); the
//! stable [`ProjectId`] survives a *relink* to a moved folder.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use crate::apache_directives::ApacheDirectives;
use crate::address::{SiteAddress, validate_addresses};
use crate::error::DomainError;
use crate::mysql::{DatabaseConnection, DatabaseName, MysqlSeries};
use crate::package::PackageKind;
use crate::php::{ExtensionName, PhpIniOverrides, PhpMinor};
use crate::project_file::ProjectFile;
use crate::service::{ServiceId, ServiceKind};
use crate::settings::Settings;
use crate::version::VersionSeries;
use crate::worktree::WorktreeEnvironment;

pub use crate::slug::slugify;
use crate::slug::{MAX_SLUG_LEN, validate_slug};

/// Stable identifier of a project: a slug, also used as config file name and default hostname.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProjectId(String);

impl ProjectId {
    pub fn new(id: impl AsRef<str>) -> Result<Self, DomainError> {
        validate_slug(id.as_ref()).map(Self)
    }

    /// Derives an id from a display name; the caller resolves collisions (e.g. `shop-2`).
    pub fn from_name(name: &str) -> Self {
        Self(slugify(name, "project"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ProjectId {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<ProjectId> for String {
    fn from(v: ProjectId) -> Self {
        v.0
    }
}

/// Identifier of a preset (built-in or custom).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PresetId(String);

impl PresetId {
    pub fn new(id: impl AsRef<str>) -> Result<Self, DomainError> {
        validate_slug(id.as_ref()).map(Self)
    }

    pub fn from_name(name: &str) -> Self {
        Self(slugify(name, "preset"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PresetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for PresetId {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<PresetId> for String {
    fn from(v: PresetId) -> Self {
        v.0
    }
}

// ---------------------------------------------------------------------------------------------
// Value objects
// ---------------------------------------------------------------------------------------------

/// Human-readable project or preset name (1–80 chars, single line).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DisplayName(String);

impl DisplayName {
    pub fn new(name: impl AsRef<str>) -> Result<Self, DomainError> {
        let n = name.as_ref().trim();
        if n.is_empty() || n.chars().count() > 80 || n.contains(['\n', '\r', '\0']) {
            return Err(DomainError::InvalidName(format!("`{}` must be 1-80 characters on one line", name.as_ref())));
        }
        Ok(Self(n.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DisplayName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for DisplayName {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<DisplayName> for String {
    fn from(v: DisplayName) -> Self {
        v.0
    }
}

/// Absolute Windows folder of a project, normalised: backslashes, upper-case drive letter,
/// no trailing separator, no `.`/`..` segments. Comparisons are case-insensitive (NTFS semantics).
#[derive(Debug, Clone, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProjectRoot(String);

impl ProjectRoot {
    pub fn new(path: impl AsRef<str>) -> Result<Self, DomainError> {
        let raw = path.as_ref().trim();
        let err = || DomainError::InvalidProjectRoot(raw.to_string());
        let p = raw.replace('/', "\\");
        let bytes = p.as_bytes();
        let (prefix, rest) = if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\' {
            (format!("{}:", (bytes[0] as char).to_ascii_uppercase()), &p[3..])
        } else if let Some(unc) = p.strip_prefix("\\\\") {
            // \\server\share\...
            let mut parts = unc.splitn(3, '\\');
            let (Some(server), Some(share)) = (parts.next(), parts.next()) else { return Err(err()) };
            if server.is_empty() || share.is_empty() || server == "?" || server == "." {
                return Err(err());
            }
            (format!("\\\\{server}\\{share}"), parts.next().unwrap_or(""))
        } else {
            return Err(err());
        };
        let mut segments = Vec::new();
        for seg in rest.split('\\').filter(|s| !s.is_empty()) {
            if seg == "." || seg == ".." || seg.contains(['<', '>', ':', '"', '|', '?', '*', '\0']) {
                return Err(err());
            }
            segments.push(seg);
        }
        let normalised = if segments.is_empty() { format!("{prefix}\\") } else { format!("{prefix}\\{}", segments.join("\\")) };
        Ok(Self(normalised))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }

    fn key(&self) -> String {
        self.0.to_lowercase()
    }

    /// Whether `path` (absolute, any separator/case) is this folder or inside it.
    pub fn contains(&self, path: &str) -> bool {
        let Ok(other) = ProjectRoot::new(path) else { return false };
        let (me, other) = (self.key(), other.key());
        if other == me {
            return true;
        }
        let base = if me.ends_with('\\') { me } else { format!("{me}\\") };
        other.starts_with(&base)
    }

    /// Number of path segments, used for longest-prefix matching.
    pub fn depth(&self) -> usize {
        self.0.split('\\').filter(|s| !s.is_empty()).count()
    }
}

impl PartialEq for ProjectRoot {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl std::hash::Hash for ProjectRoot {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}

impl fmt::Display for ProjectRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ProjectRoot {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<ProjectRoot> for String {
    fn from(v: ProjectRoot) -> Self {
        v.0
    }
}

/// Document root relative to the project folder (`public`, `web`, or empty for the folder itself).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PublicDir(String);

impl PublicDir {
    pub fn new(dir: impl AsRef<str>) -> Result<Self, DomainError> {
        let raw = dir.as_ref().trim();
        let err = || DomainError::InvalidPublicDir(raw.to_string());
        let unified = raw.replace('\\', "/");
        if unified.starts_with('/') || unified.contains(':') {
            return Err(err());
        }
        let mut segments = Vec::new();
        for seg in unified.split('/').filter(|s| !s.is_empty() && *s != ".") {
            if seg == ".." || seg.contains(['<', '>', '"', '|', '?', '*', '\0']) {
                return Err(err());
            }
            segments.push(seg);
        }
        Ok(Self(segments.join("/")))
    }

    /// The project folder itself.
    pub fn root() -> Self {
        Self(String::new())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }
}

impl TryFrom<String> for PublicDir {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<PublicDir> for String {
    fn from(v: PublicDir) -> Self {
        v.0
    }
}

/// HTTPS for a project. Only `Disabled` is implemented today; `LocalCa` is reserved for the
/// planned local-CA feature so stored configs and the GUI already have a place for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TlsMode {
    #[default]
    Disabled,
    /// Certificate issued by Lampo's local CA (not implemented yet).
    LocalCa,
}

/// How a project uses one kind of service: the version series it needs, for kinds whose
/// instances run per series (MySQL); `None` otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ServiceUse {
    #[serde(default, with = "crate::version::series_text", skip_serializing_if = "Option::is_none")]
    pub version: Option<VersionSeries>,
}

/// Everything a preset carries and a project copies at creation (no live link afterwards).
///
/// Services are a map kind → use (`[environment.services.mysql] version = "8.4"`,
/// `[environment.services.redis]`). Files written before that (`mysql = "8.4"`, `redis = true`)
/// are migrated on read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "EnvironmentFile")]
pub struct ProjectEnvironment {
    pub php: PhpMinor,
    /// Bundled extensions to enable (`intl`, `pdo_mysql`, …).
    pub extensions: BTreeSet<ExtensionName>,
    /// Add-on extensions downloaded from the catalog and pinned per PHP minor
    /// (`PackageKind::XDEBUG`, `PackageKind::PHP_REDIS`).
    pub addons: BTreeSet<PackageKind>,
    pub php_ini: PhpIniOverrides,
    pub public_dir: PublicDir,
    /// The services the project's sites need while served (MySQL `8.4`, Redis, …).
    pub services: BTreeMap<ServiceKind, ServiceUse>,
    pub apache_directives: ApacheDirectives,
}

/// The file shape: today's fields plus the legacy `mysql`/`redis` ones.
#[derive(Deserialize)]
struct EnvironmentFile {
    php: PhpMinor,
    #[serde(default)]
    extensions: BTreeSet<ExtensionName>,
    #[serde(default)]
    addons: BTreeSet<PackageKind>,
    #[serde(default)]
    php_ini: PhpIniOverrides,
    #[serde(default)]
    public_dir: PublicDir,
    #[serde(default)]
    services: BTreeMap<ServiceKind, ServiceUse>,
    #[serde(default)]
    apache_directives: ApacheDirectives,
    #[serde(default)]
    mysql: Option<MysqlSeries>,
    #[serde(default)]
    redis: bool,
}

impl From<EnvironmentFile> for ProjectEnvironment {
    fn from(f: EnvironmentFile) -> Self {
        let mut env =
            Self { php: f.php, extensions: f.extensions, addons: f.addons, php_ini: f.php_ini, public_dir: f.public_dir, services: f.services, apache_directives: f.apache_directives };
        if let Some(series) = f.mysql {
            env.set_mysql(Some(series));
        }
        if f.redis {
            env.use_service(ServiceKind::REDIS, None);
        }
        env
    }
}

impl ProjectEnvironment {
    /// A bare environment: given PHP, no extensions, no services, document root = project folder.
    pub fn minimal(php: PhpMinor) -> Self {
        Self {
            php,
            extensions: BTreeSet::new(),
            addons: BTreeSet::new(),
            php_ini: PhpIniOverrides::new(),
            public_dir: PublicDir::root(),
            services: BTreeMap::new(),
            apache_directives: ApacheDirectives::default(),
        }
    }

    /// The service instances the project needs (`mysql-8.4`, `redis`).
    pub fn service_ids(&self) -> impl Iterator<Item = ServiceId> + '_ {
        self.services.iter().map(|(kind, u)| ServiceId::new(kind.clone(), u.version))
    }

    pub fn uses(&self, kind: &ServiceKind) -> bool {
        self.services.contains_key(kind)
    }

    pub fn use_service(&mut self, kind: ServiceKind, version: Option<VersionSeries>) {
        self.services.insert(kind, ServiceUse { version });
    }

    pub fn drop_service(&mut self, kind: &ServiceKind) {
        self.services.remove(kind);
    }

    /// MySQL series used by the project (its database lives there), `None` without MySQL.
    pub fn mysql(&self) -> Option<MysqlSeries> {
        self.services.get(&ServiceKind::MYSQL).and_then(|u| u.version).and_then(|v| MysqlSeries::try_from(v).ok())
    }

    pub fn set_mysql(&mut self, series: Option<MysqlSeries>) {
        match series {
            Some(s) => self.use_service(ServiceKind::MYSQL, Some(s.as_series())),
            None => self.drop_service(&ServiceKind::MYSQL),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Aggregate
// ---------------------------------------------------------------------------------------------

/// A local PHP site managed by Lampo.
///
/// `addresses` and `database` are private: every way in ([`Project::new`], the setters and
/// deserialization, which goes through [`ProjectFile`]) keeps the address list valid and the
/// database tied to the MySQL choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ProjectFile")]
pub struct Project {
    pub id: ProjectId,
    pub name: DisplayName,
    pub root: ProjectRoot,
    addresses: Vec<SiteAddress>,
    pub environment: ProjectEnvironment,
    database: Option<DatabaseName>,
    pub tls: TlsMode,
    /// Inactive projects keep their config but get no vhost and require no service.
    pub active: bool,
    /// Preset the environment was copied from (informational only, no live link).
    pub origin_preset: Option<PresetId>,
}

impl Project {
    /// Creates an inactive project. The database name defaults to the slug when MySQL is used.
    pub fn new(id: ProjectId, name: DisplayName, root: ProjectRoot, addresses: Vec<SiteAddress>, environment: ProjectEnvironment) -> Result<Self, DomainError> {
        validate_addresses(&addresses)?;
        let database = match environment.mysql() {
            Some(_) => Some(DatabaseName::from_slug(id.as_str())?),
            None => None,
        };
        Ok(Self { id, name, root, addresses, environment, database, tls: TlsMode::Disabled, active: false, origin_preset: None })
    }

    pub fn set_addresses(&mut self, addresses: Vec<SiteAddress>) -> Result<(), DomainError> {
        validate_addresses(&addresses)?;
        self.addresses = addresses;
        Ok(())
    }

    /// How the site is reached, never empty; the first one is the primary (shown/opened by
    /// default).
    pub fn addresses(&self) -> &[SiteAddress] {
        &self.addresses
    }

    /// The primary address.
    pub fn primary_address(&self) -> &SiteAddress {
        // The list is private and every way in validates it (`validate_addresses`): never empty.
        self.addresses.first().expect("a project always has at least one address")
    }

    /// Names that need a hosts-file entry.
    pub fn hosts_file_names(&self) -> impl Iterator<Item = &crate::hostname::Hostname> {
        self.addresses.iter().filter(|a| a.needs_hosts_entry()).filter_map(|a| a.host())
    }

    /// Absolute document root (project folder + public dir).
    pub fn document_root(&self) -> PathBuf {
        let mut p = PathBuf::from(self.root.as_str());
        for seg in self.environment.public_dir.as_str().split('/').filter(|s| !s.is_empty()) {
            p.push(seg);
        }
        p
    }

    pub fn set_active(&mut self, active: bool) {
        self.active = active;
    }

    /// Points the project at a moved folder, keeping id and configuration.
    pub fn relink(&mut self, new_root: ProjectRoot) {
        self.root = new_root;
    }

    /// Replaces the environment, keeping the database name consistent with the MySQL choice.
    pub fn set_environment(&mut self, environment: ProjectEnvironment) -> Result<(), DomainError> {
        self.database = match (environment.mysql(), self.database.take()) {
            (Some(_), Some(db)) => Some(db),
            (Some(_), None) => Some(DatabaseName::from_slug(self.id.as_str())?),
            (None, _) => None,
        };
        self.environment = environment;
        Ok(())
    }

    /// Schema shown in the connection details; `None` when the project has no MySQL.
    pub fn database(&self) -> Option<&DatabaseName> {
        self.database.as_ref()
    }

    /// Names the project's schema. Ignored without MySQL: such a project has no database.
    pub fn set_database_name(&mut self, name: DatabaseName) {
        if self.environment.mysql().is_some() {
            self.database = Some(name);
        }
    }

    /// Connection details for the GUI, if the project uses MySQL.
    pub fn database_connection(&self, settings: &Settings) -> Result<Option<DatabaseConnection>, DomainError> {
        let (Some(series), Some(db)) = (self.environment.mysql(), self.database.clone()) else { return Ok(None) };
        let port = settings.mysql_port(series).ok_or_else(|| DomainError::UnknownMysqlSeries(series.to_string()))?;
        Ok(Some(DatabaseConnection::local(series, port, db)))
    }

    /// URL of the primary address (`http://shop.test`, `http://127.0.0.1:8101`, …).
    pub fn url(&self, settings: &Settings) -> String {
        self.primary_address().url(settings.apache_port, self.tls)
    }

    /// URLs of every address, primary first.
    pub fn urls(&self, settings: &Settings) -> Vec<String> {
        self.addresses.iter().map(|a| a.url(settings.apache_port, self.tls)).collect()
    }
}

/// Checks that `candidate` does not collide with other projects (id, folder, host names,
/// loopback ports), with worktree environments ([`check_worktree_conflicts`]) nor with the
/// service ports in `settings`.
/// When updating, `existing` may contain the candidate itself (same id): it is skipped,
/// unless `is_new` is true, in which case a same-id project is a conflict.
///
/// Every use case that saves a project's addresses or folder must call this (with every
/// worktree environment): two sites on one loopback port or host name make Apache serve one
/// site's code for the other.
pub fn check_conflicts(existing: &[Project], worktrees: &[WorktreeEnvironment], candidate: &Project, is_new: bool, settings: &Settings) -> Result<(), DomainError> {
    for port in candidate.addresses.iter().filter_map(SiteAddress::loopback) {
        if let Some(service) = settings.service_on_port(port) {
            return Err(DomainError::LoopbackPortTaken { port, service });
        }
    }
    check_worktree_conflicts(worktrees, candidate)?;
    for other in existing {
        if other.id == candidate.id {
            if is_new {
                return Err(DomainError::DuplicateProjectId(candidate.id.to_string()));
            }
            continue;
        }
        if other.root == candidate.root {
            return Err(DomainError::DuplicateProjectRoot { root: candidate.root.to_string(), other: other.id.to_string() });
        }
        for mine in &candidate.addresses {
            for theirs in &other.addresses {
                if let (Some(a), Some(b)) = (mine.host(), theirs.host()) {
                    if a == b {
                        return Err(DomainError::DuplicateHostname { hostname: a.to_string(), other: other.id.to_string() });
                    }
                }
                if let (Some(a), Some(b)) = (mine.loopback(), theirs.loopback()) {
                    if a == b {
                        return Err(DomainError::DuplicateLoopbackPort { port: a, other: other.id.to_string() });
                    }
                }
            }
        }
    }
    Ok(())
}

/// The worktree side of [`check_conflicts`]: every environment owns its folder, a dedicated
/// `127.0.0.1:<port>` listener and its `<slug>.<project>.localhost` alias (a `ServerAlias` of its
/// vhost), none of which a project may take.
fn check_worktree_conflicts(worktrees: &[WorktreeEnvironment], candidate: &Project) -> Result<(), DomainError> {
    for w in worktrees {
        if w.root == candidate.root {
            return Err(DomainError::DuplicateProjectRoot { root: candidate.root.to_string(), other: w.site_id().to_string() });
        }
        if candidate.addresses.iter().filter_map(SiteAddress::loopback).any(|port| port == w.loopback_port) {
            return Err(DomainError::DuplicateLoopbackPort { port: w.loopback_port, other: w.site_id().to_string() });
        }
        let alias = w.localhost_alias();
        if candidate.addresses.iter().filter_map(SiteAddress::host).any(|host| *host == alias) {
            return Err(DomainError::DuplicateHostname { hostname: alias.to_string(), other: w.site_id().to_string() });
        }
    }
    Ok(())
}

/// First free id derived from `base` (`shop`, `shop-2`, `shop-3`, …).
pub fn unique_project_id(existing: &[Project], base: &ProjectId) -> ProjectId {
    let taken = |id: &str| existing.iter().any(|p| p.id.as_str() == id);
    if !taken(base.as_str()) {
        return base.clone();
    }
    (2u32..)
        .map(|n| {
            let mut stem = base.as_str().to_string();
            let suffix = format!("-{n}");
            stem.truncate(MAX_SLUG_LEN - suffix.len());
            ProjectId(format!("{}{suffix}", stem.trim_end_matches('-')))
        })
        .find(|id| !taken(id.as_str()))
        .expect("an unbounded range always yields a free id")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::hostname::Tld;

    pub(crate) fn project(id: &str, root: &str) -> Project {
        let id = ProjectId::new(id).unwrap();
        let addresses = SiteAddress::defaults_for(id.as_str(), &Tld::default()).unwrap();
        let mut env = ProjectEnvironment::minimal(PhpMinor::parse("8.3").unwrap());
        env.set_mysql(Some(MysqlSeries::DEFAULT));
        Project::new(id.clone(), DisplayName::new(id.as_str()).unwrap(), ProjectRoot::new(root).unwrap(), addresses, env).unwrap()
    }

    #[test]
    fn ids_from_names() {
        assert_eq!(ProjectId::from_name("WordPress Blog").as_str(), "wordpress-blog");
        assert_eq!(ProjectId::from_name("Façade Straße").as_str(), "facade-strasse");
        assert!(ProjectId::new("Bad Id").is_err());
        assert!(ProjectId::new("a--b").is_err());
    }

    #[test]
    fn project_roots_are_normalised() {
        assert_eq!(ProjectRoot::new("c:/sites/shop/").unwrap().as_str(), "C:\\sites\\shop");
        assert_eq!(ProjectRoot::new("D:\\").unwrap().as_str(), "D:\\");
        assert_eq!(ProjectRoot::new("\\\\nas\\www\\shop").unwrap().as_str(), "\\\\nas\\www\\shop");
        for bad in ["sites\\shop", "C:", "C:\\a\\..\\b", "/home/x", "\\\\", "C:\\a|b"] {
            assert!(ProjectRoot::new(bad).is_err(), "{bad} should be rejected");
        }
        assert_eq!(ProjectRoot::new("C:\\Sites\\Shop").unwrap(), ProjectRoot::new("c:\\sites\\shop").unwrap());
    }

    #[test]
    fn root_containment_respects_segment_boundaries() {
        let root = ProjectRoot::new("C:\\sites\\shop").unwrap();
        assert!(root.contains("C:\\sites\\shop"));
        assert!(root.contains("c:/Sites/Shop/app/Http"));
        assert!(!root.contains("C:\\sites\\shop2"));
        assert!(!root.contains("C:\\sites"));
        assert!(ProjectRoot::new("D:\\").unwrap().contains("D:\\anything"));
    }

    #[test]
    fn public_dir_rules() {
        assert_eq!(PublicDir::new("public/").unwrap().as_str(), "public");
        assert_eq!(PublicDir::new(".\\web\\public").unwrap().as_str(), "web/public");
        assert!(PublicDir::new("").unwrap().is_root());
        for bad in ["..", "../x", "/abs", "C:\\x", "a/../../b"] {
            assert!(PublicDir::new(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn document_root_joins_public_dir() {
        let mut p = project("shop", "C:\\sites\\shop");
        p.environment.public_dir = PublicDir::new("web/public").unwrap();
        assert_eq!(p.document_root(), PathBuf::from("C:\\sites\\shop").join("web").join("public"));
    }

    #[test]
    fn database_follows_mysql_choice() {
        let mut p = project("my-shop", "C:\\sites\\shop");
        assert_eq!(p.database.as_ref().unwrap().as_str(), "my_shop");
        let mut env = p.environment.clone();
        env.set_mysql(None);
        p.set_environment(env.clone()).unwrap();
        assert!(p.database.is_none());
        env.set_mysql(Some(MysqlSeries::LEGACY_5_7));
        p.set_environment(env).unwrap();
        let conn = p.database_connection(&Settings::default()).unwrap().unwrap();
        assert_eq!(conn.port, 3307);
        assert_eq!(conn.user, "root");
    }

    #[test]
    fn a_database_name_needs_mysql() {
        let mut p = project("shop", "C:\\sites\\shop");
        p.set_database_name(DatabaseName::new("shop_db").unwrap());
        assert_eq!(p.database.as_ref().unwrap().as_str(), "shop_db");
        let mut env = p.environment.clone();
        env.set_mysql(None);
        p.set_environment(env).unwrap();
        p.set_database_name(DatabaseName::new("orphan").unwrap());
        assert!(p.database.is_none());
    }

    #[test]
    fn url_omits_default_port() {
        let p = project("shop", "C:\\sites\\shop");
        let mut s = Settings::default();
        assert_eq!(p.url(&s), "http://shop.test");
        s.apache_port = 8080;
        assert_eq!(p.url(&s), "http://shop.test:8080");
    }

    #[test]
    fn detects_conflicts() {
        let s = Settings::default();
        let mut a = project("shop", "C:\\sites\\shop");
        a.addresses.push(SiteAddress::LoopbackPort(8101));
        let existing = vec![a.clone()];
        assert!(matches!(check_conflicts(&existing, &[], &a, true, &s), Err(DomainError::DuplicateProjectId(_))));
        assert!(check_conflicts(&existing, &[], &a, false, &s).is_ok());

        let same_root = project("other", "c:\\SITES\\shop");
        assert!(matches!(check_conflicts(&existing, &[], &same_root, true, &s), Err(DomainError::DuplicateProjectRoot { .. })));

        let mut same_host = project("blog", "C:\\sites\\blog");
        same_host.set_addresses(vec![a.addresses[1].clone()]).unwrap();
        assert!(matches!(check_conflicts(&existing, &[], &same_host, true, &s), Err(DomainError::DuplicateHostname { .. })));

        let mut same_port = project("api", "C:\\sites\\api");
        same_port.addresses.push(SiteAddress::LoopbackPort(8101));
        assert!(matches!(check_conflicts(&existing, &[], &same_port, true, &s), Err(DomainError::DuplicateLoopbackPort { port: 8101, .. })));

        let mut on_mysql = project("db", "C:\\sites\\db");
        on_mysql.addresses.push(SiteAddress::LoopbackPort(3306));
        assert!(matches!(check_conflicts(&[], &[], &on_mysql, true, &s), Err(DomainError::LoopbackPortTaken { port: 3306, .. })));
    }

    #[test]
    fn loopback_ports_of_worktree_environments_are_conflicts() {
        use crate::worktree::{DatabaseMode, Occupancy};
        let s = Settings::default();
        let shop = project("shop", "C:\\sites\\shop");
        let wt = WorktreeEnvironment::derive(&shop, &Occupancy::default(), ProjectRoot::new("D:\\wt\\a").unwrap(), Some("fix".into()), 8100, &DatabaseMode::Shared, 0).unwrap();
        let mut api = project("api", "C:\\sites\\api");
        api.addresses.push(SiteAddress::LoopbackPort(8100));
        let err = check_conflicts(std::slice::from_ref(&shop), std::slice::from_ref(&wt), &api, true, &s).unwrap_err();
        assert_eq!(err, DomainError::DuplicateLoopbackPort { port: 8100, other: "shop@fix".into() });
        // Updating an existing project is held to the same rule.
        assert!(check_conflicts(&[shop.clone(), api.clone()], std::slice::from_ref(&wt), &api, false, &s).is_err());
        api.addresses.pop();
        assert!(check_conflicts(&[shop], &[wt], &api, true, &s).is_ok());
    }

    #[test]
    fn folders_and_aliases_of_worktree_environments_are_conflicts() {
        use crate::hostname::Hostname;
        use crate::worktree::{DatabaseMode, Occupancy};
        let s = Settings::default();
        let shop = project("shop", r"C:\sites\shop");
        let wt = WorktreeEnvironment::derive(&shop, &Occupancy::default(), ProjectRoot::new(r"D:\wt\fix").unwrap(), Some("fix".into()), 8100, &DatabaseMode::Shared, 0).unwrap();
        let (projects, worktrees) = (std::slice::from_ref(&shop), std::slice::from_ref(&wt));

        let in_the_worktree = project("fix", r"d:\WT\fix");
        let err = check_conflicts(projects, worktrees, &in_the_worktree, true, &s).unwrap_err();
        assert_eq!(err, DomainError::DuplicateProjectRoot { root: r"D:\WT\fix".into(), other: "shop@fix".into() });

        let mut on_the_alias = project("blog", r"C:\sites\blog");
        on_the_alias.set_addresses(vec![SiteAddress::Localhost(Hostname::new("Fix.Shop.localhost").unwrap())]).unwrap();
        let err = check_conflicts(projects, worktrees, &on_the_alias, true, &s).unwrap_err();
        assert_eq!(err, DomainError::DuplicateHostname { hostname: "fix.shop.localhost".into(), other: "shop@fix".into() });
        // The parent itself may not take its environment's alias either.
        let mut parent = shop.clone();
        parent.addresses.push(SiteAddress::Localhost(Hostname::new("fix.shop.localhost").unwrap()));
        assert!(matches!(check_conflicts(projects, worktrees, &parent, false, &s), Err(DomainError::DuplicateHostname { .. })));
        assert!(check_conflicts(projects, worktrees, &project("blog", r"C:\sites\blog"), true, &s).is_ok());
    }

    #[test]
    fn legacy_environments_are_migrated_to_the_services_map() {
        let legacy: ProjectEnvironment = serde_json::from_str(r#"{"php": "8.3", "mysql": "5.7", "redis": true}"#).unwrap();
        assert_eq!(legacy.mysql(), Some(MysqlSeries::LEGACY_5_7));
        assert!(legacy.uses(&ServiceKind::REDIS));
        assert_eq!(legacy.service_ids().map(|s| s.to_string()).collect::<Vec<_>>(), ["mysql-5.7", "redis"]);
        let saved = serde_json::to_value(&legacy).unwrap();
        assert!(saved.get("mysql").is_none() && saved.get("redis").is_none(), "written in the new shape: {saved}");
        assert_eq!(saved["services"]["mysql"]["version"], "5.7");
        let again: ProjectEnvironment = serde_json::from_value(saved).unwrap();
        assert_eq!(again, legacy);
        let none: ProjectEnvironment = serde_json::from_str(r#"{"php": "8.3"}"#).unwrap();
        assert!(none.services.is_empty());
    }

    #[test]
    fn hosts_file_names_skip_localhost_and_ports() {
        let mut p = project("shop", "C:\\sites\\shop");
        p.addresses.push(SiteAddress::LoopbackPort(8101));
        assert_eq!(p.hosts_file_names().map(|h| h.as_str()).collect::<Vec<_>>(), ["shop.test"]);
        assert_eq!(p.urls(&Settings::default()), ["http://shop.test", "http://shop.localhost", "http://127.0.0.1:8101"]);
        assert!(p.set_addresses(vec![]).is_err());
    }

    #[test]
    fn unique_ids_get_a_numeric_suffix() {
        let existing = vec![project("shop", "C:\\a"), project("shop-2", "C:\\b")];
        assert_eq!(unique_project_id(&existing, &ProjectId::new("shop").unwrap()).as_str(), "shop-3");
        assert_eq!(unique_project_id(&existing, &ProjectId::new("blog").unwrap()).as_str(), "blog");
    }
}
