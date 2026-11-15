//! Xdebug settings. Xdebug 3 replaced most Xdebug 2 settings (`xdebug.remote_enable` →
//! `xdebug.mode`, `remote_host` → `client_host`, port 9000 → 9003…). PHP 5.6–7.1 only have
//! Xdebug 2, so Lampo writes the right generation and translates project overrides written for
//! the other one.

use std::path::Path;

use lampo_domain::php::PhpMinor;
use lampo_domain::web::ExtensionLoad;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XdebugGeneration {
    V2,
    V3,
}

impl XdebugGeneration {
    /// From the add-on's install folder (`…\xdebug-3.3.2-php8.3\php_xdebug.dll`), else from the
    /// PHP version (Xdebug 3 requires PHP 7.2+).
    pub fn detect(load: &ExtensionLoad, php: PhpMinor) -> Self {
        let from_dir = load
            .dll
            .as_deref()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|n| n.to_str())
            .and_then(|n| n.to_ascii_lowercase().strip_prefix("xdebug-").and_then(|v| v.split('.').next().map(str::to_string)))
            .and_then(|major| major.parse::<u32>().ok());
        match from_dir {
            Some(2) => Self::V2,
            Some(_) => Self::V3,
            None if php < PhpMinor::new(7, 2).expect("valid") => Self::V2,
            None => Self::V3,
        }
    }

    /// Lampo's defaults: step debugging on request (cookie/query trigger), IDE on localhost.
    pub fn defaults(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::V3 => &[("xdebug.mode", "debug"), ("xdebug.start_with_request", "trigger"), ("xdebug.client_host", "127.0.0.1"), ("xdebug.client_port", "9003")],
            Self::V2 => &[("xdebug.remote_enable", "1"), ("xdebug.remote_autostart", "0"), ("xdebug.remote_host", "127.0.0.1"), ("xdebug.remote_port", "9000")],
        }
    }

    /// Rewrites one `xdebug.*` setting meant for the other generation. Returns the settings to
    /// write instead (possibly several, possibly none); other keys pass through unchanged.
    pub fn translate(self, key: &str, value: &str) -> Vec<(String, String)> {
        let one = |k: &str, v: &str| vec![(k.to_string(), v.to_string())];
        match (self, key) {
            (Self::V2, "xdebug.client_host") => one("xdebug.remote_host", value),
            (Self::V2, "xdebug.client_port") => one("xdebug.remote_port", value),
            (Self::V2, "xdebug.log") => one("xdebug.remote_log", value),
            (Self::V2, "xdebug.start_with_request") => one("xdebug.remote_autostart", if value.eq_ignore_ascii_case("yes") { "1" } else { "0" }),
            (Self::V2, "xdebug.output_dir") => vec![("xdebug.profiler_output_dir".into(), value.into()), ("xdebug.trace_output_dir".into(), value.into())],
            (Self::V2, "xdebug.mode") => {
                let modes: Vec<String> = value.split(',').map(|m| m.trim().to_ascii_lowercase()).collect();
                let has = |m: &str| if modes.iter().any(|x| x == m) { "1" } else { "0" };
                vec![("xdebug.remote_enable".into(), has("debug").into()), ("xdebug.profiler_enable_trigger".into(), has("profile").into())]
            }
            (Self::V3, "xdebug.remote_host") => one("xdebug.client_host", value),
            (Self::V3, "xdebug.remote_port") => one("xdebug.client_port", value),
            (Self::V3, "xdebug.remote_log") => one("xdebug.log", value),
            (Self::V3, "xdebug.remote_autostart") => one("xdebug.start_with_request", if is_on(value) { "yes" } else { "trigger" }),
            (Self::V3, "xdebug.remote_enable") => one("xdebug.mode", if is_on(value) { "debug" } else { "off" }),
            _ => one(key, value),
        }
    }
}

fn is_on(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "on" | "yes" | "true")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lampo_domain::php::ExtensionName;

    use super::*;

    fn load(dll: Option<&str>) -> ExtensionLoad {
        ExtensionLoad { name: ExtensionName::new("xdebug").unwrap(), zend: true, dll: dll.map(PathBuf::from) }
    }

    #[test]
    fn detects_generation() {
        let php = |s| PhpMinor::parse(s).unwrap();
        assert_eq!(XdebugGeneration::detect(&load(Some(r"C:\p\xdebug-2.9.8-php7.4\php_xdebug.dll")), php("7.4")), XdebugGeneration::V2);
        assert_eq!(XdebugGeneration::detect(&load(Some(r"C:\p\xdebug-3.3.2-php8.3\php_xdebug.dll")), php("8.3")), XdebugGeneration::V3);
        assert_eq!(XdebugGeneration::detect(&load(None), php("5.6")), XdebugGeneration::V2);
        assert_eq!(XdebugGeneration::detect(&load(None), php("7.2")), XdebugGeneration::V3);
    }

    #[test]
    fn translates_between_generations() {
        assert_eq!(XdebugGeneration::V2.translate("xdebug.client_port", "9003"), vec![("xdebug.remote_port".to_string(), "9003".to_string())]);
        assert_eq!(
            XdebugGeneration::V2.translate("xdebug.mode", "debug,develop"),
            vec![("xdebug.remote_enable".to_string(), "1".to_string()), ("xdebug.profiler_enable_trigger".to_string(), "0".to_string())]
        );
        assert_eq!(XdebugGeneration::V3.translate("xdebug.remote_autostart", "1"), vec![("xdebug.start_with_request".to_string(), "yes".to_string())]);
        assert_eq!(XdebugGeneration::V3.translate("xdebug.idekey", "PHPSTORM"), vec![("xdebug.idekey".to_string(), "PHPSTORM".to_string())]);
    }
}
