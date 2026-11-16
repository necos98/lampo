//! The extensions a Windows PHP build ships: one `ext\php_<name>.dll` per loadable module, plus
//! the modules compiled into its `php.exe` ([`crate::php_builtin_modules`]).

use std::path::Path;

use lampo_domain::php::ExtensionName;
use lampo_domain::ports::PhpExtensionScanner;
use lampo_domain::{PortError, PortResult};

use crate::layout::PHP_CLI;
use crate::php_builtin_modules::BuiltinModules;

const NOT_USER_EXTENSIONS: &[&str] = &["zend_test", "phpdbg_webhelper", "dl_test"];

/// One per process: it keeps the compiled-in modules of every build it was asked about.
#[derive(Debug, Default)]
pub struct FsPhpExtensionScanner {
    builtin: BuiltinModules,
}

impl PhpExtensionScanner for FsPhpExtensionScanner {
    fn builtin_extensions(&self, php_dir: &Path) -> PortResult<Vec<ExtensionName>> {
        self.builtin.of(&php_dir.join(PHP_CLI))
    }

    fn bundled_extensions(&self, php_dir: &Path) -> PortResult<Vec<ExtensionName>> {
        let ext_dir = php_dir.join("ext");
        let entries = match std::fs::read_dir(&ext_dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(PortError::io(format!("listing {}", ext_dir.display()), e)),
        };
        let mut names: Vec<ExtensionName> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|e| {
                let file = e.file_name().to_string_lossy().to_ascii_lowercase();
                // Only `php_<name>.dll`: the folder also holds third-party support DLLs.
                (file.starts_with("php_") && file.ends_with(".dll")).then(|| ExtensionName::new(&file).ok()).flatten()
            })
            // Test/debugger helper modules shipped by the builds, not meant to be enabled.
            .filter(|name| !NOT_USER_EXTENSIONS.contains(&name.as_str()))
            .collect();
        names.sort();
        names.dedup();
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_php_dlls_only_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let ext = dir.path().join("ext");
        std::fs::create_dir(&ext).unwrap();
        for f in ["php_mbstring.dll", "PHP_Intl.dll", "php_opcache.dll", "libsodium.dll", "php_readme.txt", "php_zend_test.dll"] {
            std::fs::write(ext.join(f), b"").unwrap();
        }
        std::fs::create_dir(ext.join("php_dir.dll")).unwrap();
        let names: Vec<String> = FsPhpExtensionScanner::default().bundled_extensions(dir.path()).unwrap().iter().map(|n| n.to_string()).collect();
        assert_eq!(names, ["intl", "mbstring", "opcache"]);
        assert!(FsPhpExtensionScanner::default().bundled_extensions(&dir.path().join("missing")).unwrap().is_empty());
    }
}
