//! What the Docker files of a folder add up to, and which PHP reference wins.

use lampo_domain::php::PhpMinor;
use lampo_domain::service::ServiceKind;

use super::image::Component;

/// How much a PHP reference says about the project's own runtime, weakest first: the strongest
/// role found wins, the highest PHP only breaks ties within one role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum PhpRole {
    /// An accessory compose service (queue worker, CLI tools…) or an intermediate Dockerfile
    /// build stage.
    Accessory,
    /// The final stage of a Dockerfile: the image `docker build` produces by default.
    DockerfileRuntime,
    /// A compose service publishing the container's port 80: the web server.
    WebService,
    /// A compose service whose image Compose builds (`build:`, Laravel Sail's runtime included).
    BuiltService,
}

#[derive(Default)]
pub(super) struct Findings {
    /// The winning PHP and the role it was found in.
    php: Option<(PhpRole, PhpMinor)>,
    pub services: Vec<ServiceKind>,
    pub evidence: Vec<String>,
    pub warnings: Vec<String>,
    pub found: bool,
}

impl Findings {
    pub fn php(&self) -> Option<PhpMinor> {
        self.php.map(|(_, php)| php)
    }

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

    /// Records one image/build reference found in `source` (`compose.yaml service app`);
    /// `role` weighs a PHP reference against the others (ignored for the other components).
    pub fn component(&mut self, component: Component, role: PhpRole, source: &str, reference: &str) {
        match component {
            Component::Php(php) => {
                self.php = self.php.max(php.map(|php| (role, php)));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn php(v: &str) -> Option<PhpMinor> {
        Some(PhpMinor::parse(v).unwrap())
    }

    #[test]
    fn the_strongest_role_wins_then_the_highest_php() {
        let mut findings = Findings::default();
        findings.component(Component::Php(php("8.4")), PhpRole::Accessory, "a", "php:8.4");
        findings.component(Component::Php(php("8.1")), PhpRole::WebService, "b", "php:8.1");
        findings.component(Component::Php(php("8.2")), PhpRole::DockerfileRuntime, "c", "php:8.2");
        findings.component(Component::Php(None), PhpRole::BuiltService, "d", "php:8");
        assert_eq!(findings.php(), php("8.1"));
        findings.component(Component::Php(php("8.3")), PhpRole::WebService, "e", "php:8.3");
        assert_eq!(findings.php(), php("8.3"));
        assert_eq!(findings.evidence.len(), 5, "every reference stays in the evidence: {:?}", findings.evidence);
    }
}
