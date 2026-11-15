//! Alternative locations of a download, tried in order when the catalog URL answers 404/410.
//!
//! Some vendors move a release to an archive path once a newer patch is out. Each rule knows one
//! origin (open-closed: a new origin is a new rule in [`default_rules`], the downloader does
//! not change).

/// Maps a URL to other official locations of the same file.
pub trait UrlFallback: Send + Sync {
    /// Alternative URLs for `url` (empty when the rule does not apply).
    fn alternatives(&self, url: &str) -> Vec<String>;
}

/// MySQL: `https://cdn.mysql.com/Downloads/MySQL-8.4/<file>` moves to
/// `https://cdn.mysql.com/archives/mysql-8.4/<file>` when the next patch release ships.
#[derive(Debug, Default, Clone, Copy)]
pub struct MysqlArchiveFallback;

impl UrlFallback for MysqlArchiveFallback {
    fn alternatives(&self, url: &str) -> Vec<String> {
        const PREFIX: &str = "https://cdn.mysql.com/Downloads/MySQL-";
        let Some(rest) = url.strip_prefix(PREFIX) else { return Vec::new() };
        let Some((series, file)) = rest.split_once('/') else { return Vec::new() };
        if file.is_empty() || file.contains('/') {
            return Vec::new();
        }
        vec![format!("https://cdn.mysql.com/archives/mysql-{series}/{file}")]
    }
}

/// windows.php.net: `/downloads/releases/<file>` moves to `/downloads/releases/archives/<file>`.
#[derive(Debug, Default, Clone, Copy)]
pub struct PhpArchiveFallback;

impl UrlFallback for PhpArchiveFallback {
    fn alternatives(&self, url: &str) -> Vec<String> {
        for base in ["https://windows.php.net/downloads/releases/", "https://downloads.php.net/~windows/releases/"] {
            if let Some(file) = url.strip_prefix(base) {
                if !file.is_empty() && !file.contains('/') {
                    return vec![format!("{base}archives/{file}")];
                }
            }
        }
        Vec::new()
    }
}

pub fn default_rules() -> Vec<Box<dyn UrlFallback>> {
    vec![Box::new(MysqlArchiveFallback), Box::new(PhpArchiveFallback)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_current_release_falls_back_to_archives() {
        let alt = MysqlArchiveFallback.alternatives("https://cdn.mysql.com/Downloads/MySQL-8.4/mysql-8.4.13-winx64.zip");
        assert_eq!(alt, ["https://cdn.mysql.com/archives/mysql-8.4/mysql-8.4.13-winx64.zip"]);
        assert!(MysqlArchiveFallback.alternatives("https://cdn.mysql.com/archives/mysql-5.7/mysql-5.7.44-winx64.zip").is_empty());
    }

    #[test]
    fn php_release_falls_back_to_archives() {
        let alt = PhpArchiveFallback.alternatives("https://windows.php.net/downloads/releases/php-8.3.1-nts-Win32-vs16-x64.zip");
        assert_eq!(alt, ["https://windows.php.net/downloads/releases/archives/php-8.3.1-nts-Win32-vs16-x64.zip"]);
        assert!(PhpArchiveFallback.alternatives("https://windows.php.net/downloads/releases/archives/x.zip").is_empty());
    }
}
