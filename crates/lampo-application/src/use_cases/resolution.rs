use std::sync::Arc;

use lampo_domain::ports::ShimIndexReader;
use lampo_domain::resolution::{Resolution, ResolutionSource, ResolveRequest, ResolverChain};

use crate::dto::{ResolveRequestDto, ResolvedKind, ResolvedSiteDto};
use crate::error::{AppError, AppResult, ErrorCode};

/// Resolves which site a front-end call is about, through the domain [`ResolverChain`]
/// (`--project` → `LAMPO_PROJECT` → directory → git worktree → global default). File-based,
/// works without the app.
pub struct SiteResolver {
    index: Arc<dyn ShimIndexReader>,
    chain: ResolverChain,
}

impl SiteResolver {
    pub fn new(index: Arc<dyn ShimIndexReader>, chain: ResolverChain) -> Self {
        Self { index, chain }
    }

    /// Domain resolution; `Ok(None)` when nothing matched and there is no default PHP.
    pub fn resolve(&self, request: &ResolveRequestDto) -> AppResult<Option<Resolution>> {
        let index = self.index.read()?;
        let req = ResolveRequest {
            cwd: request.cwd.clone(),
            explicit_site: request.explicit_site.clone(),
            inherited_site: request.inherited_site.clone(),
            inherited_root: request.inherited_root.clone(),
        };
        Ok(self.chain.resolve(&req, &index)?)
    }

    /// `ProjectNotResolved` when nothing matched.
    pub fn resolve_dto(&self, request: &ResolveRequestDto) -> AppResult<ResolvedSiteDto> {
        let resolution = self.resolve(request)?.ok_or_else(|| AppError::new(ErrorCode::ProjectNotResolved, format!("no project for `{}` and no default PHP", request.cwd)))?;
        Ok(resolution_to_dto(&resolution))
    }
}

/// Wire view of a domain resolution.
pub(crate) fn resolution_to_dto(r: &Resolution) -> ResolvedSiteDto {
    let kind = match (&r.site, &r.source) {
        (None, _) => ResolvedKind::GlobalDefault,
        (Some(_), ResolutionSource::GitWorktree { .. }) => ResolvedKind::WorktreeWithoutEnvironment,
        (Some(site), _) if site.is_worktree() => ResolvedKind::WorktreeEnvironment,
        (Some(_), _) => ResolvedKind::Project,
    };
    let via = match &r.source {
        ResolutionSource::Explicit => "explicit",
        ResolutionSource::Directory => "directory",
        ResolutionSource::GitWorktree { .. } => "git-worktree",
        ResolutionSource::GlobalDefault => "global-default",
    };
    let (worktree_root, main_root) = match &r.source {
        ResolutionSource::GitWorktree { worktree_root, main_root } => (Some(worktree_root.clone()), Some(main_root.clone())),
        _ => (None, None),
    };
    ResolvedSiteDto {
        kind,
        via: via.to_string(),
        site: r.site.as_ref().map(|s| s.to_string()),
        project_id: r.site.as_ref().map(|s| s.project().to_string()),
        root: r.root.as_ref().map(|p| p.to_string()),
        worktree_root,
        main_root,
    }
}
