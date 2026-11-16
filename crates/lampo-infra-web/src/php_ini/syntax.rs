//! php.ini value syntax (pure).
//!
//! Paths are written with forward slashes inside double quotes: PHP on Windows accepts them,
//! and they avoid the ini scanner's backslash escapes (`\"`, and `\\` collapsing in some
//! versions, which would break UNC paths).

use std::path::Path;

/// A path as a quoted php.ini value.
pub fn ini_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let raw = raw.strip_prefix(r"\\?\UNC\").map(|s| format!(r"\\{s}")).unwrap_or_else(|| raw.strip_prefix(r"\\?\").unwrap_or(&raw).to_string());
    format!("\"{}\"", raw.replace('\\', "/").replace('"', ""))
}

/// A user-supplied value, written so PHP reads back the same text:
/// * plain tokens (`On`, `512M`, `Europe/Rome`, `debug,develop`, `127.0.0.1`) stay bare;
/// * constant expressions (`E_ALL & ~E_DEPRECATED`) stay bare so PHP evaluates them;
/// * anything else is quoted — double quotes unless that would change it (`"`, `$`, `\`),
///   then single quotes (raw in PHP ≥ 5.3); a value that also holds `'` keeps double quotes
///   with every character PHP unescapes there written as an escape (see [`escape_double_quoted`]).
pub fn ini_value(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".into();
    }
    let bare_token = value.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.,:/@+-*".contains(&b));
    let expression =
        value.bytes().any(|b| b.is_ascii_uppercase()) && value.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b"_ &|~^!()".contains(&b)) && value == value.trim();
    // `none`, `null`, `yes`, … keep their ini meaning when bare, which is what users mean.
    if bare_token || expression {
        return value.to_string();
    }
    let double_safe = !value.contains(['"', '$', '\\']);
    if double_safe {
        format!("\"{value}\"")
    } else if !value.contains('\'') {
        format!("'{value}'")
    } else {
        format!("\"{}\"", escape_double_quoted(value))
    }
}

/// Inside double quotes the ini scanner turns `\\`, `\"` and `\$` into the bare character and
/// expands `${…}`: escaping all three keeps the text verbatim, and a trailing `\` can no longer
/// pair with the closing quote.
fn escape_double_quoted(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if matches!(c, '\\' | '"' | '$') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values() {
        assert_eq!(ini_value("On"), "On");
        assert_eq!(ini_value("512M"), "512M");
        assert_eq!(ini_value("-1"), "-1");
        assert_eq!(ini_value("Europe/Rome"), "Europe/Rome");
        assert_eq!(ini_value("debug,develop"), "debug,develop");
        assert_eq!(ini_value("E_ALL & ~E_DEPRECATED & ~E_STRICT"), "E_ALL & ~E_DEPRECATED & ~E_STRICT");
        assert_eq!(ini_value(""), "\"\"");
        assert_eq!(ini_value("hello world"), "\"hello world\"");
        assert_eq!(ini_value("a;b"), "\"a;b\"");
        assert_eq!(ini_value(r"C:\tmp\x"), r"'C:\tmp\x'");
        assert_eq!(ini_value("${HOME}"), "'${HOME}'");
        assert_eq!(ini_value(r#"say "hi" it's"#), r#""say \"hi\" it's""#);
    }

    /// With both quote kinds, or `'` plus `\`/`$`, the value is double-quoted and every
    /// character PHP unescapes there (`\`, `"`, `$`) is escaped, so a trailing backslash
    /// cannot swallow the closing quote and `\\`/`${…}` reach PHP unchanged.
    #[test]
    fn escapes_double_quoted_values_with_apostrophes() {
        assert_eq!(ini_value(r"it's C:\"), r#""it's C:\\""#);
        assert_eq!(ini_value(r"it's C:\\"), r#""it's C:\\\\""#);
        assert_eq!(ini_value(r"it's \\nas\share"), r#""it's \\\\nas\\share""#);
        assert_eq!(ini_value("it's ${HOME}"), r#""it's \${HOME}""#);
        assert_eq!(ini_value(r#"say "hi" it's C:\"#), r#""say \"hi\" it's C:\\""#);
        assert_eq!(ini_value(r#"it's \""#), r#""it's \\\"""#);
    }

    #[test]
    fn paths() {
        assert_eq!(ini_path(Path::new(r"C:\Lampo Data\php\ext")), "\"C:/Lampo Data/php/ext\"");
        assert_eq!(ini_path(Path::new(r"\\nas\share\x")), "\"//nas/share/x\"");
        assert_eq!(ini_path(Path::new(r"\\?\C:\x")), "\"C:/x\"");
    }
}
