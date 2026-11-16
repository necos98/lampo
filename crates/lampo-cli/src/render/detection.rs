//! `project detect`: what `project create` would use for the folder, the compose apps proposed
//! as projects of their own, then the next step.

use lampo_application::dto::{DetectedAppDto, DetectionDto};

/// Indentation of an app's details under its `app:` line.
const APP_DETAIL: &str = "                ";

fn list(items: &[String]) -> String {
    if items.is_empty() { "none".to_string() } else { items.join(", ") }
}

/// A path as it can be pasted into a command line (quoted when it holds a space).
fn shell_path(path: &str) -> String {
    if path.contains(' ') { format!("\"{path}\"") } else { path.to_string() }
}

/// Where `app` lives on this machine: its folder joined to the analysed one, unless it was
/// written as a path of its own (absolute, home-relative, a variable).
fn app_path(folder: &str, app: &DetectedAppDto) -> Option<String> {
    let own = app.folder.starts_with(['/', '\\', '~', '$']) || app.folder.as_bytes().get(1) == Some(&b':');
    match app.folder.as_str() {
        _ if own => None,
        "." => Some(folder.to_string()),
        relative => Some(format!("{}\\{}", folder.trim_end_matches('\\'), relative.replace('/', "\\"))),
    }
}

fn app(folder: &str, app: &DetectedAppDto) -> String {
    let mut out = format!("  app:        {} ({})\n", app.service, app.folder);
    let mut detail = |label: &str, value: String| out.push_str(&format!("{APP_DETAIL}{label:<14} {value}\n"));
    detail("php:", app.php_version.clone().unwrap_or_else(|| "not detected".into()));
    detail("document root:", app.document_root.clone().unwrap_or_else(|| "not detected".into()));
    detail("extensions:", list(&app.extensions));
    detail("environment:", list(&app.environment.iter().map(|(key, value)| format!("{key}={value}")).collect::<Vec<_>>()));
    detail("services:", list(&app.service_hosts));
    if let Some(path) = app_path(folder, app) {
        let php = app.php_version.as_ref().map(|php| format!(" --php {php}")).unwrap_or_default();
        detail("register:", format!("lampo project create {}{php}", shell_path(&path)));
    }
    out
}

pub fn detection(d: &DetectionDto) -> String {
    let or_none = |value: &Option<String>, none: &str| value.clone().unwrap_or_else(|| none.to_string());
    let mut out = format!("{}\n", d.folder);
    out.push_str(&format!("  type:       {}\n", or_none(&d.project_type, "not recognised")));
    out.push_str(&format!("  preset:     {}\n", or_none(&d.preset_id, "none (project create falls back to generic-php)")));
    out.push_str(&format!(
        "  php:        {}\n",
        d.php_version.as_ref().map(|p| format!("{p} (minimum: project create uses the preset's PHP if newer)")).unwrap_or_else(|| "not detected (the preset's PHP is used)".into())
    ));
    out.push_str(&format!("  name:       {} (id: {})\n", d.suggested_name, d.suggested_id));
    out.push_str(&format!("  services:   {}\n", list(&d.services)));
    out.push_str(&format!("  evidence:   {}\n", list(&d.evidence)));
    for warning in &d.warnings {
        out.push_str(&format!("  warning:    {warning}\n"));
    }
    match &d.already_registered {
        Some(id) => out.push_str(&format!("  registered: yes, as `{id}` (see `lampo project info {id}`)\n")),
        None => out.push_str(&format!("  registered: no (register it with `lampo project create {}`)\n", shell_path(&d.folder))),
    }
    if !d.apps.is_empty() {
        out.push_str(&format!("  apps:       {} PHP apps in the compose file, one project each:\n", d.apps.len()));
        for detected in &d.apps {
            out.push_str(&app(&d.folder, detected));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detected(folder: &str) -> DetectedAppDto {
        DetectedAppDto {
            service: "shop".into(),
            folder: folder.into(),
            php_version: Some("8.3".into()),
            extensions: vec!["intl".into(), "pdo_mysql".into()],
            document_root: Some("public".into()),
            environment: [("DB_HOST".to_string(), "mysql".to_string()), ("APP_ENV".into(), "local".into())].into(),
            service_hosts: vec!["mysql".into()],
        }
    }

    #[test]
    fn an_app_with_its_details_and_how_to_register_it() {
        let text = app(r"C:\My Stack\", &detected("apps/shop"));
        let expected = [
            "  app:        shop (apps/shop)",
            "                php:           8.3",
            "                document root: public",
            "                extensions:    intl, pdo_mysql",
            "                environment:   APP_ENV=local, DB_HOST=mysql",
            "                services:      mysql",
            r#"                register:      lampo project create "C:\My Stack\apps\shop" --php 8.3"#,
        ];
        assert_eq!(text.lines().collect::<Vec<_>>(), expected);
    }

    #[test]
    fn app_paths() {
        let path = |folder: &str| app_path(r"C:\stack", &detected(folder));
        assert_eq!(path(".").as_deref(), Some(r"C:\stack"));
        assert_eq!(path("../shared/a").as_deref(), Some(r"C:\stack\..\shared\a"));
        for own in ["/srv/shop", r"D:\shop", "~/shop", "${SHOP}"] {
            assert_eq!(path(own), None, "{own}");
        }
    }
}
