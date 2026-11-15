use lampo_domain::PortResult;
use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::CustomPresetRepository;
use lampo_domain::preset::{Preset, PresetId};

use crate::atomic::{read_all_toml, remove_file_if_exists, write_toml};

/// `%APPDATA%\Lampo\presets\<id>.toml` (custom presets only).
pub struct TomlPresetRepository {
    paths: LampoPaths,
}

impl TomlPresetRepository {
    pub fn new(paths: LampoPaths) -> Self {
        Self { paths }
    }
}

impl CustomPresetRepository for TomlPresetRepository {
    /// Sorted by id; unreadable files are skipped and logged.
    fn list(&self) -> PortResult<Vec<Preset>> {
        let mut presets: Vec<Preset> = read_all_toml(&self.paths.presets_dir())?;
        presets.retain(|p| !p.builtin);
        presets.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(presets)
    }

    fn save(&self, preset: &Preset) -> PortResult<()> {
        preset.ensure_editable()?;
        write_toml(&self.paths.preset_file(&preset.id), preset)
    }

    fn delete(&self, id: &PresetId) -> PortResult<()> {
        remove_file_if_exists(&self.paths.preset_file(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{paths, project};
    use lampo_domain::project::DisplayName;

    #[test]
    fn save_list_delete() {
        let dir = tempfile::tempdir().unwrap();
        let repo = TomlPresetRepository::new(paths(&dir));
        let p = project("shop", "C:\\sites\\shop");
        let b = Preset::from_project(PresetId::new("b-preset").unwrap(), DisplayName::new("B").unwrap(), "desc", &p);
        let a = Preset::from_project(PresetId::new("a-preset").unwrap(), DisplayName::new("A").unwrap(), "", &p);
        repo.save(&b).unwrap();
        repo.save(&a).unwrap();
        std::fs::write(paths(&dir).presets_dir().join("bad.toml"), "x = ").unwrap();
        assert_eq!(repo.list().unwrap(), vec![a.clone(), b.clone()]);
        repo.delete(&a.id).unwrap();
        repo.delete(&a.id).unwrap();
        assert_eq!(repo.list().unwrap(), vec![b]);
    }

    #[test]
    fn builtin_presets_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let repo = TomlPresetRepository::new(paths(&dir));
        let mut pr = Preset::from_project(PresetId::new("x").unwrap(), DisplayName::new("X").unwrap(), "", &project("shop", "C:\\s"));
        pr.builtin = true;
        assert!(repo.save(&pr).is_err());
    }
}
