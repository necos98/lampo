//! Which site (project or worktree environment) a command runs for.
//!
//! Resolution is an open chain of [`ProjectResolver`]s tried in order; the first answer wins.
//! The default chain ([`ResolverChain::standard`]) is:
//!
//! 1. [`ExplicitSiteResolver`] — the `LAMPO_PROJECT` environment variable (`shop` or
//!    `shop@feature-x`). An unknown value is an error, never silently ignored.
//! 2. [`DirectoryResolver`] — longest-prefix match of the current directory against registered
//!    project folders *and* worktree-environment folders.
//! 3. [`GitWorktreeResolver`] — the cwd is inside a git worktree without an environment: walk up
//!    to the `.git` *file*, follow `gitdir:` to the main repository, and match that path (plus the
//!    cwd's relative path) against projects. Reads files only (via [`GitInspector`]), never spawns git.
//! 4. [`GlobalDefaultResolver`] — the global default PHP.
//!
//! A new strategy is a new `ProjectResolver` inserted in the chain by the composition root.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::{PortError, PortResult};
use crate::project::ProjectRoot;
use crate::shim::{ShimIndex, ShimTarget};
use crate::site::SiteId;

/// Inputs captured by the caller (CLI composition root / shim) from its process environment.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResolveRequest {
    /// Absolute current directory.
    pub cwd: String,
    /// Value of `LAMPO_PROJECT`, if set and non-empty.
    pub explicit_site: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "via", rename_all = "kebab-case")]
pub enum ResolutionSource {
    /// `LAMPO_PROJECT`.
    Explicit,
    /// The cwd is inside a registered project or worktree-environment folder.
    Directory,
    /// The cwd is inside a git worktree (with no environment) of a registered project.
    GitWorktree { worktree_root: String, main_root: String },
    /// Nothing matched; global default PHP.
    GlobalDefault,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    /// `None` for the global default.
    pub site: Option<SiteId>,
    /// Folder of the matched site (`None` for the global default).
    pub root: Option<ProjectRoot>,
    pub target: ShimTarget,
    pub source: ResolutionSource,
}

/// One resolution strategy.
pub trait ProjectResolver: Send + Sync {
    fn name(&self) -> &'static str;
    /// `Ok(None)` to let the next resolver try; `Err` stops the chain.
    fn resolve(&self, request: &ResolveRequest, index: &ShimIndex) -> PortResult<Option<Resolution>>;
}

/// Ordered list of resolvers.
#[derive(Clone)]
pub struct ResolverChain {
    resolvers: Vec<Arc<dyn ProjectResolver>>,
}

impl ResolverChain {
    pub fn new(resolvers: Vec<Arc<dyn ProjectResolver>>) -> Self {
        Self { resolvers }
    }

    /// Explicit → directory → git worktree → global default.
    pub fn standard(git: Arc<dyn GitInspector>) -> Self {
        Self::new(vec![Arc::new(ExplicitSiteResolver), Arc::new(DirectoryResolver), Arc::new(GitWorktreeResolver::new(git)), Arc::new(GlobalDefaultResolver)])
    }

    /// `Ok(None)` only when nothing matched and there is no global default.
    pub fn resolve(&self, request: &ResolveRequest, index: &ShimIndex) -> PortResult<Option<Resolution>> {
        for r in &self.resolvers {
            if let Some(found) = r.resolve(request, index)? {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }
}

pub struct ExplicitSiteResolver;
impl ProjectResolver for ExplicitSiteResolver {
    fn name(&self) -> &'static str {
        "explicit"
    }
    fn resolve(&self, request: &ResolveRequest, index: &ShimIndex) -> PortResult<Option<Resolution>> {
        let Some(raw) = request.explicit_site.as_deref().map(str::trim).filter(|s| !s.is_empty()) else { return Ok(None) };
        let site: SiteId = raw.parse()?;
        let entry = index.by_site(&site).ok_or_else(|| PortError::not_found("site", raw))?;
        Ok(Some(Resolution { site: Some(entry.site.clone()), root: Some(entry.root.clone()), target: entry.target.clone(), source: ResolutionSource::Explicit }))
    }
}

pub struct DirectoryResolver;
impl ProjectResolver for DirectoryResolver {
    fn name(&self) -> &'static str {
        "directory"
    }
    fn resolve(&self, request: &ResolveRequest, index: &ShimIndex) -> PortResult<Option<Resolution>> {
        Ok(index.by_path(&request.cwd).map(|e| Resolution { site: Some(e.site.clone()), root: Some(e.root.clone()), target: e.target.clone(), source: ResolutionSource::Directory }))
    }
}

pub struct GlobalDefaultResolver;
impl ProjectResolver for GlobalDefaultResolver {
    fn name(&self) -> &'static str {
        "global-default"
    }
    fn resolve(&self, _request: &ResolveRequest, index: &ShimIndex) -> PortResult<Option<Resolution>> {
        Ok(index.default.clone().map(|target| Resolution { site: None, root: None, target, source: ResolutionSource::GlobalDefault }))
    }
}

/// Facts about the git checkout containing a folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitCheckout {
    /// Top folder of the checkout (where `.git` is).
    pub worktree_root: ProjectRoot,
    /// Main repository folder when `worktree_root` is a *linked* worktree; `None` for the main checkout.
    pub main_root: Option<ProjectRoot>,
    /// Checked-out branch (`None` when detached).
    pub branch: Option<String>,
}

/// Reads git metadata from files (`.git` file/dir, `gitdir:` pointer, `HEAD`). Never spawns git.
/// Implemented by `lampo-infra-store` with the pure helpers in [`git`].
pub trait GitInspector: Send + Sync {
    /// Walks up from `folder` to the nearest `.git`; `Ok(None)` if not inside a checkout.
    fn inspect(&self, folder: &str) -> PortResult<Option<GitCheckout>>;
}

pub struct GitWorktreeResolver {
    git: Arc<dyn GitInspector>,
}

impl GitWorktreeResolver {
    pub fn new(git: Arc<dyn GitInspector>) -> Self {
        Self { git }
    }
}

impl ProjectResolver for GitWorktreeResolver {
    fn name(&self) -> &'static str {
        "git-worktree"
    }
    fn resolve(&self, request: &ResolveRequest, index: &ShimIndex) -> PortResult<Option<Resolution>> {
        let Some(checkout) = self.git.inspect(&request.cwd)? else { return Ok(None) };
        let Some(main) = checkout.main_root else { return Ok(None) };
        let Some(mapped) = git::map_into(&request.cwd, &checkout.worktree_root, &main) else { return Ok(None) };
        Ok(index.by_path(&mapped).map(|e| Resolution {
            site: Some(e.site.clone()),
            root: Some(e.root.clone()),
            target: e.target.clone(),
            source: ResolutionSource::GitWorktree { worktree_root: checkout.worktree_root.to_string(), main_root: main.to_string() },
        }))
    }
}

/// Pure helpers for reading git metadata files.
pub mod git {
    use super::*;

    /// The path from a `.git` *file* (`gitdir: C:/repo/.git/worktrees/feature`).
    pub fn parse_gitdir_pointer(dot_git_file: &str) -> Option<&str> {
        dot_git_file.lines().find_map(|l| l.trim().strip_prefix("gitdir:")).map(str::trim).filter(|s| !s.is_empty())
    }

    /// Resolves a (possibly relative) `gitdir:` pointer against the worktree folder and
    /// normalises `.`/`..`. Returns `None` if the result is not a valid absolute path.
    pub fn resolve_pointer(worktree_root: &ProjectRoot, pointer: &str) -> Option<ProjectRoot> {
        let p = pointer.replace('/', "\\");
        let joined =
            if ProjectRoot::new(&p).is_ok() || p.starts_with("\\\\") || p.get(1..3) == Some(":\\") { p } else { format!("{}\\{}", worktree_root.as_str().trim_end_matches('\\'), p) };
        let (prefix, rest) = if let Some(unc) = joined.strip_prefix("\\\\") {
            let mut it = unc.splitn(3, '\\');
            let server = it.next()?;
            let share = it.next()?;
            (format!("\\\\{server}\\{share}"), it.next().unwrap_or("").to_string())
        } else {
            (joined.get(..2)?.to_string(), joined.get(3..).unwrap_or("").to_string())
        };
        let mut segs: Vec<&str> = Vec::new();
        for s in rest.split('\\').filter(|s| !s.is_empty() && *s != ".") {
            if s == ".." {
                segs.pop()?;
            } else {
                segs.push(s);
            }
        }
        ProjectRoot::new(format!("{prefix}\\{}", segs.join("\\"))).ok()
    }

    /// `<main>\.git\worktrees\<name>` → `<main>`. `None` for other git dirs (submodules, …).
    pub fn main_repo_from_gitdir(gitdir: &ProjectRoot) -> Option<ProjectRoot> {
        let s = gitdir.as_str();
        let idx = s.to_ascii_lowercase().rfind("\\.git\\worktrees\\")?;
        ProjectRoot::new(&s[..idx]).ok()
    }

    /// Branch from a `HEAD` file (`ref: refs/heads/feature/x` → `feature/x`); `None` if detached.
    pub fn parse_head_branch(head: &str) -> Option<String> {
        head.trim().strip_prefix("ref:").map(str::trim).and_then(|r| r.strip_prefix("refs/heads/")).filter(|b| !b.is_empty()).map(str::to_string)
    }

    /// Maps `cwd` (inside `worktree_root`) to the same relative location inside `main_root`.
    pub fn map_into(cwd: &str, worktree_root: &ProjectRoot, main_root: &ProjectRoot) -> Option<String> {
        if !worktree_root.contains(cwd) {
            return None;
        }
        let cwd = ProjectRoot::new(cwd).ok()?;
        let rel = cwd.as_str().get(worktree_root.as_str().trim_end_matches('\\').len()..).unwrap_or("").trim_start_matches('\\');
        Some(if rel.is_empty() { main_root.to_string() } else { format!("{}\\{rel}", main_root.as_str().trim_end_matches('\\')) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shim::tests::{entry, target};

    fn root(s: &str) -> ProjectRoot {
        ProjectRoot::new(s).unwrap()
    }

    struct FakeGit(Option<GitCheckout>);
    impl GitInspector for FakeGit {
        fn inspect(&self, _folder: &str) -> PortResult<Option<GitCheckout>> {
            Ok(self.0.clone())
        }
    }

    fn index() -> ShimIndex {
        ShimIndex {
            default: Some(target("8.4")),
            projects: vec![entry("shop", "C:\\sites\\shop", "8.2"), entry("api", "C:\\sites\\mono\\api", "8.3"), entry("shop@fix-1", "D:\\wt\\fix-1", "8.2")],
            ..Default::default()
        }
    }

    fn chain(git: Option<GitCheckout>) -> ResolverChain {
        ResolverChain::standard(Arc::new(FakeGit(git)))
    }

    fn req(cwd: &str, explicit: Option<&str>) -> ResolveRequest {
        ResolveRequest { cwd: cwd.into(), explicit_site: explicit.map(Into::into) }
    }

    #[test]
    fn explicit_wins_and_unknown_is_an_error() {
        let r = chain(None).resolve(&req("C:\\sites\\shop", Some("api")), &index()).unwrap().unwrap();
        assert_eq!(r.site.unwrap().to_string(), "api");
        assert_eq!(r.source, ResolutionSource::Explicit);
        assert!(chain(None).resolve(&req("C:\\x", Some("nope")), &index()).is_err());
        // Empty variable is ignored.
        assert_eq!(chain(None).resolve(&req("C:\\sites\\shop", Some(" ")), &index()).unwrap().unwrap().source, ResolutionSource::Directory);
    }

    #[test]
    fn worktree_environment_folder_resolves_by_directory() {
        let r = chain(None).resolve(&req("D:\\wt\\fix-1\\src", None), &index()).unwrap().unwrap();
        assert_eq!(r.site.unwrap().to_string(), "shop@fix-1");
    }

    #[test]
    fn git_worktree_without_environment_maps_to_parent() {
        let git = GitCheckout { worktree_root: root("E:\\agents\\mono-feat"), main_root: Some(root("C:\\sites\\mono")), branch: Some("feat".into()) };
        let r = chain(Some(git)).resolve(&req("E:\\agents\\mono-feat\\api\\src", None), &index()).unwrap().unwrap();
        assert_eq!(r.site.unwrap().to_string(), "api");
        assert!(matches!(r.source, ResolutionSource::GitWorktree { .. }));
    }

    #[test]
    fn falls_back_to_default() {
        let main_checkout = GitCheckout { worktree_root: root("F:\\other"), main_root: None, branch: None };
        let r = chain(Some(main_checkout)).resolve(&req("F:\\other", None), &index()).unwrap().unwrap();
        assert_eq!(r.source, ResolutionSource::GlobalDefault);
        assert!(r.site.is_none());
        let empty = ShimIndex::default();
        assert!(chain(None).resolve(&req("F:\\other", None), &empty).unwrap().is_none());
    }

    #[test]
    fn git_file_helpers() {
        assert_eq!(git::parse_gitdir_pointer("gitdir: C:/sites/shop/.git/worktrees/fix-1\n"), Some("C:/sites/shop/.git/worktrees/fix-1"));
        assert_eq!(git::parse_gitdir_pointer("nothing"), None);
        let wt = root("D:\\wt\\fix-1");
        let abs = git::resolve_pointer(&wt, "C:/sites/shop/.git/worktrees/fix-1").unwrap();
        assert_eq!(abs.as_str(), "C:\\sites\\shop\\.git\\worktrees\\fix-1");
        let rel = git::resolve_pointer(&root("C:\\sites\\shop-wt"), "../shop/.git/worktrees/x").unwrap();
        assert_eq!(rel.as_str(), "C:\\sites\\shop\\.git\\worktrees\\x");
        assert_eq!(git::main_repo_from_gitdir(&abs).unwrap().as_str(), "C:\\sites\\shop");
        assert!(git::main_repo_from_gitdir(&root("C:\\r\\.git\\modules\\sub")).is_none());
        assert_eq!(git::parse_head_branch("ref: refs/heads/feature/x\n").as_deref(), Some("feature/x"));
        assert_eq!(git::parse_head_branch("3f2a9c0d"), None);
        assert_eq!(git::map_into("d:\\WT\\fix-1\\src\\a", &wt, &root("C:\\sites\\shop")).unwrap(), "C:\\sites\\shop\\src\\a");
        assert_eq!(git::map_into("D:\\wt\\fix-1", &wt, &root("C:\\sites\\shop")).unwrap(), "C:\\sites\\shop");
        assert!(git::map_into("D:\\elsewhere", &wt, &root("C:\\sites\\shop")).is_none());
    }
}
