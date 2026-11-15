//! The request the app hands to `lampo-hosts-helper.exe`: its command line and the JSON block
//! file it names, with the validation both sides apply.
//!
//! Command line: `--block <file.json> --sha256 <hex> --tld <tld> [--hosts <path>]` (the last
//! only in debug builds). The block cannot travel on the command line (`ShellExecuteExW` limits
//! its length) nor through stdin or an inherited handle (an elevated launch through UAC inherits
//! neither), so it is a file in the user's `%TEMP%` (whose ACL admits only the user, SYSTEM and
//! administrators), removed as soon as the helper exits. Its SHA-256 travels on the command line,
//! which another process cannot change: the helper reads the file once, checks the digest, and
//! parses those same bytes, so a file swapped or edited after the launch is refused.

use std::fmt;
use std::io::Read;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use lampo_domain::hostname::{Hostname, RESERVED_LOCAL_TLDS, Tld};
use lampo_domain::hosts::HostsBlock;
use lampo_domain::package::Sha256Digest;
use sha2::{Digest, Sha256};

/// Upper bound for the request file.
pub const MAX_REQUEST_BYTES: u64 = 1024 * 1024;
/// Upper bound for the number of block entries.
pub const MAX_ENTRIES: usize = 4096;

const BLOCK_FLAG: &str = "--block";
const SHA256_FLAG: &str = "--sha256";
const TLD_FLAG: &str = "--tld";
const HOSTS_FLAG: &str = "--hosts";

/// The helper's parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperArgs {
    pub block_file: PathBuf,
    /// SHA-256 of the exact bytes of `block_file`.
    pub digest: Sha256Digest,
    /// The TLD configured in Lampo's settings, allowed besides [`RESERVED_LOCAL_TLDS`].
    pub tld: Tld,
    /// Debug builds only.
    pub hosts_override: Option<PathBuf>,
}

impl HelperArgs {
    /// Strict parsing: every flag at most once, each with a valid value, nothing else.
    /// `--hosts` is refused in release builds.
    pub fn parse(args: &[String]) -> Option<Self> {
        let (mut block_file, mut digest, mut tld, mut hosts_override) = (None, None, None, None);
        let mut it = args.iter();
        while let Some(flag) = it.next() {
            let value = it.next()?;
            match flag.as_str() {
                BLOCK_FLAG if block_file.is_none() => block_file = Some(PathBuf::from(value)),
                SHA256_FLAG if digest.is_none() => digest = Some(Sha256Digest::parse_hex(value).ok()?),
                TLD_FLAG if tld.is_none() => tld = Some(Tld::new(value).ok()?),
                HOSTS_FLAG if cfg!(debug_assertions) && hosts_override.is_none() => hosts_override = Some(PathBuf::from(value)),
                _ => return None,
            }
        }
        Some(Self { block_file: block_file?, digest: digest?, tld: tld?, hosts_override })
    }

    /// The command line [`HelperArgs::parse`] reads back.
    pub fn to_args(&self) -> Vec<String> {
        let mut args = vec![BLOCK_FLAG.into(), self.block_file.display().to_string(), SHA256_FLAG.into(), self.digest.to_string(), TLD_FLAG.into(), self.tld.to_string()];
        if let Some(hosts) = &self.hosts_override {
            args.extend([HOSTS_FLAG.into(), hosts.display().to_string()]);
        }
        args
    }
}

/// Why a block may not be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
    TooManyEntries(usize),
    NotLoopback(IpAddr),
    NotLocalName(Hostname),
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyEntries(n) => write!(f, "{n} entries, at most {MAX_ENTRIES} can be written"),
            Self::NotLoopback(ip) => write!(f, "{ip} is not a loopback address"),
            Self::NotLocalName(host) => {
                write!(f, "`{host}` is not under a local top-level domain ({}, or the one set in Lampo's settings)", RESERVED_LOCAL_TLDS.map(|t| format!(".{t}")).join(", "))
            }
        }
    }
}

/// A block is acceptable when it is bounded, points only at loopback addresses and names only
/// hosts under a local TLD (reserved, or `tld` from the settings).
pub fn validate_request(block: &HostsBlock, tld: &Tld) -> Result<(), RequestError> {
    let count = block.entries().count();
    if count > MAX_ENTRIES {
        return Err(RequestError::TooManyEntries(count));
    }
    block.entries().try_for_each(|e| {
        if !is_loopback(e.ip) {
            Err(RequestError::NotLoopback(e.ip))
        } else if !e.hostname.is_under_local_tld(tld) {
            Err(RequestError::NotLocalName(e.hostname.clone()))
        } else {
            Ok(())
        }
    })
}

fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()),
    }
}

/// The request file contents for `block` and their digest.
pub fn encode_request(block: &HostsBlock) -> serde_json::Result<(Vec<u8>, Sha256Digest)> {
    let bytes = serde_json::to_vec(block)?;
    let digest = digest_of(&bytes);
    Ok((bytes, digest))
}

/// SHA-256 of a request file's bytes.
pub fn digest_of(bytes: &[u8]) -> Sha256Digest {
    Sha256Digest::from_bytes(Sha256::digest(bytes).into())
}

/// Reads the request file once and returns the block if its bytes match `args.digest` and it
/// passes [`validate_request`]. `None` otherwise (missing, oversized, tampered, invalid).
pub fn load_request(args: &HelperArgs) -> Option<HostsBlock> {
    let bytes = read_bounded(&args.block_file)?;
    if digest_of(&bytes) != args.digest {
        return None;
    }
    let block: HostsBlock = serde_json::from_slice(&bytes).ok()?;
    validate_request(&block, &args.tld).is_ok().then_some(block)
}

fn read_bounded(path: &Path) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(MAX_REQUEST_BYTES + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= MAX_REQUEST_BYTES).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use lampo_domain::hosts::HostsEntry;

    use super::*;

    const SHOP_REQUEST: &str = r#"{"entries":[{"ip":"127.0.0.1","hostname":"shop.test"}]}"#;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn entry(ip: &str, host: &str) -> HostsEntry {
        HostsEntry { ip: ip.parse().unwrap(), hostname: Hostname::new(host).unwrap() }
    }

    fn args_for(file: &Path, contents: &[u8]) -> HelperArgs {
        HelperArgs { block_file: file.into(), digest: digest_of(contents), tld: Tld::default(), hosts_override: None }
    }

    #[test]
    fn argument_parsing_is_strict() {
        let digest = digest_of(b"x").to_string();
        let full = |extra: &[&str]| s(&[&["--block", "a.json", "--sha256", &digest, "--tld", "lan"], extra].concat());
        let parsed = HelperArgs::parse(&full(&[])).unwrap();
        assert_eq!(parsed, HelperArgs { block_file: "a.json".into(), digest: digest_of(b"x"), tld: Tld::new("lan").unwrap(), hosts_override: None });
        assert_eq!(HelperArgs::parse(&parsed.to_args()), Some(parsed), "to_args round-trips");
        assert_eq!(HelperArgs::parse(&s(&[])), None);
        assert_eq!(HelperArgs::parse(&s(&["--block"])), None);
        assert_eq!(HelperArgs::parse(&full(&["--block", "b"])), None, "duplicate flag");
        assert_eq!(HelperArgs::parse(&full(&["--other", "b"])), None, "unknown flag");
        assert_eq!(HelperArgs::parse(&s(&["--block", "a", "--tld", "test"])), None, "the digest is mandatory");
        assert_eq!(HelperArgs::parse(&s(&["--block", "a", "--sha256", &digest])), None, "the TLD is mandatory");
        assert_eq!(HelperArgs::parse(&s(&["--block", "a", "--sha256", "abc", "--tld", "test"])), None, "malformed digest");
        assert_eq!(HelperArgs::parse(&s(&["--block", "a", "--sha256", &digest, "--tld", "com"])), None, "public TLD");
        // Tests run in debug builds, where the override exists.
        assert_eq!(HelperArgs::parse(&full(&["--hosts", "h"])).unwrap().hosts_override, Some(PathBuf::from("h")));
    }

    #[test]
    fn only_loopback_entries_are_accepted() {
        let tld = Tld::default();
        let ok = HostsBlock::new([entry("127.0.0.1", "a.test"), entry("::1", "a.test"), entry("127.0.0.2", "b.test"), entry("::ffff:127.0.0.1", "c.test")]);
        assert_eq!(validate_request(&ok, &tld), Ok(()));
        assert_eq!(validate_request(&HostsBlock::new([entry("10.0.0.1", "bank.test")]), &tld), Err(RequestError::NotLoopback("10.0.0.1".parse().unwrap())));
        assert!(validate_request(&HostsBlock::new([entry("::ffff:8.8.8.8", "bank.test")]), &tld).is_err());
    }

    #[test]
    fn only_names_under_local_tlds_are_accepted() {
        let lan = Tld::new("lan").unwrap();
        for ok in ["shop.test", "shop.example", "shop.invalid", "shop.localhost", "shop.local", "api.shop.lan"] {
            assert_eq!(validate_request(&HostsBlock::new([entry("127.0.0.1", ok)]), &lan), Ok(()), "{ok}");
        }
        for bad in ["windowsupdate.com", "login.microsoftonline.com", "bank.co.uk", "shop.dev", "test.org"] {
            let err = validate_request(&HostsBlock::new([entry("127.0.0.1", "ok.test"), entry("127.0.0.1", bad)]), &lan).unwrap_err();
            assert_eq!(err, RequestError::NotLocalName(Hostname::new(bad).unwrap()));
            assert!(err.to_string().contains(bad), "the error names the host: {err}");
        }
        assert!(validate_request(&HostsBlock::new([entry("127.0.0.1", "shop.lan")]), &Tld::default()).is_err(), "`lan` is allowed only when configured");
    }

    #[test]
    fn oversized_blocks_are_refused() {
        let many = HostsBlock::new((0..=MAX_ENTRIES).map(|i| entry("127.0.0.1", &format!("s{i}.test"))));
        assert_eq!(validate_request(&many, &Tld::default()), Err(RequestError::TooManyEntries(MAX_ENTRIES + 1)));
    }

    #[test]
    fn encoded_requests_load_back() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("req.json");
        let block = HostsBlock::new([entry("127.0.0.1", "shop.test")]);
        let (bytes, digest) = encode_request(&block).unwrap();
        std::fs::write(&file, &bytes).unwrap();
        assert_eq!(load_request(&HelperArgs { block_file: file, digest, tld: Tld::default(), hosts_override: None }), Some(block));
    }

    #[test]
    fn request_files_are_validated() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("req.json");
        std::fs::write(&file, SHOP_REQUEST).unwrap();
        assert_eq!(load_request(&args_for(&file, SHOP_REQUEST.as_bytes())).unwrap().entries().count(), 1);
        for bad in [
            r#"{"entries":[{"ip":"127.0.0.1","hostname":"shop.test\r\n1.2.3.4 evil.test"}]}"#,
            r#"{"entries":[{"ip":"1.2.3.4","hostname":"shop.test"}]}"#,
            r#"{"entries":[{"ip":"127.0.0.1","hostname":"windowsupdate.com"}]}"#,
            r#"{"entries":[{"ip":"127.0.0.1","hostname":"x"}]}"#,
            "not json",
        ] {
            std::fs::write(&file, bad).unwrap();
            assert!(load_request(&args_for(&file, bad.as_bytes())).is_none(), "{bad}");
        }
        let oversized = vec![b' '; (MAX_REQUEST_BYTES + 1) as usize];
        std::fs::write(&file, &oversized).unwrap();
        assert!(load_request(&args_for(&file, &oversized)).is_none());
        assert!(load_request(&args_for(&dir.path().join("missing.json"), b"")).is_none());
    }

    #[test]
    fn a_tampered_request_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("req.json");
        let args = args_for(&file, SHOP_REQUEST.as_bytes());
        // A valid block, but not the one whose digest the app put on the command line.
        std::fs::write(&file, r#"{"entries":[{"ip":"127.0.0.1","hostname":"other.test"}]}"#).unwrap();
        assert!(load_request(&args).is_none());
    }
}
