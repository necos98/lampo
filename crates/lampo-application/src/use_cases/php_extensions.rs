//! What an installed PHP build offers: the extensions it ships as DLLs (the project editor's
//! checkboxes, `lampo package extensions`) and the modules compiled into it (`bcmath`…), which a
//! project may name but which need no php.ini line.

use std::collections::BTreeSet;
use std::sync::Arc;

use lampo_domain::package::InstalledPackage;
use lampo_domain::php::{ExtensionName, PhpMinor};
use lampo_domain::ports::{InstalledPackageRegistry, PhpExtensionScanner};

use super::support::{package_missing, php_dir};
use crate::dto::PhpExtensionDto;
use crate::error::AppResult;

/// The extensions of an installed PHP minor.
pub struct PhpExtensionsQuery {
    installed: Arc<dyn InstalledPackageRegistry>,
    scanner: Arc<dyn PhpExtensionScanner>,
}

/// What one build offers for some wanted names.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct BuildExtensions {
    /// Every `ext\php_<name>.dll` of the build, sorted.
    pub loadable: Vec<ExtensionName>,
    /// Its compiled-in modules, sorted; only asked when a wanted name has no DLL (else empty).
    pub builtin: Vec<ExtensionName>,
}

impl BuildExtensions {
    /// Those of `names` compiled into the build rather than loaded from a DLL.
    pub(crate) fn compiled_in<'a>(&self, names: impl IntoIterator<Item = &'a ExtensionName>) -> BTreeSet<ExtensionName> {
        names.into_iter().filter(|name| !self.loadable.contains(name) && self.builtin.contains(name)).cloned().collect()
    }
}

impl PhpExtensionsQuery {
    pub fn new(installed: Arc<dyn InstalledPackageRegistry>, scanner: Arc<dyn PhpExtensionScanner>) -> Self {
        Self { installed, scanner }
    }

    /// Extensions of the newest installed build of `php_version` (`"8.3"`), sorted.
    /// `PackageMissing` if that minor is not installed.
    pub fn list(&self, php_version: &str) -> AppResult<Vec<PhpExtensionDto>> {
        let php = PhpMinor::parse(php_version.trim())?;
        let dir = php_dir(&self.installed.list()?, php).ok_or_else(|| package_missing(format!("PHP {php}")))?;
        Ok(self.scanner.bundled_extensions(&dir)?.into_iter().map(|name| PhpExtensionDto { zend: lampo_domain::web::is_zend_extension(&name), name: name.to_string() }).collect())
    }

    /// What the newest build of `php` among `installed` offers for `wanted` (`None` when that
    /// minor is not installed). The build itself is run only when a wanted name has no DLL; if it
    /// cannot tell its modules (it does not start…), `builtin` stays empty and that is logged.
    pub(crate) fn build_extensions(&self, installed: &[InstalledPackage], php: PhpMinor, wanted: &BTreeSet<ExtensionName>) -> AppResult<Option<BuildExtensions>> {
        let Some(dir) = php_dir(installed, php) else { return Ok(None) };
        let loadable = self.scanner.bundled_extensions(&dir)?;
        let builtin = match wanted.iter().any(|name| !loadable.contains(name)) {
            true => self.scanner.builtin_extensions(&dir).unwrap_or_else(|e| {
                tracing::warn!("cannot list the modules compiled into PHP {php}: {e}");
                Vec::new()
            }),
            false => Vec::new(),
        };
        Ok(Some(BuildExtensions { loadable, builtin }))
    }

    /// [`Self::build_extensions`] with the installed packages listed now.
    pub(crate) fn installed_build_extensions(&self, php: PhpMinor, wanted: &BTreeSet<ExtensionName>) -> AppResult<Option<BuildExtensions>> {
        self.build_extensions(&self.installed.list()?, php, wanted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<ExtensionName> {
        list.iter().map(|n| ExtensionName::new(n).unwrap()).collect()
    }

    #[test]
    fn compiled_in_is_builtin_without_a_dll() {
        let build = BuildExtensions { loadable: names(&["gd", "zip"]), builtin: names(&["bcmath", "zip"]) };
        let wanted = names(&["bcmath", "gd", "intl", "zip"]);
        assert_eq!(build.compiled_in(&wanted), names(&["bcmath"]).into_iter().collect());
        assert!(BuildExtensions::default().compiled_in(&wanted).is_empty());
    }
}
