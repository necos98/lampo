//! Rotation of the logs nobody else rotates while Lampo runs: PHP's `error_log` files (one per
//! site, plus the CLI one). PHP reopens the file for every entry, so moving it aside is safe at
//! any time; a site's log is also rotated when its php.ini is written. Apache's logs are rotated
//! when its configuration is applied and the captured service output as it is written, so
//! neither is listed here.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use lampo_domain::logs::LogSource;
use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::{LogRotator, ProjectRepository, Timer, WorktreeRepository};

use super::support::all_sites;
use crate::error::AppResult;

/// How often the PHP logs are checked while the app runs.
pub const LOG_ROTATION_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Rotates every PHP error log that grew past the size limit, every [`LOG_ROTATION_INTERVAL`].
pub struct PeriodicLogRotation {
    rotator: Arc<dyn LogRotator>,
    timer: Arc<dyn Timer>,
    projects: Arc<dyn ProjectRepository>,
    worktrees: Arc<dyn WorktreeRepository>,
    paths: LampoPaths,
}

impl PeriodicLogRotation {
    pub fn new(rotator: Arc<dyn LogRotator>, timer: Arc<dyn Timer>, projects: Arc<dyn ProjectRepository>, worktrees: Arc<dyn WorktreeRepository>, paths: LampoPaths) -> Self {
        Self { rotator, timer, projects, worktrees, paths }
    }

    /// Runs for the lifetime of the app (spawned once by the composition root).
    pub async fn run(&self) {
        loop {
            self.timer.sleep(LOG_ROTATION_INTERVAL).await;
            self.rotate_now();
        }
    }

    /// One pass over the PHP logs. Failures are logged and the other files still rotated: an
    /// unrotated log only keeps growing until the next pass.
    pub fn rotate_now(&self) {
        let files = match self.php_logs() {
            Ok(files) => files,
            Err(e) => {
                tracing::warn!("PHP logs not rotated: the sites could not be listed: {}", e.message);
                return;
            }
        };
        for file in files {
            if let Err(e) = self.rotator.rotate_if_large(&file) {
                tracing::warn!("{e}");
            }
        }
    }

    /// The `error_log` of every site (projects and worktree environments) and the CLI one.
    fn php_logs(&self) -> AppResult<Vec<PathBuf>> {
        let projects = self.projects.list()?;
        let worktrees = self.worktrees.list_all()?;
        let sites = all_sites(&projects, &worktrees);
        let mut files: Vec<PathBuf> = sites.into_iter().map(|site| LogSource::SitePhp(site.id).file(&self.paths)).collect();
        files.push(self.paths.cli_php_error_log());
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Mutex;

    use lampo_domain::project::{ProjectId, ProjectRoot};
    use lampo_domain::tcp_port::Port;
    use lampo_domain::worktree::{DatabaseMode, Occupancy, WorktreeEnvironment};
    use lampo_domain::{PortError, PortResult};

    use super::*;
    use crate::testing::*;

    /// Records the files it is asked to rotate; fails on the ones named in `failing`.
    #[derive(Default)]
    struct RecordingRotator {
        files: Mutex<Vec<PathBuf>>,
        failing: Vec<PathBuf>,
    }

    impl LogRotator for RecordingRotator {
        fn rotate_if_large(&self, file: &Path) -> PortResult<()> {
            self.files.lock().unwrap().push(file.to_path_buf());
            match self.failing.iter().any(|f| f == file) {
                true => Err(PortError::io(format!("rotating {}", file.display()), "locked")),
                false => Ok(()),
            }
        }
    }

    fn rotation(w: &World, rotator: Arc<RecordingRotator>) -> PeriodicLogRotation {
        PeriodicLogRotation::new(rotator, Arc::new(ManualClock::default()), w.projects.clone(), w.worktrees.clone(), w.paths.clone())
    }

    fn php_log(w: &World, site: &str) -> PathBuf {
        LogSource::SitePhp(site.parse().unwrap()).file(&w.paths)
    }

    fn worktree_of_shop(w: &World) -> WorktreeEnvironment {
        let shop = w.projects.get(&ProjectId::new("shop").unwrap()).unwrap().unwrap();
        let root = ProjectRoot::new(r"C:\sites\shop-feature").unwrap();
        let env = WorktreeEnvironment::derive(&shop, &Occupancy::default(), root, Some("feature-x".into()), Port::of(8101), &DatabaseMode::Shared, 1).unwrap();
        w.worktrees.save(&env).unwrap();
        env
    }

    #[test]
    fn rotates_the_php_log_of_every_site_and_the_cli_one() {
        let w = World::new();
        w.project("shop", r"C:\sites\shop", true);
        w.project("blog", r"C:\sites\blog", false);
        let worktree = worktree_of_shop(&w);
        let rotator = Arc::new(RecordingRotator::default());
        rotation(&w, rotator.clone()).rotate_now();
        let files = rotator.files.lock().unwrap().clone();
        assert_eq!(files, [php_log(&w, "blog"), php_log(&w, "shop"), LogSource::SitePhp(worktree.site_id()).file(&w.paths), w.paths.cli_php_error_log()]);
    }

    #[test]
    fn a_failure_does_not_stop_the_other_rotations() {
        let w = World::new();
        w.project("blog", r"C:\sites\blog", true);
        w.project("shop", r"C:\sites\shop", true);
        let rotator = Arc::new(RecordingRotator { failing: vec![php_log(&w, "blog")], ..Default::default() });
        rotation(&w, rotator.clone()).rotate_now();
        assert_eq!(rotator.files.lock().unwrap().len(), 3);
    }
}
