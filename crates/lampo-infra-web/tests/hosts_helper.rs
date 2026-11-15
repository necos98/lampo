//! The real `lampo-hosts-helper` binary, run non-elevated against temp copies of fixture
//! hosts files through its debug-only `--hosts` option. Never touches the system hosts file and
//! never triggers UAC.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use lampo_domain::hostname::Hostname;
use lampo_domain::hosts::{HostsBlock, HostsEntry};
use lampo_domain::ports::{HostsFileReader, HostsFileWriter};
use lampo_domain::{PortError, PortResult};
use lampo_infra_web::hosts::elevation::{HelperLauncher, LaunchOutcome};
use lampo_infra_web::hosts::{ElevatedHostsFile, helper_exit};

const HELPER: &str = env!("CARGO_BIN_EXE_lampo-hosts-helper");

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join("hosts").join(name)).unwrap()
}

fn block(names: &[&str]) -> HostsBlock {
    HostsBlock::new(names.iter().flat_map(|n| {
        let h = Hostname::new(n).unwrap();
        [HostsEntry { ip: IpAddr::V4(Ipv4Addr::LOCALHOST), hostname: h.clone() }, HostsEntry { ip: IpAddr::V6(Ipv6Addr::LOCALHOST), hostname: h }]
    }))
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

    file.write_block(&block(&["shop.test", "blog.test"])).await.unwrap();
    let written = std::fs::read(&hosts).unwrap();
    assert!(written.starts_with(&original), "bytes before the block are untouched (BOM, ANSI, CRLF)");
    assert_eq!(&written[original.len()..original.len() + 2], b"\r\n");
    assert_eq!(file.read_block().unwrap(), Some(block(&["blog.test", "shop.test"])));

    // Removing every project restores the original bytes exactly.
    file.write_block(&HostsBlock::default()).await.unwrap();
    assert_eq!(std::fs::read(&hosts).unwrap(), [original.as_slice(), b"\r\n"].concat());
    assert_eq!(file.read_block().unwrap(), None);
}

#[tokio::test]
async fn existing_block_in_an_lf_file_is_replaced_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let hosts = temp_hosts(&dir, &fixture("lf-with-existing-block.hosts"));
    let file = ElevatedHostsFile::with_launcher(hosts.clone(), PathBuf::from(HELPER), Arc::new(DirectLauncher));
    file.write_block(&block(&["new.test"])).await.unwrap();
    let text = std::fs::read_to_string(&hosts).unwrap();
    assert!(text.starts_with("127.0.0.1 localhost\n# >>> Lampo"));
    assert!(text.ends_with("# <<< Lampo managed block <<<\n10.1.1.1 keep.lan\n"));
    assert!(!text.contains("old.test") && !text.contains('\r'));
}

fn run_helper(args: &[&str]) -> i32 {
    Command::new(HELPER).args(args).status().unwrap().code().unwrap()
}

#[test]
fn helper_rejects_bad_requests_without_touching_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let hosts = temp_hosts(&dir, b"127.0.0.1 localhost\r\n");
    let hosts_arg = hosts.to_str().unwrap();
    let request = dir.path().join("req.json");
    let request_arg = request.to_str().unwrap();

    assert_eq!(run_helper(&[]), helper_exit::BAD_ARGUMENTS);
    assert_eq!(run_helper(&["--block"]), helper_exit::BAD_ARGUMENTS);
    assert_eq!(run_helper(&["--block", request_arg, "--extra", "x"]), helper_exit::BAD_ARGUMENTS);
    assert_eq!(run_helper(&["--block", request_arg, "--hosts", hosts_arg]), helper_exit::INVALID_REQUEST, "missing request file");

    std::fs::write(&request, r#"{"entries":[{"ip":"203.0.113.7","hostname":"bank.test"}]}"#).unwrap();
    assert_eq!(run_helper(&["--block", request_arg, "--hosts", hosts_arg]), helper_exit::INVALID_REQUEST, "non-loopback address");
    std::fs::write(&request, "\"arbitrary content\"").unwrap();
    assert_eq!(run_helper(&["--block", request_arg, "--hosts", hosts_arg]), helper_exit::INVALID_REQUEST);
    assert_eq!(std::fs::read(&hosts).unwrap(), b"127.0.0.1 localhost\r\n");

    std::fs::write(&request, serde_json::to_string(&block(&["ok.test"])).unwrap()).unwrap();
    assert_eq!(run_helper(&["--block", request_arg, "--hosts", hosts_arg]), helper_exit::OK);
    assert!(std::fs::read_to_string(&hosts).unwrap().contains("::1\tok.test\r\n"));
}

#[test]
fn helper_refuses_utf16_hosts_files() {
    let dir = tempfile::tempdir().unwrap();
    let utf16: Vec<u8> = [0xFF, 0xFE].into_iter().chain("127.0.0.1 localhost\r\n".encode_utf16().flat_map(u16::to_le_bytes)).collect();
    let hosts = temp_hosts(&dir, &utf16);
    let request = dir.path().join("req.json");
    std::fs::write(&request, serde_json::to_string(&block(&["ok.test"])).unwrap()).unwrap();
    assert_eq!(run_helper(&["--block", request.to_str().unwrap(), "--hosts", hosts.to_str().unwrap()]), helper_exit::UNSUPPORTED_HOSTS_FILE);
    assert_eq!(std::fs::read(&hosts).unwrap(), utf16);
}
