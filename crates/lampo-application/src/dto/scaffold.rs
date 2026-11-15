//! Setting up the folder of a new project (git clone, Laravel Installer) before creating it.

use serde::{Deserialize, Serialize};

/// Creates `<parent_dir>\<folder_name>` from a source. Runs as a background task; on success the
/// front-end continues with detection and `CreateProjectRequest` on [`ScaffoldProjectResultDto::folder`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScaffoldProjectRequest {
    /// Existing folder the new project folder is created in.
    pub parent_dir: String,
    /// Name of the new folder. Required for Laravel; for a clone it defaults to the repository
    /// name (`https://github.com/acme/shop.git` → `shop`).
    #[serde(default)]
    pub folder_name: Option<String>,
    pub source: ScaffoldSourceDto,
}

/// Wire form: `{ "kind": "git-clone", "url": "...", "branch": null }`,
/// `{ "kind": "laravel", "phpVersion": "8.4", "starterKit": "react", "testing": "pest", "git": true, "npm": false }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ScaffoldSourceDto {
    GitClone {
        url: String,
        /// Branch or tag to check out (default: the remote's default branch).
        #[serde(default)]
        branch: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Laravel {
        /// PHP minor running the installer (`"8.4"`); default: the newest installed PHP.
        #[serde(default)]
        php_version: Option<String>,
        #[serde(default)]
        starter_kit: StarterKitDto,
        #[serde(default)]
        testing: TestingFrameworkDto,
        /// Initialise a git repository (`--git`, needs git).
        #[serde(default)]
        git: bool,
        /// Install and build the front-end (`--npm`, needs Node.js).
        #[serde(default)]
        npm: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StarterKitDto {
    #[default]
    None,
    React,
    Vue,
    Livewire,
    Svelte,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TestingFrameworkDto {
    #[default]
    Pest,
    Phpunit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScaffoldProjectResultDto {
    /// The folder that was created (absolute Windows path).
    pub folder: String,
    /// PHP minor the Laravel Installer ran with (`None` for a clone): a good default for the project.
    pub php_version: Option<String>,
}
