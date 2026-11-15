//! Presets: reusable starting environments. A preset is *copied* into a project at creation;
//! there is no live link. A project can be saved back as a custom preset.
//!
//! Built-in presets are values produced by [`PresetDefinition`] implementations. Adding a
//! built-in preset means adding a type implementing the trait and listing it in [`builtin::all`]
//! — the application layer never matches on preset ids.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::mysql::MysqlSeries;
use crate::php::{ExtensionName, PhpMinor, php_ini_overrides};
use crate::project::{ApacheDirectives, DisplayName, Project, ProjectEnvironment, PublicDir};

pub use crate::project::PresetId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preset {
    pub id: PresetId,
    pub name: DisplayName,
    #[serde(default)]
    pub description: String,
    /// Built-in presets are read-only; custom ones are stored as TOML and can be deleted.
    #[serde(default)]
    pub builtin: bool,
    pub environment: ProjectEnvironment,
}

impl Preset {
    /// Snapshot of a project's environment as a new custom preset.
    pub fn from_project(id: PresetId, name: DisplayName, description: impl Into<String>, project: &Project) -> Self {
        Self { id, name, description: description.into(), builtin: false, environment: project.environment.clone() }
    }

    pub fn ensure_editable(&self) -> Result<(), DomainError> {
        if self.builtin { Err(DomainError::BuiltinPresetIsReadOnly(self.id.to_string())) } else { Ok(()) }
    }
}

// Note: built-in names/descriptions are English fallbacks. The GUI shows the Italian text from
// its messages module, keyed by preset id (`builtin: true`); custom presets show their own name.

/// Facts that built-in presets may depend on at the time they are listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresetContext {
    /// Newest PHP minor known (installed or in the catalog).
    pub latest_php: PhpMinor,
    /// The always-on MySQL series from settings.
    pub default_mysql: MysqlSeries,
}

/// A built-in preset. Implement this to add one (open-closed).
pub trait PresetDefinition: Send + Sync {
    fn id(&self) -> PresetId;
    fn build(&self, ctx: &PresetContext) -> Preset;
}

/// Built-in preset definitions.
pub mod builtin {
    use super::*;

    fn ext(names: &[&str]) -> BTreeSet<ExtensionName> {
        names.iter().map(|n| ExtensionName::new(n).expect("built-in extension names are valid")).collect()
    }

    fn make(id: &str, name: &str, description: &str, environment: ProjectEnvironment) -> Preset {
        Preset {
            id: PresetId::new(id).expect("built-in preset ids are valid"),
            name: DisplayName::new(name).expect("built-in preset names are valid"),
            description: description.to_string(),
            builtin: true,
            environment,
        }
    }

    fn env(php: PhpMinor, extensions: &[&str], ini: &[(&str, &str)], public_dir: &str, mysql: Option<MysqlSeries>) -> ProjectEnvironment {
        let mut env = ProjectEnvironment {
            php,
            extensions: ext(extensions),
            addons: BTreeSet::new(),
            php_ini: php_ini_overrides(ini.iter().copied()).expect("built-in ini values are valid"),
            public_dir: PublicDir::new(public_dir).expect("built-in public dirs are valid"),
            services: Default::default(),
            apache_directives: ApacheDirectives::default(),
        };
        env.set_mysql(mysql);
        env
    }

    const COMMON_INI: &[(&str, &str)] = &[("memory_limit", "512M"), ("upload_max_filesize", "64M"), ("post_max_size", "64M"), ("max_execution_time", "120")];

    pub struct Laravel;
    impl PresetDefinition for Laravel {
        fn id(&self) -> PresetId {
            PresetId::new("laravel").expect("valid")
        }
        fn build(&self, ctx: &PresetContext) -> Preset {
            make(
                "laravel",
                "Laravel",
                "Laravel: document root `public`, MySQL, latest PHP.",
                env(ctx.latest_php, &["curl", "fileinfo", "intl", "mbstring", "openssl", "pdo_mysql", "pdo_sqlite", "sqlite3", "zip"], COMMON_INI, "public", Some(ctx.default_mysql)),
            )
        }
    }

    pub struct WordPress;
    impl PresetDefinition for WordPress {
        fn id(&self) -> PresetId {
            PresetId::new("wordpress").expect("valid")
        }
        fn build(&self, ctx: &PresetContext) -> Preset {
            make(
                "wordpress",
                "WordPress",
                "WordPress: document root = project folder, MySQL, image extensions.",
                env(ctx.latest_php, &["curl", "exif", "fileinfo", "gd", "intl", "mbstring", "mysqli", "openssl", "zip"], COMMON_INI, "", Some(ctx.default_mysql)),
            )
        }
    }

    pub struct Symfony;
    impl PresetDefinition for Symfony {
        fn id(&self) -> PresetId {
            PresetId::new("symfony").expect("valid")
        }
        fn build(&self, ctx: &PresetContext) -> Preset {
            make(
                "symfony",
                "Symfony",
                "Symfony: document root `public`, MySQL, intl.",
                env(ctx.latest_php, &["curl", "intl", "mbstring", "openssl", "pdo_mysql", "sodium", "zip"], COMMON_INI, "public", Some(ctx.default_mysql)),
            )
        }
    }

    pub struct GenericPhp;
    impl PresetDefinition for GenericPhp {
        fn id(&self) -> PresetId {
            PresetId::new("generic-php").expect("valid")
        }
        fn build(&self, ctx: &PresetContext) -> Preset {
            make(
                "generic-php",
                "Generic PHP",
                "Plain PHP site served from the project folder.",
                env(ctx.latest_php, &["curl", "fileinfo", "mbstring", "mysqli", "openssl", "pdo_mysql"], &[], "", Some(ctx.default_mysql)),
            )
        }
    }

    pub struct Legacy;
    impl PresetDefinition for Legacy {
        fn id(&self) -> PresetId {
            PresetId::new("legacy").expect("valid")
        }
        fn build(&self, _ctx: &PresetContext) -> Preset {
            make(
                "legacy",
                "Legacy",
                "Older applications: PHP 7.4 and MySQL 5.7.",
                env(PhpMinor::new(7, 4).expect("valid"), &["curl", "fileinfo", "gd", "mbstring", "mysqli", "openssl", "pdo_mysql"], COMMON_INI, "", Some(MysqlSeries::LEGACY_5_7)),
            )
        }
    }

    /// Every built-in preset, in display order. The composition root passes this list to the
    /// application layer.
    pub fn all() -> Vec<Arc<dyn PresetDefinition>> {
        vec![Arc::new(Laravel), Arc::new(WordPress), Arc::new(Symfony), Arc::new(GenericPhp), Arc::new(Legacy)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> PresetContext {
        PresetContext { latest_php: PhpMinor::parse("8.4").unwrap(), default_mysql: MysqlSeries::DEFAULT }
    }

    #[test]
    fn builtins_have_unique_ids_matching_their_preset() {
        let all = builtin::all();
        let ids: BTreeSet<_> = all.iter().map(|d| d.id()).collect();
        assert_eq!(ids.len(), all.len());
        for d in &all {
            let p = d.build(&ctx());
            assert_eq!(p.id, d.id());
            assert!(p.builtin);
            assert!(p.ensure_editable().is_err());
        }
    }

    #[test]
    fn legacy_pins_php_74_and_mysql_57() {
        let p = builtin::Legacy.build(&ctx());
        assert_eq!(p.environment.php.to_string(), "7.4");
        assert_eq!(p.environment.mysql(), Some(MysqlSeries::LEGACY_5_7));
    }

    #[test]
    fn laravel_uses_latest_php_and_public_dir() {
        let p = builtin::Laravel.build(&ctx());
        assert_eq!(p.environment.php.to_string(), "8.4");
        assert_eq!(p.environment.public_dir.as_str(), "public");
    }

    #[test]
    fn project_can_be_saved_as_custom_preset() {
        let project = crate::project::tests::project("shop", "C:\\sites\\shop");
        let preset = Preset::from_project(PresetId::new("my-stack").unwrap(), DisplayName::new("My stack").unwrap(), "", &project);
        assert!(!preset.builtin);
        assert_eq!(preset.environment, project.environment);
        preset.ensure_editable().unwrap();
    }
}
