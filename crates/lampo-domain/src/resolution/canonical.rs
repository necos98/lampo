//! One retry of the folder resolvers with the cwd's final path.
//!
//! Project folders are matched textually, so a folder reached through another name does not
//! match: a `subst` drive (`S:\shop` for `C:\work\shop`), a junction or symbolic link, an 8.3
//! name (`C:\PROGRA~1`). When the textual pass finds nothing, [`CanonicalPathResolver`] asks the
//! file system for the folder's final name once and tries again with it.

use std::sync::Arc;

use super::{ProjectResolver, Resolution, ResolveRequest, first_match};
use crate::error::PortResult;
use crate::project::ProjectRoot;
use crate::shim::ShimIndex;

/// The final path of a folder, as the file system names it: the volume's drive letter (not a
/// `subst` alias), junctions and symbolic links followed, long names instead of 8.3 ones.
/// Implemented by `lampo-infra-store` (`GetFinalPathNameByHandleW`).
pub trait PathCanonicalizer: Send + Sync {
    /// `None` when the folder has no other name to offer: it does not exist or cannot be opened.
    fn canonicalize(&self, folder: &str) -> Option<String>;
}

pub struct CanonicalPathResolver {
    canonicalizer: Arc<dyn PathCanonicalizer>,
    /// The resolvers retried with the final path (directory, git worktree).
    retried: Vec<Arc<dyn ProjectResolver>>,
}

impl CanonicalPathResolver {
    pub fn new(canonicalizer: Arc<dyn PathCanonicalizer>, retried: Vec<Arc<dyn ProjectResolver>>) -> Self {
        Self { canonicalizer, retried }
    }

    /// The final path in plain form (`C:\x`, not `\\?\C:\x`), when it differs from `cwd`.
    fn other_name(&self, cwd: &str) -> Option<ProjectRoot> {
        let canonical = ProjectRoot::new(self.canonicalizer.canonicalize(cwd)?).ok()?;
        let same = ProjectRoot::new(cwd).is_ok_and(|textual| textual == canonical);
        (!same).then_some(canonical)
    }
}

impl ProjectResolver for CanonicalPathResolver {
    fn name(&self) -> &'static str {
        "canonical-path"
    }
    fn resolve(&self, request: &ResolveRequest, index: &ShimIndex) -> PortResult<Option<Resolution>> {
        let Some(canonical) = self.other_name(&request.cwd) else { return Ok(None) };
        let retry = ResolveRequest { cwd: canonical.to_string(), ..request.clone() };
        first_match(&self.retried, &retry, index)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::super::tests::{FakeCanonical, chain_with, index, req};
    use super::*;
    use crate::resolution::ResolutionSource;

    #[test]
    fn a_folder_reached_through_another_name_resolves_to_its_project() {
        let subst = FakeCanonical::new(&[(r"S:\shop\app", r"\\?\C:\sites\shop\app")]);
        let r = chain_with(None, subst).resolve(&req(r"S:\shop\app", None), &index()).unwrap().unwrap();
        assert_eq!(r.site.unwrap().to_string(), "shop");
        assert_eq!(r.source, ResolutionSource::Directory);
    }

    #[test]
    fn verbatim_cwd_matches_without_a_retry() {
        let r = chain_with(None, FakeCanonical::new(&[])).resolve(&req(r"\\?\C:\sites\shop\app", None), &index()).unwrap().unwrap();
        assert_eq!(r.site.unwrap().to_string(), "shop");
    }

    /// Counts the calls: the final path is asked for only when the textual pass failed, once.
    struct Counting(Mutex<Vec<String>>);
    impl PathCanonicalizer for Counting {
        fn canonicalize(&self, folder: &str) -> Option<String> {
            self.0.lock().unwrap().push(folder.to_string());
            Some(folder.to_string())
        }
    }

    #[test]
    fn retries_once_and_only_without_a_textual_match() {
        let counting = Arc::new(Counting(Mutex::new(Vec::new())));
        let chain = super::super::ResolverChain::standard(Arc::new(super::super::tests::FakeGit(None)), counting.clone());
        chain.resolve(&req(r"C:\sites\shop", None), &index()).unwrap();
        assert!(counting.0.lock().unwrap().is_empty(), "a textual match needs no file system call");
        let r = chain.resolve(&req(r"F:\elsewhere", None), &index()).unwrap().unwrap();
        assert_eq!(r.source, ResolutionSource::GlobalDefault, "same name: nothing more to try");
        assert_eq!(*counting.0.lock().unwrap(), [r"F:\elsewhere"]);
    }

    #[test]
    fn unopenable_folder_falls_through() {
        let r = chain_with(None, FakeCanonical::new(&[])).resolve(&req(r"F:\gone", None), &index()).unwrap().unwrap();
        assert_eq!(r.source, ResolutionSource::GlobalDefault);
    }
}
