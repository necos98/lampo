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

/// `list` without any entry equal to `dir` (same comparison as [`contains`]); every other
/// segment, empty ones included, is kept verbatim and in order.
pub fn remove(list: &str, dir: &Path, env: EnvLookup) -> String {
    let wanted = key(&dir.to_string_lossy(), env);
    list.split(';').filter(|segment| segment.trim().is_empty() || key(segment, env) != wanted).collect::<Vec<_>>().join(";")
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
    fn remove_drops_every_matching_entry_and_keeps_the_rest_verbatim() {
        let dir = Path::new("C:\\Users\\Me\\AppData\\Local\\Lampo\\bin");
        assert_eq!(remove("C:\\a;%LOCALAPPDATA%\\Lampo\\bin;%SystemRoot%", dir, &env), "C:\\a;%SystemRoot%");
        assert_eq!(remove("\"c:\\users\\me\\appdata\\local\\lampo\\bin\\\";C:\\a", dir, &env), "C:\\a");
        assert_eq!(remove("C:\\a;C:/Users/Me/AppData/Local/Lampo/bin;C:\\b;C:\\Users\\Me\\AppData\\Local\\Lampo\\bin", dir, &env), "C:\\a;C:\\b");
        assert_eq!(remove("C:\\a;;C:\\b;", dir, &env), "C:\\a;;C:\\b;", "nothing to remove: unchanged");
        assert_eq!(remove("C:\\a;C:\\Users\\Me\\AppData\\Local\\Lampo\\bin;", dir, &env), "C:\\a;");
        assert_eq!(remove("%LOCALAPPDATA%\\Lampo\\bin", dir, &env), "");
        assert_eq!(remove("", dir, &env), "");
        assert_eq!(remove("C:\\Users\\Me\\AppData\\Local\\Lampo\\bin2", dir, &env), "C:\\Users\\Me\\AppData\\Local\\Lampo\\bin2");
    }

    #[test]
    fn remove_undoes_append() {
        let dir = Path::new("D:\\Lampo\\bin");
        for list in ["C:\\a;C:\\b", "C:\\a;%SystemRoot%", "C:\\äöü\\日本"] {
            let appended = append(list, dir, &env);
            assert_eq!(remove(&appended, dir, &env), list);
            assert!(!contains(&remove(&appended, dir, &env), dir, &env));
        }
    }

    #[test]
    fn drive_root_key_is_stable() {
        assert_eq!(key("C:", &env), "c:\\");
        assert_eq!(key("C:\\", &env), "c:\\");
    }
}
