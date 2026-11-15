//! Apache httpd (Apache Lounge build) with mod_fcgid.
//!
//! Generated layout (`LampoPaths::apache_config_dir`):
//! * `httpd.conf` — from `templates/httpd.conf.j2`: modules, one `Listen` per
//!   `WebServerConfig::listen_ports()` (loopback only), mod_fcgid process management, the
//!   denial of VCS metadata/secrets/logs/dumps, a catch-all vhost on every listened address
//!   (404 for a Host no site claims, DNS-rebinding protection included),
//!   `IncludeOptional ${LAMPO_VHOSTS}` (= `vhosts/*.conf` unless the validation wrapper
//!   redefines it to point at the staged tree);
//! * `vhosts/<site-id>.conf` — from `templates/vhost.conf.j2`: name-based vhost on the shared
//!   port (`server_names`) and/or `<VirtualHost 127.0.0.1:<loopback_port>>` (site names plus
//!   `localhost`/`127.0.0.1`), no directory listing, `AllowOverride All`
//!   (so `.htaccess` works), `FcgidWrapper "<php-cgi.exe>" .php`,
//!   `FcgidInitialEnv PHPRC "<phprc_dir>"`, then the project's custom directives verbatim.
//!
//! Responsibilities are split: [`render`] (pure text), [`staging`] (files on disk),
//! [`validation`] (`httpd -t` via `CommandRunner`), [`config`] (the `WebServerConfigurator`
//! adapter composing them), [`runner`] + [`signal`] (process control).

pub mod config;
pub mod render;
pub mod runner;
pub mod signal;
pub mod staging;
pub mod syntax;
pub mod validation;

#[cfg(test)]
mod hardening_tests;

/// Templates embedded at compile time (minijinja syntax).
pub mod templates {
    use std::sync::LazyLock;

    /// Raw template sources, as checked out. `.gitattributes` keeps them LF, but a checkout
    /// with other settings may embed CRLF: the renderer uses [`httpd_conf`] / [`vhost_conf`].
    const HTTPD_CONF: &str = include_str!("templates/httpd.conf.j2");
    const VHOST_CONF: &str = include_str!("templates/vhost.conf.j2");

    static HTTPD_CONF_LF: LazyLock<String> = LazyLock::new(|| to_lf(HTTPD_CONF));
    static VHOST_CONF_LF: LazyLock<String> = LazyLock::new(|| to_lf(VHOST_CONF));

    /// `httpd.conf.j2` with LF line endings, whatever the checkout used.
    pub fn httpd_conf() -> &'static str {
        &HTTPD_CONF_LF
    }

    /// `vhost.conf.j2` with LF line endings, whatever the checkout used.
    pub fn vhost_conf() -> &'static str {
        &VHOST_CONF_LF
    }

    pub(crate) fn to_lf(text: &str) -> String {
        text.replace("\r\n", "\n")
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn templates_are_normalized_to_lf() {
            assert_eq!(super::to_lf("a\r\nb\nc\r\n"), "a\nb\nc\n");
            assert!(!super::httpd_conf().contains('\r'));
            assert!(!super::vhost_conf().contains('\r'));
        }
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use std::path::PathBuf;

    use lampo_domain::hostname::Hostname;
    use lampo_domain::project::{ApacheDirectives, TlsMode};
    use lampo_domain::site::SiteId;
    use lampo_domain::web::{VirtualHost, WebServerConfig};

    use super::render::RenderOptions;

    pub fn vhost(site: &str, names: &[&str], port: Option<u16>, root: &str) -> VirtualHost {
        let site: SiteId = site.parse().unwrap();
        let key = site.file_key();
        VirtualHost {
            server_names: names.iter().map(|n| Hostname::new(n).unwrap()).collect(),
            loopback_port: port,
            project_root: PathBuf::from(root),
            document_root: PathBuf::from(root).join("public"),
            php_cgi: PathBuf::from(r"C:\Lampo Data\packages\php-8.3.14\php-cgi.exe"),
            phprc_dir: PathBuf::from(format!(r"C:\Lampo Data\generated\php\{key}")),
            custom_directives: ApacheDirectives::default(),
            tls: TlsMode::Disabled,
            error_log: PathBuf::from(format!(r"C:\Lampo Data\logs\projects\{key}\error.log")),
            access_log: PathBuf::from(format!(r"C:\Lampo Data\logs\projects\{key}\access.log")),
            site,
        }
    }

    pub fn sample_config() -> WebServerConfig {
        let mut shop = vhost("shop", &["shop.test", "shop.localhost"], None, r"C:\Sites\My Shop");
        shop.custom_directives = ApacheDirectives::new("Header set X-Shop \"1\"\r\n<Location /admin>\r\n    Require ip 127.0.0.1\r\n</Location>").unwrap();
        let mut legacy = vhost("legacy", &["legacy.test"], Some(8080), r"D:\legacy");
        legacy.php_cgi = PathBuf::from(r"C:\Lampo\php-5.6.40\php-cgi.exe");
        legacy.document_root = PathBuf::from(r"D:\legacy");
        let worktree = vhost("shop@feature-x", &[], Some(8101), r"C:\Sites\My Shop.worktrees\feature-x");
        WebServerConfig {
            server_root: PathBuf::from(r"C:\Lampo Data\packages\apache-2.4.62"),
            mod_fcgid: PathBuf::from(r"C:\Lampo Data\packages\mod_fcgid-2.3.10\mod_fcgid.so"),
            listen_port: 80,
            config_dir: PathBuf::from(r"C:\Lampo Data\generated\apache"),
            log_dir: PathBuf::from(r"C:\Lampo Data\logs\apache"),
            vhosts: vec![legacy, shop, worktree],
        }
    }

    pub fn sample_options() -> RenderOptions {
        RenderOptions::with_env([
            ("SystemRoot".to_string(), r"C:\WINDOWS".to_string()),
            ("ComSpec".to_string(), r"C:\WINDOWS\system32\cmd.exe".to_string()),
            ("PATH".to_string(), r"C:\WINDOWS\system32;C:\WINDOWS;;C:\Program Files\Git\cmd\;C:\bad${X}".to_string()),
            ("APPDATA".to_string(), r#"C:\Users\a"b"#.to_string()),
        ])
    }
}
