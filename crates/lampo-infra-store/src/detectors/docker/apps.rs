//! The PHP apps of a compose file that holds several, one proposed project each: the
//! service's code folder, PHP, extensions, document root, environment and the services it
//! reaches. Read-only: only files inside the analysed folder are read.

use std::collections::BTreeSet;
use std::path::Path;

use lampo_domain::detection::DetectedApp;

use super::compose::{ComposeFile, ComposeService, Mount};
use super::dockerfile;
use super::findings::PhpRole;
use super::image::{Component, classify_image, expand_defaults, sail_php};
use super::read_small;
use super::webroot::{self, APACHE_DOCUMENT_ROOT, DEFAULT_WEB_ROOT};

/// Apps are proposed only from this many distinct code folders up: with fewer, the folder's
/// own detection already describes the project.
const MIN_APPS: usize = 2;
/// Container folders where images keep the application code (`/var/www/html`, `/app`…).
const CODE_ROOTS: &[&str] = &["/var/www", "/app", "/srv", "/usr/src/app", "/code"];
/// Dockerfile of a build context when `build.dockerfile` does not name one.
const DEFAULT_DOCKERFILE: &str = "Dockerfile";
/// Web-server config files mounted into a container.
const CONF_EXTENSION: &str = ".conf";
/// Environment variables naming a service by host (`DB_HOST=mysql`).
const HOST_SUFFIX: &str = "_HOST";

/// `./apps/shop/` → `apps/shop`, `.` → `.`, `apps/blog/../../docker` → `docker`; none for a
/// path outside the folder (absolute, climbing above it, home-relative or holding an
/// unresolved variable).
fn inside(raw: &str) -> Option<String> {
    let path = expand_defaults(raw).replace('\\', "/");
    if path.starts_with(['/', '~']) || path.contains('$') || path.as_bytes().get(1) == Some(&b':') {
        return None;
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    Some(if parts.is_empty() { ".".to_string() } else { parts.join("/") })
}

/// The text of `raw` (relative to `folder`) if it is a small file inside the folder.
fn read_inside(folder: &Path, raw: &str) -> Option<String> {
    read_small(&folder.join(inside(raw)?))
}

fn is_code_root(target: &str) -> bool {
    CODE_ROOTS.iter().any(|root| target.strip_prefix(root).is_some_and(|rest| rest.is_empty() || rest.starts_with('/')))
}

/// The mount bringing the service's code in: a host folder on one of the [`CODE_ROOTS`].
fn code_mount<'a>(folder: &Path, service: &'a ComposeService) -> Option<&'a Mount> {
    service.bind_mounts().find(|m| is_code_root(&m.target) && !inside(&m.source).is_some_and(|rel| folder.join(rel).is_file()))
}

/// The web-server config files mounted into `service`, read.
fn conf_files<'a>(folder: &'a Path, service: &'a ComposeService) -> impl Iterator<Item = String> + 'a {
    service.bind_mounts().filter(|m| m.source.to_ascii_lowercase().ends_with(CONF_EXTENSION)).filter_map(|m| read_inside(folder, &m.source))
}

/// What the service runs: the PHP of its image, of its Sail runtime or of its Dockerfile.
fn runtime(service: &ComposeService, dockerfile: Option<&str>) -> Component {
    match service.image.as_deref().map(classify_image) {
        Some(php @ Component::Php(_)) => php,
        _ => match service.sail_runtime() {
            Some(context) => Component::Php(sail_php(context)),
            None => dockerfile.and_then(dockerfile::runtime).unwrap_or(Component::Other),
        },
    }
}

/// One compose file and the folder it sits in.
struct ComposeProject<'a> {
    folder: &'a Path,
    compose: &'a ComposeFile,
}

impl ComposeProject<'_> {
    /// The app `service` runs, if it is a PHP service with a code folder.
    fn app(&self, service: &ComposeService) -> Option<DetectedApp> {
        let code = code_mount(self.folder, service);
        let source = code.map(|m| m.source.as_str()).or(service.context.as_deref().filter(|_| service.sail_runtime().is_none()))?;
        let dockerfile = service.context.as_deref().and_then(|context| read_inside(self.folder, &format!("{context}/{}", service.dockerfile.as_deref().unwrap_or(DEFAULT_DOCKERFILE))));
        let Component::Php(php) = runtime(service, dockerfile.as_deref()) else { return None };
        let folder = inside(source).unwrap_or_else(|| source.to_string());
        Some(DetectedApp {
            service: service.name.clone(),
            document_root: self.document_root(service, &folder, code.map_or(DEFAULT_WEB_ROOT, |m| m.target.as_str()), dockerfile.as_deref()),
            extensions: dockerfile.as_deref().map(|text| dockerfile::php_extensions(text, php)).unwrap_or_default(),
            environment: service.environment.iter().cloned().collect(),
            service_hosts: self.service_hosts(service),
            folder,
            php,
        })
    }

    /// The first document root found, relative to the code (mounted at `base`): the service's
    /// `APACHE_DOCUMENT_ROOT`, its Dockerfile's, its mounted Apache/nginx config, then the
    /// config of an nginx service passing PHP to it (relative to where that one mounts the code).
    fn document_root(&self, service: &ComposeService, folder: &str, base: &str, dockerfile: Option<&str>) -> Option<String> {
        let mut own = service
            .env(APACHE_DOCUMENT_ROOT)
            .map(str::to_string)
            .into_iter()
            .chain(dockerfile.and_then(webroot::dockerfile_env))
            .chain(conf_files(self.folder, service).filter_map(|text| webroot::apache_document_root(&text).or_else(|| webroot::nginx_root(&text, None))));
        if let Some(root) = own.find_map(|path| webroot::relative(base, &path)) {
            return Some(root);
        }
        self.compose.services.iter().filter(|front| front.name != service.name).find_map(|front| {
            let front_base = front.bind_mounts().find(|m| inside(&m.source).as_deref() == Some(folder)).map_or(base, |m| m.target.as_str());
            conf_files(self.folder, front).find_map(|text| webroot::relative(front_base, &webroot::nginx_root(&text, Some(&service.name))?))
        })
    }

    /// `depends_on` plus the services named by a `*_HOST` variable, sorted.
    fn service_hosts(&self, service: &ComposeService) -> Vec<String> {
        let names: BTreeSet<&str> = self.compose.services.iter().map(|s| s.name.as_str()).collect();
        let by_variable = service.environment.iter().filter(|(key, value)| key.ends_with(HOST_SUFFIX) && names.contains(value.as_str())).map(|(_, value)| value);
        let hosts: BTreeSet<&String> = service.depends_on.iter().chain(by_variable).filter(|host| **host != service.name).collect();
        hosts.into_iter().cloned().collect()
    }
}

/// One app per code folder of the compose file's PHP services (the service with the strongest
/// [`PhpRole`] speaks for a folder shared by several, a queue worker beside the web server);
/// none below [`MIN_APPS`] folders.
pub(super) fn apps(folder: &Path, compose: &ComposeFile) -> Vec<DetectedApp> {
    let project = ComposeProject { folder, compose };
    let mut found: Vec<(PhpRole, DetectedApp)> = Vec::new();
    for service in &compose.services {
        let Some(app) = project.app(service) else { continue };
        let role = service.php_role();
        match found.iter_mut().find(|(_, known)| known.folder == app.folder) {
            Some(slot) if slot.0 < role => *slot = (role, app),
            Some(_) => {}
            None => found.push((role, app)),
        }
    }
    if found.len() < MIN_APPS {
        return Vec::new();
    }
    found.into_iter().map(|(_, app)| app).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_inside_the_folder() {
        assert_eq!(inside("./apps/shop/").as_deref(), Some("apps/shop"));
        assert_eq!(inside(".").as_deref(), Some("."));
        assert_eq!(inside(".\\blog").as_deref(), Some("blog"));
        assert_eq!(inside("${SHOP_PATH:-./shop}").as_deref(), Some("shop"));
        assert_eq!(inside("./apps/blog/../../docker/blog/Dockerfile").as_deref(), Some("docker/blog/Dockerfile"));
        for outside in ["../shared", "/srv/shop", "C:\\sites\\shop", "~/shop", "${SHOP_PATH}", "./a/../../b"] {
            assert_eq!(inside(outside), None, "{outside}");
        }
    }

    #[test]
    fn code_roots() {
        assert!(is_code_root("/var/www/html") && is_code_root("/app") && is_code_root("/srv/shop"));
        assert!(!is_code_root("/application") && !is_code_root("/etc/nginx/conf.d/default.conf"));
    }
}
