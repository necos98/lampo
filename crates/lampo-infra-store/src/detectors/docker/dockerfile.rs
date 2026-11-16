//! Dockerfiles: where they usually are, their `FROM` stages (the last one is the runtime, the
//! others build stages) and the extensions of `docker-php-ext-install`.

use std::path::{Path, PathBuf};

use lampo_domain::php::{ExtensionName, PhpMinor};

use super::findings::{Findings, PhpRole};
use super::image::{Component, classify_image};

/// Where Dockerfiles usually are, besides the project folder itself.
const DOCKER_DIRS: &[&str] = &["docker", ".docker"];

/// `Dockerfile`, `Dockerfile.dev`, `app.Dockerfile`…
fn is_dockerfile(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "dockerfile" || lower.starts_with("dockerfile.") || lower.ends_with(".dockerfile")
}

/// Dockerfiles in the folder, in `docker/`/`.docker/` and one level below those, as
/// (name relative to the folder, path).
pub(super) fn dockerfiles(folder: &Path) -> Vec<(String, PathBuf)> {
    let list = |dir: &Path| -> Vec<std::fs::DirEntry> {
        let mut entries: Vec<_> = std::fs::read_dir(dir).map(|e| e.flatten().collect()).unwrap_or_default();
        entries.sort_by_key(|e| e.file_name());
        entries
    };
    let mut found = Vec::new();
    let mut push_files = |dir: &Path, prefix: &str| {
        for entry in list(dir) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_dockerfile(&name) && entry.path().is_file() {
                found.push((format!("{prefix}{name}"), entry.path()));
            }
        }
    };
    push_files(folder, "");
    for dir in DOCKER_DIRS {
        let base = folder.join(dir);
        push_files(&base, &format!("{dir}/"));
        for sub in list(&base).into_iter().filter(|e| e.path().is_dir()) {
            let sub_name = sub.file_name().to_string_lossy().into_owned();
            push_files(&sub.path(), &format!("{dir}/{sub_name}/"));
        }
    }
    found
}

/// One `FROM` of a Dockerfile.
#[derive(Debug, PartialEq, Eq)]
struct Stage {
    image: String,
    /// `AS name`, lowercase (stage names are case-insensitive).
    name: Option<String>,
    /// What the image stands for; a stage built `FROM` an earlier one inherits its component.
    component: Component,
}

/// `[--platform=…] image [AS name]` → (image, name).
fn parse_from(rest: &str) -> Option<(&str, Option<String>)> {
    let mut words = rest.split_whitespace().filter(|w| !w.starts_with("--"));
    let image = words.next()?;
    let name = match (words.next(), words.next()) {
        (Some(keyword), Some(name)) if keyword.eq_ignore_ascii_case("as") => Some(name.to_ascii_lowercase()),
        _ => None,
    };
    Some((image, name))
}

fn stages(text: &str) -> Vec<Stage> {
    let mut stages: Vec<Stage> = Vec::new();
    for line in text.lines().map(str::trim) {
        let Some(rest) = line.get(..5).filter(|from| from.eq_ignore_ascii_case("FROM ")).map(|_| &line[5..]) else { continue };
        let Some((image, name)) = parse_from(rest) else { continue };
        let lower = image.to_ascii_lowercase();
        let earlier = stages.iter().find(|stage| stage.name.as_deref() == Some(lower.as_str()));
        let component = earlier.map_or_else(|| classify_image(image), |stage| stage.component);
        stages.push(Stage { image: image.to_string(), name, component });
    }
    stages
}

/// `docker-php-ext-install` arguments (flags and variables left out), sorted and unique, named
/// as Lampo's builds of `php` name them when it is known (`gd` is `gd2` before PHP 8.0).
fn php_extensions(text: &str, php: Option<PhpMinor>) -> Vec<String> {
    let mut extensions: Vec<String> = Vec::new();
    for line in text.lines() {
        if let Some((_, rest)) = line.split_once("docker-php-ext-install") {
            let names = rest.split(['&', ';', '\\']).next().unwrap_or_default();
            extensions.extend(names.split_whitespace().filter(|w| !w.starts_with('-') && !w.starts_with('$')).map(|name| lampo_name(name, php)));
        }
    }
    extensions.sort();
    extensions.dedup();
    extensions
}

fn lampo_name(name: &str, php: Option<PhpMinor>) -> String {
    match (ExtensionName::new(name), php) {
        (Ok(extension), Some(php)) => extension.for_php(php).to_string(),
        _ => name.to_string(),
    }
}

/// Records the stages of `file` (the last one as the runtime) and its PHP extensions.
pub(super) fn scan(file: &str, text: &str, findings: &mut Findings) {
    let stages = stages(text);
    let runtime_php = match stages.last().map(|stage| stage.component) {
        Some(Component::Php(php)) => php,
        _ => None,
    };
    let last = stages.len().saturating_sub(1);
    for (index, stage) in stages.into_iter().enumerate() {
        let role = if index == last { PhpRole::DockerfileRuntime } else { PhpRole::Accessory };
        findings.component(stage.component, role, file, &stage.image);
    }
    let extensions = php_extensions(text, runtime_php);
    if !extensions.is_empty() {
        findings.evidence.push(format!("{file} installs PHP extensions: {}", extensions.join(", ")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_with_names_and_inherited_components() {
        let text = "ARG X=1\nFROM --platform=linux/amd64 php:8.2-fpm AS Base\nRUN true\nfrom node:20 as assets\nFROM base\nFROM unknown-stage\n";
        let parsed = stages(text);
        let summary: Vec<_> = parsed.iter().map(|s| (s.image.as_str(), s.name.as_deref(), s.component)).collect();
        let php82 = Component::Php(Some(PhpMinor::parse("8.2").unwrap()));
        assert_eq!(summary, [("php:8.2-fpm", Some("base"), php82), ("node:20", Some("assets"), Component::Other), ("base", None, php82), ("unknown-stage", None, Component::Other)]);
        assert!(stages(" FROM\nFROMAGE x\n").is_empty());
    }

    #[test]
    fn extensions_without_flags_or_variables() {
        let text = "RUN docker-php-ext-install pdo_mysql gd \\\n    && pecl install redis\nRUN docker-php-ext-install -j$(nproc) intl zip gd $EXTRA\n";
        assert_eq!(php_extensions(text, None), ["gd", "intl", "pdo_mysql", "zip"]);
    }

    #[test]
    fn extensions_are_named_as_the_runtime_php_names_them() {
        let text = "RUN docker-php-ext-install gd intl
";
        assert_eq!(php_extensions(text, Some(PhpMinor::parse("7.4").unwrap())), ["gd2", "intl"]);
        assert_eq!(php_extensions(text, Some(PhpMinor::parse("8.3").unwrap())), ["gd", "intl"]);

        let mut findings = Findings::default();
        scan(
            "Dockerfile",
            "FROM composer:2 AS vendor
FROM php:7.4-apache
RUN docker-php-ext-install gd zip
",
            &mut findings,
        );
        assert!(findings.evidence.contains(&"Dockerfile installs PHP extensions: gd2, zip".to_string()), "{:?}", findings.evidence);
    }
}
