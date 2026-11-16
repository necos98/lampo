//! Extensions the Windows PHP builds name differently across releases: GD is `php_gd2.dll` up to
//! PHP 7.4 and `php_gd.dll` from PHP 8.0. A project (or a preset, an import, a Dockerfile) may use
//! either name; [`ExtensionName::for_php`] gives the one the project's build loads, so `--ext gd`
//! on PHP 7.4 enables `gd2` and a switch to PHP 8.3 turns it back into `gd`.
//!
//! The table is explicit and small on purpose: only real renames of the same extension, never a
//! fuzzy match (`mysqli` and `pdo_mysql` are two different extensions).

use std::collections::BTreeSet;

use super::{ExtensionName, PhpMinor};

/// One extension renamed by a PHP release: `before` up to the minor preceding `since`, `after`
/// from `since` on.
struct Rename {
    before: &'static str,
    after: &'static str,
    since: PhpMinor,
}

const RENAMES: &[Rename] = &[Rename { before: "gd2", after: "gd", since: PhpMinor { major: 8, minor: 0 } }];

impl ExtensionName {
    /// The name `php`'s builds give this extension (itself when it was never renamed).
    pub fn for_php(&self, php: PhpMinor) -> ExtensionName {
        match RENAMES.iter().find(|r| self.0 == r.before || self.0 == r.after) {
            Some(rename) if php >= rename.since => ExtensionName(rename.after.to_string()),
            Some(rename) => ExtensionName(rename.before.to_string()),
            None => self.clone(),
        }
    }
}

/// `names` as `php`'s builds name them (two aliases of one extension become one entry).
pub fn extensions_for_php(names: &BTreeSet<ExtensionName>, php: PhpMinor) -> BTreeSet<ExtensionName> {
    names.iter().map(|name| name.for_php(php)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ext(name: &str) -> ExtensionName {
        ExtensionName::new(name).unwrap()
    }

    fn php(minor: &str) -> PhpMinor {
        PhpMinor::parse(minor).unwrap()
    }

    #[test]
    fn gd_is_gd2_before_php_8() {
        for minor in ["5.6", "7.0", "7.4"] {
            assert_eq!(ext("gd").for_php(php(minor)), ext("gd2"), "PHP {minor}");
            assert_eq!(ext("gd2").for_php(php(minor)), ext("gd2"), "PHP {minor}");
        }
        for minor in ["8.0", "8.3", "8.10"] {
            assert_eq!(ext("gd2").for_php(php(minor)), ext("gd"), "PHP {minor}");
            assert_eq!(ext("php_gd2.dll").for_php(php(minor)), ext("gd"), "PHP {minor}");
            assert_eq!(ext("gd").for_php(php(minor)), ext("gd"), "PHP {minor}");
        }
    }

    #[test]
    fn other_names_are_left_alone() {
        for name in ["mysqli", "pdo_mysql", "intl", "gd3", "libgd"] {
            assert_eq!(ext(name).for_php(php("7.4")), ext(name));
            assert_eq!(ext(name).for_php(php("8.3")), ext(name));
        }
    }

    #[test]
    fn a_set_merges_both_aliases_into_one() {
        let names: BTreeSet<ExtensionName> = [ext("gd"), ext("gd2"), ext("intl")].into_iter().collect();
        assert_eq!(extensions_for_php(&names, php("7.4")), [ext("gd2"), ext("intl")].into_iter().collect());
        assert_eq!(extensions_for_php(&names, php("8.3")), [ext("gd"), ext("intl")].into_iter().collect());
        assert!(extensions_for_php(&BTreeSet::new(), php("8.3")).is_empty());
    }
}
