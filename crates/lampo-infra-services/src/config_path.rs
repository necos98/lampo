//! Paths inside the generated service configuration files (MySQL `my.ini`, `redis.conf`).

use std::path::Path;

/// A path as a configuration value: forward slashes (backslashes are escape characters in MySQL
/// option files) inside double quotes (paths with spaces). Every Windows build of MySQL and
/// Redis accepts this form.
pub(crate) fn config_file_path(path: &Path) -> String {
    format!("\"{}\"", path.display().to_string().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_quoted_with_forward_slashes() {
        assert_eq!(config_file_path(Path::new(r"C:\Program Files\Lampo\data")), "\"C:/Program Files/Lampo/data\"");
        assert_eq!(config_file_path(Path::new("relative/dir")), "\"relative/dir\"");
    }
}
