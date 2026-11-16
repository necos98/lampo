//! `project import`: registers a project exported from Lampo, showing what was left out of the
//! file because it could run code on this machine.

use lampo_application::control::ControlRequest;
use lampo_application::dto::{ImportProjectRequest, ImportProjectResultDto};
use lampo_application::AppResult;

use super::Cmd;
use crate::render;

const DROPPED_WARNING: &str = "lampo: warning: settings of the file that could run code on this machine were not imported; review them before adding them back with `lampo project set`";

pub async fn import(c: &Cmd<'_>, file: String, root: Option<String>) -> AppResult<()> {
    let request = ImportProjectRequest { file: c.ctx.env.absolute(&file), root: root.map(|r| c.ctx.env.absolute(&r)) };
    let result: ImportProjectResultDto = c.app(ControlRequest::ProjectImport(request)).await?;
    c.io.emit(&result, imported_project);
    if dropped_anything(&result) {
        c.io.err_line(DROPPED_WARNING);
    }
    Ok(())
}

fn dropped_anything(r: &ImportProjectResultDto) -> bool {
    r.dropped_apache_directives.is_some() || !r.dropped_php_ini_keys.is_empty() || !r.dropped_env_vars.is_empty()
}

/// The project, then what was not imported. The dropped text comes from a file that may not
/// be the user's own: control characters are escaped so it cannot drive the terminal.
fn imported_project(r: &ImportProjectResultDto) -> String {
    let mut out = render::project(&r.project);
    if !r.dropped_php_ini_keys.is_empty() {
        let keys: Vec<String> = r.dropped_php_ini_keys.iter().map(|k| printable(k.trim())).collect();
        out.push_str(&format!("  not imported (php.ini): {}\n", keys.join(", ")));
    }
    if !r.dropped_env_vars.is_empty() {
        let names: Vec<String> = r.dropped_env_vars.iter().map(|n| printable(n)).collect();
        out.push_str(&format!("  not imported (environment variables): {}\n", names.join(", ")));
    }
    if let Some(directives) = &r.dropped_apache_directives {
        out.push_str("  not imported (Apache directives):\n");
        for line in directives.lines() {
            out.push_str(&format!("    {}\n", printable(line)));
        }
    }
    out
}

fn printable(text: &str) -> String {
    text.chars().map(|c| if c.is_control() && c != '\t' { c.escape_default().to_string() } else { c.to_string() }).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::testing::*;
    use lampo_application::control::ControlRequest;

    fn import_reply(directives: Option<&str>, keys: &[&str], env_vars: &[&str]) -> serde_json::Value {
        let mut reply = project_json();
        reply["droppedApacheDirectives"] = json!(directives);
        reply["droppedPhpIniKeys"] = json!(keys);
        reply["droppedEnvVars"] = json!(env_vars);
        reply
    }

    #[tokio::test]
    async fn import_resolves_paths_and_lists_what_was_left_out() {
        let fake = Fake::new();
        fake.reply(Ok(import_reply(Some("Include C:/evil.conf\nCustomLog \"|x\u{1b}[2J\" common"), &["auto_prepend_file"], &["COMSPEC", "APP_KEY"])));
        let (code, out, err) = run_cli(&fake, &["project", "import", "shop.lampo.toml", "--root", "sites\\shop"]).await;
        assert_eq!(code, 0, "{err}");
        let (ControlRequest::ProjectImport(r), _) = fake.last_request() else { panic!("wrong request") };
        assert!(r.file.ends_with("\\shop.lampo.toml") && r.file.len() > "shop.lampo.toml".len(), "{}", r.file);
        assert!(r.root.as_deref().is_some_and(|root| root.ends_with("\\sites\\shop")), "{:?}", r.root);
        assert!(out.contains("  not imported (php.ini): auto_prepend_file\n"), "{out}");
        assert!(out.contains("  not imported (environment variables): COMSPEC, APP_KEY\n"), "{out}");
        assert!(out.contains("    Include C:/evil.conf\n"), "{out}");
        assert!(out.contains("\\u{1b}[2J") && !out.contains('\u{1b}'), "control characters are escaped: {out:?}");
        assert!(err.contains("were not imported"), "{err}");
    }

    #[tokio::test]
    async fn import_json_carries_the_dropped_settings_and_a_clean_import_warns_nothing() {
        let fake = Fake::new();
        fake.reply(Ok(import_reply(None, &[], &[])));
        let (code, out, err) = run_cli(&fake, &["project", "import", "C:\\exports\\shop.lampo.toml", "--json"]).await;
        assert_eq!(code, 0);
        let doc: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(doc["droppedApacheDirectives"], serde_json::Value::Null);
        assert_eq!(doc["droppedPhpIniKeys"], json!([]));
        assert_eq!(doc["droppedEnvVars"], json!([]));
        assert_eq!(doc["id"], project_json()["id"]);
        assert!(!err.contains("not imported"), "{err}");
        let (ControlRequest::ProjectImport(r), _) = fake.last_request() else { panic!("wrong request") };
        assert_eq!((r.file.as_str(), r.root.as_deref()), ("C:\\exports\\shop.lampo.toml", None));
    }
}
