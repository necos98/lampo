//! `%APPDATA%\Lampo\projects\<id>.worktrees\<slug>.toml`.

use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::WorktreeRepository;
use lampo_domain::project::{ProjectId, ProjectRoot};
use lampo_domain::site::WorktreeSlug;
use lampo_domain::worktree::WorktreeEnvironment;
use lampo_domain::{PortError, PortResult};

use crate::atomic::{read_all_toml, read_toml, remove_file_if_exists, write_toml};

const DIR_SUFFIX: &str = ".worktrees";

pub struct TomlWorktreeRepository {
    paths: LampoPaths,
}

impl TomlWorktreeRepository {
    pub fn new(paths: LampoPaths) -> Self {
        Self { paths }
    }

    /// Parents that have a `<id>.worktrees` folder.
    fn parents_with_folders(&self) -> PortResult<Vec<ProjectId>> {
        let dir = self.paths.projects_dir();
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(PortError::io(format!("listing {}", dir.display()), e)),
        };
        let mut ids: Vec<ProjectId> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(DIR_SUFFIX)).and_then(|id| ProjectId::new(id).ok()))
            .collect();
        ids.sort();
        Ok(ids)
    }
}

fn sorted(mut v: Vec<WorktreeEnvironment>) -> Vec<WorktreeEnvironment> {
    v.sort_by(|a, b| (&a.parent, a.slug.as_str()).cmp(&(&b.parent, b.slug.as_str())));
    v
}

impl WorktreeRepository for TomlWorktreeRepository {
    fn list_all(&self) -> PortResult<Vec<WorktreeEnvironment>> {
        let mut all = Vec::new();
        for parent in self.parents_with_folders()? {
            all.extend(self.list_for(&parent)?);
        }
        Ok(sorted(all))
    }

    fn list_for(&self, parent: &ProjectId) -> PortResult<Vec<WorktreeEnvironment>> {
        Ok(sorted(read_all_toml(&self.paths.worktrees_dir(parent))?))
    }

    fn get(&self, parent: &ProjectId, slug: &WorktreeSlug) -> PortResult<Option<WorktreeEnvironment>> {
        read_toml(&self.paths.worktree_file(parent, slug))
    }

    fn find_by_root(&self, root: &ProjectRoot) -> PortResult<Option<WorktreeEnvironment>> {
        Ok(self.list_all()?.into_iter().find(|e| &e.root == root))
    }

    fn save(&self, environment: &WorktreeEnvironment) -> PortResult<()> {
        write_toml(&self.paths.worktree_file(&environment.parent, &environment.slug), environment)
    }

    fn delete(&self, parent: &ProjectId, slug: &WorktreeSlug) -> PortResult<()> {
        remove_file_if_exists(&self.paths.worktree_file(parent, slug))?;
        // Tidy the folder once empty; failure (not empty, gone) is irrelevant.
        let _ = std::fs::remove_dir(self.paths.worktrees_dir(parent));
        Ok(())
    }

    fn delete_all_for(&self, parent: &ProjectId) -> PortResult<()> {
        let dir = self.paths.worktrees_dir(parent);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(PortError::io(format!("deleting {}", dir.display()), e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{paths, worktree};

    #[test]
    fn crud_and_listing() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(&dir);
        let repo = TomlWorktreeRepository::new(p.clone());
        assert!(repo.list_all().unwrap().is_empty());
        let a = worktree("shop", "fix-1", "D:\\wt\\fix-1", 8100);
        let b = worktree("shop", "feat", "D:\\wt\\feat", 8101);
        let c = worktree("api", "x", "E:\\wt\\x", 8102);
        for e in [&a, &b, &c] {
            repo.save(e).unwrap();
        }
        assert_eq!(repo.get(&a.parent, &a.slug).unwrap().unwrap(), a);
        let shop = ProjectId::new("shop").unwrap();
        assert_eq!(repo.list_for(&shop).unwrap(), vec![b.clone(), a.clone()]);
        assert_eq!(repo.list_all().unwrap(), vec![c.clone(), b.clone(), a.clone()]);
        repo.delete(&a.parent, &a.slug).unwrap();
        repo.delete(&a.parent, &a.slug).unwrap();
        assert_eq!(repo.list_for(&shop).unwrap(), vec![b]);
        repo.delete_all_for(&shop).unwrap();
        repo.delete_all_for(&shop).unwrap();
        assert_eq!(repo.list_all().unwrap(), vec![c]);
    }

    #[test]
    fn find_by_root_is_case_and_separator_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let repo = TomlWorktreeRepository::new(paths(&dir));
        let a = worktree("shop", "fix-1", "D:\\Wt\\Fix-1", 8100);
        repo.save(&a).unwrap();
        let found = repo.find_by_root(&ProjectRoot::new("d:/wt/fix-1").unwrap()).unwrap();
        assert_eq!(found, Some(a));
        assert!(repo.find_by_root(&ProjectRoot::new("D:\\wt\\other").unwrap()).unwrap().is_none());
    }

    #[test]
    fn one_corrupt_environment_does_not_hide_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(&dir);
        let repo = TomlWorktreeRepository::new(p.clone());
        let a = worktree("shop", "fix-1", "D:\\wt\\fix-1", 8100);
        repo.save(&a).unwrap();
        std::fs::write(p.worktrees_dir(&a.parent).join("bad.toml"), "slug = ").unwrap();
        assert_eq!(repo.list_all().unwrap(), vec![a]);
    }
}
