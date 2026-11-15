//! Byte-level rewrite of the Lampo block (pure). Same rules as
//! `lampo_domain::hosts::HostsBlock::apply_to`, but on bytes: everything outside the block —
//! BOM, ANSI-encoded comments, line endings, a missing final newline — is preserved byte for
//! byte, and the block itself uses the file's line-ending style.

use std::fmt;

use lampo_domain::hosts::{BEGIN_MARKER, END_MARKER, HostsBlock};

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeError {
    /// UTF-16 hosts files exist (saved by some editors) but cannot be edited safely as bytes.
    Utf16,
    /// A begin marker without an end marker: rewriting could swallow the user's lines.
    UnterminatedBlock,
}

impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Utf16 => f.write_str("the hosts file is saved as UTF-16; save it as ANSI or UTF-8 and retry"),
            Self::UnterminatedBlock => write!(f, "the hosts file contains `{BEGIN_MARKER}` without `{END_MARKER}`; remove the incomplete Lampo block and retry"),
        }
    }
}

/// Text view of a hosts file for reading the block (BOM stripped, invalid UTF-8 replaced:
/// only the ASCII block content matters).
pub fn decode(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes)).into_owned()
}

/// The managed block currently in `bytes`, if any.
pub fn read_block(bytes: &[u8]) -> Option<HostsBlock> {
    HostsBlock::extract(&decode(bytes))
}

/// `original` with the managed block replaced by `block` (appended if absent, removed if
/// `block` is empty).
pub fn merge_block(original: &[u8], block: &HostsBlock) -> Result<Vec<u8>, MergeError> {
    if original.starts_with(b"\xFF\xFE") || original.starts_with(b"\xFE\xFF") {
        return Err(MergeError::Utf16);
    }
    let crlf = original.windows(2).any(|w| w == b"\r\n") || !original.contains(&b'\n');
    let rendered = match (block.is_empty(), crlf) {
        (true, _) => String::new(),
        (false, true) => block.render(),
        (false, false) => block.render().replace("\r\n", "\n"),
    };
    let rendered = rendered.as_bytes();

    if let Some(start) = find(original, BEGIN_MARKER.as_bytes(), 0) {
        let end = find(original, END_MARKER.as_bytes(), start).ok_or(MergeError::UnterminatedBlock)? + END_MARKER.len();
        let rest = &original[end..];
        let rest = rest.strip_prefix(b"\r\n").or_else(|| rest.strip_prefix(b"\n")).unwrap_or(rest);
        let mut out = Vec::with_capacity(original.len() + rendered.len());
        out.extend_from_slice(&original[..start]);
        out.extend_from_slice(rendered);
        out.extend_from_slice(rest);
        return Ok(out);
    }
    if rendered.is_empty() {
        return Ok(original.to_vec());
    }
    let mut out = original.to_vec();
    let content = original.strip_prefix(UTF8_BOM).unwrap_or(original);
    if !content.is_empty() && !content.ends_with(b"\n") {
        out.extend_from_slice(if crlf { b"\r\n" } else { b"\n" });
    }
    out.extend_from_slice(rendered);
    Ok(out)
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|i| i + from)
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use lampo_domain::hosts::HostsEntry;
    use lampo_domain::hostname::Hostname;

    use super::*;

    fn block(names: &[&str]) -> HostsBlock {
        HostsBlock::new(names.iter().flat_map(|n| {
            let h = Hostname::new(n).unwrap();
            [HostsEntry { ip: IpAddr::V4(Ipv4Addr::LOCALHOST), hostname: h.clone() }, HostsEntry { ip: IpAddr::V6(Ipv6Addr::LOCALHOST), hostname: h }]
        }))
    }

    /// A realistic Windows hosts file: BOM, CRLF, Latin-1 comment (invalid UTF-8), no final newline.
    fn fixture() -> Vec<u8> {
        let mut f = UTF8_BOM.to_vec();
        f.extend_from_slice(b"# Copyright (c) 1993-2009 Microsoft Corp.\r\n#\r\n# Caff\xE8 server\r\n127.0.0.1\tlocalhost\r\n10.0.0.5 nas.lan");
        f
    }

    #[test]
    fn appends_and_preserves_every_byte_outside_the_block() {
        let original = fixture();
        let out = merge_block(&original, &block(&["shop.test"])).unwrap();
        assert!(out.starts_with(&original));
        let tail = &out[original.len()..];
        assert_eq!(tail, format!("\r\n{}", block(&["shop.test"]).render()).as_bytes());
        assert_eq!(read_block(&out), Some(block(&["shop.test"])));
    }

    #[test]
    fn replaces_in_place_and_is_idempotent() {
        let first = merge_block(&fixture(), &block(&["shop.test"])).unwrap();
        let mut with_footer = first.clone();
        with_footer.extend_from_slice(b"# user line after\r\n");
        let second = merge_block(&with_footer, &block(&["blog.test", "shop.test"])).unwrap();
        assert!(second.starts_with(&fixture()));
        assert!(second.ends_with(b"# user line after\r\n"));
        assert_eq!(read_block(&second), Some(block(&["blog.test", "shop.test"])));
        assert_eq!(merge_block(&second, &block(&["blog.test", "shop.test"])).unwrap(), second);
    }

    #[test]
    fn empty_block_restores_the_original_bytes() {
        let original = b"a\r\nb\r\n".to_vec();
        let with = merge_block(&original, &block(&["x.test"])).unwrap();
        assert_eq!(merge_block(&with, &HostsBlock::default()).unwrap(), original);
        assert_eq!(merge_block(&original, &HostsBlock::default()).unwrap(), original);
    }

    #[test]
    fn keeps_lf_files_lf() {
        let out = merge_block(b"127.0.0.1 localhost\n", &block(&["shop.test"])).unwrap();
        assert!(!out.windows(2).any(|w| w == b"\r\n"));
        assert_eq!(read_block(&out), Some(block(&["shop.test"])));
    }

    #[test]
    fn bom_only_and_empty_files() {
        let out = merge_block(UTF8_BOM, &block(&["shop.test"])).unwrap();
        assert_eq!(out, [UTF8_BOM, block(&["shop.test"]).render().as_bytes()].concat());
        let out = merge_block(b"", &block(&["shop.test"])).unwrap();
        assert_eq!(out, block(&["shop.test"]).render().as_bytes());
    }

    #[test]
    fn matches_the_domain_rules_on_utf8_text() {
        for original in ["", "127.0.0.1 localhost\r\n", "x\r\ny", &format!("h\r\n{BEGIN_MARKER}\r\n127.0.0.1\told.test\r\n{END_MARKER}\r\nf\r\n")] {
            for b in [block(&["shop.test"]), HostsBlock::default()] {
                assert_eq!(merge_block(original.as_bytes(), &b).unwrap(), b.apply_to(original).into_bytes(), "{original:?}");
            }
        }
    }

    #[test]
    fn refuses_unsafe_inputs() {
        assert_eq!(merge_block(b"\xFF\xFEa\x00", &block(&["a.test"])), Err(MergeError::Utf16));
        let broken = format!("x\r\n{BEGIN_MARKER}\r\n127.0.0.1 a.test\r\nmine.lan\r\n");
        assert_eq!(merge_block(broken.as_bytes(), &block(&["a.test"])), Err(MergeError::UnterminatedBlock));
    }
}
