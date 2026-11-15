//! The ports a generated `httpd.conf` listens on, read back from its `Listen` directives (see
//! [`super::render`]): the runner waits until httpd owns every one of them, not just the
//! shared port, since a vhost's dedicated loopback port is just as much part of being up.

use std::path::Path;

use lampo_domain::{PortError, PortResult};

const LISTEN: &str = "Listen";

/// Ports of the `Listen` directives (`127.0.0.1:80`, `[::1]:80`, `8080`, an optional protocol
/// after the address), sorted and without duplicates. Unparsable directives are skipped:
/// `httpd -t` validated the file before it went live.
pub(crate) fn listened_ports(httpd_conf: &str) -> Vec<u16> {
    let mut ports: Vec<u16> = httpd_conf.lines().filter_map(listen_port).collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

fn listen_port(line: &str) -> Option<u16> {
    let mut words = line.split_whitespace();
    if !words.next()?.eq_ignore_ascii_case(LISTEN) {
        return None;
    }
    let address = words.next()?;
    let port = address.rsplit_once(':').map_or(address, |(_, port)| port);
    port.parse().ok().filter(|&p| p != 0)
}

/// [`listened_ports`] of the live `httpd.conf`, always including `shared_port`.
pub(crate) fn ports_to_own(httpd_conf: &Path, shared_port: u16) -> PortResult<Vec<u16>> {
    let text = std::fs::read_to_string(httpd_conf).map_err(|e| PortError::io(format!("reading {}", httpd_conf.display()), e))?;
    let mut ports = listened_ports(&text);
    if !ports.contains(&shared_port) {
        ports.push(shared_port);
        ports.sort_unstable();
    }
    Ok(ports)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_listened_port_once() {
        let conf = "# Listen 1\nListen 127.0.0.1:80\nListen [::1]:80\nlisten 8080\n  Listen 127.0.0.1:8443 https\nListenBacklog 511\nListen\nListen 127.0.0.1:http\nListen 127.0.0.1:0\n";
        assert_eq!(listened_ports(conf), [80, 8080, 8443]);
    }

    #[test]
    fn reads_the_rendered_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let conf = dir.path().join("httpd.conf");
        std::fs::write(&conf, "Listen 127.0.0.1:8101\nListen 127.0.0.1:80\n").unwrap();
        assert_eq!(ports_to_own(&conf, 80).unwrap(), [80, 8101]);
        assert_eq!(ports_to_own(&conf, 8080).unwrap(), [80, 8080, 8101], "the shared port is always waited for");
        assert!(matches!(ports_to_own(&dir.path().join("missing.conf"), 80), Err(PortError::Io { .. })));
    }
}
