//! What an imported project export (`.lampo.toml`, possibly shared by someone else) may not
//! bring along. The file is untrusted: settings that run code on this machine are left out and
//! reported, so the user can add them back knowingly.

use crate::project::Project;

/// php.ini keys never imported: they load code, run a file on every request or loosen PHP's own
/// restrictions.
pub const UNTRUSTED_PHP_INI_KEYS: &[&str] = &["auto_prepend_file", "auto_append_file", "extension", "zend_extension", "extension_dir", "open_basedir", "disable_functions", "include_path"];

/// Whether an imported php.ini key must be left out (case and surrounding spaces ignored).
pub fn is_untrusted_php_ini_key(key: &str) -> bool {
    let key = key.trim();
    UNTRUSTED_PHP_INI_KEYS.iter().any(|k| k.eq_ignore_ascii_case(key))
}

/// The settings left out of an import.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DroppedOnImport {
    /// The custom Apache directives of the export, verbatim: they are never imported.
    pub apache_directives: Option<String>,
    /// The php.ini keys left out ([`UNTRUSTED_PHP_INI_KEYS`]), as written in the file.
    pub php_ini_keys: Vec<String>,
    /// Names of the environment variables of the export: none is imported, since a variable
    /// (`COMSPEC`, `NODE_OPTIONS`, …) can run code here. The values are not kept.
    pub env_vars: Vec<String>,
}

/// A parsed export: the project to save (without the dropped settings) and what was dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedProject {
    pub project: Project,
    pub dropped: DroppedOnImport,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untrusted_keys_are_matched_case_insensitively() {
        for key in UNTRUSTED_PHP_INI_KEYS {
            assert!(is_untrusted_php_ini_key(key));
        }
        assert!(is_untrusted_php_ini_key(" Auto_Prepend_File "));
        assert!(!is_untrusted_php_ini_key("memory_limit"));
        assert!(!is_untrusted_php_ini_key("xdebug.mode"));
    }
}
