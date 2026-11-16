//! Where a PHP container serves pages from: `APACHE_DOCUMENT_ROOT` (the convention of the
//! official `php:*-apache` images), Apache's `DocumentRoot`, nginx's `root`. All of them are
//! container paths, made relative to the app's code by [`relative`].

use super::compose::unquote;
use super::image::expand_defaults;

/// Where the official `php` images keep the code when no mount says otherwise.
pub(super) const DEFAULT_WEB_ROOT: &str = "/var/www/html";
/// Document-root variable of the official `php:*-apache` images (and their derivatives).
pub(super) const APACHE_DOCUMENT_ROOT: &str = "APACHE_DOCUMENT_ROOT";

/// What follows the keyword `name` (any case) and its whitespace on `line`.
fn after_keyword<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let head = line.get(..name.len()).filter(|head| head.eq_ignore_ascii_case(name))?;
    Some(line[head.len()..].strip_prefix(char::is_whitespace)?.trim())
}

/// The value of `directive` on a config line (`DocumentRoot "/x"`, `root /x;`), any case.
fn directive<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    after_keyword(line, name).map(|rest| unquote(rest.trim_end_matches(';')))
}

/// `ENV APACHE_DOCUMENT_ROOT=/var/www/html/public` or `ENV APACHE_DOCUMENT_ROOT /…` of a
/// Dockerfile (the last one wins, as in Docker).
pub(super) fn dockerfile_env(text: &str) -> Option<String> {
    let assignment = format!("{APACHE_DOCUMENT_ROOT}=");
    let mut found = None;
    for line in text.lines().map(str::trim) {
        let Some(rest) = after_keyword(line, "ENV") else { continue };
        let value = match directive(rest, APACHE_DOCUMENT_ROOT) {
            Some(value) => Some(value),
            None => rest.split_whitespace().find_map(|word| word.strip_prefix(&assignment)).map(unquote),
        };
        found = value.map(str::to_string).or(found);
    }
    found
}

/// The first `DocumentRoot` of an Apache config.
pub(super) fn apache_document_root(text: &str) -> Option<String> {
    text.lines().find_map(|line| directive(line.trim(), "DocumentRoot")).map(str::to_string)
}

/// `server {` (and `server{`), not `server_name`.
fn opens_server_block(line: &str) -> bool {
    line.strip_prefix("server").is_some_and(|rest| rest.trim_start().starts_with('{'))
}

/// The host of a `fastcgi_pass shop:9000;` on `line`, also inside a one-line `location { … }`.
fn fastcgi_host(line: &str) -> Option<&str> {
    const DIRECTIVE: &str = "fastcgi_pass";
    let rest = after_keyword(&line[line.find(DIRECTIVE)?..], DIRECTIVE)?;
    rest.split([':', ';', ' ']).next().filter(|host| !host.is_empty())
}

/// The `root` of the first nginx `server` block that passes PHP to `service`
/// (`fastcgi_pass shop:9000;`); with no service, the first `root` of the file.
pub(super) fn nginx_root(text: &str, service: Option<&str>) -> Option<String> {
    let (mut root, mut serves) = (None, service.is_none());
    for line in text.lines().map(str::trim).filter(|line| !line.starts_with('#')) {
        if opens_server_block(line) {
            if serves && root.is_some() {
                break;
            }
            (root, serves) = (None, service.is_none());
        }
        if let Some(path) = directive(line, "root") {
            root = root.or(Some(path));
        }
        if let Some(service) = service {
            serves |= fastcgi_host(line) == Some(service);
        }
    }
    root.filter(|_| serves).map(str::to_string)
}

/// `path` relative to `base` (`/var/www/html/public` under `/var/www/html` → `public`, `base`
/// itself → `.`); none when it lies elsewhere or is an unresolved variable.
pub(super) fn relative(base: &str, path: &str) -> Option<String> {
    let path = expand_defaults(path);
    let (base, path) = (base.trim_end_matches('/'), path.trim_end_matches('/'));
    if path.contains('$') {
        return None;
    }
    if path == base {
        return Some(".".into());
    }
    path.strip_prefix(base)?.strip_prefix('/').filter(|rest| !rest.is_empty()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apache_document_root_from_env_and_config() {
        let dockerfile = "FROM php:8.2-apache\nENV APACHE_DOCUMENT_ROOT /var/www/html/old\nenv COMPOSER=1 APACHE_DOCUMENT_ROOT=\"/var/www/html/public\"\nRUN sed -ri -e 's!/var/www/html!${APACHE_DOCUMENT_ROOT}!g' /etc/apache2/sites-available/*.conf\n";
        assert_eq!(dockerfile_env(dockerfile).as_deref(), Some("/var/www/html/public"));
        assert_eq!(dockerfile_env("FROM php:8.2-apache\nENV APACHE_DOCUMENT_ROOTS=/x\n"), None);
        let conf = "<VirtualHost *:80>\n    ServerName shop.test\n    documentroot \"/var/www/html/web\"\n</VirtualHost>\n";
        assert_eq!(apache_document_root(conf).as_deref(), Some("/var/www/html/web"));
    }

    #[test]
    fn the_nginx_root_of_the_block_serving_the_service() {
        let conf = "server {\n  server_name blog.test;\n  # fastcgi_pass shop:9000;\n  root /var/www/blog;\n  location ~ \\.php$ { fastcgi_pass blog:9000; }\n}\nserver{\n  root /var/www/shop/public;\n  location ~ \\.php$ {\n    fastcgi_pass shop:9000;\n  }\n}\n";
        assert_eq!(nginx_root(conf, Some("shop")).as_deref(), Some("/var/www/shop/public"));
        assert_eq!(nginx_root(conf, Some("blog")).as_deref(), Some("/var/www/blog"));
        assert_eq!(nginx_root(conf, Some("api")), None);
        assert_eq!(nginx_root(conf, None).as_deref(), Some("/var/www/blog"));
    }

    #[test]
    fn container_paths_relative_to_the_code() {
        assert_eq!(relative("/var/www/html", "/var/www/html/public/").as_deref(), Some("public"));
        assert_eq!(relative("/var/www/html/", "/var/www/html").as_deref(), Some("."));
        assert_eq!(relative("/app", "${ROOT:-/app/web}").as_deref(), Some("web"));
        assert_eq!(relative("/var/www/html", "/var/www/htmlx"), None);
        assert_eq!(relative("/var/www/html", "${APACHE_DOCUMENT_ROOT}"), None);
        assert_eq!(relative("/var/www/html", "/srv/public"), None);
    }
}
