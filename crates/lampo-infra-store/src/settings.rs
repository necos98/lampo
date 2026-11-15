use lampo_domain::PortResult;
use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::SettingsRepository;
use lampo_domain::settings::Settings;

use crate::atomic::{read_toml, write_toml};

/// `%APPDATA%\Lampo\settings.toml`; missing file → `Settings::default()`.
pub struct TomlSettingsRepository {
    paths: LampoPaths,
}

impl TomlSettingsRepository {
    pub fn new(paths: LampoPaths) -> Self {
        Self { paths }
    }
}

impl SettingsRepository for TomlSettingsRepository {
    fn load(&self) -> PortResult<Settings> {
        Ok(read_toml(&self.paths.settings_file())?.unwrap_or_default())
    }

    fn save(&self, settings: &Settings) -> PortResult<()> {
        write_toml(&self.paths.settings_file(), settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::paths;

    #[test]
    fn missing_file_gives_defaults_and_roundtrip_works() {
        let dir = tempfile::tempdir().unwrap();
        let repo = TomlSettingsRepository::new(paths(&dir));
        assert_eq!(repo.load().unwrap(), Settings::default());
        let s = Settings { apache_port: 8080, close_to_tray: false, default_php: Some(lampo_domain::php::PhpMinor::new(8, 3).unwrap()), ..Default::default() };
        repo.save(&s).unwrap();
        assert_eq!(repo.load().unwrap(), s);
    }

    #[test]
    fn partial_file_fills_defaults_and_corrupt_file_is_an_error_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(&dir);
        std::fs::create_dir_all(&p.config_root).unwrap();
        std::fs::write(p.settings_file(), "apache_port = 81\n").unwrap();
        let repo = TomlSettingsRepository::new(p.clone());
        let s = repo.load().unwrap();
        assert_eq!(s.apache_port, 81);
        assert_eq!(s.service_port(&"redis".parse().unwrap()), Some(6379));
        std::fs::write(p.settings_file(), "apache_port = \"x").unwrap();
        assert!(repo.load().unwrap_err().to_string().contains("settings.toml"));
    }

    /// A file edited by hand into invalid settings does not load (the error names it), and a
    /// save over it keeps a copy first.
    #[test]
    fn invalid_settings_are_an_error_naming_the_file_and_are_backed_up_on_save() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(&dir);
        std::fs::create_dir_all(&p.config_root).unwrap();
        let clash = "apache_port = 3306\n";
        std::fs::write(p.settings_file(), clash).unwrap();
        let repo = TomlSettingsRepository::new(p.clone());
        let err = repo.load().unwrap_err().to_string();
        assert!(err.contains("settings.toml") && err.contains("port 3306"), "{err}");

        repo.save(&Settings::default()).unwrap();
        assert_eq!(repo.load().unwrap(), Settings::default());
        let copies: Vec<_> =
            std::fs::read_dir(&p.config_root).unwrap().map(|e| e.unwrap().path()).filter(|f| f.file_name().unwrap().to_string_lossy().starts_with("settings.toml.corrupt-")).collect();
        assert_eq!(copies.len(), 1, "{copies:?}");
        assert_eq!(std::fs::read_to_string(&copies[0]).unwrap(), clash);
    }

    /// H2: a `settings.toml` written before services became data still loads; saving it
    /// writes the new shape.
    #[test]
    fn legacy_settings_files_are_migrated_on_read() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(&dir);
        std::fs::create_dir_all(&p.config_root).unwrap();
        let legacy = "apache_port = 80\nredis_port = 6390\nfirst_run_completed = true\n\n[[mysql_instances]]\nseries = \"8.4\"\nport = 3306\nalways_on = true\n\n[[mysql_instances]]\nseries = \"5.7\"\nport = 3307\nalways_on = false\n";
        std::fs::write(p.settings_file(), legacy).unwrap();
        let repo = TomlSettingsRepository::new(p.clone());
        let s = repo.load().unwrap();
        assert!(s.first_run_completed);
        assert_eq!(
            s.services.iter().map(|i| (i.service.to_string(), i.port)).collect::<Vec<_>>(),
            [("mysql-8.4".to_string(), 3306), ("mysql-5.7".to_string(), 3307), ("redis".to_string(), 6390), ("mailpit".to_string(), 1025)]
        );
        repo.save(&s).unwrap();
        let text = std::fs::read_to_string(p.settings_file()).unwrap();
        assert!(text.contains("[[services]]") && !text.contains("mysql_instances") && !text.contains("redis_port"), "{text}");
        assert!(text.contains("ui_port = 8025"), "Mailpit's web UI port is written: {text}");
        assert_eq!(repo.load().unwrap(), s);
    }
}
