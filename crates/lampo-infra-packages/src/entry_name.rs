//! Names coming from a package (zip entries, single-file payload names) as safe relative paths:
//! each one must stay inside the destination and be created by Windows exactly as named.
//!
//! * relative, without `..`, roots, drive prefixes or `:` (zip-slip, NTFS alternate streams);
//! * no reserved device name as a component, with or without an extension (`CON`, `nul.txt`,
//!   `COM1`, `CONIN$`…): Windows opens the device instead of creating a file;
//! * no component ending in `.` or a space: Windows strips them, so `a.` silently becomes `a`;
//! * none of the characters Windows refuses in names (`<>"|?*`, control characters).
//!
//! Names that differ only by case are the same file on Windows: [`collision_key`] lets callers
//! detect them.

use std::path::{Component, Path, PathBuf};

/// Device names Windows reserves in every directory, whatever the extension.
const RESERVED_DEVICE_NAMES: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];

/// Device names reserved when followed by one digit (`COM1`, `LPT9`, `COM²`).
const NUMBERED_DEVICE_PREFIXES: [&str; 2] = ["COM", "LPT"];

/// Characters Windows refuses in a file name (besides `:`, `/`, `\` and control characters,
/// which are handled separately).
const INVALID_NAME_CHARS: [char; 6] = ['<', '>', '"', '|', '?', '*'];

/// `name` (`/` or `\` separated) as a safe relative path, or `None` if it could escape the
/// destination or is not a valid Windows name.
pub(crate) fn safe_relative_path(name: &str) -> Option<PathBuf> {
    let normalized = name.replace('\\', "/");
    if normalized.starts_with('/') || normalized.contains(':') {
        return None;
    }
    let mut out = PathBuf::new();
    for part in normalized.split('/') {
        match part {
            "" | "." => continue,
            ".." => return None,
            p if !is_valid_windows_name(p) => return None,
            p => {
                // Each part must be a single normal component (rejects prefixes such as `C:`).
                let mut comps = Path::new(p).components();
                match (comps.next(), comps.next()) {
                    (Some(Component::Normal(_)), None) => out.push(p),
                    _ => return None,
                }
            }
        }
    }
    Some(out)
}

/// `name` as a plain file name (one safe component), or `None`.
pub(crate) fn safe_file_name(name: &str) -> Option<PathBuf> {
    safe_relative_path(name).filter(|p| p.components().count() == 1)
}

/// The key under which Windows (case-insensitive file systems) sees `path`.
pub(crate) fn collision_key(path: &Path) -> String {
    path.to_string_lossy().to_uppercase()
}

fn is_valid_windows_name(component: &str) -> bool {
    !component.ends_with(['.', ' ']) && !component.chars().any(|c| c.is_control() || INVALID_NAME_CHARS.contains(&c)) && !is_reserved_device_name(component)
}

/// `CON`, `aux.txt`, `nul .tar.gz`, `COM1`, `lpt².log`…: the part before the first dot, without
/// trailing spaces, is a device name.
fn is_reserved_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component).trim_end_matches(' ').to_ascii_uppercase();
    if RESERVED_DEVICE_NAMES.contains(&stem.as_str()) {
        return true;
    }
    NUMBERED_DEVICE_PREFIXES.iter().any(|prefix| {
        let mut rest = stem.strip_prefix(prefix).unwrap_or_default().chars();
        matches!((rest.next(), rest.next()), (Some(d), None) if d.is_ascii_digit() || matches!(d, '¹' | '²' | '³'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_ordinary_names() {
        assert_eq!(safe_relative_path("a/b/c.txt").unwrap(), Path::new("a").join("b").join("c.txt"));
        assert_eq!(safe_relative_path("./a//b").unwrap(), Path::new("a").join("b"));
        assert_eq!(safe_relative_path("ext\\php_curl.dll").unwrap(), Path::new("ext").join("php_curl.dll"));
        for ordinary in ["console.txt", "auxiliary", "nul_device", "COM10", "LPT", "com1x", ".htaccess", "con-fig"] {
            assert!(safe_relative_path(ordinary).is_some(), "{ordinary}");
        }
    }

    #[test]
    fn refuses_names_escaping_the_destination() {
        for evil in ["..", "a/../../x", "/abs", "\\\\server\\share\\x", "C:/x", "C:x", "file.txt:stream", "a\0b"] {
            assert!(safe_relative_path(evil).is_none(), "{evil:?}");
        }
    }

    #[test]
    fn refuses_reserved_device_names_with_or_without_extension() {
        for reserved in ["CON", "con", "PRN", "aux.txt", "sub/NUL", "nul.tar.gz", "Nul .txt", "COM1", "com9.log", "LPT0", "lpt²", "COM³.txt", "CONIN$", "conout$.x"] {
            assert!(safe_relative_path(reserved).is_none(), "{reserved}");
        }
    }

    #[test]
    fn refuses_trailing_dots_and_spaces_and_invalid_characters() {
        for invalid in ["trail.", "dir./x", "trail ", "dir /x", "...", "a<b", "a>b", "a\"b", "a|b", "a?b", "a*b", "a\tb"] {
            assert!(safe_relative_path(invalid).is_none(), "{invalid:?}");
        }
    }

    #[test]
    fn file_names_are_a_single_component() {
        assert_eq!(safe_file_name("composer.phar").unwrap(), Path::new("composer.phar"));
        for bad in ["a/composer.phar", "..\\composer.phar", "", "CON", "x."] {
            assert!(safe_file_name(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn collision_keys_ignore_case() {
        assert_eq!(collision_key(Path::new("a/B.txt")), collision_key(Path::new("A/b.TXT")));
        assert_ne!(collision_key(Path::new("a/b.txt")), collision_key(Path::new("a/c.txt")));
    }
}
