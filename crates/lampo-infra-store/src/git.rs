//! Git metadata from files only (never spawns git): walks up to `.git`; a `.git` *file* means a
//! linked worktree (`gitdir:` → `<main>\.git\worktrees\<name>`), a `.git` *dir* the main
//! checkout. Uses `lampo_domain::resolution::git` helpers for parsing.
//!
//! Layouts handled:
//! * normal repository: `<root>\.git\` directory, branch from `<root>\.git\HEAD`;
//! * linked worktree: `<wt>\.git` file with `gitdir: <main>\.git\worktrees\<name>` (absolute or
//!   relative, any separators/casing); the per-worktree `HEAD` lives in that gitdir and its
//!   `commondir` file (usually `../..`) points back to `<main>\.git`;
//! * submodule or unrecognised gitdir: reported as a checkout without a main root.

use std::path::Path;

use lampo_domain::PortResult;
use lampo_domain::ports::GitInspector;
use lampo_domain::project::ProjectRoot;
use lampo_domain::resolution::{GitCheckout, git};

#[derive(Debug, Default, Clone, Copy)]
pub struct FsGitInspector;

impl GitInspector for FsGitInspector {
    fn inspect(&self, folder: &str) -> PortResult<Option<GitCheckout>> {
        let Ok(mut current) = ProjectRoot::new(folder) else { return Ok(None) };
        loop {
            match std::fs::metadata(current.as_path().join(".git")) {
                Ok(m) if m.is_dir() => {
                    let branch = read_branch(&current.as_path().join(".git"));
                    return Ok(Some(GitCheckout { worktree_root: current, main_root: None, branch }));
                }
                Ok(m) if m.is_file() => return Ok(Some(linked_checkout(current))),
                _ => {}
            }
            match parent(&current) {
                Some(p) => current = p,
                None => return Ok(None),
            }
        }
    }
}

/// Builds the checkout for a folder whose `.git` is a file. Unreadable or unparsable pointers
/// degrade to "a checkout of unknown origin" rather than an error, because shims call this on
/// every invocation inside such folders.
fn linked_checkout(worktree_root: ProjectRoot) -> GitCheckout {
    let dot_git = worktree_root.as_path().join(".git");
    let gitdir = std::fs::read_to_string(&dot_git).ok().and_then(|text| git::parse_gitdir_pointer(&text).and_then(|p| git::resolve_pointer(&worktree_root, p)));
    let Some(gitdir) = gitdir else {
        tracing::warn!("cannot read a gitdir pointer from {}", dot_git.display());
        return GitCheckout { worktree_root, main_root: None, branch: None };
    };
    let branch = read_branch(gitdir.as_path());
    let main_root = main_from_commondir(&gitdir).or_else(|| git::main_repo_from_gitdir(&gitdir));
    GitCheckout { worktree_root, main_root, branch }
}

/// `<gitdir>\commondir` → `<main>\.git` → `<main>`. `None` if absent or not a `.git` folder.
fn main_from_commondir(gitdir: &ProjectRoot) -> Option<ProjectRoot> {
    let text = std::fs::read_to_string(gitdir.as_path().join("commondir")).ok()?;
    let pointer = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let common = git::resolve_pointer(gitdir, pointer)?;
    if !common.as_str().rsplit('\\').next()?.eq_ignore_ascii_case(".git") {
        return None;
    }
    parent(&common)
}

fn read_branch(git_dir: &Path) -> Option<String> {
    git::parse_head_branch(&std::fs::read_to_string(git_dir.join("HEAD")).ok()?)
}

/// The enclosing folder of `root`; `None` at a drive root or UNC share root.
fn parent(root: &ProjectRoot) -> Option<ProjectRoot> {
    let s = root.as_str();
    let prefix_len = if let Some(unc) = s.strip_prefix("\\\\") {
        // \\server\share
        let mut parts = unc.splitn(3, '\\');
        2 + parts.next()?.len() + 1 + parts.next()?.len()
    } else {
        2
    };
    let rest = s[prefix_len..].trim_matches('\\');
    if rest.is_empty() {
        return None;
    }
    let up = rest.rsplit_once('\\').map(|(head, _)| head).unwrap_or("");
    ProjectRoot::new(format!("{}\\{}", &s[..prefix_len], up)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn root_of(dir: &tempfile::TempDir) -> PathBuf {
        // Avoid 8.3 short names so string comparisons are stable.
        fs::canonicalize(dir.path()).unwrap().to_string_lossy().trim_start_matches("\\\\?\\").to_string().into()
    }

    fn s(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    fn main_repo(base: &Path) -> PathBuf {
        let main = base.join("main");
        fs::create_dir_all(main.join(".git").join("worktrees")).unwrap();
        fs::write(main.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
        main
    }

    fn linked(main: &Path, wt: &Path, name: &str, pointer: &str, branch: &str) {
        let gitdir = main.join(".git").join("worktrees").join(name);
        fs::create_dir_all(&gitdir).unwrap();
        fs::write(gitdir.join("HEAD"), format!("ref: refs/heads/{branch}\n")).unwrap();
        fs::write(gitdir.join("commondir"), "../..\n").unwrap();
        fs::create_dir_all(wt).unwrap();
        fs::write(wt.join(".git"), format!("gitdir: {pointer}\n")).unwrap();
    }

    #[test]
    fn normal_repository_from_root_and_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        let base = root_of(&dir);
        let main = main_repo(&base);
        let deep = main.join("src").join("app");
        fs::create_dir_all(&deep).unwrap();
        for start in [main.clone(), deep] {
            let c = FsGitInspector.inspect(&s(&start)).unwrap().unwrap();
            assert_eq!(c.worktree_root, ProjectRoot::new(s(&main)).unwrap());
            assert!(c.main_root.is_none());
            assert_eq!(c.branch.as_deref(), Some("main"));
        }
    }

    #[test]
    fn outside_any_repository_is_none() {
        let dir = tempfile::tempdir().unwrap();
        // The temp dir may itself sit under a repo on odd machines; only assert the common case.
        let inner = root_of(&dir).join("plain");
        fs::create_dir_all(&inner).unwrap();
        if let Some(c) = FsGitInspector.inspect(&s(&inner)).unwrap() {
            assert!(!c.worktree_root.as_str().to_lowercase().starts_with(&root_of(&dir).to_string_lossy().to_lowercase()));
        }
        assert!(FsGitInspector.inspect("not a path").unwrap().is_none());
    }

    #[test]
    fn linked_worktree_with_absolute_gitdir_and_forward_slashes() {
        let dir = tempfile::tempdir().unwrap();
        let base = root_of(&dir);
        let main = main_repo(&base);
        let wt = base.join("agents").join("feat");
        let pointer = s(&main.join(".git").join("worktrees").join("feat")).replace('\\', "/");
        linked(&main, &wt, "feat", &pointer, "feature/login");
        let sub = wt.join("api").join("src");
        fs::create_dir_all(&sub).unwrap();
        let c = FsGitInspector.inspect(&s(&sub)).unwrap().unwrap();
        assert_eq!(c.worktree_root, ProjectRoot::new(s(&wt)).unwrap());
        assert_eq!(c.main_root, Some(ProjectRoot::new(s(&main)).unwrap()));
        assert_eq!(c.branch.as_deref(), Some("feature/login"));
    }

    #[test]
    fn linked_worktree_with_relative_gitdir() {
        let dir = tempfile::tempdir().unwrap();
        let base = root_of(&dir);
        let main = main_repo(&base);
        let wt = base.join("wt-rel");
        linked(&main, &wt, "wt-rel", "../main/.git/worktrees/wt-rel", "rel");
        let c = FsGitInspector.inspect(&s(&wt)).unwrap().unwrap();
        assert_eq!(c.main_root, Some(ProjectRoot::new(s(&main)).unwrap()));
        assert_eq!(c.branch.as_deref(), Some("rel"));
    }

    #[test]
    fn path_casing_and_separators_do_not_matter() {
        let dir = tempfile::tempdir().unwrap();
        let base = root_of(&dir);
        let main = main_repo(&base);
        let wt = base.join("wt");
        let pointer = s(&main.join(".git").join("worktrees").join("wt")).to_uppercase();
        linked(&main, &wt, "wt", &pointer, "x");
        let weird = s(&wt.join("sub")).replace('\\', "/").to_lowercase();
        fs::create_dir_all(wt.join("sub")).unwrap();
        let c = FsGitInspector.inspect(&weird).unwrap().unwrap();
        assert_eq!(c.worktree_root, ProjectRoot::new(s(&wt)).unwrap());
        assert_eq!(c.main_root, Some(ProjectRoot::new(s(&main)).unwrap()));
    }

    #[test]
    fn commondir_wins_when_gitdir_is_not_under_worktrees() {
        let dir = tempfile::tempdir().unwrap();
        let base = root_of(&dir);
        let main = main_repo(&base);
        // gitdir stored elsewhere (e.g. moved); commondir is absolute.
        let gitdir = base.join("odd").join("gd");
        fs::create_dir_all(&gitdir).unwrap();
        fs::write(gitdir.join("HEAD"), "ref: refs/heads/odd\n").unwrap();
        fs::write(gitdir.join("commondir"), s(&main.join(".git"))).unwrap();
        let wt = base.join("wt-odd");
        fs::create_dir_all(&wt).unwrap();
        fs::write(wt.join(".git"), format!("gitdir: {}", s(&gitdir))).unwrap();
        let c = FsGitInspector.inspect(&s(&wt)).unwrap().unwrap();
        assert_eq!(c.main_root, Some(ProjectRoot::new(s(&main)).unwrap()));
    }

    #[test]
    fn detached_head_submodule_and_broken_pointer_degrade_gracefully() {
        let dir = tempfile::tempdir().unwrap();
        let base = root_of(&dir);
        let main = main_repo(&base);
        let wt = base.join("wt-det");
        linked(&main, &wt, "wt-det", &s(&main.join(".git").join("worktrees").join("wt-det")), "x");
        fs::write(main.join(".git").join("worktrees").join("wt-det").join("HEAD"), "3f2a9c0d3f2a9c0d3f2a9c0d3f2a9c0d3f2a9c0d\n").unwrap();
        assert_eq!(FsGitInspector.inspect(&s(&wt)).unwrap().unwrap().branch, None);

        let sub = base.join("sub");
        let modules = main.join(".git").join("modules").join("sub");
        fs::create_dir_all(&modules).unwrap();
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join(".git"), format!("gitdir: {}", s(&modules))).unwrap();
        let c = FsGitInspector.inspect(&s(&sub)).unwrap().unwrap();
        assert!(c.main_root.is_none());

        let broken = base.join("broken");
        fs::create_dir_all(&broken).unwrap();
        fs::write(broken.join(".git"), "garbage").unwrap();
        let c = FsGitInspector.inspect(&s(&broken)).unwrap().unwrap();
        assert!(c.main_root.is_none() && c.branch.is_none());
    }

    #[test]
    fn parent_stops_at_roots() {
        let p = |s: &str| parent(&ProjectRoot::new(s).unwrap()).map(|r| r.to_string());
        assert_eq!(p("C:\\a\\b").as_deref(), Some("C:\\a"));
        assert_eq!(p("C:\\a").as_deref(), Some("C:\\"));
        assert_eq!(p("C:\\"), None);
        assert_eq!(p("\\\\srv\\share\\x").as_deref(), Some("\\\\srv\\share\\"));
        assert_eq!(p("\\\\srv\\share"), None);
    }
}
