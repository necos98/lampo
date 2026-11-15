//! A *site* is anything Apache serves and the shims resolve: a registered project, or a worktree
//! environment derived from one. Vhosts, php.ini files, logs and shim entries are keyed by
//! [`SiteId`], so both kinds flow through the same machinery.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::project::{ProjectId, validate_slug};

/// Identifier of a worktree environment within its parent project (`feature-login`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct WorktreeSlug(String);

impl WorktreeSlug {
    pub fn new(s: impl AsRef<str>) -> Result<Self, DomainError> {
        validate_slug(s.as_ref()).map(Self)
    }

    /// From a branch name (`feature/Login-Form` → `feature-login-form`) or folder name.
    pub fn from_label(label: &str) -> Self {
        Self(crate::project::slugify(label, "worktree"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WorktreeSlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for WorktreeSlug {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<WorktreeSlug> for String {
    fn from(v: WorktreeSlug) -> Self {
        v.0
    }
}

/// Text form: `shop` (project) or `shop@feature-login` (worktree environment of `shop`).
/// Also the value accepted by the `LAMPO_PROJECT` environment variable.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum SiteId {
    Project(ProjectId),
    Worktree { parent: ProjectId, slug: WorktreeSlug },
}

impl SiteId {
    /// The registered project this site belongs to.
    pub fn project(&self) -> &ProjectId {
        match self {
            Self::Project(p) | Self::Worktree { parent: p, .. } => p,
        }
    }

    pub fn is_worktree(&self) -> bool {
        matches!(self, Self::Worktree { .. })
    }

    /// Filesystem-safe key for per-site files (`shop`, `shop@feature-login`).
    pub fn file_key(&self) -> String {
        self.to_string()
    }
}

impl From<ProjectId> for SiteId {
    fn from(p: ProjectId) -> Self {
        Self::Project(p)
    }
}

impl fmt::Display for SiteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Project(p) => write!(f, "{p}"),
            Self::Worktree { parent, slug } => write!(f, "{parent}@{slug}"),
        }
    }
}

impl FromStr for SiteId {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        match s.split_once('@') {
            Some((p, w)) => Ok(Self::Worktree { parent: ProjectId::new(p)?, slug: WorktreeSlug::new(w)? }),
            None => Ok(Self::Project(ProjectId::new(s)?)),
        }
    }
}

impl TryFrom<String> for SiteId {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<SiteId> for String {
    fn from(v: SiteId) -> Self {
        v.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_roundtrip() {
        for s in ["shop", "shop@feature-login"] {
            assert_eq!(s.parse::<SiteId>().unwrap().to_string(), s);
        }
        assert!("shop@".parse::<SiteId>().is_err());
        assert!("Shop".parse::<SiteId>().is_err());
        let w: SiteId = "shop@x".parse().unwrap();
        assert_eq!(w.project().as_str(), "shop");
        assert!(w.is_worktree());
    }

    #[test]
    fn slugs_from_branch_names() {
        assert_eq!(WorktreeSlug::from_label("feature/Login-Form").as_str(), "feature-login-form");
        assert_eq!(WorktreeSlug::from_label("///").as_str(), "worktree");
    }
}
