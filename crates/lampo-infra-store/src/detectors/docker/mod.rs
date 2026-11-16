//! Docker Compose files and Dockerfiles (Laravel Sail included) → an environment-only hint
//! ([`ENVIRONMENT_ONLY`]): the PHP of the `php:8.2-*` / `laravelsail/php82-composer` image or
//! the Sail runtime (`vendor/laravel/sail/runtimes/8.3`), the MySQL/MariaDB/Redis/Mailpit (and
//! MailHog, mapped to Mailpit) services,
//! and warnings for what Lampo does not provide (PostgreSQL, search engines…).
//!
//! Like Docker, only the first compose file found is read. The PHP is the one of the project's
//! own runtime, not the highest one seen: a service Compose builds (Sail) first, then the one
//! publishing port 80, then the final stage of a Dockerfile, and only then accessory services
//! and build stages ([`findings::PhpRole`]).
//!
//! A compose file running several PHP apps, each from its own folder, also yields one proposed
//! project per app ([`DetectedApp`], [`apps`]); the hint itself stays the folder's.
//!
//! Line-based on purpose (no YAML parser): it only looks at `image:`, `build:`,
//! `ports:`/`expose:`, `environment:`, `depends_on:` and `volumes:` values, the service names
//! under `services:`, `FROM` / `ENV` / `docker-php-ext-install` lines and the document root of
//! mounted Apache/nginx configs.

mod apps;
mod compose;
mod dockerfile;
mod findings;
mod image;
mod mount;
mod webroot;

use std::path::Path;

use lampo_domain::PortResult;
use lampo_domain::detection::{DetectedApp, DetectionHint, ENVIRONMENT_ONLY};
use lampo_domain::ports::ProjectDetector;

use super::MAX_FILE_BYTES;
use super::manifest::preset;
use compose::ComposeFile;
use findings::Findings;

#[derive(Debug, Default, Clone, Copy)]
pub struct DockerDetector;

fn read_small(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes)).into_owned())
}

impl ProjectDetector for DockerDetector {
    fn name(&self) -> &str {
        "docker"
    }

    fn detect(&self, folder: &Path) -> PortResult<Option<DetectionHint>> {
        let mut findings = Findings::default();
        let mut apps = Vec::new();
        if let Some(file) = compose::compose_file(folder)
            && let Some(text) = read_small(&folder.join(file))
        {
            let compose = ComposeFile::parse(&text);
            findings.found = true;
            findings.evidence.push(format!("found {file}"));
            compose::scan(file, &compose, &mut findings);
            apps = apps::apps(folder, &compose);
            if !apps.is_empty() {
                findings.evidence.push(apps_evidence(file, &apps));
            }
        }
        for (name, path) in dockerfile::dockerfiles(folder) {
            if let Some(text) = read_small(&path) {
                findings.found = true;
                dockerfile::scan(&name, &text, &mut findings);
            }
        }
        if !findings.found {
            return Ok(None);
        }
        let php = findings.php();
        let Findings { services, evidence, warnings, .. } = findings;
        let mut hint = DetectionHint::new("docker", preset("generic-php"), php, ENVIRONMENT_ONLY, evidence);
        hint.services = services;
        hint.warnings = warnings;
        hint.apps = apps;
        Ok(Some(hint))
    }
}

/// `compose.yaml runs 2 PHP apps: shop (apps/shop), blog (apps/blog)`.
fn apps_evidence(file: &str, apps: &[DetectedApp]) -> String {
    let list: Vec<String> = apps.iter().map(|app| format!("{} ({})", app.service, app.folder)).collect();
    format!("{file} runs {} PHP apps: {}", apps.len(), list.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lampo_domain::php::PhpMinor;
    use lampo_domain::service::ServiceKind;

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
    }

    #[test]
    fn dockerfiles_in_the_folder_and_docker_dirs() {
        let dockerfile = "FROM --platform=linux/amd64 php:8.1.27-fpm-alpine AS base\nRUN docker-php-ext-install pdo_mysql gd \\\n    && pecl install redis\nRUN docker-php-ext-install -j$(nproc) intl zip\n";
        let hint = detect(&[("docker/php/Dockerfile", dockerfile)]).unwrap();
        assert_eq!(hint.php, php("8.1"));
        assert!(hint.evidence.contains(&"docker/php/Dockerfile uses PHP 8.1 (php:8.1.27-fpm-alpine)".to_string()), "{:?}", hint.evidence);
        assert!(hint.evidence.contains(&"docker/php/Dockerfile installs PHP extensions: gd, intl, pdo_mysql, zip".to_string()), "{:?}", hint.evidence);
        // Between references of the same role (two runtime stages), the highest PHP wins.
        let both = detect(&[("Dockerfile", "FROM php:7.4-apache\n"), ("docker/app.Dockerfile", "FROM php:${PHP_VERSION:-8.3}-cli\n")]).unwrap();
        assert_eq!(both.php, php("8.3"));
    }

    #[test]
    fn only_the_first_compose_file_in_docker_order_is_read() {
        let first = "services:\n  app:\n    image: php:8.3-apache\n  cache:\n    image: redis:7\n";
        let ignored = "services:\n  app:\n    image: php:7.4-apache\n  db:\n    image: postgres:16\n";
        let hint = detect(&[("compose.yaml", first), ("docker-compose.yml", ignored)]).unwrap();
        assert_eq!(hint.php, php("8.3"));
        assert_eq!(hint.services, [ServiceKind::REDIS]);
        assert!(hint.warnings.is_empty(), "{:?}", hint.warnings);
        assert!(!hint.evidence.iter().any(|e| e.contains("docker-compose.yml")), "{:?}", hint.evidence);
        // docker-compose.yml comes before docker-compose.yaml.
        let legacy = detect(&[("docker-compose.yaml", ignored), ("docker-compose.yml", first)]).unwrap();
        assert_eq!(legacy.php, php("8.3"));
    }

    #[test]
    fn the_main_service_gives_the_php_not_the_highest_image() {
        // Sail: the built service wins over a newer PHP in an accessory service.
        let sail = "services:\n  tools:\n    image: php:8.4-cli\n  laravel.test:\n    build:\n      context: ./vendor/laravel/sail/runtimes/8.2\n";
        assert_eq!(detect(&[("compose.yaml", sail)]).unwrap().php, php("8.2"));
        // The service serving port 80 wins over a queue worker.
        let web = "services:\n  worker:\n    image: php:8.4-cli\n  app:\n    image: php:8.1-apache\n    ports:\n      - \"127.0.0.1:8080:80/tcp\"\n";
        assert_eq!(detect(&[("compose.yaml", web)]).unwrap().php, php("8.1"));
        let long = "services:\n  worker:\n    image: php:8.4-cli\n  app:\n    image: php:8.1-apache\n    ports:\n      - target: 80\n        published: 8080\n";
        assert_eq!(detect(&[("compose.yaml", long)]).unwrap().php, php("8.1"));
        // Port 8080 inside the container is not port 80.
        let not_http = "services:\n  a:\n    image: php:8.1-cli\n    ports: [\"80:8080\"]\n  b:\n    image: php:8.3-cli\n";
        assert_eq!(detect(&[("compose.yaml", not_http)]).unwrap().php, php("8.3"));
    }

    #[test]
    fn the_final_dockerfile_stage_gives_the_php_not_a_build_stage() {
        let multi = "FROM php:8.4-cli AS vendor\nRUN composer install\nFROM php:8.2-fpm AS runtime\nCOPY --from=vendor /app /app\n";
        assert_eq!(detect(&[("Dockerfile", multi)]).unwrap().php, php("8.2"));
        // A final stage built on an earlier one inherits its PHP (stage names are case-insensitive).
        let derived = "FROM php:8.1-fpm AS Base\nFROM php:8.4-cli AS tools\nFROM base\n";
        assert_eq!(detect(&[("Dockerfile", derived)]).unwrap().php, php("8.1"));
        // The image built by the compose service wins over an accessory PHP image.
        let compose = "services:\n  app:\n    build: .\n  worker:\n    image: php:8.4-cli\n";
        assert_eq!(detect(&[("compose.yaml", compose), ("Dockerfile", "FROM php:7.4-apache\n")]).unwrap().php, php("7.4"));
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(name)
    }

    fn env(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn a_compose_file_with_several_php_apps_proposes_one_project_each() {
        let hint = DockerDetector.detect(&fixture("compose-multi-app")).unwrap().unwrap();
        let summary: Vec<_> = hint.apps.iter().map(|a| (a.service.as_str(), a.folder.as_str(), a.php.map(|p| p.to_string()))).collect();
        assert_eq!(
            summary,
            [("shop", "apps/shop", Some("8.3".into())), ("blog", "apps/blog", Some("7.4".into())), ("legacy", "legacy", Some("8.1".into()))],
            "shop-queue shares shop's folder"
        );
        let [shop, blog, legacy] = &hint.apps[..] else { unreachable!() };

        assert_eq!(shop.extensions, ["intl", "pdo_mysql"]);
        assert_eq!(shop.document_root.as_deref(), Some("public"), "ENV APACHE_DOCUMENT_ROOT of its Dockerfile");
        assert_eq!(shop.environment, env(&[("APP_ENV", "local"), ("DB_DATABASE", "shop"), ("DB_HOST", "mysql"), ("REDIS_HOST", "redis")]));
        assert_eq!(shop.service_hosts, ["mysql", "redis"]);

        assert_eq!(blog.extensions, ["gd2", "mysqli"], "named as PHP 7.4 names them");
        assert_eq!(blog.document_root.as_deref(), Some("web"), "root of the nginx block passing PHP to it");
        assert_eq!(blog.environment, env(&[("WORDPRESS_DB_HOST", "mysql"), ("WORDPRESS_DB_NAME", "blog")]));
        assert_eq!(blog.service_hosts, ["mysql"]);

        assert!(legacy.extensions.is_empty());
        assert_eq!(legacy.document_root.as_deref(), Some("htdocs"), "DocumentRoot of its mounted vhost");
        assert_eq!(legacy.environment, env(&[("APP_DEBUG", "1")]));
        assert_eq!(legacy.service_hosts, ["mysql"]);

        assert!(hint.evidence.contains(&"docker-compose.yml runs 3 PHP apps: shop (apps/shop), blog (apps/blog), legacy (legacy)".to_string()), "{:?}", hint.evidence);
        assert_eq!(hint.services, [ServiceKind::MYSQL, ServiceKind::REDIS], "the folder's own result is unchanged");
    }

    #[test]
    fn one_php_app_proposes_no_apps() {
        // A web server and its queue worker share one folder: one app, the folder's own result.
        let one = "services:\n  app:\n    build: .\n    volumes: [\".:/var/www/html\"]\n  worker:\n    build: .\n    volumes:\n      - .:/var/www/html\n";
        assert!(detect(&[("compose.yaml", one), ("Dockerfile", "FROM php:8.3-cli\n")]).unwrap().apps.is_empty());
        // Sail: the runtime context is not the code, the mounted folder is.
        let sail = "services:\n  laravel.test:\n    build:\n      context: ./vendor/laravel/sail/runtimes/8.4\n    volumes:\n      - '.:/var/www/html'\n";
        assert!(detect(&[("compose.yaml", sail)]).unwrap().apps.is_empty());
    }

    #[test]
    fn apps_outside_the_folder_keep_their_path_and_nothing_outside_is_read() {
        let compose = "services:\n  a:\n    image: php:8.2-fpm\n    volumes:\n      - ../shared/a:/app\n  b:\n    build:\n      context: ./b\n      dockerfile: ../../outside/Dockerfile\n    environment:\n      APACHE_DOCUMENT_ROOT: /var/www/html/public\n  c:\n    image: php:8.4-cli\n";
        let hint = detect(&[("compose.yaml", compose), ("b/index.php", "")]).unwrap();
        let summary: Vec<_> = hint.apps.iter().map(|a| (a.service.as_str(), a.folder.as_str(), a.php, a.document_root.as_deref())).collect();
        // `b` has no readable Dockerfile (outside the folder): not a PHP service; `c` has no code folder.
        assert!(summary.is_empty(), "{summary:?}");
        let both = compose.replace("context: ./b\n      dockerfile: ../../outside/Dockerfile", "context: ./b");
        let hint = detect(&[("compose.yaml", both.as_str()), ("b/Dockerfile", "FROM php:8.1-apache\n")]).unwrap();
        let summary: Vec<_> = hint.apps.iter().map(|a| (a.service.as_str(), a.folder.as_str(), a.php, a.document_root.as_deref())).collect();
        assert_eq!(summary, [("a", "../shared/a", php("8.2"), None), ("b", "b", php("8.1"), Some("public"))]);
    }

    #[test]
    fn nothing_docker_means_no_hint() {
        assert!(detect(&[("composer.json", "{}")]).is_none());
        let unknown = detect(&[("compose.yaml", "services:\n  web:\n    image: nginx:1.27\n")]).unwrap();
        assert_eq!((unknown.php, unknown.services.len(), unknown.warnings.len()), (None, 0, 0));
    }
}
