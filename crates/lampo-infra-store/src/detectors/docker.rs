//! Docker Compose files and Dockerfiles (Laravel Sail included) → an environment-only hint
//! ([`ENVIRONMENT_ONLY`]): the PHP of the `php:8.2-*` / `laravelsail/php82-composer` image or
//! the Sail runtime (`vendor/laravel/sail/runtimes/8.3`), the MySQL/MariaDB/Redis/Mailpit (and
//! MailHog, mapped to Mailpit) services,
//! and warnings for what Lampo does not provide (PostgreSQL, search engines…).
//!
//! Line-based on purpose (no YAML parser): it only looks at `image:`, `context:`/`dockerfile:`
//! values, the service names under `services:` and `FROM` / `docker-php-ext-install` lines.

use std::path::Path;

use lampo_domain::PortResult;
use lampo_domain::detection::{DetectionHint, ENVIRONMENT_ONLY};
use lampo_domain::php::PhpMinor;
use lampo_domain::ports::ProjectDetector;
use lampo_domain::service::ServiceKind;

use super::manifest::preset;

/// Compose file names, in Docker's own lookup order.
const COMPOSE_FILES: &[&str] = &["compose.yaml", "compose.yml", "docker-compose.yml", "docker-compose.yaml"];
/// Where Dockerfiles usually are, besides the project folder itself.
const DOCKER_DIRS: &[&str] = &["docker", ".docker"];
/// Files larger than this are not configuration: skipped.
const MAX_FILE_BYTES: u64 = 256 * 1024;

#[derive(Debug, Default, Clone, Copy)]
pub struct DockerDetector;

/// What an image or build reference stands for.
#[derive(Debug, PartialEq, Eq)]
enum Component {
    Php(Option<PhpMinor>),
    Mysql,
    MariaDb,
    Redis,
    Postgres,
    /// Mailpit, or MailHog (`true`), which Lampo replaces with Mailpit.
    Mail {
        mailhog: bool,
    },
    /// Something Lampo does not run (its label).
    Unsupported(&'static str),
    Other,
}

/// Images Lampo has no counterpart for, by repository-name fragment.
const UNSUPPORTED: &[(&str, &str)] = &[
    ("meilisearch", "Meilisearch"),
    ("elasticsearch", "Elasticsearch"),
    ("opensearch", "OpenSearch"),
    ("typesense", "Typesense"),
    ("memcached", "Memcached"),
    ("minio", "MinIO"),
    ("rabbitmq", "RabbitMQ"),
    ("soketi", "Soketi"),
    ("selenium", "Selenium"),
    ("mongo", "MongoDB"),
];

/// `${PHP_VERSION:-8.2}` → `8.2`; `${X}` stays as is (unknown).
fn expand_defaults(value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = &rest[start + 2..start + end];
        match inner.split_once(":-").or_else(|| inner.split_once('-')) {
            Some((_, default)) => out.push_str(default),
            None => out.push_str(&rest[start..=start + end]),
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out
}

/// `key: value` of a YAML line (`- ` list markers, quotes and comments removed).
fn yaml_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let line = line.trim_start().trim_start_matches("- ").trim_start();
    let value = line.strip_prefix(key)?.strip_prefix(':')?;
    let value = value.split(" #").next().unwrap_or(value).trim();
    Some(value.trim_matches(|c| c == '"' || c == '\''))
}

/// `8.2`, `8.2.12`, `8.2-apache`, `8.2.12-fpm-alpine` → PHP 8.2; `8-apache`, `latest` → none.
fn php_from_tag(tag: &str) -> Option<PhpMinor> {
    let version = tag.split('-').next()?;
    let mut parts = version.split('.');
    let (major, minor) = (parts.next()?, parts.next()?);
    PhpMinor::parse(&format!("{major}.{minor}")).ok()
}

/// `82` → 8.2, `74` → 7.4 (Sail's `laravelsail/php82-composer`).
fn php_from_digits(digits: &str) -> Option<PhpMinor> {
    let mut chars = digits.chars();
    let (major, minor) = (chars.next()?, chars.next()?);
    if chars.next().is_some() {
        return None;
    }
    PhpMinor::parse(&format!("{major}.{minor}")).ok()
}

/// `…/sail/runtimes/8.3`, `sail-8.3/app` → 8.3.
fn sail_php(value: &str) -> Option<PhpMinor> {
    if let Some((_, after)) = value.split_once("sail/runtimes/") {
        return php_from_tag(after.split('/').next()?);
    }
    let after = value.split_once("sail-")?.1;
    php_from_tag(after.split('/').next()?)
}

/// Classifies an image reference (`php:8.2-apache`, `docker.io/library/mysql:8.0`,
/// `bitnami/postgresql:16`, `laravelsail/php83-composer:latest`).
fn classify_image(image: &str) -> Component {
    let image = expand_defaults(image).to_ascii_lowercase();
    if let Some(php) = sail_php(&image) {
        return Component::Php(Some(php));
    }
    let (repository, tag) = match image.rsplit_once(':') {
        Some((repo, tag)) if !tag.contains('/') => (repo.to_string(), tag.to_string()),
        _ => (image.clone(), String::new()),
    };
    let name = repository.rsplit('/').next().unwrap_or(&repository).to_string();
    if name == "php" {
        return Component::Php(php_from_tag(&tag));
    }
    if let Some(digits) = name.strip_prefix("php").and_then(|rest| rest.split('-').next()).filter(|d| d.len() == 2 && d.chars().all(|c| c.is_ascii_digit())) {
        return Component::Php(php_from_digits(digits));
    }
    match name.as_str() {
        "mysql" | "mysql-server" | "percona" | "percona-server" => return Component::Mysql,
        "mariadb" => return Component::MariaDb,
        "redis" | "redis-stack" | "valkey" => return Component::Redis,
        "postgres" | "postgresql" | "postgis" => return Component::Postgres,
        _ if name.contains("mailpit") => return Component::Mail { mailhog: false },
        _ if name.contains("mailhog") => return Component::Mail { mailhog: true },
        _ => {}
    }
    UNSUPPORTED.iter().find(|(fragment, _)| name.contains(fragment)).map_or(Component::Other, |(_, label)| Component::Unsupported(label))
}

/// What the files of a folder add up to.
#[derive(Default)]
struct Findings {
    php: Option<PhpMinor>,
    services: Vec<ServiceKind>,
    evidence: Vec<String>,
    warnings: Vec<String>,
    found: bool,
}

impl Findings {
    fn service(&mut self, kind: ServiceKind) {
        if !self.services.contains(&kind) {
            self.services.push(kind);
        }
    }

    fn warn(&mut self, warning: String) {
        if !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }

    /// Records one image/build reference found in `source` (`compose.yaml service app`).
    fn component(&mut self, component: Component, source: &str, reference: &str) {
        match component {
            Component::Php(php) => {
                self.php = self.php.max(php);
                match php {
                    Some(php) => self.evidence.push(format!("{source} uses PHP {php} ({reference})")),
                    None => self.evidence.push(format!("{source} uses PHP without a minor version ({reference})")),
                }
            }
            Component::Mysql => {
                self.service(ServiceKind::MYSQL);
                self.evidence.push(format!("{source} uses MySQL ({reference})"));
            }
            Component::MariaDb => {
                self.service(ServiceKind::MYSQL);
                self.evidence.push(format!("{source} uses MariaDB ({reference})"));
                self.warn("MariaDB is not available in Lampo: the project gets MySQL; check that MariaDB-specific SQL and dumps import cleanly".into());
            }
            Component::Redis => {
                self.service(ServiceKind::REDIS);
                self.evidence.push(format!("{source} uses Redis ({reference})"));
            }
            Component::Mail { mailhog } => {
                self.service(ServiceKind::MAILPIT);
                match mailhog {
                    false => self.evidence.push(format!("{source} uses Mailpit ({reference})")),
                    true => {
                        self.evidence.push(format!("{source} uses MailHog ({reference})"));
                        self.warn("MailHog is not available in Lampo: the project gets Mailpit, a drop-in replacement (SMTP on the same port, web UI and API differ)".into());
                    }
                }
            }
            Component::Postgres => {
                self.evidence.push(format!("{source} uses PostgreSQL ({reference})"));
                self.warn("PostgreSQL is not supported by Lampo: the database cannot be migrated to Lampo".into());
            }
            Component::Unsupported(label) => {
                self.evidence.push(format!("{source} uses {label} ({reference})"));
                self.warn(format!("{label} is not provided by Lampo: keep running it outside Lampo"));
            }
            Component::Other => {}
        }
    }
}

fn read_small(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes)).into_owned())
}

/// The service a line belongs to, tracked through the `services:` block's indentation.
struct ServiceTracker {
    in_services: bool,
    level: Option<usize>,
    current: Option<String>,
}

impl ServiceTracker {
    fn new() -> Self {
        Self { in_services: false, level: None, current: None }
    }

    fn feed(&mut self, line: &str) {
        let indent = line.len() - line.trim_start().len();
        let text = line.trim();
        if indent == 0 {
            self.in_services = text == "services:";
            self.level = None;
            self.current = None;
            return;
        }
        if !self.in_services {
            return;
        }
        let level = *self.level.get_or_insert(indent);
        if indent == level {
            self.current = text.strip_suffix(':').map(|name| name.trim_matches(|c| c == '"' || c == '\'').to_string());
        }
    }
}

fn scan_compose(file: &str, text: &str, findings: &mut Findings) {
    let mut tracker = ServiceTracker::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        tracker.feed(line);
        let source = match &tracker.current {
            Some(service) => format!("{file} service `{service}`"),
            None => file.to_string(),
        };
        if let Some(image) = yaml_value(line, "image").filter(|v| !v.is_empty()) {
            findings.component(classify_image(image), &source, image);
        } else if let Some(context) = yaml_value(line, "context").or_else(|| yaml_value(line, "build")).filter(|v| v.contains("sail/runtimes/")) {
            findings.component(Component::Php(sail_php(context)), &source, context);
        }
    }
}

fn scan_dockerfile(file: &str, text: &str, findings: &mut Findings) {
    let mut extensions: Vec<String> = Vec::new();
    for line in text.lines().map(str::trim) {
        let upper = line.to_ascii_uppercase();
        if let Some(rest) = upper.strip_prefix("FROM ").map(|_| line[5..].trim()) {
            let image = rest.split_whitespace().find(|w| !w.starts_with("--")).unwrap_or_default();
            findings.component(classify_image(image), file, image);
        }
        if let Some((_, rest)) = line.split_once("docker-php-ext-install") {
            let names = rest.split(['&', ';', '\\']).next().unwrap_or_default();
            extensions.extend(names.split_whitespace().filter(|w| !w.starts_with('-') && !w.starts_with('$')).map(str::to_string));
        }
    }
    if !extensions.is_empty() {
        extensions.sort();
        extensions.dedup();
        findings.evidence.push(format!("{file} installs PHP extensions: {}", extensions.join(", ")));
    }
}

/// `Dockerfile`, `Dockerfile.dev`, `app.Dockerfile`…
fn is_dockerfile(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "dockerfile" || lower.starts_with("dockerfile.") || lower.ends_with(".dockerfile")
}

/// Dockerfiles in the folder, in `docker/`/`.docker/` and one level below those.
fn dockerfiles(folder: &Path) -> Vec<(String, std::path::PathBuf)> {
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

impl ProjectDetector for DockerDetector {
    fn name(&self) -> &str {
        "docker"
    }

    fn detect(&self, folder: &Path) -> PortResult<Option<DetectionHint>> {
        let mut findings = Findings::default();
        for file in COMPOSE_FILES {
            if let Some(text) = read_small(&folder.join(file)) {
                findings.found = true;
                findings.evidence.push(format!("found {file}"));
                scan_compose(file, &text, &mut findings);
            }
        }
        for (name, path) in dockerfiles(folder) {
            if let Some(text) = read_small(&path) {
                findings.found = true;
                scan_dockerfile(&name, &text, &mut findings);
            }
        }
        if !findings.found {
            return Ok(None);
        }
        let Findings { php, services, evidence, warnings, .. } = findings;
        let mut hint = DetectionHint::new("docker", preset("generic-php"), php, ENVIRONMENT_ONLY, evidence);
        hint.services = services;
        hint.warnings = warnings;
        Ok(Some(hint))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            let p = dir.path().join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        }
        dir
    }

    fn detect(files: &[(&str, &str)]) -> Option<DetectionHint> {
        DockerDetector.detect(folder(files).path()).unwrap()
    }

    fn php(v: &str) -> Option<PhpMinor> {
        Some(PhpMinor::parse(v).unwrap())
    }

    const COMPOSE: &str = r#"
# Development stack
services:
  app:
    image: "php:8.2-apache"
    ports:
      - "8080:80"
    depends_on:
      - db
  db:
    image: mysql:8.0  # same as production
    environment:
      MYSQL_DATABASE: shop
  cache:
    image: redis:7-alpine
  search:
    image: getmeili/meilisearch:v1.6
volumes:
  dbdata:
"#;

    #[test]
    fn compose_file_gives_php_services_and_evidence() {
        let hint = detect(&[("docker-compose.yml", COMPOSE)]).unwrap();
        assert_eq!((hint.project_type.as_str(), hint.confidence, hint.preset.as_str()), ("docker", ENVIRONMENT_ONLY, "generic-php"));
        assert_eq!(hint.php, php("8.2"));
        assert_eq!(hint.services, [ServiceKind::MYSQL, ServiceKind::REDIS]);
        assert!(hint.evidence.contains(&"found docker-compose.yml".to_string()));
        assert!(hint.evidence.contains(&"docker-compose.yml service `app` uses PHP 8.2 (php:8.2-apache)".to_string()), "{:?}", hint.evidence);
        assert!(hint.evidence.contains(&"docker-compose.yml service `db` uses MySQL (mysql:8.0)".to_string()), "{:?}", hint.evidence);
        assert_eq!(hint.warnings, ["Meilisearch is not provided by Lampo: keep running it outside Lampo"]);
    }

    #[test]
    fn laravel_sail_runtimes_and_images() {
        let sail = r#"services:
    laravel.test:
        build:
            context: './vendor/laravel/sail/runtimes/8.3'
            dockerfile: Dockerfile
        image: 'sail-8.3/app'
    mariadb:
        image: 'mariadb:11'
    pgsql:
        image: 'postgres:17'
    mailpit:
        image: 'axllent/mailpit:latest'
"#;
        let hint = detect(&[("compose.yaml", sail)]).unwrap();
        assert_eq!(hint.php, php("8.3"));
        assert_eq!(hint.services, [ServiceKind::MYSQL, ServiceKind::MAILPIT]);
        assert!(hint.evidence.iter().any(|e| e.contains("service `laravel.test` uses PHP 8.3")), "{:?}", hint.evidence);
        assert!(hint.evidence.contains(&"compose.yaml service `mailpit` uses Mailpit (axllent/mailpit:latest)".to_string()), "{:?}", hint.evidence);
        assert!(hint.warnings.iter().any(|w| w.starts_with("MariaDB")));
        assert!(hint.warnings.iter().any(|w| w.starts_with("PostgreSQL is not supported")));
        assert!(!hint.warnings.iter().any(|w| w.contains("Mailpit")), "Mailpit is supported: {:?}", hint.warnings);
        // Older Sail: a prebuilt image per PHP.
        let old = detect(&[("docker-compose.yml", "services:\n  laravel.test:\n    image: laravelsail/php74-composer:latest\n")]).unwrap();
        assert_eq!(old.php, php("7.4"));
    }

    #[test]
    fn mailhog_is_replaced_by_mailpit_with_a_note() {
        let hint = detect(&[("docker-compose.yml", "services:\n  mail:\n    image: mailhog/mailhog:v1.0.1\n")]).unwrap();
        assert_eq!(hint.services, [ServiceKind::MAILPIT]);
        assert!(hint.evidence.contains(&"docker-compose.yml service `mail` uses MailHog (mailhog/mailhog:v1.0.1)".to_string()), "{:?}", hint.evidence);
        assert_eq!(hint.warnings.len(), 1);
        assert!(hint.warnings[0].starts_with("MailHog is not available in Lampo: the project gets Mailpit"), "{:?}", hint.warnings);
        assert_eq!(classify_image("docker.io/axllent/mailpit:v1.33"), Component::Mail { mailhog: false });
    }

    #[test]
    fn dockerfiles_in_the_folder_and_docker_dirs() {
        let dockerfile = "FROM --platform=linux/amd64 php:8.1.27-fpm-alpine AS base\nRUN docker-php-ext-install pdo_mysql gd \\\n    && pecl install redis\nRUN docker-php-ext-install -j$(nproc) intl zip\n";
        let hint = detect(&[("docker/php/Dockerfile", dockerfile)]).unwrap();
        assert_eq!(hint.php, php("8.1"));
        assert!(hint.evidence.contains(&"docker/php/Dockerfile uses PHP 8.1 (php:8.1.27-fpm-alpine)".to_string()), "{:?}", hint.evidence);
        assert!(hint.evidence.contains(&"docker/php/Dockerfile installs PHP extensions: gd, intl, pdo_mysql, zip".to_string()), "{:?}", hint.evidence);
        // The highest PHP of several files wins.
        let both = detect(&[("Dockerfile", "FROM php:7.4-apache\n"), ("compose.yml", "services:\n  app:\n    image: php:${PHP_VERSION:-8.3}-cli\n")]).unwrap();
        assert_eq!(both.php, php("8.3"));
    }

    #[test]
    fn nothing_docker_means_no_hint() {
        assert!(detect(&[("composer.json", "{}")]).is_none());
        let unknown = detect(&[("compose.yaml", "services:\n  web:\n    image: nginx:1.27\n")]).unwrap();
        assert_eq!((unknown.php, unknown.services.len(), unknown.warnings.len()), (None, 0, 0));
    }

    #[test]
    fn image_tags() {
        assert_eq!(classify_image("php:8-apache"), Component::Php(None));
        assert_eq!(classify_image("docker.io/library/php:7.3.33-fpm"), Component::Php(php("7.3")));
        assert_eq!(classify_image("registry.example.com:5000/mysql"), Component::Mysql);
        assert_eq!(classify_image("bitnami/postgresql:16"), Component::Postgres);
        assert_eq!(classify_image("valkey/valkey:8"), Component::Redis);
        assert_eq!(expand_defaults("php:${V:-8.2}-fpm ${X}"), "php:8.2-fpm ${X}");
    }
}
