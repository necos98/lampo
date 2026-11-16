//! Shell-specific rendering of the toolchain environment (`lampo env --shell ...`): quoting is
//! the part that goes wrong, so each shell gets its own tested escaper.

use lampo_application::dto::ToolchainEnvDto;

use crate::cli::ShellKind;

/// PowerShell single-quoted literal: `'` is doubled, and so are the typographic quotes
/// (U+2018, U+2019, U+201A, U+201B) that PowerShell also treats as single quotes.
pub fn quote_powershell(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for c in value.chars() {
        out.push(c);
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// POSIX single-quoted literal: `'` becomes `'\''`.
pub fn quote_bash(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// Value for `set "NAME=<value>"` in a batch file: `%` is doubled (otherwise the batch parser
/// expands it) and `"` is rejected by the caller (cmd cannot quote it inside `set "..."`).
pub fn escape_cmd_value(value: &str) -> String {
    value.replace('%', "%%")
}

/// `C:\Users\me\bin` -> `/c/Users/me/bin` (Git Bash / MSYS form). Anything that does not look
/// like a drive path is only converted to forward slashes.
pub fn to_bash_path(path: &str) -> String {
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        let rest = path[2..].replace('\\', "/");
        let rest = rest.trim_start_matches('/');
        let drive = (bytes[0] as char).to_ascii_lowercase();
        return if rest.is_empty() { format!("/{drive}") } else { format!("/{drive}/{rest}") };
    }
    path.replace('\\', "/")
}

/// The script fragment that applies `env` in `shell`: PATH additions first (kept in order, in
/// front of the existing PATH), then the variables.
pub fn render_env(shell: ShellKind, env: &ToolchainEnvDto) -> String {
    let mut lines = Vec::new();
    match shell {
        ShellKind::Powershell => {
            if !env.path_prepend.is_empty() {
                lines.push(format!("$env:PATH = {} + ';' + $env:PATH", quote_powershell(&env.path_prepend.join(";"))));
            }
            lines.extend(env.vars.iter().map(|(k, v)| format!("$env:{k} = {}", quote_powershell(v))));
        }
        ShellKind::Cmd => {
            if !env.path_prepend.is_empty() {
                let dirs: Vec<String> = env.path_prepend.iter().map(|d| escape_cmd_value(d)).collect();
                lines.push(format!("set \"PATH={};%PATH%\"", dirs.join(";")));
            }
            lines.extend(env.vars.iter().map(|(k, v)| format!("set \"{k}={}\"", escape_cmd_value(v))));
        }
        ShellKind::Bash => {
            if !env.path_prepend.is_empty() {
                let dirs: Vec<String> = env.path_prepend.iter().map(|d| to_bash_path(d)).collect();
                lines.push(format!("export PATH={}\":$PATH\"", quote_bash(&dirs.join(":"))));
            }
            lines.extend(env.vars.iter().map(|(k, v)| format!("export {k}={}", quote_bash(v))));
        }
    }
    lines.join("\n")
}

/// Whether `value` can be put in a `set "NAME=value"` line (cmd has no escape for `"` or line
/// breaks there).
pub fn cmd_can_express(env: &ToolchainEnvDto) -> Option<String> {
    let bad = |s: &str| s.contains('"') || s.contains('\n') || s.contains('\r');
    env.path_prepend.iter().find(|d| bad(d)).cloned().or_else(|| env.vars.values().find(|v| bad(v)).cloned())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn env() -> ToolchainEnvDto {
        ToolchainEnvDto {
            site: Some("shop".into()),
            path_prepend: vec![r"C:\Users\me\AppData\Local\Lampo\bin".into(), r"C:\php\8.3".into()],
            vars: BTreeMap::from([("LAMPO_PROJECT".to_string(), "shop".to_string()), ("PHPRC".to_string(), r"C:\ini dir\it's".to_string())]),
            notice: None,
        }
    }

    #[test]
    fn powershell_quotes_single_quotes_and_typographic_ones() {
        assert_eq!(quote_powershell("it's"), "'it''s'");
        assert_eq!(quote_powershell("a\u{2019}b"), "'a\u{2019}\u{2019}b'");
        assert_eq!(quote_powershell(r"C:\a b\$x`y"), r"'C:\a b\$x`y'");
    }

    #[test]
    fn bash_quotes_single_quotes() {
        assert_eq!(quote_bash("it's"), r"'it'\''s'");
        assert_eq!(quote_bash("$HOME `x` \"y\""), "'$HOME `x` \"y\"'");
    }

    #[test]
    fn bash_paths_use_msys_form() {
        assert_eq!(to_bash_path(r"C:\Users\me\bin"), "/c/Users/me/bin");
        assert_eq!(to_bash_path(r"d:\"), "/d");
        assert_eq!(to_bash_path(r"rel\dir"), "rel/dir");
    }

    #[test]
    fn cmd_doubles_percent() {
        assert_eq!(escape_cmd_value("100%"), "100%%");
    }

    #[test]
    fn powershell_script() {
        let script = render_env(ShellKind::Powershell, &env());
        assert_eq!(script, "$env:PATH = 'C:\\Users\\me\\AppData\\Local\\Lampo\\bin;C:\\php\\8.3' + ';' + $env:PATH\n$env:LAMPO_PROJECT = 'shop'\n$env:PHPRC = 'C:\\ini dir\\it''s'");
    }

    #[test]
    fn cmd_script() {
        let script = render_env(ShellKind::Cmd, &env());
        assert_eq!(script, "set \"PATH=C:\\Users\\me\\AppData\\Local\\Lampo\\bin;C:\\php\\8.3;%PATH%\"\nset \"LAMPO_PROJECT=shop\"\nset \"PHPRC=C:\\ini dir\\it's\"");
    }

    #[test]
    fn bash_script() {
        let script = render_env(ShellKind::Bash, &env());
        assert_eq!(script, "export PATH='/c/Users/me/AppData/Local/Lampo/bin:/c/php/8.3'\":$PATH\"\nexport LAMPO_PROJECT='shop'\nexport PHPRC='C:\\ini dir\\it'\\''s'");
    }

    #[test]
    fn empty_environment_renders_nothing() {
        let empty = ToolchainEnvDto { site: None, path_prepend: vec![], vars: BTreeMap::new(), notice: None };
        for shell in [ShellKind::Powershell, ShellKind::Cmd, ShellKind::Bash] {
            assert_eq!(render_env(shell, &empty), "");
        }
    }

    #[test]
    fn cmd_rejects_quotes() {
        assert!(cmd_can_express(&env()).is_none());
        let mut bad = env();
        bad.vars.insert("X".into(), "a\"b".into());
        assert_eq!(cmd_can_express(&bad).as_deref(), Some("a\"b"));
    }
}
