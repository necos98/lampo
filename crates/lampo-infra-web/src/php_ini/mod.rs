//! Per-site php.ini.
//!
//! The file is the PHP build's own `php.ini-development`, verbatim, followed by a `[Lampo]`
//! section (later assignments win in php.ini): `extension_dir`, the selected extensions, the
//! site's error log / session / temp paths, development defaults, Xdebug defaults for the
//! right Xdebug generation, then the merged Lampo + project overrides (`spec.values`).

pub mod syntax;
pub mod xdebug;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use lampo_domain::php::{ExtensionName, PhpMinor};
use lampo_domain::ports::PhpIniWriter;
use lampo_domain::web::{ExtensionLoad, PhpIniSpec};
use lampo_domain::PortResult;
use lampo_infra_fs::log_rotation::{MAX_LOG_BYTES, rotate_if_large};
use lampo_infra_fs::{ensure_dir, write_atomic};

use self::syntax::{ini_path, ini_value};
use self::xdebug::XdebugGeneration;
use crate::site_layout;

/// The template shipped in every PHP Windows zip.
pub const BASE_TEMPLATE: &str = "php.ini-development";

/// Lampo's defaults, written before `spec.values` (which override them).
const DEV_DEFAULTS: &[(&str, &str)] = &[("log_errors", "On"), ("date.timezone", "UTC"), ("upload_max_filesize", "256M"), ("post_max_size", "256M"), ("cgi.fix_pathinfo", "1")];

/// Facts read from disk that rendering depends on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IniInputs {
    /// Contents of `<php_dir>\php.ini-development`, if present.
    pub base: Option<String>,
    /// Requested extensions whose DLL does not exist: written commented out, so PHP does not
    /// print a start-up warning on every page.
    pub missing: Vec<ExtensionName>,
}

/// Renders and writes php.ini files. Rendering ([`PhpIniRenderer::render`]) is pure; the
/// [`PhpIniWriter`] implementation gathers [`IniInputs`] from disk, writes atomically and rotates the
/// site's error log when it is too large.
#[derive(Debug, Default, Clone, Copy)]
pub struct PhpIniRenderer;

impl PhpIniRenderer {
    /// Pure rendering, unit-testable.
    pub fn render(&self, spec: &PhpIniSpec, inputs: &IniInputs) -> String {
        let mut out = String::new();
        if let Some(base) = &inputs.base {
            out.push_str(base);
            if !base.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
        }
        let rule = "; ============================================================================\n";
        out.push_str(rule);
        let _ = writeln!(out, "; Lampo settings for site `{}` (PHP {}). Generated: do not edit,", spec.site, spec.php);
        out.push_str("; change the PHP settings of the project in Lampo instead.\n");
        out.push_str(rule);
        out.push_str("[Lampo]\n");
        let _ = writeln!(out, "extension_dir = {}", ini_path(&spec.php_dir.join("ext")));

        out.push_str("\n; Extensions\n");
        let mut extensions: Vec<&ExtensionLoad> = spec.extensions.iter().collect();
        // Plain extensions first, then Zend ones with Xdebug last (it must load after OPcache).
        extensions.sort_by_key(|e| (e.zend, e.name.as_str() == "xdebug"));
        for ext in &extensions {
            let line = extension_line(ext, spec.php, &spec.php_dir);
            if inputs.missing.contains(&ext.name) {
                let _ = writeln!(out, "; {line}  ; not installed");
            } else {
                let _ = writeln!(out, "{line}");
            }
        }

        out.push_str("\n; Files of this site\n");
        let temp = site_layout::temp_dir(&spec.output_dir);
        for (key, path) in [
            ("error_log", spec.error_log.clone()),
            ("session.save_path", site_layout::session_dir(&spec.output_dir)),
            ("upload_tmp_dir", temp.clone()),
            ("sys_temp_dir", temp.clone()),
            ("soap.wsdl_cache_dir", temp),
        ] {
            let _ = writeln!(out, "{key} = {}", ini_path(&path));
        }

        out.push_str("\n; Development defaults\n");
        for (k, v) in DEV_DEFAULTS {
            let _ = writeln!(out, "{k} = {v}");
        }

        let xdebug = spec.extensions.iter().find(|e| e.name.as_str() == "xdebug" && !inputs.missing.contains(&e.name)).map(|e| XdebugGeneration::detect(e, spec.php));
        if let Some(generation) = xdebug {
            out.push_str("\n; Xdebug\n");
            for (k, v) in generation.defaults() {
                let _ = writeln!(out, "{k} = {v}");
            }
        }

        if !spec.values.is_empty() {
            out.push_str("\n; Lampo and project settings\n");
            for (key, value) in &spec.values {
                let pairs = match xdebug {
                    Some(g) if key.as_str().starts_with("xdebug.") => g.translate(key.as_str(), value.as_str()),
                    _ => vec![(key.to_string(), value.as_str().to_string())],
                };
                for (k, v) in pairs {
                    let _ = writeln!(out, "{k} = {}", ini_value(&v));
                }
            }
        }
        out
    }

    /// Reads what [`Self::render`] needs from the PHP installation.
    pub fn gather_inputs(&self, spec: &PhpIniSpec) -> IniInputs {
        let base_path = spec.php_dir.join(BASE_TEMPLATE);
        let base = match std::fs::read(&base_path) {
            Ok(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
            Err(e) => {
                tracing::warn!("{} not readable ({e}): php.ini of `{}` starts from PHP's built-in defaults", base_path.display(), spec.site);
                None
            }
        };
        let missing = spec.extensions.iter().filter(|e| !extension_dll(e, &spec.php_dir).is_file()).map(|e| e.name.clone()).collect();
        IniInputs { base, missing }
    }
}

impl PhpIniWriter for PhpIniRenderer {
    fn write(&self, spec: &PhpIniSpec) -> PortResult<()> {
        let text = self.render(spec, &self.gather_inputs(spec));
        ensure_dir(&site_layout::session_dir(&spec.output_dir))?;
        ensure_dir(&site_layout::temp_dir(&spec.output_dir))?;
        if let Some(dir) = spec.error_log.parent() {
            ensure_dir(dir)?;
        }
        // PHP opens the log for every entry, so it can be moved aside while sites run too
        // (`PeriodicLogRotation` in the application does that).
        if let Err(e) = rotate_if_large(&spec.error_log, MAX_LOG_BYTES) {
            tracing::warn!("cannot rotate {}: {e}", spec.error_log.display());
        }
        write_atomic(&spec.output_dir.join("php.ini"), text.as_bytes())
    }
}

/// Short names (`extension=intl`) exist since PHP 7.2; older series need the DLL file name,
/// and Zend extensions there are safest with an absolute path.
fn uses_short_names(php: PhpMinor) -> bool {
    php >= PhpMinor::new(7, 2).expect("valid")
}

fn extension_line(ext: &ExtensionLoad, php: PhpMinor, php_dir: &Path) -> String {
    let directive = if ext.zend { "zend_extension" } else { "extension" };
    let value = match &ext.dll {
        Some(dll) => ini_path(dll),
        None if uses_short_names(php) => ext.name.to_string(),
        None if ext.zend => ini_path(&extension_dll(ext, php_dir)),
        None => format!("php_{}.dll", ext.name),
    };
    format!("{directive}={value}")
}

/// The DLL an extension loads from.
fn extension_dll(ext: &ExtensionLoad, php_dir: &Path) -> PathBuf {
    ext.dll.clone().unwrap_or_else(|| php_dir.join("ext").join(format!("php_{}.dll", ext.name)))
}

#[cfg(test)]
mod tests {
    use lampo_domain::php::php_ini_overrides;
    use lampo_domain::web::is_zend_extension;

    use super::*;
    use crate::test_snapshot::assert_snapshot;

    fn ext(name: &str, dll: Option<&str>) -> ExtensionLoad {
        let name = ExtensionName::new(name).unwrap();
        ExtensionLoad { zend: is_zend_extension(&name), dll: dll.map(PathBuf::from), name }
    }

    fn spec(php: &str, php_dir: &str, extensions: Vec<ExtensionLoad>, values: &[(&str, &str)]) -> PhpIniSpec {
        PhpIniSpec {
            site: "shop".parse().unwrap(),
            php: PhpMinor::parse(php).unwrap(),
            php_dir: PathBuf::from(php_dir),
            output_dir: PathBuf::from(r"C:\Lampo Data\generated\php\shop"),
            extensions,
            values: php_ini_overrides(values.iter().copied()).unwrap(),
            error_log: PathBuf::from(r"C:\Lampo Data\logs\projects\shop\php_errors.log"),
        }
    }

    #[test]
    fn php83_with_xdebug3() {
        let s = spec(
            "8.3",
            r"C:\Lampo Data\packages\php-8.3.14",
            vec![ext("xdebug", Some(r"C:\Lampo Data\packages\xdebug-3.3.2-php8.3\php_xdebug.dll")), ext("opcache", None), ext("intl", None), ext("pdo_mysql", None)],
            &[("memory_limit", "512M"), ("xdebug.client_port", "9009"), ("error_reporting", "E_ALL & ~E_DEPRECATED"), ("mail.log", r"C:\mail log\x.log")],
        );
        let inputs = IniInputs { base: Some("[PHP]\r\nmemory_limit = 128M\r\n".into()), missing: vec![] };
        assert_snapshot("php83-xdebug3.ini", &PhpIniRenderer.render(&s, &inputs));
    }

    #[test]
    fn php56_uses_dll_names_and_xdebug2() {
        let s = spec(
            "5.6",
            r"C:\Lampo\php-5.6.40",
            vec![ext("mbstring", None), ext("opcache", None), ext("xdebug", Some(r"C:\Lampo\xdebug-2.5.5-php5.6\php_xdebug.dll")), ext("redis", None)],
            &[("xdebug.mode", "debug"), ("xdebug.start_with_request", "yes")],
        );
        let inputs = IniInputs { base: None, missing: vec![ExtensionName::new("redis").unwrap()] };
        assert_snapshot("php56-xdebug2.ini", &PhpIniRenderer.render(&s, &inputs));
    }

    /// The Mailpit values the application merges in for sites using it render as plain
    /// directives `mail()` reads.
    #[test]
    fn mail_settings_render_as_plain_directives() {
        let s = spec("8.4", r"C:\Lampo\php-8.4.1", vec![], &[("SMTP", "127.0.0.1"), ("smtp_port", "1025"), ("sendmail_from", "lampo@localhost")]);
        let ini = PhpIniRenderer.render(&s, &IniInputs::default());
        for line in ["SMTP = 127.0.0.1\n", "smtp_port = 1025\n", "sendmail_from = lampo@localhost\n"] {
            assert!(ini.contains(line), "{line:?} missing from:\n{ini}");
        }
    }

    #[test]
    fn writer_uses_php_ini_development_and_creates_site_dirs() {
        let root = tempfile::tempdir().unwrap();
        let php_dir = root.path().join("php 8.3");
        std::fs::create_dir_all(php_dir.join("ext")).unwrap();
        std::fs::write(php_dir.join(BASE_TEMPLATE), "[PHP]\nengine = On\n").unwrap();
        std::fs::write(php_dir.join("ext").join("php_intl.dll"), b"").unwrap();
        let mut s = spec("8.3", php_dir.to_str().unwrap(), vec![ext("intl", None), ext("sodium", None)], &[]);
        s.output_dir = root.path().join("generated").join("php").join("shop");
        s.error_log = root.path().join("logs").join("shop").join("php_errors.log");
        PhpIniRenderer.write(&s).unwrap();
        let text = std::fs::read_to_string(s.output_dir.join("php.ini")).unwrap();
        assert!(text.starts_with("[PHP]\nengine = On\n"));
        assert!(text.contains("\nextension=intl\n"));
        assert!(text.contains("; extension=sodium  ; not installed"));
        assert!(s.output_dir.join("sessions").is_dir());
        assert!(s.output_dir.join("tmp").is_dir());
        assert!(root.path().join("logs").join("shop").is_dir());
    }
}
