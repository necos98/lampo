use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use lampo_domain::ports::{CommandRunner, WebServerConfigurator};
use lampo_domain::web::WebServerConfig;
use lampo_domain::{PortError, PortResult};
use lampo_infra_fs::ensure_dir;

use super::render::{ApacheConfigRenderer, MAIN_CONF, RenderOptions, VHOSTS_DIR, validation_wrapper};
use super::staging::{ConfigTree, VALIDATION_WRAPPER};
use super::syntax::apache_path;
use super::validation::{HttpdValidator, rejection_report};

/// Renders into `<config_dir>.staging`, runs `httpd.exe -t` against the staged tree, and on
/// success swaps the staging dir in (keeping `<config_dir>.previous`). Composes the pure
/// [`ApacheConfigRenderer`], the [`HttpdValidator`] (via `CommandRunner`) and the on-disk
/// [`ConfigTree`]. Applies are serialised.
pub struct ApacheConfigurator {
    renderer: ApacheConfigRenderer,
    validator: HttpdValidator,
    lock: tokio::sync::Mutex<()>,
}

impl ApacheConfigurator {
    /// Uses the default tuning and captures PHP's environment from the current process.
    pub fn new(commands: Arc<dyn CommandRunner>) -> Self {
        Self::with_options(commands, RenderOptions::from_process_environment())
    }

    pub fn with_options(commands: Arc<dyn CommandRunner>, options: RenderOptions) -> Self {
        Self { renderer: ApacheConfigRenderer::new(options), validator: HttpdValidator::new(commands), lock: tokio::sync::Mutex::new(()) }
    }

    /// Pure rendering (unit-testable without Apache): `(relative path, contents)` pairs.
    pub fn render(&self, config: &WebServerConfig) -> PortResult<Vec<(String, String)>> {
        self.renderer.render(config)
    }
}

#[async_trait]
impl WebServerConfigurator for ApacheConfigurator {
    async fn apply(&self, config: &WebServerConfig) -> PortResult<()> {
        let files = self.render(config)?;
        let _serialised = self.lock.lock().await;
        let tree = ConfigTree::new(&config.config_dir);

        // Stage (and create the folders Apache will need to open its logs at start-up).
        let staged = {
            let (tree, files, config) = (tree.clone(), files.clone(), config.clone());
            blocking(move || {
                prepare_runtime_dirs(&config)?;
                if tree.live_matches(&files) {
                    return Ok(None);
                }
                let staging = tree.stage(&files)?;
                let glob = format!("{}/{VHOSTS_DIR}/*.conf", apache_path(&staging)?);
                let wrapper = tree.write_staged(VALIDATION_WRAPPER, &validation_wrapper(&staging.join(MAIN_CONF), &glob)?)?;
                Ok(Some((staging, wrapper)))
            })
            .await?
        };
        let Some((staging, wrapper)) = staged else {
            tracing::debug!("Apache configuration unchanged");
            return Ok(());
        };

        let verdict = self.validator.check(&config.server_root, &wrapper).await;
        let promote_tree = tree.clone();
        match verdict {
            Ok(None) => blocking(move || promote_tree.promote()).await,
            Ok(Some(output)) => {
                let _ = blocking(move || promote_tree.discard_staging()).await;
                Err(PortError::ConfigRejected { output: rejection_report(&output, &staging, config) })
            }
            Err(e) => {
                let _ = blocking(move || promote_tree.discard_staging()).await;
                Err(e)
            }
        }
    }
}

/// Apache refuses to start when a log file's folder is missing (`httpd -t` does not check it).
fn prepare_runtime_dirs(config: &WebServerConfig) -> PortResult<()> {
    ensure_dir(&config.log_dir)?;
    let parents = config.vhosts.iter().flat_map(|v| [v.error_log.parent(), v.access_log.parent()]).flatten();
    for dir in parents {
        ensure_dir(dir)?;
    }
    Ok(())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> PortResult<T> + Send + 'static) -> PortResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| PortError::io("generating the Apache configuration", e))?
}

/// Where the live configuration is read from (`<config_dir>\httpd.conf`).
pub fn main_conf_path(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join(MAIN_CONF)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Mutex;
    use std::time::Duration;

    use lampo_domain::ports::{CommandOutput, CommandSpec};

    use super::*;
    use crate::apache::tests_support::{sample_config, sample_options};

    /// Plays `httpd -t`: rejects any staged vhost containing `Bogus`.
    #[derive(Default)]
    struct FakeHttpd {
        runs: Mutex<Vec<CommandSpec>>,
    }

    #[async_trait]
    impl CommandRunner for FakeHttpd {
        async fn run(&self, spec: CommandSpec, _timeout: Duration) -> PortResult<CommandOutput> {
            self.runs.lock().unwrap().push(spec.clone());
            let wrapper = std::fs::read_to_string(&spec.args[2]).unwrap();
            assert!(wrapper.contains(".staging/vhosts/*.conf"), "validates the staged tree");
            let staging = Path::new(&spec.args[2]).parent().unwrap().to_path_buf();
            for entry in std::fs::read_dir(staging.join("vhosts")).unwrap() {
                let path = entry.unwrap().path();
                if std::fs::read_to_string(&path).unwrap().contains("Bogus") {
                    let shown = path.display().to_string().replace('\\', "/");
                    return Ok(CommandOutput { exit_code: Some(1), stdout: String::new(), stderr: format!("AH00526: Syntax error on line 30 of {shown}:\nInvalid command 'Bogus'") });
                }
            }
            Ok(CommandOutput { exit_code: Some(0), stdout: String::new(), stderr: "Syntax OK".into() })
        }
    }

    fn setup() -> (tempfile::TempDir, WebServerConfig, Arc<FakeHttpd>, ApacheConfigurator) {
        let dir = tempfile::tempdir().unwrap();
        let mut config = sample_config();
        config.server_root = dir.path().join("apache 2.4");
        std::fs::create_dir_all(config.server_root.join("bin")).unwrap();
        std::fs::write(config.server_root.join("bin").join("httpd.exe"), b"").unwrap();
        config.config_dir = dir.path().join("generated").join("apache");
        config.log_dir = dir.path().join("logs").join("apache");
        for v in &mut config.vhosts {
            v.error_log = dir.path().join("logs").join(v.site.file_key()).join("error.log");
            v.access_log = dir.path().join("logs").join(v.site.file_key()).join("access.log");
        }
        let httpd = Arc::new(FakeHttpd::default());
        let configurator = ApacheConfigurator::with_options(httpd.clone(), sample_options());
        (dir, config, httpd, configurator)
    }

    #[tokio::test]
    async fn valid_config_is_promoted_and_unchanged_config_is_not_revalidated() {
        let (_dir, config, httpd, configurator) = setup();
        configurator.apply(&config).await.unwrap();
        let live = &config.config_dir;
        assert!(live.join("httpd.conf").is_file());
        assert!(live.join("vhosts").join("shop@feature-x.conf").is_file());
        assert!(!live.join("_lampo-validate.conf").exists());
        assert!(config.log_dir.is_dir());
        assert!(config.vhosts[0].error_log.parent().unwrap().is_dir());
        let run = httpd.runs.lock().unwrap()[0].clone();
        assert_eq!(run.args[..2], ["-t".to_string(), "-f".to_string()]);
        assert_eq!(run.working_dir.as_deref(), Some(config.server_root.as_path()));

        configurator.apply(&config).await.unwrap();
        assert_eq!(httpd.runs.lock().unwrap().len(), 1);

        // Removing a site removes its file.
        let mut fewer = config.clone();
        fewer.vhosts.pop();
        configurator.apply(&fewer).await.unwrap();
        assert!(!live.join("vhosts").join("shop@feature-x.conf").exists());
        assert!(config.config_dir.with_file_name("apache.previous").join("vhosts").join("shop@feature-x.conf").exists());
    }

    #[tokio::test]
    async fn rejected_config_leaves_the_live_tree_untouched() {
        let (_dir, config, _httpd, configurator) = setup();
        configurator.apply(&config).await.unwrap();
        let before = std::fs::read_to_string(config.config_dir.join("vhosts").join("shop.conf")).unwrap();

        let mut broken = config.clone();
        broken.vhosts[1].custom_directives = lampo_domain::project::ApacheDirectives::new("Bogus on").unwrap();
        let err = configurator.apply(&broken).await.unwrap_err();
        let PortError::ConfigRejected { output } = err else { panic!("{err:?}") };
        assert!(output.starts_with("Site `shop`:\n"), "{output}");
        assert!(!output.contains(".staging"), "{output}");
        assert_eq!(std::fs::read_to_string(config.config_dir.join("vhosts").join("shop.conf")).unwrap(), before);
        assert!(!config.config_dir.with_file_name("apache.staging").exists());
    }

    #[tokio::test]
    async fn missing_httpd_is_not_found() {
        let (_dir, mut config, _httpd, configurator) = setup();
        config.server_root = config.server_root.join("nope");
        assert!(matches!(configurator.apply(&config).await, Err(PortError::NotFound { .. })));
        assert!(!config.config_dir.exists());
    }
}
