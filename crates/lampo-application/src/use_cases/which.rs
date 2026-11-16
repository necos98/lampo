//! `lampo which <tool>`: what a shim would run from a folder, and why. File-based, works
//! without the app, runs the same resolution and the same [`ShimTool`] as the shim itself.

use std::sync::Arc;

use lampo_domain::resolution::{Resolution, ResolutionSource};
use lampo_domain::shim::{FolderProbe, ShimTool};

use super::resolution::{SiteResolver, resolution_to_dto};
use super::support::validation;
use crate::dto::{ResolutionReasonDto, ResolveRequestDto, WhichDto};
use crate::error::{AppError, AppResult, ErrorCode};

pub struct ShimToolQuery {
    resolver: Arc<SiteResolver>,
    /// The registered shim tools (the same list the shims are installed from).
    tools: Vec<Arc<dyn ShimTool>>,
    /// What the tools' defaults depend on (whether the site's database exists).
    folders: Arc<dyn FolderProbe>,
}

impl ShimToolQuery {
    pub fn new(resolver: Arc<SiteResolver>, tools: Vec<Arc<dyn ShimTool>>, folders: Arc<dyn FolderProbe>) -> Self {
        Self { resolver, tools, folders }
    }

    /// The site, the executable and the default arguments of `tool`'s shim for `request`.
    /// `Validation` for a name that is not a shim, `ProjectNotResolved` when nothing resolves,
    /// `PackageMissing` when the site has no such tool (no Composer, no MySQL instance...).
    pub fn which(&self, request: &ResolveRequestDto, tool: &str) -> AppResult<WhichDto> {
        let shim = self.tool(tool)?;
        let resolution = self.resolver.resolve(request)?.ok_or_else(|| AppError::new(ErrorCode::ProjectNotResolved, format!("no project for `{}` and no default PHP", request.cwd)))?;
        let invocation = shim.invocation(&resolution.target, &[], self.folders.as_ref())?;
        let notices = resolution.ignored_inherited_notice().into_iter().chain(invocation.notice).collect();
        Ok(WhichDto {
            tool: shim.name().to_string(),
            resolution: resolution_to_dto(&resolution),
            reason: reason(&resolution, request),
            php_version: resolution.target.php.to_string(),
            binary: invocation.program.display().to_string(),
            default_args: invocation.args,
            notices,
        })
    }

    fn tool(&self, name: &str) -> AppResult<&Arc<dyn ShimTool>> {
        self.tools.iter().find(|t| t.name().eq_ignore_ascii_case(name.trim())).ok_or_else(|| {
            let known: Vec<&str> = self.tools.iter().map(|t| t.name()).collect();
            validation(format!("`{name}` is not a Lampo shim (expected one of: {})", known.join(", ")))
        })
    }
}

/// The resolution step that answered. `--project` is tried before `LAMPO_PROJECT`, so an
/// `Explicit` answer comes from the flag whenever one was given.
fn reason(resolution: &Resolution, request: &ResolveRequestDto) -> ResolutionReasonDto {
    match resolution.source {
        ResolutionSource::Explicit if request.explicit_site.as_deref().is_some_and(|s| !s.trim().is_empty()) => ResolutionReasonDto::ProjectFlag,
        ResolutionSource::Explicit => ResolutionReasonDto::LampoProject,
        ResolutionSource::Directory => ResolutionReasonDto::Directory,
        ResolutionSource::GitWorktree { .. } => ResolutionReasonDto::GitWorktree,
        ResolutionSource::GlobalDefault => ResolutionReasonDto::GlobalDefault,
    }
}
