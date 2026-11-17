//! How the MySQL accounts a project's code connects with authenticate, given its PHP.
//!
//! MySQL 8 authenticates new accounts with `caching_sha2_password`, which PHP's `mysqlnd` only
//! speaks from PHP 7.4 ([`CACHING_SHA2_MIN_PHP`]): an older PHP fails at runtime with error 2054
//! ("The server requested authentication method unknown to the client"). Such a project needs
//! its account on `mysql_native_password`, which MySQL 8.4 ships disabled: Lampo's `my.ini`
//! enables it on the series that need that ([`MysqlSeries::needs_native_password_enabled`]).
//! MySQL 5.7 uses `mysql_native_password` for every account already.

use crate::mysql::MysqlSeries;
use crate::php::PhpMinor;
use crate::project::ProjectEnvironment;

/// The oldest PHP whose `mysqlnd` authenticates with `caching_sha2_password`.
pub const CACHING_SHA2_MIN_PHP: PhpMinor = PhpMinor::MYSQL_CACHING_SHA2_MIN;

/// An authentication plugin Lampo sets on the accounts it manages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MysqlAuthPlugin {
    /// MySQL 8's default.
    CachingSha2,
    /// What PHP before [`CACHING_SHA2_MIN_PHP`] speaks.
    Native,
}

impl MysqlAuthPlugin {
    /// The name `IDENTIFIED WITH` takes and `mysql.user.plugin` holds.
    pub fn sql_name(self) -> &'static str {
        match self {
            Self::CachingSha2 => "caching_sha2_password",
            Self::Native => "mysql_native_password",
        }
    }

    /// The plugin a `series` account used by code running on `php` must have; `None` where the
    /// series' default already suits every PHP (5.7), so the server default is kept.
    pub fn for_client(series: MysqlSeries, php: PhpMinor) -> Option<Self> {
        if !series.defaults_to_caching_sha2() {
            None
        } else if php_lacks_caching_sha2(php) {
            Some(Self::Native)
        } else {
            Some(Self::CachingSha2)
        }
    }
}

fn php_lacks_caching_sha2(php: PhpMinor) -> bool {
    php < CACHING_SHA2_MIN_PHP
}

/// The MySQL series `env` uses when its PHP cannot authenticate with that series' default plugin,
/// so its account is switched to `mysql_native_password`: what the warnings of a save and the
/// doctor report on. `None` without MySQL or when the combination needs nothing.
pub fn legacy_php_auth_series(env: &ProjectEnvironment) -> Option<MysqlSeries> {
    env.mysql().filter(|series| MysqlAuthPlugin::for_client(*series, env.php) == Some(MysqlAuthPlugin::Native))
}

/// The warning for `env` (see [`legacy_php_auth_series`]), with the alternative.
pub fn legacy_php_auth_warning(env: &ProjectEnvironment) -> Option<String> {
    legacy_php_auth_series(env).map(|series| {
        format!(
            "PHP {php} cannot use MySQL {series}'s default authentication (caching_sha2_password, PHP {CACHING_SHA2_MIN_PHP}+): \
             Lampo switches the project's MySQL account to mysql_native_password; if the connection still fails (error 2054), \
             move the project to MySQL {legacy} (`--service mysql@{legacy}`)",
            php = env.php,
            legacy = MysqlSeries::LEGACY_5_7,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn php(s: &str) -> PhpMinor {
        PhpMinor::parse(s).unwrap()
    }

    fn env(php_version: &str, mysql: Option<MysqlSeries>) -> ProjectEnvironment {
        let mut env = ProjectEnvironment::minimal(php(php_version));
        env.set_mysql(mysql);
        env
    }

    #[test]
    fn php_before_7_4_gets_native_password_on_mysql_8_only() {
        assert_eq!(MysqlAuthPlugin::for_client(MysqlSeries::DEFAULT, php("7.3")), Some(MysqlAuthPlugin::Native));
        assert_eq!(MysqlAuthPlugin::for_client(MysqlSeries::DEFAULT, php("5.6")), Some(MysqlAuthPlugin::Native));
        assert_eq!(MysqlAuthPlugin::for_client(MysqlSeries::DEFAULT, php("7.4")), Some(MysqlAuthPlugin::CachingSha2));
        assert_eq!(MysqlAuthPlugin::for_client(MysqlSeries::DEFAULT, php("8.3")), Some(MysqlAuthPlugin::CachingSha2));
        assert_eq!(MysqlAuthPlugin::for_client(MysqlSeries::LEGACY_5_7, php("7.2")), None, "5.7 is native already");
        assert_eq!(MysqlAuthPlugin::Native.sql_name(), "mysql_native_password");
        assert_eq!(MysqlAuthPlugin::CachingSha2.sql_name(), "caching_sha2_password");
    }

    #[test]
    fn only_8_4_and_later_8_x_need_the_native_plugin_turned_on() {
        let series = |s: &str| MysqlSeries::parse(s).unwrap();
        assert!(MysqlSeries::DEFAULT.needs_native_password_enabled());
        assert!(!MysqlSeries::LEGACY_5_7.needs_native_password_enabled());
        assert!(!series("8.0").needs_native_password_enabled(), "always on before 8.4");
        assert!(!series("9.1").needs_native_password_enabled(), "removed in 9.0");
        assert!(series("8.0").defaults_to_caching_sha2() && !MysqlSeries::LEGACY_5_7.defaults_to_caching_sha2());
    }

    #[test]
    fn only_old_php_on_mysql_8_is_warned_with_the_5_7_alternative() {
        assert_eq!(legacy_php_auth_series(&env("7.2", Some(MysqlSeries::DEFAULT))), Some(MysqlSeries::DEFAULT));
        assert_eq!(legacy_php_auth_series(&env("7.4", Some(MysqlSeries::DEFAULT))), None);
        assert_eq!(legacy_php_auth_series(&env("7.2", Some(MysqlSeries::LEGACY_5_7))), None);
        assert_eq!(legacy_php_auth_series(&env("7.2", None)), None);
        let warning = legacy_php_auth_warning(&env("7.3", Some(MysqlSeries::DEFAULT))).unwrap();
        assert!(warning.starts_with("PHP 7.3 cannot use MySQL 8.4's default authentication"), "{warning}");
        assert!(warning.contains("mysql_native_password") && warning.contains("`--service mysql@5.7`"), "{warning}");
    }
}
