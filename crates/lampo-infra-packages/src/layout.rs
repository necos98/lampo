//! Where each package kind keeps the files other components use: the single place encoding
//! package layouts. [`resolve`] fills `InstalledPackage::entry_points` (and the add-on
//! extension name) when the registry lists a package, so the application asks for a role
//! (`EntryRole::FastCgi`) instead of joining `php-cgi.exe` onto an install dir.
//!
//! Adding a kind = adding a row to [`LAYOUTS`]; kinds without a row simply offer no entry point.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lampo_domain::package::{EntryRole, InstalledPackage, PackageKind};
use lampo_domain::php::ExtensionName;

/// One kind's layout: files by role, relative to the install dir, and the extension name of a
/// PHP add-on.
struct Layout {
    kind: &'static str,
    entries: &'static [(EntryRole, &'static str)],
    php_extension: Option<&'static str>,
}

const LAYOUTS: &[Layout] = &[
    Layout { kind: "php", entries: &[(EntryRole::Cli, "php.exe"), (EntryRole::FastCgi, "php-cgi.exe")], php_extension: None },
    Layout { kind: "mod-fcgid", entries: &[(EntryRole::ApacheModule, "mod_fcgid.so")], php_extension: None },
    Layout { kind: "mysql", entries: &[(EntryRole::ClientDir, "bin")], php_extension: None },
    Layout { kind: "redis", entries: &[(EntryRole::Cli, "redis-cli.exe"), (EntryRole::ClientDir, "")], php_extension: None },
    Layout { kind: "mailpit", entries: &[(EntryRole::Cli, "mailpit.exe"), (EntryRole::ClientDir, "")], php_extension: None },
    Layout { kind: "composer", entries: &[(EntryRole::Phar, "composer.phar")], php_extension: None },
    Layout { kind: "xdebug", entries: &[(EntryRole::PhpExtension, "php_xdebug.dll")], php_extension: Some("xdebug") },
    Layout { kind: "php-redis", entries: &[(EntryRole::PhpExtension, "php_redis.dll")], php_extension: Some("redis") },
];

fn join(dir: &Path, relative: &str) -> PathBuf {
    if relative.is_empty() { dir.to_path_buf() } else { dir.join(relative) }
}

/// `package` with its entry points resolved for its kind.
pub fn resolve(mut package: InstalledPackage) -> InstalledPackage {
    let Some(layout) = LAYOUTS.iter().find(|l| package.id.kind == PackageKind::new(l.kind).expect("valid kind")) else { return package };
    package.entry_points = layout.entries.iter().map(|(role, relative)| (*role, join(&package.install_dir, relative))).collect::<BTreeMap<_, _>>();
    package.php_extension = layout.php_extension.map(|n| ExtensionName::new(n).expect("valid extension name"));
    package
}

#[cfg(test)]
mod tests {
    use super::*;
    use lampo_domain::package::PackageId;
    use lampo_domain::php::PhpMinor;
    use lampo_domain::version::PackageVersion;

    fn installed(kind: PackageKind, php: Option<&str>) -> InstalledPackage {
        let id = PackageId { kind, version: PackageVersion::parse("1.2.3").unwrap(), php: php.map(|p| PhpMinor::parse(p).unwrap()) };
        resolve(InstalledPackage::new(id, PathBuf::from("C:\\p\\x"), None))
    }

    #[test]
    fn entry_points_per_kind() {
        let php = installed(PackageKind::PHP, None);
        assert_eq!(php.entry(EntryRole::Cli), Some(Path::new("C:\\p\\x\\php.exe")));
        assert_eq!(php.entry(EntryRole::FastCgi), Some(Path::new("C:\\p\\x\\php-cgi.exe")));
        assert_eq!(installed(PackageKind::MYSQL, None).entry(EntryRole::ClientDir), Some(Path::new("C:\\p\\x\\bin")));
        assert_eq!(installed(PackageKind::COMPOSER, None).entry(EntryRole::Phar), Some(Path::new("C:\\p\\x\\composer.phar")));
        assert_eq!(installed(PackageKind::MOD_FCGID, None).entry(EntryRole::ApacheModule), Some(Path::new("C:\\p\\x\\mod_fcgid.so")));
        let addon = installed(PackageKind::PHP_REDIS, Some("8.3"));
        assert_eq!(addon.php_extension.as_ref().map(|e| e.as_str()), Some("redis"));
        assert_eq!(addon.entry(EntryRole::PhpExtension), Some(Path::new("C:\\p\\x\\php_redis.dll")));
        assert!(installed(PackageKind::APACHE, None).entry_points.is_empty());
        assert!(installed(PackageKind::new("postgresql").unwrap(), None).entry_points.is_empty(), "unknown kinds offer nothing");
    }
}
