//! The compose file Docker uses, read line by line into its services (no YAML parser: only
//! `image:`, `build:` (`context`, `dockerfile`), `ports:`, `expose:`, `environment:`,
//! `depends_on:` and the bind mounts of `volumes:` of each service matter).

use std::ops::RangeInclusive;
use std::path::Path;

use super::findings::{Findings, PhpRole};
use super::image::{Component, classify_image, expand_defaults, sail_php};
pub(super) use super::mount::Mount;

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
    /// `build.dockerfile`, relative to the context.
    pub dockerfile: Option<String>,
    /// Container ports from `ports:` and `expose:`.
    pub ports: Vec<RangeInclusive<u16>>,
    /// `environment:` (`KEY: value` map or `- KEY=value` list), in file order.
    pub environment: Vec<(String, String)>,
    /// Service names of `depends_on:` (list or map).
    pub depends_on: Vec<String>,
    /// Entries of `volumes:`, short (`./src:/var/www/html:ro`) and long (`source`/`target`)
    /// syntax; named volumes included ([`Mount::is_bind`] tells them apart).
    pub mounts: Vec<Mount>,
}

/// `KEY=value` (or `KEY` alone) of an `environment:` list item.
fn env_item(item: &str) -> (String, String) {
    let (key, value) = item.split_once('=').unwrap_or((item, ""));
    (key.trim().to_string(), value.trim().to_string())
}

impl ComposeService {
    fn named(name: &str) -> Self {
        Self { name: name.to_string(), ..Self::default() }
    }

    pub fn exposes(&self, port: u16) -> bool {
        self.ports.iter().any(|range| range.contains(&port))
    }

    /// The build context when it is one of Laravel Sail's runtimes (it names the PHP).
    pub fn sail_runtime(&self) -> Option<&str> {
        self.context.as_deref().filter(|context| context.contains(SAIL_RUNTIMES))
    }

    /// The bind mounts of `volumes:`.
    pub fn bind_mounts(&self) -> impl Iterator<Item = &Mount> {
        self.mounts.iter().filter(|m| m.is_bind() && !m.target.is_empty())
    }

    /// The value of the `environment:` variable `key`, if set.
    pub fn env(&self, key: &str) -> Option<&str> {
        self.environment.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
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
            "environment" => self.environment.extend(flow_list(value).map(env_item)),
            "depends_on" => self.depends_on.extend(flow_list(value).map(str::to_string)),
            "volumes" => self.mounts.extend(flow_list(value).filter_map(Mount::parse_short)),
            _ => {}
        }
    }

    /// A line nested under the service-level `key` (`context: …`, `- "8080:80"`, `- target: 80`);
    /// `direct` when it is a direct child of the key (not deeper: `condition:` of a
    /// `depends_on` entry).
    fn nested(&mut self, key: &str, line: &str, direct: bool) {
        let item = line.strip_prefix('-').map(|item| unquote(strip_comment(item)));
        match key {
            "build" => {
                if let Some(context) = yaml_value(line, "context").filter(|c| !c.is_empty()) {
                    self.context = Some(context.to_string());
                }
                if let Some(dockerfile) = yaml_value(line, "dockerfile").filter(|d| !d.is_empty()) {
                    self.dockerfile = Some(dockerfile.to_string());
                }
            }
            "environment" if direct => match (item, line.split_once(':')) {
                (Some(item), _) => self.environment.push(env_item(item)),
                (None, Some((key, value))) => self.environment.push((unquote(key).to_string(), unquote(strip_comment(value)).to_string())),
                (None, None) => {}
            },
            "depends_on" if direct => {
                let name = item.or_else(|| strip_comment(line).strip_suffix(':').map(unquote));
                self.depends_on.extend(name.filter(|n| !n.is_empty()).map(str::to_string));
            }
            "volumes" => match item {
                Some(item) => self.mounts.extend(Mount::from_item(item)),
                None => {
                    if let Some(mount) = self.mounts.last_mut() {
                        mount.long_key(line);
                    }
                }
            },
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
    /// Indentation of the direct children of `key`.
    nested_indent: Option<usize>,
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
                        at.nested_indent = None;
                        service.key(&at.key, unquote(strip_comment(value)));
                    }
                    None => {
                        let direct = indent <= *at.nested_indent.get_or_insert(indent);
                        service.nested(&at.key, trimmed, direct);
                    }
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

/// `"value"`, `'value'` → `value`.
pub(super) fn unquote(value: &str) -> &str {
    value.trim().trim_matches(|c| c == '"' || c == '\'')
}

/// `key: value` of a YAML line (`- ` list markers, quotes and comments removed).
pub(super) fn yaml_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
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
pub(super) fn scan(file: &str, compose: &ComposeFile, findings: &mut Findings) {
    for image in &compose.loose_images {
        findings.component(classify_image(image), PhpRole::Accessory, file, image);
    }
    for service in &compose.services {
        let source = format!("{file} service `{}`", service.name);
        let role = service.php_role();
        if let Some(image) = &service.image {
            findings.component(classify_image(image), role, &source, image);
        }
        if let Some(context) = service.sail_runtime() {
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
    fn environment_dependencies_and_mounts() {
        let text = r#"
services:
  app:
    build:
      context: ./app
      dockerfile: docker/Dockerfile
    environment:
      DB_HOST: mysql   # the compose service
      'APP_URL': "http://localhost:8080"
    depends_on:
      mysql:
        condition: service_healthy
      redis:
        condition: service_started
    volumes:
      - ./app:/var/www/html:cached
      - C:\sites\shared:/srv/shared
      - vendor:/var/www/html/vendor
      - /var/www/html/node_modules
      - type: bind
        source: ./docker/vhost.conf
        target: /etc/apache2/sites-enabled/000-default.conf
        read_only: true
  worker:
    environment:
    - QUEUE=default
    - DEBUG
    depends_on: [app, "mysql"]
    volumes: ["./app:/app"]
"#;
        let compose = ComposeFile::parse(text);
        let [app, worker] = &compose.services[..] else { unreachable!() };
        assert_eq!((app.context.as_deref(), app.dockerfile.as_deref()), (Some("./app"), Some("docker/Dockerfile")));
        assert_eq!(app.environment, [("DB_HOST".to_string(), "mysql".to_string()), ("APP_URL".into(), "http://localhost:8080".into())]);
        assert_eq!((app.env("DB_HOST"), app.env("MISSING")), (Some("mysql"), None));
        assert_eq!(app.depends_on, ["mysql", "redis"], "conditions are not services");
        let binds: Vec<_> = app.bind_mounts().map(|m| (m.source.as_str(), m.target.as_str())).collect();
        assert_eq!(binds, [("./app", "/var/www/html"), ("C:\\sites\\shared", "/srv/shared"), ("./docker/vhost.conf", "/etc/apache2/sites-enabled/000-default.conf")]);
        assert_eq!(app.mounts.len(), 4, "the named volume is kept, the anonymous one is not a mount: {:?}", app.mounts);
        assert_eq!(worker.environment, [("QUEUE".to_string(), "default".to_string()), ("DEBUG".into(), String::new())]);
        assert_eq!(worker.depends_on, ["app", "mysql"]);
        assert_eq!(worker.bind_mounts().map(|m| m.target.as_str()).collect::<Vec<_>>(), ["/app"]);
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
