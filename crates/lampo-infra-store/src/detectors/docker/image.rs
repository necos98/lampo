//! What an image reference (`php:8.2-apache`, `laravelsail/php83-composer`, `mysql:8.0`) or a
//! Sail runtime path stands for.

use lampo_domain::php::PhpMinor;

/// What an image or build reference stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Component {
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
pub(super) fn expand_defaults(value: &str) -> String {
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
pub(super) fn sail_php(value: &str) -> Option<PhpMinor> {
    if let Some((_, after)) = value.split_once("sail/runtimes/") {
        return php_from_tag(after.split('/').next()?);
    }
    let after = value.split_once("sail-")?.1;
    php_from_tag(after.split('/').next()?)
}

/// Classifies an image reference (`php:8.2-apache`, `docker.io/library/mysql:8.0`,
/// `bitnami/postgresql:16`, `laravelsail/php83-composer:latest`).
pub(super) fn classify_image(image: &str) -> Component {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn php(v: &str) -> Option<PhpMinor> {
        Some(PhpMinor::parse(v).unwrap())
    }

    #[test]
    fn image_tags() {
        assert_eq!(classify_image("php:8-apache"), Component::Php(None));
        assert_eq!(classify_image("docker.io/library/php:7.3.33-fpm"), Component::Php(php("7.3")));
        assert_eq!(classify_image("registry.example.com:5000/mysql"), Component::Mysql);
        assert_eq!(classify_image("bitnami/postgresql:16"), Component::Postgres);
        assert_eq!(classify_image("valkey/valkey:8"), Component::Redis);
        assert_eq!(classify_image("docker.io/axllent/mailpit:v1.33"), Component::Mail { mailhog: false });
        assert_eq!(expand_defaults("php:${V:-8.2}-fpm ${X}"), "php:8.2-fpm ${X}");
    }
}
