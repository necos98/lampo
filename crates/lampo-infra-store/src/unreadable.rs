//! Best-effort facts about a project file that does not load.
//!
//! Such a project is hidden, not gone: the user may repair the file. Its loopback ports must stay
//! reserved, or the allocator would hand its `127.0.0.1:<port>` listener to a new site and the
//! repaired project would collide with it. Reading leniently, reserving too much is harmless;
//! missing a port is not.

use std::collections::BTreeSet;
use std::path::Path;

use lampo_domain::tcp_port::Port;
use toml::Value;

use crate::atomic::read_table;

const ADDRESSES_KEY: &str = "addresses";
const PORT_KEY: &str = "port";

/// The ports named by `file`'s addresses: from the parsed `addresses` when the file is valid
/// TOML (only its values were rejected), else from every `port = <n>` in the text.
pub fn loopback_ports_in(file: &Path) -> BTreeSet<Port> {
    match read_table(file) {
        Ok(Some(table)) => table.get(ADDRESSES_KEY).map(address_ports).unwrap_or_default(),
        Ok(None) => BTreeSet::new(),
        Err(_) => std::fs::read(file).map(|bytes| ports_in_text(&String::from_utf8_lossy(&bytes))).unwrap_or_default(),
    }
}

fn address_ports(addresses: &Value) -> BTreeSet<Port> {
    let Some(list) = addresses.as_array() else { return BTreeSet::new() };
    list.iter().filter_map(|a| a.get(PORT_KEY)?.as_integer()).filter_map(port).collect()
}

/// Every `port = <n>` (bare or quoted key) in malformed TOML. A key ending in `port`
/// (`apache_port`) is not a match.
fn ports_in_text(text: &str) -> BTreeSet<Port> {
    let is_key_char = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.';
    text.match_indices(PORT_KEY).filter(|(at, _)| !text[..*at].ends_with(is_key_char)).filter_map(|(at, _)| value_after_key(&text[at + PORT_KEY.len()..])).collect()
}

/// The integer after `= ` in `rest` (what follows a `port` key).
fn value_after_key(rest: &str) -> Option<Port> {
    let blank = [' ', '\t'];
    let rest = rest.strip_prefix('"').unwrap_or(rest).trim_start_matches(blank).strip_prefix('=')?.trim_start_matches(blank);
    let digits = &rest[..rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len())];
    digits.parse::<i64>().ok().and_then(port)
}

fn port(value: i64) -> Option<Port> {
    u16::try_from(value).ok().and_then(Port::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ports_of(text: &str) -> Vec<u16> {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("p.toml");
        std::fs::write(&file, text).unwrap();
        loopback_ports_in(&file).into_iter().map(Port::get).collect()
    }

    #[test]
    fn valid_toml_reads_the_address_ports_only() {
        let text = "id = \"x\"\n[[addresses]]\nkind = \"localhost\"\nhost = \"x.localhost\"\n\n[[addresses]]\nkind = \"loopback-port\"\nport = 8101\n\n[environment.php_ini]\n\"xdebug.client_port\" = \"9003\"\n";
        assert_eq!(ports_of(text), [8101]);
        assert!(ports_of("addresses = []\n").is_empty());
        assert!(ports_of("addresses = 5\n").is_empty());
    }

    #[test]
    fn malformed_toml_falls_back_to_the_text() {
        let text = "id = [[[\n[[addresses]]\nkind = \"loopback-port\"\nport = 8102\naddresses = [{ kind = \"loopback-port\", \"port\" = 8103 }]\napache_port = 80\nport = 0\nport = 70000\n";
        assert_eq!(ports_of(text), [8102, 8103]);
    }

    #[test]
    fn a_missing_file_has_no_ports() {
        let dir = tempfile::tempdir().unwrap();
        assert!(loopback_ports_in(&dir.path().join("gone.toml")).is_empty());
    }
}
