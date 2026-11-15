//! `httpd -t` through the `CommandRunner` port, and turning its output into a message the user
//! can act on (live paths instead of staging paths, the offending site named first).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use lampo_domain::ports::{CommandRunner, CommandSpec};
use lampo_domain::web::WebServerConfig;
use lampo_domain::{PortError, PortResult};

use super::render::vhost_file_name;
use super::syntax::apache_path;

/// `<server_root>\bin\httpd.exe`.
pub fn httpd_exe(server_root: &Path) -> PathBuf {
    server_root.join("bin").join("httpd.exe")
}

/// Runs `httpd.exe -t -f <conf>`.
pub struct HttpdValidator {
    commands: Arc<dyn CommandRunner>,
    timeout: Duration,
}

impl HttpdValidator {
    pub fn new(commands: Arc<dyn CommandRunner>) -> Self {
        Self { commands, timeout: Duration::from_secs(60) }
    }

    /// `Ok(None)` when the configuration is valid, `Ok(Some(output))` when Apache rejects it,
    /// `Err` when `httpd -t` could not be run at all.
    pub async fn check(&self, server_root: &Path, conf: &Path) -> PortResult<Option<String>> {
        let program = httpd_exe(server_root);
        if !program.is_file() {
            return Err(PortError::not_found("Apache executable", program.display().to_string()));
        }
        let spec = CommandSpec {
            program,
            args: vec!["-t".into(), "-f".into(), conf.display().to_string()],
            working_dir: Some(server_root.to_path_buf()),
            env: Vec::new(),
            path_prepend: Vec::new(),
        };
        let out = self.commands.run(spec, self.timeout).await?;
        if out.success() {
            return Ok(None);
        }
        let text = [out.stderr.trim(), out.stdout.trim()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
        Ok(Some(if text.is_empty() { format!("httpd -t failed with exit code {:?}", out.exit_code) } else { text }))
    }
}

/// Rewrites `httpd -t` output for the user: staging paths become the live ones, and when the
/// error is inside a site's file the output starts with `Site `<id>`:`.
pub fn rejection_report(output: &str, staging: &Path, config: &WebServerConfig) -> String {
    let mut text = output.to_string();
    for (from, to) in [(apache_path(staging), apache_path(&config.config_dir)), (Ok(staging.display().to_string()), Ok(config.config_dir.display().to_string()))] {
        if let (Ok(from), Ok(to)) = (from, to) {
            text = replace_ignore_ascii_case(&text, &from, &to);
        }
    }
    let lower = text.to_ascii_lowercase().replace('\\', "/");
    let culprit = config.vhosts.iter().find(|v| lower.contains(&format!("/{}", vhost_file_name(v).to_ascii_lowercase())));
    match culprit {
        Some(v) => format!("Site `{}`:\n{text}", v.site),
        None => text,
    }
}

/// Windows paths are case-insensitive and Apache may echo them with another drive-letter case.
fn replace_ignore_ascii_case(haystack: &str, from: &str, to: &str) -> String {
    if from.is_empty() {
        return haystack.to_string();
    }
    let lower_h = haystack.to_ascii_lowercase();
    let lower_f = from.to_ascii_lowercase();
    let mut out = String::with_capacity(haystack.len());
    let mut pos = 0;
    while let Some(i) = lower_h[pos..].find(&lower_f) {
        out.push_str(&haystack[pos..pos + i]);
        out.push_str(to);
        pos += i + from.len();
    }
    out.push_str(&haystack[pos..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apache::tests_support::sample_config;

    #[test]
    fn report_names_the_site_and_hides_staging() {
        let config = sample_config();
        let staging = PathBuf::from(r"C:\Lampo Data\generated\apache.staging");
        let output = "AH00526: Syntax error on line 40 of c:/Lampo Data/generated/apache.staging/vhosts/shop.conf:\nInvalid command 'Foo', perhaps misspelled";
        let report = rejection_report(output, &staging, &config);
        assert_eq!(report, "Site `shop`:\nAH00526: Syntax error on line 40 of C:/Lampo Data/generated/apache/vhosts/shop.conf:\nInvalid command 'Foo', perhaps misspelled");
    }

    #[test]
    fn global_errors_are_left_unattributed() {
        let config = sample_config();
        let report = rejection_report("Cannot load modules/mod_fcgid.so", Path::new(r"C:\x.staging"), &config);
        assert_eq!(report, "Cannot load modules/mod_fcgid.so");
    }
}
