//! Pure manipulation of `PATH`-style strings (`a;b;c`), kept free of the registry so it can be
//! tested without touching the real environment.

use std::path::Path;

/// Looks up an environment variable for `%NAME%` expansion.
pub type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<String>;

/// Non-empty, trimmed entries of a `;`-separated list.
pub fn entries(list: &str) -> Vec<&str> {
    list.split(';').map(str::trim).filter(|e| !e.is_empty()).collect()
}

/// Expands `%NAME%` references; unknown names are left as written (like `cmd`).
pub fn expand_vars(text: &str, env: EnvLookup) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => match env(&after[..end]) {
                Some(value) => {
                    out.push_str(&value);
                    rest = &after[end + 1..];
                }
                None => {
                    out.push('%');
                    rest = after;
                }
            },
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Canonical comparison key of a directory entry: unquoted, expanded, backslashes, no trailing
/// separator, lower case (Windows paths are case-insensitive).
pub fn key(entry: &str, env: EnvLookup) -> String {
    let unquoted = entry.trim().trim_matches('"');
    let expanded = expand_vars(unquoted, env).replace('/', "\\");
    let trimmed = expanded.trim_end_matches('\\');
    // Keep `C:\` distinct from `C:`.
    let trimmed = if trimmed.len() == 2 && trimmed.ends_with(':') { format!("{trimmed}\\") } else { trimmed.to_string() };
    trimmed.to_lowercase()
}

/// Whether `list` already contains `dir` (comparing expanded, normalised entries).
pub fn contains(list: &str, dir: &Path, env: EnvLookup) -> bool {
    let wanted = key(&dir.to_string_lossy(), env);
    entries(list).iter().any(|e| key(e, env) == wanted)
}

/// `list` with `dir` appended, unless already present. Existing text is otherwise preserved.
pub fn append(list: &str, dir: &Path, env: EnvLookup) -> String {
    if contains(list, dir, env) {
        return list.to_string();
    }
    let dir = dir.to_string_lossy();
    let base = list.trim_end().trim_end_matches(';');
    if base.trim().is_empty() { dir.into_owned() } else { format!("{base};{dir}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(name: &str) -> Option<String> {
        match name.to_ascii_lowercase().as_str() {
            "localappdata" => Some("C:\\Users\\Me\\AppData\\Local".into()),
            "systemroot" => Some("C:\\Windows".into()),
            _ => None,
        }
    }

    #[test]
    fn expands_known_variables_only() {
        assert_eq!(expand_vars("%SystemRoot%\\system32", &env), "C:\\Windows\\system32");
        assert_eq!(expand_vars("%NOPE%\\x", &env), "%NOPE%\\x");
        assert_eq!(expand_vars("100%", &env), "100%");
        assert_eq!(expand_vars("a%%b", &env), "a%%b");
    }

    #[test]
    fn contains_ignores_case_quotes_slashes_trailing_separator_and_variables() {
        let list = "C:\\Windows;\"C:\\Tools\\\";%LOCALAPPDATA%\\Lampo\\bin;;";
        assert!(contains(list, Path::new("c:\\windows\\"), &env));
        assert!(contains(list, Path::new("C:/tools"), &env));
        assert!(contains(list, Path::new("C:\\Users\\me\\appdata\\local\\lampo\\BIN"), &env));
        assert!(!contains(list, Path::new("C:\\Tools2"), &env));
        assert!(!contains("", Path::new("C:\\x"), &env));
    }

    #[test]
    fn append_adds_once_and_keeps_the_rest_verbatim() {
        let dir = Path::new("D:\\Lampo\\bin");
        assert_eq!(append("", dir, &env), "D:\\Lampo\\bin");
        assert_eq!(append("C:\\a;C:\\b", dir, &env), "C:\\a;C:\\b;D:\\Lampo\\bin");
        assert_eq!(append("C:\\a;C:\\b;", dir, &env), "C:\\a;C:\\b;D:\\Lampo\\bin");
        let once = append("C:\\a;%SystemRoot%", dir, &env);
        assert_eq!(once, "C:\\a;%SystemRoot%;D:\\Lampo\\bin");
        assert_eq!(append(&once, Path::new("d:\\lampo\\bin\\"), &env), once);
    }

    #[test]
    fn drive_root_key_is_stable() {
        assert_eq!(key("C:", &env), "c:\\");
        assert_eq!(key("C:\\", &env), "c:\\");
    }
}
