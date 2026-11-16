//! `LAMPO_PROJECT`: the site a parent `lampo exec`, `lampo env` or the app's terminal exported.
//!
//! The variable outlives the folder it was meant for: in the terminal of `shop`, `cd ..\blog`
//! keeps `LAMPO_PROJECT=shop`. [`InheritedSiteResolver`] therefore honours it only while the cwd
//! is inside its root (`LAMPO_PROJECT_ROOT`, else the site's registered folder) or outside every
//! project; otherwise the folder's own site wins and the resolution records the ignored one.

use std::sync::Arc;

use super::{ProjectResolver, Resolution, ResolutionSource, ResolveRequest, first_match, non_blank};
use crate::error::{PortError, PortResult};
use crate::project::ProjectRoot;
use crate::shim::{ShimEntry, ShimIndex};
use crate::site::SiteId;

pub struct InheritedSiteResolver {
    /// The resolvers that tell which project a folder belongs to (directory, git worktree, ...).
    by_folder: Vec<Arc<dyn ProjectResolver>>,
}

impl InheritedSiteResolver {
    pub fn new(by_folder: Vec<Arc<dyn ProjectResolver>>) -> Self {
        Self { by_folder }
    }
}

impl ProjectResolver for InheritedSiteResolver {
    fn name(&self) -> &'static str {
        "inherited"
    }
    fn resolve(&self, request: &ResolveRequest, index: &ShimIndex) -> PortResult<Option<Resolution>> {
        let Some(raw) = non_blank(request.inherited_site.as_deref()) else { return Ok(None) };
        let site: SiteId = raw.parse()?;
        let entry = index.by_site(&site);
        if entry.is_some_and(|e| in_scope(request, e)) {
            return Ok(entry.map(|e| Resolution::of(e, ResolutionSource::Explicit)));
        }
        match first_match(&self.by_folder, request, index)? {
            Some(found) if found.site.as_ref() != Some(&site) => Ok(Some(Resolution { ignored_inherited_site: Some(site), ..found })),
            _ => entry.map(|e| Some(Resolution::of(e, ResolutionSource::Explicit))).ok_or_else(|| PortError::not_found("site", raw)),
        }
    }
}

/// Whether the cwd is inside the folder the variable was exported for.
fn in_scope(request: &ResolveRequest, entry: &ShimEntry) -> bool {
    let exported = non_blank(request.inherited_root.as_deref()).and_then(|r| ProjectRoot::new(r).ok());
    exported.as_ref().unwrap_or(&entry.root).contains(&request.cwd)
}

/// The one-line note shown when the folder's site wins over `LAMPO_PROJECT`.
pub(super) fn ignored_notice(ignored: &SiteId, used: &SiteId) -> String {
    format!("LAMPO_PROJECT={ignored} ignored: this folder belongs to {used} (set {}=1 to hide this note)", super::env_vars::QUIET)
}

#[cfg(test)]
mod tests {
    use super::super::tests::{FakeCanonical, chain, chain_with, index};
    use super::*;
    use crate::resolution::GitCheckout;

    fn inherited(cwd: &str, site: &str, root: Option<&str>) -> ResolveRequest {
        ResolveRequest { cwd: cwd.into(), inherited_site: Some(site.into()), inherited_root: root.map(Into::into), ..Default::default() }
    }

    fn site(id: &str) -> SiteId {
        id.parse().unwrap()
    }

    #[test]
    fn holds_inside_its_root() {
        let r = chain(None).resolve(&inherited(r"C:\sites\shop\app", "shop", Some(r"C:\sites\shop")), &index()).unwrap().unwrap();
        assert_eq!(r.site, Some(site("shop")));
        assert_eq!(r.source, ResolutionSource::Explicit);
        assert_eq!(r.ignored_inherited_site, None);
    }

    #[test]
    fn the_folder_of_another_project_wins() {
        let r = chain(None).resolve(&inherited(r"C:\sites\mono\api", "shop", Some(r"C:\sites\shop")), &index()).unwrap().unwrap();
        assert_eq!(r.site, Some(site("api")));
        assert_eq!(r.source, ResolutionSource::Directory);
        assert_eq!(r.ignored_inherited_site, Some(site("shop")));
        assert_eq!(r.ignored_inherited_notice().unwrap(), "LAMPO_PROJECT=shop ignored: this folder belongs to api (set LAMPO_QUIET=1 to hide this note)");
    }

    #[test]
    fn without_an_exported_root_the_registered_folder_is_the_scope() {
        let r = chain(None).resolve(&inherited(r"C:\sites\mono\api\src", "shop", None), &index()).unwrap().unwrap();
        assert_eq!(r.site, Some(site("api")));
        let r = chain(None).resolve(&inherited(r"C:\sites\shop\src", "shop", None), &index()).unwrap().unwrap();
        assert_eq!(r.source, ResolutionSource::Explicit);
    }

    #[test]
    fn holds_outside_every_project() {
        let r = chain(None).resolve(&inherited(r"E:\scratch", "shop", Some(r"C:\sites\shop")), &index()).unwrap().unwrap();
        assert_eq!(r.site, Some(site("shop")));
        assert_eq!(r.source, ResolutionSource::Explicit);
    }

    #[test]
    fn a_worktree_environment_inside_the_project_is_its_own_site() {
        let mut idx = index();
        idx.projects.push(crate::shim::tests::entry("shop@next", r"C:\sites\shop\.worktrees\next", "8.3"));
        let r = chain(None).resolve(&inherited(r"C:\sites\shop\.worktrees\next", "shop", Some(r"C:\sites\shop")), &idx).unwrap().unwrap();
        assert_eq!(r.site, Some(site("shop")), "inside the exported root the variable holds");
    }

    #[test]
    fn a_git_worktree_of_another_project_wins() {
        let git = GitCheckout { worktree_root: ProjectRoot::new(r"E:\agents\mono-feat").unwrap(), main_root: Some(ProjectRoot::new(r"C:\sites\mono").unwrap()), branch: None };
        let r = chain(Some(git)).resolve(&inherited(r"E:\agents\mono-feat\api", "shop", Some(r"C:\sites\shop")), &index()).unwrap().unwrap();
        assert_eq!(r.site, Some(site("api")));
        assert_eq!(r.ignored_inherited_site, Some(site("shop")));
    }

    #[test]
    fn its_own_folder_under_another_name_keeps_it() {
        let subst = FakeCanonical::new(&[(r"S:\app", r"C:\sites\shop\app")]);
        let r = chain_with(None, subst).resolve(&inherited(r"S:\app", "shop", Some(r"C:\sites\shop")), &index()).unwrap().unwrap();
        assert_eq!(r.site, Some(site("shop")));
        assert_eq!(r.ignored_inherited_site, None);
    }

    #[test]
    fn unknown_site_is_an_error_outside_projects_and_ignored_inside_one() {
        assert!(chain(None).resolve(&inherited(r"E:\scratch", "nope", None), &index()).is_err());
        let r = chain(None).resolve(&inherited(r"C:\sites\shop", "nope", None), &index()).unwrap().unwrap();
        assert_eq!(r.site, Some(site("shop")));
        assert_eq!(r.ignored_inherited_site, Some(site("nope")));
    }

    #[test]
    fn explicit_flag_beats_the_inherited_variable() {
        let mut request = inherited(r"C:\sites\shop", "shop", None);
        request.explicit_site = Some("api".into());
        let r = chain(None).resolve(&request, &index()).unwrap().unwrap();
        assert_eq!(r.site, Some(site("api")));
    }
}
