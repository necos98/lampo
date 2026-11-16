//! The compose file Docker uses, read line by line into its services (no YAML parser: only
//! `image:`, `build:` / `build.context`, `ports:` and `expose:` of each service matter).

use std::ops::RangeInclusive;
use std::path::Path;

use super::findings::{Findings, PhpRole};
use super::image::{Component, classify_image, expand_defaults, sail_php};

/// Compose file names, in Docker's own lookup order: Docker reads only the first one present.
const COMPOSE_FILES: &[&str] = &["compose.yaml", "compose.yml", "docker-compose.yml", "docker-compose.yaml"];
/// The container port of the web server.
const HTTP_PORT: u16 = 80;
/// Build contexts inside Laravel Sail's runtimes (`vendor/laravel/sail/runtimes/8.3`).
const SAIL_RUNTIMES: &str = "sail/runtimes/";

/// The compose file Docker would use in `folder`, if any.
pub(super) fn compose_file(folder: &Path) -> Option<&'static str> {
    COMPOSE_FILES.iter().copied().find(|file| folder.join(file).is_file())
}

/// One entry of the `services:` block, as far as the detector reads it.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ComposeService {
    pub name: String,
    pub image: Option<String>,
    /// `build:` is present: Compose builds this service's image.
    pub builds: bool,
    /// The build context (`build: ./dir` or `build.context`).
    pub context: Option<String>,
    /// Container ports from `ports:` and `expose:`.
    pub ports: Vec<RangeInclusive<u16>>,
}

impl ComposeService {
    fn named(name: &str) -> Self {
        Self { name: name.to_string(), ..Self::default() }
    }

    pub fn exposes(&self, port: u16) -> bool {
        self.ports.iter().any(|range| range.contains(&port))
    }

    /// How much this service's PHP says about the project's runtime.
    pub fn php_role(&self) -> PhpRole {
        if self.builds {
            PhpRole::BuiltService
        } else if self.exposes(HTTP_PORT) {
            PhpRole::WebService
        } else {
            PhpRole::Accessory
        }
    }

    /// A service-level key (`image: php:8.2`, `build:`, `ports: ["80:80"]`).
    fn key(&mut self, key: &str, value: &str) {
        match key {
            "image" if !value.is_empty() => self.image = Some(value.to_string()),
            "build" => {
                self.builds = true;
                if !value.is_empty() {
                    self.context = Some(value.to_string());
                }
            }
            "ports" | "expose" => self.ports.extend(flow_list(value).filter_map(container_ports)),
            _ => {}
        }
    }

    /// A line nested under the service-level `key` (`context: …`, `- "8080:80"`, `- target: 80`).
    fn nested(&mut self, key: &str, line: &str) {
        match key {
            "build" => {
                if let Some(context) = yaml_value(line, "context").filter(|c| !c.is_empty()) {
                    self.context = Some(context.to_string());
                }
            }
            "ports" | "expose" => {
                let port = match yaml_value(line, "target") {
                    Some(target) => container_ports(target),
                    None => line.strip_prefix('-').and_then(|item| container_ports(unquote(strip_comment(item)))),
                };
                self.ports.extend(port);
            }
            _ => {}
        }
    }
}

/// What the detector reads of a compose file.
#[derive(Debug, Default)]
pub(super) struct ComposeFile {
    pub services: Vec<ComposeService>,
    /// `image:` values outside `services:` (`x-` extension fields, YAML anchors).
    pub loose_images: Vec<String>,
}

/// Where the parser is: which service, and which of its keys, the next lines belong to.
#[derive(Default)]
struct Cursor {
    in_services: bool,
    /// Indentation of the service names.
    service_indent: Option<usize>,
    /// Whether the lines below belong to the last service (`false` after an unreadable entry).
    in_service: bool,
    /// Indentation of the service-level keys.
    key_indent: Option<usize>,
    key: String,
}

impl ComposeFile {
    pub fn parse(text: &str) -> Self {
        let mut file = Self::default();
        let mut at = Cursor::default();
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let indent = line.len() - line.trim_start().len();
            if indent == 0 {
                at = Cursor { in_services: strip_comment(trimmed) == "services:", ..Cursor::default() };
            } else if !at.in_services {
                file.loose_images.extend(yaml_value(trimmed, "image").filter(|v| !v.is_empty()).map(str::to_string));
            } else if indent <= *at.service_indent.get_or_insert(indent) {
                let name = strip_comment(trimmed).strip_suffix(':').map(unquote);
                at.in_service = name.is_some();
                at.key_indent = None;
                file.services.extend(name.map(ComposeService::named));
            } else if let Some(service) = file.services.last_mut().filter(|_| at.in_service) {
                let key_indent = *at.key_indent.get_or_insert(indent);
                // A list may sit at its key's own indentation (`ports:` then `- "80:80"`).
                let is_key = indent < key_indent || (indent == key_indent && !trimmed.starts_with('-'));
                match trimmed.split_once(':').filter(|_| is_key) {
                    Some((key, value)) => {
                        at.key = key.trim().to_string();
                        service.key(&at.key, unquote(strip_comment(value)));
                    }
                    None => service.nested(&at.key, trimmed),
                }
            }
        }
        file
    }
}

/// `value # comment` → `value`.
fn strip_comment(value: &str) -> &str {
    value.split(" #").next().unwrap_or(value).trim()
}

fn unquote(value: &str) -> &str {
    value.trim().trim_matches(|c| c == '"' || c == '\'')
}

/// `key: value` of a YAML line (`- ` list markers, quotes and comments removed).
fn yaml_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let line = line.trim_start().trim_start_matches("- ").trim_start();
    let value = line.strip_prefix(key)?.strip_prefix(':')?;
    Some(unquote(strip_comment(value)))
}

/// The items of an inline `[a, "b"]` list; nothing for a block list (empty value).
fn flow_list(value: &str) -> impl Iterator<Item = &str> {
    let items = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')).unwrap_or_default();
    items.split(',').map(unquote).filter(|item| !item.is_empty())
}

/// The container side of a port mapping: `80`, `8080:80`, `127.0.0.1:8080:80/tcp`,
/// `${APP_PORT:-80}:80`, `8000-8001:80-81` (a range).
fn container_ports(mapping: &str) -> Option<RangeInclusive<u16>> {
    let mapping = expand_defaults(mapping);
    let container = mapping.rsplit(':').next()?.split('/').next()?.trim();
    let (first, last) = container.split_once('-').unwrap_or((container, container));
    let (first, last) = (first.parse().ok()?, last.parse().ok()?);
    (first <= last).then_some(first..=last)
}

/// Records the components of `file`'s services (and of its loose images) into `findings`.
pub(super) fn scan(file: &str, text: &str, findings: &mut Findings) {
    let compose = ComposeFile::parse(text);
    for image in &compose.loose_images {
        findings.component(classify_image(image), PhpRole::Accessory, file, image);
    }
    for service in &compose.services {
        let source = format!("{file} service `{}`", service.name);
        let role = service.php_role();
        if let Some(image) = &service.image {
            findings.component(classify_image(image), role, &source, image);
        }
        if let Some(context) = service.context.as_deref().filter(|c| c.contains(SAIL_RUNTIMES)) {
            findings.component(Component::Php(sail_php(context)), role, &source, context);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn services_with_their_image_build_and_ports() {
        let text = r#"
x-php: &php
  image: php:8.2-cli
services:
  "web":  # front
    image: nginx:1.27
    ports:
    - "${APP_PORT:-80}:80"
    - 443
  app:
    build:
      context: ./docker/php
    expose: [ "9000" ]
  odd entry
    image: ignored
  worker:
    build: .
    ports:
      - target: 8080
        published: 80
volumes:
  data:
"#;
        let compose = ComposeFile::parse(text);
        assert_eq!(compose.loose_images, ["php:8.2-cli"]);
        let names: Vec<_> = compose.services.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["web", "app", "worker"]);
        let [web, app, worker] = &compose.services[..] else { unreachable!() };
        assert_eq!((web.image.as_deref(), web.builds, &web.ports[..]), (Some("nginx:1.27"), false, &[80..=80, 443..=443][..]));
        assert_eq!(web.php_role(), PhpRole::WebService);
        assert_eq!((app.builds, app.context.as_deref(), &app.ports[..]), (true, Some("./docker/php"), &[9000..=9000][..]));
        assert_eq!(app.php_role(), PhpRole::BuiltService);
        assert_eq!((worker.context.as_deref(), &worker.ports[..]), (Some("."), &[8080..=8080][..]));
    }

    #[test]
    fn container_side_of_port_mappings() {
        assert_eq!(container_ports("80"), Some(80..=80));
        assert_eq!(container_ports("127.0.0.1:8080:80/tcp"), Some(80..=80));
        assert_eq!(container_ports("8000-8001:79-81"), Some(79..=81));
        assert_eq!(container_ports("80:8080"), Some(8080..=8080));
        assert_eq!(container_ports("${PORT}"), None);
        assert_eq!(container_ports("81-80"), None);
        assert_eq!(container_ports("99999"), None);
    }

    #[test]
    fn the_first_compose_file_in_docker_order() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(compose_file(dir.path()), None);
        for file in ["docker-compose.yaml", "docker-compose.yml", "compose.yml"] {
            std::fs::write(dir.path().join(file), "").unwrap();
            assert_eq!(compose_file(dir.path()), Some(file));
        }
        std::fs::create_dir(dir.path().join("compose.yaml")).unwrap();
        assert_eq!(compose_file(dir.path()), Some("compose.yml"), "a folder is not a compose file");
    }
}
