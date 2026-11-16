//! Whether the site's database exists, read from the MySQL instance's data dir without
//! connecting: every database is a folder `<datadir>\<name>` (5.7 and 8.x), its name written in
//! MySQL's file-name encoding (`shop-feature` → `shop@002dfeature`). One `metadata` call.
//!
//! Anything uncertain (no data dir in the index, a name this module does not encode, a data dir
//! that cannot be read) counts as existing: the shim then selects the database as it always
//! did, and a missing one gives the client's own visible error rather than a silent change.

use std::fmt::Write as _;

use super::{FolderProbe, FolderState, ShimService};

/// Windows device names MySQL stores with an `@@@` suffix: left to the client (treated as
/// existing) rather than guessed.
const RESERVED_DEVICE_NAMES: &[&str] =
    &["con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9"];

/// The site's database as the client defaults should see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SiteDatabase<'a> {
    /// No site database to add: none for this target (default target, site without one), or
    /// the user chose one.
    None,
    /// Exists, or its existence cannot be established.
    Use(&'a str),
    /// Its folder is not in the instance's data dir: the database has not been created yet.
    Missing(&'a str),
}

impl<'a> SiteDatabase<'a> {
    pub fn of(service: &'a ShimService, folders: &dyn FolderProbe) -> Self {
        let Some(name) = service.database.as_deref() else { return Self::None };
        let folder = service.data_dir.as_deref().zip(folder_name(name)).map(|(dir, folder)| dir.join(folder));
        match folder.as_deref().map(|f| folders.state(f)) {
            Some(FolderState::Missing) => Self::Missing(name),
            _ => Self::Use(name),
        }
    }

    /// The database to select: `None` unless it is known to exist (or may exist).
    pub fn usable(self) -> Option<&'a str> {
        match self {
            Self::Use(name) => Some(name),
            Self::None | Self::Missing(_) => None,
        }
    }
}

/// The folder MySQL keeps database `name` in, in its file-name encoding: ASCII letters, digits
/// and `_` as they are, every other ASCII character as `@` and four hex digits of its code
/// (`-` → `@002d`). `None` for names this module does not encode: non-ASCII characters (MySQL
/// uses a lookup table for many of them), Windows device names (`con` → `con@@@`) and the empty
/// name.
pub(super) fn folder_name(name: &str) -> Option<String> {
    if name.is_empty() || !name.is_ascii() || RESERVED_DEVICE_NAMES.iter().any(|r| name.eq_ignore_ascii_case(r)) {
        return None;
    }
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            write!(out, "@{:04x}", u32::from(c)).expect("writing to a String cannot fail");
        }
    }
    Some(out)
}

/// The one-line note `mysql` prints when it skips a missing site database.
pub(super) fn missing_notice(name: &str) -> String {
    format!("database '{name}' does not exist yet, connecting without a default database")
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use super::*;

    /// Answers every question with `state` and records the paths asked.
    pub(crate) struct FakeFolders {
        state: FolderState,
        asked: Mutex<Vec<PathBuf>>,
    }

    impl FakeFolders {
        pub(crate) fn new(state: FolderState) -> Self {
            Self { state, asked: Mutex::new(Vec::new()) }
        }
        pub(crate) fn asked(&self) -> Vec<PathBuf> {
            self.asked.lock().unwrap().clone()
        }
    }

    impl FolderProbe for FakeFolders {
        fn state(&self, path: &Path) -> FolderState {
            self.asked.lock().unwrap().push(path.to_path_buf());
            self.state
        }
    }

    fn service(database: Option<&str>, data_dir: Option<&str>) -> ShimService {
        ShimService {
            service: "mysql-8.4".parse().unwrap(),
            client_dir: PathBuf::from(r"C:\mysql\bin"),
            host: "127.0.0.1".into(),
            port: crate::tcp_port::Port::of(3306),
            user: Some("root".into()),
            database: database.map(str::to_string),
            data_dir: data_dir.map(PathBuf::from),
            env_prefix: "DB".into(),
            project_account: None,
        }
    }

    #[test]
    fn folder_names_follow_mysqls_encoding() {
        assert_eq!(folder_name("shop_feature_1").as_deref(), Some("shop_feature_1"));
        assert_eq!(folder_name("Shop").as_deref(), Some("Shop"));
        assert_eq!(folder_name("shop-feature").as_deref(), Some("shop@002dfeature"));
        assert_eq!(folder_name("a.b c$").as_deref(), Some("a@002eb@0020c@0024"));
        assert_eq!(folder_name("x@y").as_deref(), Some("x@0040y"));
    }

    #[test]
    fn names_it_cannot_encode_are_left_to_the_client() {
        for name in ["", "caf\u{e9}", "\u{65e5}\u{672c}", "con", "LPT1", "Nul"] {
            assert_eq!(folder_name(name), None, "{name:?}");
        }
        assert_eq!(folder_name("console").as_deref(), Some("console"));
    }

    #[test]
    fn an_existing_database_is_used() {
        let folders = FakeFolders::new(FolderState::Present);
        assert_eq!(SiteDatabase::of(&service(Some("shop"), Some(r"C:\mysql\data")), &folders), SiteDatabase::Use("shop"));
        assert_eq!(folders.asked(), [PathBuf::from(r"C:\mysql\data").join("shop")]);
    }

    #[test]
    fn a_missing_database_is_reported_by_its_encoded_folder() {
        let folders = FakeFolders::new(FolderState::Missing);
        let mysql = service(Some("shop-feature"), Some(r"C:\mysql\data"));
        let site = SiteDatabase::of(&mysql, &folders);
        assert_eq!(site, SiteDatabase::Missing("shop-feature"));
        assert_eq!(site.usable(), None);
        assert_eq!(folders.asked(), [PathBuf::from(r"C:\mysql\data").join("shop@002dfeature")]);
    }

    #[test]
    fn an_unreadable_data_dir_keeps_the_database() {
        let folders = FakeFolders::new(FolderState::Unreadable);
        assert_eq!(SiteDatabase::of(&service(Some("shop"), Some(r"C:\mysql\data")), &folders).usable(), Some("shop"));
    }

    #[test]
    fn without_a_data_dir_or_an_encodable_name_nothing_is_read() {
        let folders = FakeFolders::new(FolderState::Missing);
        assert_eq!(SiteDatabase::of(&service(Some("shop"), None), &folders), SiteDatabase::Use("shop"), "index written before data_dir existed");
        assert_eq!(SiteDatabase::of(&service(Some("caf\u{e9}"), Some(r"C:\mysql\data")), &folders), SiteDatabase::Use("caf\u{e9}"));
        assert_eq!(SiteDatabase::of(&service(None, Some(r"C:\mysql\data")), &folders), SiteDatabase::None);
        assert!(folders.asked().is_empty());
    }

    #[test]
    fn the_notice_names_the_database() {
        assert_eq!(missing_notice("shop"), "database 'shop' does not exist yet, connecting without a default database");
    }
}
