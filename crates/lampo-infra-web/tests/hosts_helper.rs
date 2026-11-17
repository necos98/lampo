//! The real `lampo-hosts-helper` binary, run non-elevated against temp copies of fixture
//! hosts files through its debug-only `--hosts` option. Never touches the system hosts file and
//! never triggers UAC.

use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use lampo_domain::hostname::{Hostname, Tld};
use lampo_domain::hosts::{HostsBlock, HostsEntry};
use lampo_domain::ports::{HostsFileReader, HostsFileWriter};
use lampo_domain::{PortError, PortResult};
use lampo_infra_web::elevation::{HelperLauncher, LaunchOutcome};
use lampo_infra_web::hosts::request::{HelperArgs, digest_of};
use lampo_infra_web::hosts::{ElevatedHostsFile, helper_exit};

const HELPER: &str = env!("CARGO_BIN_EXE_lampo-hosts-helper");

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join("hosts").join(name)).unwrap()
}

fn block(names: &[&str]) -> HostsBlock {
    HostsBlock::new(names.iter().map(|n| HostsEntry { ip: IpAddr::V4(Ipv4Addr::LOCALHOST), hostname: Hostname::new(n).unwrap() }))
}

/// Runs the helper as a normal child process (the test stand-in for the UAC launcher).
struct DirectLauncher;

impl HelperLauncher for DirectLauncher {
    fn run(&self, helper: &Path, args: &[String]) -> PortResult<LaunchOutcome> {
        let status = Command::new(helper).args(args).status().map_err(|e| PortError::process("helper", e))?;
        Ok(LaunchOutcome::Exited(status.code().unwrap_or(-1)))
    }
}

fn temp_hosts(dir: &tempfile::TempDir, contents: &[u8]) -> PathBuf {
    let path = dir.path().join("hosts");
    std::fs::write(&path, contents).unwrap();
    path
}

#[tokio::test]
async fn adapter_and_real_helper_preserve_the_rest_of_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let original = fixture("windows-bom-ansi-no-final-newline.hosts");
    let hosts = temp_hosts(&dir, &original);
    let file = ElevatedHostsFile::with_launcher(hosts.clone(), PathBuf::from(HELPER), Arc::new(DirectLauncher));

    file.write_block(&block(&["shop.test", "blog.test"]), &Tld::default()).await.unwrap();
    let written = std::fs::read(&hosts).unwrap();
    assert!(written.starts_with(&original), "bytes before the block are untouched (BOM, ANSI, CRLF)");
    assert_eq!(&written[original.len()..original.len() + 2], b"\r\n");
    assert_eq!(file.read_block().unwrap(), Some(block(&["blog.test", "shop.test"])));

    // Removing every project restores the original bytes exactly.
    file.write_block(&HostsBlock::default(), &Tld::default()).await.unwrap();
    assert_eq!(std::fs::read(&hosts).unwrap(), [original.as_slice(), b"\r\n"].concat());
    assert_eq!(file.read_block().unwrap(), None);
}

#[tokio::test]
async fn existing_block_in_an_lf_file_is_replaced_in_place() {
    // The fixture holds a block written by an older Lampo, with `::1` lines.
    let dir = tempfile::tempdir().unwrap();
    let hosts = temp_hosts(&dir, &fixture("lf-with-existing-block.hosts"));
    let file = ElevatedHostsFile::with_launcher(hosts.clone(), PathBuf::from(HELPER), Arc::new(DirectLauncher));
    file.write_block(&block(&["new.test"]), &Tld::default()).await.unwrap();
    let text = std::fs::read_to_string(&hosts).unwrap();
    assert!(text.starts_with("127.0.0.1 localhost\n# >>> Lampo"));
    assert!(text.ends_with("# <<< Lampo managed block <<<\n10.1.1.1 keep.lan\n"));
    assert!(!text.contains("old.test") && !text.contains('\r'));
    assert!(text.contains("127.0.0.1\tnew.test\n") && !text.contains("::1"));
}

fn run_helper(args: &[String]) -> i32 {
    Command::new(HELPER).args(args).status().unwrap().code().unwrap()
}

/// Writes `contents` to `request` and returns the command line naming it with its digest.
fn request_for(request: &Path, contents: &[u8], hosts: &Path) -> Vec<String> {
    request_with_tld(request, contents, hosts, Tld::default())
}

fn request_with_tld(request: &Path, contents: &[u8], hosts: &Path, tld: Tld) -> Vec<String> {
    std::fs::write(request, contents).unwrap();
    HelperArgs { block_file: request.into(), digest: digest_of(contents), tld, hosts_override: Some(hosts.into()) }.to_args()
}

fn args(v: &[&str]) -> Vec<String> {
    v.iter().map(|a| a.to_string()).collect()
}

#[test]
fn helper_rejects_bad_requests_without_touching_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let hosts = temp_hosts(&dir, b"127.0.0.1 localhost\r\n");
    let request = dir.path().join("req.json");
    let valid = serde_json::to_vec(&block(&["ok.test"])).unwrap();

    assert_eq!(run_helper(&args(&[])), helper_exit::BAD_ARGUMENTS);
    assert_eq!(run_helper(&args(&["--block"])), helper_exit::BAD_ARGUMENTS);
    assert_eq!(run_helper(&args(&["--block", request.to_str().unwrap(), "--hosts", hosts.to_str().unwrap()])), helper_exit::BAD_ARGUMENTS, "no digest and TLD");
    let mut extra = request_for(&request, &valid, &hosts);
    extra.extend(args(&["--extra", "x"]));
    assert_eq!(run_helper(&extra), helper_exit::BAD_ARGUMENTS);

    let missing = request_for(&request, &valid, &hosts);
    std::fs::remove_file(&request).unwrap();
    assert_eq!(run_helper(&missing), helper_exit::INVALID_REQUEST, "missing request file");

    for (bad, why) in [
        (&br#"{"entries":[{"ip":"203.0.113.7","hostname":"bank.test"}]}"#[..], "non-loopback address"),
        (br#"{"entries":[{"ip":"127.0.0.1","hostname":"windowsupdate.com"}]}"#, "public name"),
        (br#""arbitrary content""#, "not a block"),
    ] {
        assert_eq!(run_helper(&request_for(&request, bad, &hosts)), helper_exit::INVALID_REQUEST, "{why}");
    }
    // A valid block that is not the one whose digest is on the command line.
    let tampered = request_for(&request, &valid, &hosts);
    std::fs::write(&request, serde_json::to_vec(&block(&["other.test"])).unwrap()).unwrap();
    assert_eq!(run_helper(&tampered), helper_exit::INVALID_REQUEST, "tampered request");
    assert_eq!(std::fs::read(&hosts).unwrap(), b"127.0.0.1 localhost\r\n");

    assert_eq!(run_helper(&request_for(&request, &valid, &hosts)), helper_exit::OK);
    assert!(std::fs::read_to_string(&hosts).unwrap().contains("127.0.0.1\tok.test\r\n"));
}

#[test]
fn helper_accepts_the_configured_tld_only_when_named() {
    let dir = tempfile::tempdir().unwrap();
    let hosts = temp_hosts(&dir, b"127.0.0.1 localhost\r\n");
    let request = dir.path().join("req.json");
    let lan = serde_json::to_vec(&block(&["shop.lan"])).unwrap();
    assert_eq!(run_helper(&request_for(&request, &lan, &hosts)), helper_exit::INVALID_REQUEST, "`lan` is not reserved");
    assert_eq!(run_helper(&request_with_tld(&request, &lan, &hosts, Tld::new("lan").unwrap())), helper_exit::OK);
    assert!(std::fs::read_to_string(&hosts).unwrap().contains("127.0.0.1\tshop.lan\r\n"));
}

#[test]
fn helper_refuses_utf16_hosts_files() {
    let dir = tempfile::tempdir().unwrap();
    let utf16: Vec<u8> = [0xFF, 0xFE].into_iter().chain("127.0.0.1 localhost\r\n".encode_utf16().flat_map(u16::to_le_bytes)).collect();
    let hosts = temp_hosts(&dir, &utf16);
    let request = dir.path().join("req.json");
    assert_eq!(run_helper(&request_for(&request, &serde_json::to_vec(&block(&["ok.test"])).unwrap(), &hosts)), helper_exit::UNSUPPORTED_HOSTS_FILE);
    assert_eq!(std::fs::read(&hosts).unwrap(), utf16);
}
