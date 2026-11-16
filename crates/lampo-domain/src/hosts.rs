//! The Lampo-owned block of the Windows hosts file.
//!
//! Lampo only ever touches the lines between [`BEGIN_MARKER`] and [`END_MARKER`]. The rewrite
//! logic is pure (string in, string out) so the elevated helper and the tests share it.
//!
//! The block lists *every* project hostname, active or not: toggling a project must not trigger
//! a UAC prompt. It only changes when projects are added, removed or renamed.

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr};

use serde::{Deserialize, Serialize};

use crate::hostname::Hostname;
use crate::project::Project;

pub const BEGIN_MARKER: &str = "# >>> Lampo managed block - do not edit >>>";
pub const END_MARKER: &str = "# <<< Lampo managed block <<<";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct HostsEntry {
    pub ip: IpAddr,
    pub hostname: Hostname,
}

/// The desired content of the managed block (sorted, de-duplicated).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HostsBlock {
    entries: BTreeSet<HostsEntry>,
}

impl HostsBlock {
    pub fn new(entries: impl IntoIterator<Item = HostsEntry>) -> Self {
        Self { entries: entries.into_iter().collect() }
    }

    /// One IPv4 loopback entry per project address that needs one (`*.localhost` names and
    /// loopback ports need none). No `::1` entry, as Laragon and XAMPP do: an app that
    /// allow-lists `127.0.0.1` answers 403 to a browser arriving over IPv6. A block written by an
    /// older Lampo with `::1` lines differs from this one, so the next sync rewrites it.
    pub fn for_projects<'a>(projects: impl IntoIterator<Item = &'a Project>) -> Self {
        Self::new(projects.into_iter().flat_map(|p| p.hosts_file_names().cloned().collect::<Vec<_>>()).map(|hostname| HostsEntry { ip: IpAddr::V4(Ipv4Addr::LOCALHOST), hostname }))
    }

    pub fn entries(&self) -> impl Iterator<Item = &HostsEntry> {
        self.entries.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The block as text, markers included, lines separated by `\r\n`.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(BEGIN_MARKER);
        out.push_str("\r\n");
        for e in &self.entries {
            out.push_str(&format!("{}\t{}\r\n", e.ip, e.hostname));
        }
        out.push_str(END_MARKER);
        out.push_str("\r\n");
        out
    }

    /// Reads the managed block from a hosts file. `None` if there is no (well-formed) block.
    /// Lines inside the block that cannot be parsed are ignored.
    pub fn extract(hosts_file: &str) -> Option<HostsBlock> {
        let (start, end) = block_range(hosts_file)?;
        let inner = &hosts_file[start..end];
        let entries = inner.lines().skip(1).filter_map(|line| {
            let line = line.split('#').next()?.trim();
            let mut parts = line.split_whitespace();
            let ip: IpAddr = parts.next()?.parse().ok()?;
            let host = Hostname::new(parts.next()?).ok()?;
            Some(HostsEntry { ip, hostname: host })
        });
        Some(HostsBlock::new(entries))
    }

    /// Returns `hosts_file` with the managed block replaced by this block (or appended if absent).
    /// An empty block removes the managed section entirely. Text outside the block is preserved.
    pub fn apply_to(&self, hosts_file: &str) -> String {
        let rendered = if self.is_empty() { String::new() } else { self.render() };
        match block_range(hosts_file) {
            Some((start, end)) => {
                let mut out = String::with_capacity(hosts_file.len() + rendered.len());
                out.push_str(&hosts_file[..start]);
                out.push_str(&rendered);
                let rest = &hosts_file[end..];
                out.push_str(rest.strip_prefix("\r\n").or_else(|| rest.strip_prefix('\n')).unwrap_or(rest));
                out
            }
            None if rendered.is_empty() => hosts_file.to_string(),
            None => {
                let mut out = hosts_file.to_string();
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push_str("\r\n");
                }
                out.push_str(&rendered);
                out
            }
        }
    }
}

/// Byte range `[start of BEGIN line, end of END marker)` of the managed block.
fn block_range(text: &str) -> Option<(usize, usize)> {
    let start = text.find(BEGIN_MARKER)?;
    let end_rel = text[start..].find(END_MARKER)?;
    Some((start, start + end_rel + END_MARKER.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::tests::project;

    fn block() -> HostsBlock {
        HostsBlock::for_projects(&[project("shop", "C:\\a"), project("blog", "C:\\b")])
    }

    #[test]
    fn renders_sorted_ipv4_entries_only() {
        let text = block().render();
        assert_eq!(text, format!("{BEGIN_MARKER}\r\n127.0.0.1\tblog.test\r\n127.0.0.1\tshop.test\r\n{END_MARKER}\r\n"));
    }

    #[test]
    fn a_block_with_ipv6_entries_is_out_of_date_and_gets_rewritten() {
        let legacy = format!("{BEGIN_MARKER}\r\n127.0.0.1\tblog.test\r\n::1\tblog.test\r\n127.0.0.1\tshop.test\r\n::1\tshop.test\r\n{END_MARKER}\r\n");
        assert_ne!(HostsBlock::extract(&legacy), Some(block()));
        assert_eq!(block().apply_to(&legacy), block().render());
    }

    #[test]
    fn appends_when_missing_and_preserves_user_lines() {
        let original = "127.0.0.1 localhost\r\n10.0.0.5 nas.lan";
        let out = block().apply_to(original);
        assert!(out.starts_with("127.0.0.1 localhost\r\n10.0.0.5 nas.lan\r\n"));
        assert_eq!(HostsBlock::extract(&out), Some(block()));
    }

    #[test]
    fn replaces_existing_block_in_place() {
        let original = format!("# header\r\n{BEGIN_MARKER}\r\n127.0.0.1\told.test\r\n{END_MARKER}\r\n# footer\r\n");
        let out = block().apply_to(&original);
        assert!(out.starts_with("# header\r\n"));
        assert!(out.ends_with("# footer\r\n"));
        assert!(!out.contains("old.test"));
        assert_eq!(out.matches(BEGIN_MARKER).count(), 1);
        // Idempotent.
        assert_eq!(block().apply_to(&out), out);
    }

    #[test]
    fn empty_block_removes_section() {
        let original = format!("a\r\n{BEGIN_MARKER}\r\n127.0.0.1\told.test\r\n{END_MARKER}\r\nb\r\n");
        assert_eq!(HostsBlock::default().apply_to(&original), "a\r\nb\r\n");
        assert_eq!(HostsBlock::default().apply_to("a\r\n"), "a\r\n");
    }

    #[test]
    fn extract_ignores_garbage_and_missing_markers() {
        assert_eq!(HostsBlock::extract("127.0.0.1 x.test"), None);
        let text = format!("{BEGIN_MARKER}\nnot an entry\n127.0.0.1 ok.test # comment\n{END_MARKER}\n");
        let b = HostsBlock::extract(&text).unwrap();
        assert_eq!(b.entries().count(), 1);
    }
}
