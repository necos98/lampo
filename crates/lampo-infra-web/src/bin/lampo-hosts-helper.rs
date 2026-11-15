//! Elevated helper: `lampo-hosts-helper.exe --block <file.json>`.
//!
//! Reads the desired `HostsBlock` (JSON) from `<file.json>`, validates it (loopback addresses
//! only), splices it into the system hosts file preserving everything outside the Lampo
//! markers byte for byte, writes atomically, and exits with
//! `lampo_infra_web::hosts::helper_exit` codes. Does nothing else; see
//! `lampo_infra_web::hosts::helper` for the security rules. Debug builds also accept
//! `--hosts <path>` (tests); release builds do not.

// No console window flashes when started through UAC.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    use lampo_infra_web::hosts::{helper, helper_exit};
    let args: Option<Vec<String>> = std::env::args_os().skip(1).map(|a| a.into_string().ok()).collect();
    let code = match args {
        Some(args) => helper::run(&args),
        None => helper_exit::BAD_ARGUMENTS,
    };
    std::process::exit(code);
}
