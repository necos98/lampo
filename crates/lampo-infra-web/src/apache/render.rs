//! Pure rendering of the Apache configuration: `WebServerConfig` + [`RenderOptions`] → text.
//! No IO, no environment access (the host environment is captured in [`RenderOptions`]).

use std::path::Path;

use lampo_domain::project::{ApacheDirectives, TlsMode};
use lampo_domain::tcp_port::Port;
use lampo_domain::web::{VirtualHost, WebServerConfig};
use lampo_domain::{PortError, PortResult};
use minijinja::{Environment, UndefinedBehavior};
use serde::Serialize;

use super::syntax::{apache_path, fcgid_wrapper_command, quoted, quoted_text};
use super::templates;
use crate::site_layout;

/// Name of the main configuration file inside the config dir.
pub const MAIN_CONF: &str = "httpd.conf";
/// Directory (relative to the config dir) holding one file per site.
pub const VHOSTS_DIR: &str = "vhosts";

/// mod_fcgid process management and limits. Defaults favour low idle RAM (no process kept
/// alive without traffic) and development workloads (long debugging pauses, large uploads).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FcgidTuning {
    pub max_processes: u32,
    pub max_processes_per_class: u32,
    /// 0: a site with no traffic keeps no php-cgi.exe alive.
    pub min_processes_per_class: u32,
    pub idle_timeout_secs: u32,
    pub idle_scan_interval_secs: u32,
    pub process_lifetime_secs: u32,
    /// Also passed to PHP as `PHP_FCGI_MAX_REQUESTS` (must be ≥ this value, see mod_fcgid docs).
    pub max_requests_per_process: u32,
    pub io_timeout_secs: u32,
    pub busy_timeout_secs: u32,
    pub connect_timeout_secs: u32,
    pub max_request_len: u64,
    pub max_request_in_mem: u64,
}

impl Default for FcgidTuning {
    fn default() -> Self {
        Self {
            max_processes: 64,
            max_processes_per_class: 8,
            min_processes_per_class: 0,
            idle_timeout_secs: 120,
            idle_scan_interval_secs: 30,
            process_lifetime_secs: 3600,
            max_requests_per_process: 1000,
            // One hour: an Xdebug session paused on a breakpoint must not get a 500.
            io_timeout_secs: 3600,
            busy_timeout_secs: 3600,
            connect_timeout_secs: 20,
            // 1 GiB uploads; bodies above 1 MiB are spooled to a temp file, not kept in RAM.
            max_request_len: 1024 * 1024 * 1024,
            max_request_in_mem: 1024 * 1024,
        }
    }
}

/// One `NAME=value` passed to every php-cgi.exe through `FcgidInitialEnv`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

/// Host facts and tuning the renderer needs besides the `WebServerConfig`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderOptions {
    pub fcgid: FcgidTuning,
    /// Also listen on `[::1]` for the shared port: browsers resolve `*.localhost` names to both
    /// loopbacks. Hosts-file names map to `127.0.0.1` only, which is always listened on.
    pub ipv6_loopback: bool,
    pub threads_per_child: u32,
    /// Environment for php-cgi.exe. On Windows mod_fcgid starts PHP with *only* the variables
    /// given by `FcgidInitialEnv`; without `SystemRoot` Winsock (and so curl, MySQL, sockets)
    /// fails, without `ComSpec` `exec()` fails, and Composer needs `APPDATA`.
    pub php_env: Vec<EnvVar>,
}

/// Variables copied from Lampo's environment into PHP's.
const INHERITED_ENV: &[&str] = &[
    "SystemRoot",
    "SystemDrive",
    "windir",
    "ComSpec",
    "PATHEXT",
    "PATH",
    "USERPROFILE",
    "USERNAME",
    "USERDOMAIN",
    "COMPUTERNAME",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "CommonProgramFiles",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "OS",
];

/// Apache reads config lines into an 8 KiB buffer: keep long values (PATH) well below it.
const MAX_ENV_VALUE: usize = 4000;

impl RenderOptions {
    /// Captures the inherited variables from the current process environment.
    pub fn from_process_environment() -> Self {
        Self::with_env(INHERITED_ENV.iter().filter_map(|name| std::env::var(name).ok().map(|v| (name.to_string(), v))))
    }

    /// Default tuning with the given environment (sanitised; unusable values are dropped).
    pub fn with_env(vars: impl IntoIterator<Item = (String, String)>) -> Self {
        let php_env = vars.into_iter().filter_map(|(name, value)| sanitize_env(&name, &value).map(|value| EnvVar { name, value })).collect();
        Self { fcgid: FcgidTuning::default(), ipv6_loopback: true, threads_per_child: 64, php_env }
    }
}

/// Makes a value safe inside `"…"` on an Apache config line, or drops it.
fn sanitize_env(name: &str, value: &str) -> Option<String> {
    let name_ok = !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'(' | b')'));
    if !name_ok {
        return None;
    }
    let clean = |s: &str| !s.contains('"') && !s.contains("${") && !s.chars().any(char::is_control);
    if name.eq_ignore_ascii_case("PATH") {
        // Keep the usable entries, in order, up to the length budget.
        let mut out = String::new();
        for entry in value.split(';').map(|e| e.trim().trim_end_matches('\\')).filter(|e| !e.is_empty() && clean(e)) {
            if out.len() + entry.len() + 1 > MAX_ENV_VALUE {
                break;
            }
            if !out.is_empty() {
                out.push(';');
            }
            out.push_str(entry);
        }
        return Some(out);
    }
    // A trailing backslash would escape the closing quote.
    let v = value.trim_end_matches('\\');
    (clean(v) && v.len() <= MAX_ENV_VALUE).then(|| v.to_string())
}

#[derive(Serialize)]
struct Module {
    name: &'static str,
    path: String,
}

#[derive(Serialize)]
struct MainView {
    server_root: String,
    vhosts_glob: String,
    listen: Vec<String>,
    catch_all_addresses: String,
    catch_all_name: &'static str,
    modules: Vec<Module>,
    pid_file: String,
    /// Quoted `ErrorLog` file.
    error_log: String,
    threads_per_child: u32,
    fcgid: FcgidTuning,
}

#[derive(Serialize)]
struct VhostBlock {
    addresses: String,
    server_name: String,
    aliases: Vec<String>,
}

#[derive(Serialize)]
struct VhostView {
    site: String,
    notices: Vec<String>,
    blocks: Vec<VhostBlock>,
    document_root: String,
    /// Quoted `ErrorLog` / `CustomLog` files.
    error_log: String,
    access_log: String,
    php_label: String,
    fcgid_wrapper: String,
    env: Vec<EnvVar>,
    /// The project's own variables (`SetEnv`), values already quoted.
    project_env: Vec<EnvVar>,
    custom_directives: String,
}

/// Modules loaded from `${SRVROOT}/modules`: what typical `.htaccess` files of PHP frameworks
/// and CMSs use (rewrite, headers, expires, deflate, basic auth, Apache 2.2 `Order/Allow`).
const MODULES: &[(&str, &str)] = &[
    ("access_compat_module", "mod_access_compat.so"),
    ("alias_module", "mod_alias.so"),
    ("auth_basic_module", "mod_auth_basic.so"),
    ("authn_core_module", "mod_authn_core.so"),
    ("authn_file_module", "mod_authn_file.so"),
    ("authz_core_module", "mod_authz_core.so"),
    ("authz_groupfile_module", "mod_authz_groupfile.so"),
    ("authz_host_module", "mod_authz_host.so"),
    ("authz_user_module", "mod_authz_user.so"),
    ("autoindex_module", "mod_autoindex.so"),
    ("deflate_module", "mod_deflate.so"),
    ("dir_module", "mod_dir.so"),
    ("env_module", "mod_env.so"),
    ("expires_module", "mod_expires.so"),
    ("filter_module", "mod_filter.so"),
    ("headers_module", "mod_headers.so"),
    ("log_config_module", "mod_log_config.so"),
    ("mime_module", "mod_mime.so"),
    ("negotiation_module", "mod_negotiation.so"),
    ("rewrite_module", "mod_rewrite.so"),
    ("setenvif_module", "mod_setenvif.so"),
    ("version_module", "mod_version.so"),
];

/// Renders `httpd.conf` and the vhost files.
pub struct ApacheConfigRenderer {
    env: Environment<'static>,
    options: RenderOptions,
}

impl ApacheConfigRenderer {
    pub fn new(options: RenderOptions) -> Self {
        let mut env = Environment::new();
        env.set_auto_escape_callback(|_| minijinja::AutoEscape::None);
        env.set_undefined_behavior(UndefinedBehavior::Strict);
        env.set_trim_blocks(true);
        env.set_lstrip_blocks(true);
        env.set_keep_trailing_newline(true);
        env.add_template("httpd.conf", templates::httpd_conf()).expect("valid httpd.conf template");
        env.add_template("vhost.conf", templates::vhost_conf()).expect("valid vhost.conf template");
        Self { env, options }
    }

    pub fn options(&self) -> &RenderOptions {
        &self.options
    }

    /// `(path relative to the config dir, contents)` for every generated file, using
    /// `/` as separator: `httpd.conf` first, then `vhosts/<site>.conf` in site order.
    pub fn render(&self, config: &WebServerConfig) -> PortResult<Vec<(String, String)>> {
        let mut files = vec![(MAIN_CONF.to_string(), self.render_main(config)?)];
        for vhost in &config.vhosts {
            files.push((vhost_file_name(vhost), self.render_vhost(config, vhost)?));
        }
        Ok(files)
    }

    /// The main configuration. It includes `<config_dir>/vhosts/*.conf` unless `LAMPO_VHOSTS`
    /// is defined beforehand (see [`validation_wrapper`]).
    pub fn render_main(&self, config: &WebServerConfig) -> PortResult<String> {
        let server_root = apache_path(&config.server_root)?;
        let log_dir = apache_path(&config.log_dir)?;
        let mut modules: Vec<Module> = MODULES.iter().map(|(name, file)| Module { name, path: format!("${{SRVROOT}}/modules/{file}") }).collect();
        modules.push(Module { name: "fcgid_module", path: apache_path(&config.mod_fcgid)? });
        let listen = self.listen_addresses(config);
        let view = MainView {
            vhosts_glob: format!("{}/{VHOSTS_DIR}/*.conf", apache_path(&config.config_dir)?),
            catch_all_addresses: listen.join(" "),
            catch_all_name: CATCH_ALL_SERVER_NAME,
            listen,
            modules,
            pid_file: format!("{log_dir}/httpd.pid"),
            error_log: quoted(&format!("{log_dir}/error.log")),
            threads_per_child: self.options.threads_per_child,
            fcgid: self.options.fcgid.clone(),
            server_root,
        };
        self.env.get_template("httpd.conf").and_then(|t| t.render(minijinja::context! { main => view })).map_err(render_error)
    }

    /// One site's file.
    pub fn render_vhost(&self, config: &WebServerConfig, vhost: &VirtualHost) -> PortResult<String> {
        let mut notices = Vec::new();
        let mut blocks = Vec::new();
        // mod_fcgid tells process classes (and so their PHPRC) apart per virtual host; give
        // every site a unique ServerName even when it is reachable only on a loopback port.
        let internal_name = internal_server_name(vhost);
        if let Some((first, rest)) = vhost.server_names.split_first() {
            blocks.push(VhostBlock {
                addresses: self.shared_addresses(config.listen_port).join(" "),
                server_name: first.to_string(),
                aliases: rest.iter().map(ToString::to_string).collect(),
            });
        }
        match vhost.loopback_port {
            Some(port) if port == config.listen_port => notices.push(format!("loopback port {port} is the shared port: served by name only")),
            Some(port) => blocks.push(dedicated_port_block(vhost, port, internal_name)),
            None => {}
        }
        if blocks.is_empty() {
            notices.push("the site has no address: it is not served".into());
        }
        if vhost.tls == TlsMode::LocalCa {
            notices.push("HTTPS with the local CA is not implemented yet: served over HTTP".into());
        }

        let phprc = apache_path(&vhost.phprc_dir)?;
        let temp = apache_path(&site_layout::temp_dir(&vhost.phprc_dir))?;
        let mut env = vec![
            EnvVar { name: "PHPRC".into(), value: phprc },
            EnvVar { name: "PHP_FCGI_MAX_REQUESTS".into(), value: self.options.fcgid.max_requests_per_process.to_string() },
            EnvVar { name: "PHP_FCGI_CHILDREN".into(), value: "0".into() },
            EnvVar { name: "TEMP".into(), value: temp.clone() },
            EnvVar { name: "TMP".into(), value: temp },
        ];
        let reserved = |n: &str| env_reserved(n);
        env.extend(self.options.php_env.iter().filter(|v| !reserved(&v.name)).cloned());

        let php_cgi = apache_path(&vhost.php_cgi)?;
        let view = VhostView {
            site: vhost.site.to_string(),
            notices,
            blocks,
            document_root: apache_path(&vhost.document_root)?,
            error_log: quoted(&apache_path(&vhost.error_log)?),
            access_log: quoted(&apache_path(&vhost.access_log)?),
            php_label: php_label(&vhost.php_cgi),
            fcgid_wrapper: fcgid_wrapper_command(&php_cgi),
            env,
            project_env: project_env(vhost)?,
            custom_directives: rendered_directives(&vhost.custom_directives),
        };
        self.env.get_template("vhost.conf").and_then(|t| t.render(minijinja::context! { vhost => view })).map_err(render_error)
    }

    /// `Listen` arguments: the shared port on the loopback(s), then each dedicated port.
    fn listen_addresses(&self, config: &WebServerConfig) -> Vec<String> {
        let mut out = self.shared_addresses(config.listen_port);
        out.extend(config.listen_ports().into_iter().filter(|p| *p != config.listen_port).map(|p| format!("127.0.0.1:{p}")));
        out
    }

    fn shared_addresses(&self, port: Port) -> Vec<String> {
        let mut out = vec![format!("127.0.0.1:{port}")];
        if self.options.ipv6_loopback {
            out.push(format!("[::1]:{port}"));
        }
        out
    }
}

/// `vhosts/<site-file-key>.conf`.
pub fn vhost_file_name(vhost: &VirtualHost) -> String {
    format!("{VHOSTS_DIR}/{}.conf", vhost.site.file_key())
}

/// A wrapper main config that points `LAMPO_VHOSTS` at `vhosts_glob` and includes `main_conf`.
/// Used to validate a staged tree (or a single vhost) with `httpd -t -f <wrapper>`.
pub fn validation_wrapper(main_conf: &Path, vhosts_glob: &str) -> PortResult<String> {
    Ok(format!("# Generated by Lampo for `httpd -t`.\nDefine LAMPO_VHOSTS \"{vhosts_glob}\"\nInclude \"{}\"\n", apache_path(main_conf)?))
}

/// Name of the catch-all vhost of `httpd.conf`: never resolvable (RFC 2606), so no browser
/// request carries it and it claims no host name a site could use.
const CATCH_ALL_SERVER_NAME: &str = "catch-all.lampo.invalid";

/// Host names that reach a dedicated loopback port besides the site's own names. Any other
/// `Host` (a DNS-rebound domain) gets the catch-all's 404.
const LOOPBACK_HOST_NAMES: &[&str] = &["localhost", "127.0.0.1"];

/// `<VirtualHost 127.0.0.1:<port>>`: answers to every name of the site (or its internal name
/// when it has none) and to the loopback names. Apache matches them without the `:port`.
fn dedicated_port_block(vhost: &VirtualHost, port: Port, internal_name: String) -> VhostBlock {
    let mut names = vhost.server_names.iter().map(ToString::to_string);
    let server_name = names.next().unwrap_or(internal_name);
    let mut aliases: Vec<String> = names.collect();
    let loopback = LOOPBACK_HOST_NAMES.iter().filter(|l| !vhost.server_names.iter().any(|n| n.as_str().eq_ignore_ascii_case(l)));
    aliases.extend(loopback.map(ToString::to_string));
    VhostBlock { addresses: format!("127.0.0.1:{port}"), server_name, aliases }
}

/// `feature-x.shop.lampo.invalid` / `shop.lampo.invalid`: unique, never resolvable (RFC 2606).
fn internal_server_name(vhost: &VirtualHost) -> String {
    match &vhost.site {
        lampo_domain::site::SiteId::Project(p) => format!("{p}.lampo.invalid"),
        lampo_domain::site::SiteId::Worktree { parent, slug } => format!("{slug}.{parent}.lampo.invalid"),
    }
}

/// The project's variables as `SetEnv` arguments (request environment: `getenv()`, `$_SERVER`).
/// A name Lampo manages (only in a file written by hand) is skipped, so it never shadows the
/// `FcgidInitialEnv` values above.
fn project_env(vhost: &VirtualHost) -> PortResult<Vec<EnvVar>> {
    vhost.env_vars.effective().map(|(name, value)| Ok(EnvVar { name: name.to_string(), value: quoted_text(value)? })).collect()
}

/// The custom directives as they go into the vhost: verbatim without the trailing line feeds,
/// empty when blank (no section is rendered). The last line keeps its own spaces and tabs, so
/// a `\` followed by a blank does not continue it, as `ApacheDirectives::validate` reads it.
/// Every other logical line reaches Apache as validated; only the last one, when it still ends
/// in `\`, continues into the fixed `# --- End of custom directives ---` comment, which never
/// ends in `\` and so closes it before Lampo's own directives.
fn rendered_directives(directives: &ApacheDirectives) -> String {
    if directives.is_empty() {
        return String::new();
    }
    directives.as_str().trim_end_matches('\n').to_string()
}

/// Variables Lampo sets per site; inherited values with these names are ignored.
fn env_reserved(name: &str) -> bool {
    ["PHPRC", "PHP_FCGI_MAX_REQUESTS", "PHP_FCGI_CHILDREN", "TEMP", "TMP"].iter().any(|r| r.eq_ignore_ascii_case(name))
}

/// Human hint for the comment line: the PHP install folder name (`php-8.3.14`).
fn php_label(php_cgi: &Path) -> String {
    php_cgi.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn render_error(e: minijinja::Error) -> PortError {
    PortError::invalid_data("Apache configuration template", e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apache::tests_support::{sample_config, sample_options, vhost};
    use crate::test_snapshot::assert_snapshot;

    fn render(config: &WebServerConfig) -> Vec<(String, String)> {
        ApacheConfigRenderer::new(sample_options()).render(config).unwrap()
    }

    #[test]
    fn snapshot_of_a_complete_configuration() {
        let files = render(&sample_config());
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["httpd.conf", "vhosts/legacy.conf", "vhosts/shop.conf", "vhosts/shop@feature-x.conf"]);
        for (name, text) in &files {
            assert_snapshot(&format!("apache-{}", name.replace('/', "-")), text);
        }
    }

    #[test]
    fn listens_on_loopback_for_shared_and_dedicated_ports() {
        let main = &render(&sample_config())[0].1;
        let listens: Vec<&str> = main.lines().filter(|l| l.starts_with("Listen ")).collect();
        assert_eq!(listens, ["Listen 127.0.0.1:80", "Listen [::1]:80", "Listen 127.0.0.1:8080", "Listen 127.0.0.1:8101"]);
        let mut opts = sample_options();
        opts.ipv6_loopback = false;
        let main = ApacheConfigRenderer::new(opts).render_main(&sample_config()).unwrap();
        assert!(!main.contains("[::1]"));
    }

    #[test]
    fn worktree_gets_a_unique_internal_server_name_and_its_own_phprc() {
        let files = render(&sample_config());
        let wt = &files[3].1;
        assert!(wt.contains("<VirtualHost 127.0.0.1:8101>"));
        assert!(wt.contains("ServerName feature-x.shop.lampo.invalid"));
        assert!(wt.contains(r#"FcgidInitialEnv PHPRC "C:/Lampo Data/generated/php/shop@feature-x""#));
        // Paths with spaces are double-quoted inside the FcgidWrapper argument.
        assert!(wt.contains(r#"FcgidWrapper "\"C:/Lampo Data/packages/php-8.3.14/php-cgi.exe\"" .php"#));
    }

    #[test]
    fn site_with_names_and_port_gets_two_blocks() {
        let mut config = sample_config();
        config.vhosts = vec![vhost("blog", &["blog.test"], Some(8200), r"C:\blog")];
        let text = &render(&config)[1].1;
        assert_eq!(text.matches("<VirtualHost").count(), 2);
        assert!(text.contains("<VirtualHost 127.0.0.1:80 [::1]:80>"));
        assert!(text.contains("<VirtualHost 127.0.0.1:8200>"));
        assert_eq!(text.matches("ServerName blog.test").count(), 2);
    }

    #[test]
    fn port_equal_to_shared_port_is_not_duplicated() {
        let mut config = sample_config();
        config.vhosts = vec![vhost("blog", &["blog.test"], Some(80), r"C:\blog")];
        let files = render(&config);
        assert_eq!(files[0].1.matches("Listen 127.0.0.1:80\n").count(), 1);
        assert_eq!(files[1].1.matches("<VirtualHost").count(), 1);
    }

    #[test]
    fn rejects_paths_apache_cannot_quote() {
        let mut config = sample_config();
        config.vhosts[0].document_root = std::path::PathBuf::from(r"C:\x${SRVROOT}");
        assert!(matches!(ApacheConfigRenderer::new(sample_options()).render(&config), Err(PortError::InvalidData { .. })));
    }

    #[test]
    fn project_variables_are_set_escaped_after_lampo_ones() {
        let files = render(&sample_config());
        let shop = &files[2].1;
        let lines: Vec<&str> = shop.lines().map(str::trim).filter(|l| l.starts_with("SetEnv ")).collect();
        assert_eq!(lines, ["SetEnv APP_ENV \"local\"", r#"SetEnv APP_NAME "My \"Shop\"""#, r#"SetEnv STORAGE_DIR "D:\\shop data\\""#]);
        assert!(shop.find("SetEnv APP_ENV").unwrap() > shop.rfind("FcgidInitialEnv").unwrap());
        assert!(!files[1].1.contains("SetEnv"), "a site without variables gets none");
    }

    #[test]
    fn a_stored_reserved_variable_never_shadows_lampo_ones() {
        let mut config = sample_config();
        config.vhosts[1].env_vars = serde_json::from_str(r#"{"PHPRC": "C:/evil", "TEMP": "C:/x", "APP_ENV": "local"}"#).unwrap();
        let shop = ApacheConfigRenderer::new(sample_options()).render_vhost(&config, &config.vhosts[1]).unwrap();
        let set: Vec<&str> = shop.lines().map(str::trim).filter(|l| l.starts_with("SetEnv ")).collect();
        assert_eq!(set, ["SetEnv APP_ENV \"local\""]);
    }

    #[test]
    fn php_environment_is_sanitised() {
        let opts = sample_options();
        let names: Vec<&str> = opts.php_env.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["SystemRoot", "ComSpec", "PATH"], "APPDATA with a quote is dropped");
        let path = &opts.php_env[2].value;
        assert_eq!(path, r"C:\WINDOWS\system32;C:\WINDOWS;C:\Program Files\Git\cmd");
    }

    #[test]
    fn validation_wrapper_points_at_the_staged_vhosts() {
        let text = validation_wrapper(Path::new(r"C:\a b\apache.staging\httpd.conf"), "C:/a b/apache.staging/vhosts/*.conf").unwrap();
        assert_eq!(text, "# Generated by Lampo for `httpd -t`.\nDefine LAMPO_VHOSTS \"C:/a b/apache.staging/vhosts/*.conf\"\nInclude \"C:/a b/apache.staging/httpd.conf\"\n");
    }
}
