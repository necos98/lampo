//! `which`: the executable a shim would run, the site it belongs to and why that site.

use lampo_application::dto::{ResolutionReasonDto, ResolvedSiteDto, WhichDto};

/// Why the site was chosen, in words.
fn reason(reason: ResolutionReasonDto, resolution: &ResolvedSiteDto) -> String {
    let site = resolution.site.as_deref().unwrap_or("?");
    let folder = |path: &Option<String>| path.clone().unwrap_or_else(|| "?".into());
    match reason {
        ResolutionReasonDto::ProjectFlag => format!("--project {site}"),
        ResolutionReasonDto::LampoProject => format!("LAMPO_PROJECT={site}"),
        ResolutionReasonDto::Directory => format!("the current folder is inside {}", folder(&resolution.root)),
        ResolutionReasonDto::GitWorktree => format!("git worktree {} of {}", folder(&resolution.worktree_root), folder(&resolution.main_root)),
        ResolutionReasonDto::GlobalDefault => "no project for this folder: the global default PHP".into(),
    }
}

/// Arguments as they could be typed (quoted when they hold a space or are empty).
fn command_line(args: &[String]) -> String {
    let quoted: Vec<String> = args.iter().map(|a| if a.is_empty() || a.contains(' ') { format!("\"{a}\"") } else { a.clone() }).collect();
    quoted.join(" ")
}

pub fn which(w: &WhichDto) -> String {
    let mut out = format!("{}: {}\n", w.tool, w.binary);
    if !w.default_args.is_empty() {
        out.push_str(&format!("  args:    {} (before yours)\n", command_line(&w.default_args)));
    }
    let site = match (&w.resolution.site, &w.resolution.root) {
        (Some(site), Some(root)) => format!("{site} ({root})"),
        (Some(site), None) => site.clone(),
        (None, _) => "none".into(),
    };
    out.push_str(&format!("  site:    {site}\n  reason:  {}\n  PHP:     {}\n", reason(w.reason, &w.resolution), w.php_version));
    for notice in &w.notices {
        out.push_str(&format!("  note:    {notice}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use lampo_application::dto::ResolvedKind;

    use super::*;

    fn dto(reason: ResolutionReasonDto, site: Option<&str>) -> WhichDto {
        WhichDto {
            tool: "composer".into(),
            resolution: ResolvedSiteDto {
                kind: if site.is_some() { ResolvedKind::Project } else { ResolvedKind::GlobalDefault },
                via: "directory".into(),
                site: site.map(Into::into),
                project_id: site.map(Into::into),
                root: site.map(|s| format!(r"C:\work\{s}")),
                worktree_root: None,
                main_root: None,
            },
            reason,
            php_version: "8.3".into(),
            binary: r"C:\php\8.3\php.exe".into(),
            default_args: vec![r"C:\Program Files\composer.phar".into()],
            notices: vec!["LAMPO_PROJECT=blog ignored: this folder belongs to shop".into()],
        }
    }

    #[test]
    fn shows_binary_arguments_site_reason_and_notes() {
        let text = which(&dto(ResolutionReasonDto::Directory, Some("shop")));
        assert_eq!(
            text,
            "composer: C:\\php\\8.3\\php.exe\n  args:    \"C:\\Program Files\\composer.phar\" (before yours)\n  site:    shop (C:\\work\\shop)\n  reason:  the current folder is inside C:\\work\\shop\n  PHP:     8.3\n  note:    LAMPO_PROJECT=blog ignored: this folder belongs to shop\n"
        );
    }

    #[test]
    fn every_reason_reads_as_a_sentence() {
        assert!(which(&dto(ResolutionReasonDto::ProjectFlag, Some("shop"))).contains("reason:  --project shop\n"));
        assert!(which(&dto(ResolutionReasonDto::LampoProject, Some("shop"))).contains("reason:  LAMPO_PROJECT=shop\n"));
        let mut worktree = dto(ResolutionReasonDto::GitWorktree, Some("shop"));
        worktree.resolution.worktree_root = Some(r"D:\wt\fix".into());
        worktree.resolution.main_root = Some(r"C:\work\shop".into());
        assert!(which(&worktree).contains(r"reason:  git worktree D:\wt\fix of C:\work\shop"));
        let default = which(&dto(ResolutionReasonDto::GlobalDefault, None));
        assert!(default.contains("site:    none\n") && default.contains("the global default PHP"), "{default}");
    }
}
