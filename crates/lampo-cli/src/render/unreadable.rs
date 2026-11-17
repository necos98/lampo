//! Project files that do not load: hidden from the listings, so the listings say they exist.

use lampo_application::dto::UnreadableProjectDto;

/// The section after a project listing (empty when every file loads). Each reason names its file.
pub fn unreadable_projects(unreadable: &[UnreadableProjectDto]) -> String {
    if unreadable.is_empty() {
        return String::new();
    }
    let mut out = format!("\n\nProject files that do not load ({}), hidden until repaired:\n", unreadable.len());
    for u in unreadable {
        out.push_str(&format!("  {}\n", u.reason));
    }
    out
}

/// The one-line stderr warning of a `--json` listing, whose document has no room for them.
pub fn unreadable_projects_warning(unreadable: &[UnreadableProjectDto]) -> Option<String> {
    if unreadable.is_empty() {
        return None;
    }
    let reasons: Vec<&str> = unreadable.iter().map(|u| u.reason.as_str()).collect();
    Some(format!("lampo: warning: {} project file(s) do not load and are not listed: {} (see `lampo doctor`)", unreadable.len(), reasons.join("; ")))
}
