//! Elevated helper, two modes:
//! * `lampo-hosts-helper.exe --block <file.json> --sha256 <hex> --tld <tld>`: reads the desired
//!   `HostsBlock` (JSON) from `<file.json>`, checks it against its SHA-256, validates it (loopback
//!   addresses only, names under a local TLD or `<tld>`), splices it into the system hosts file
//!   preserving everything outside the Lampo markers byte for byte, writes atomically, and exits
//!   with `lampo_infra_web::hosts::helper_exit` codes. Debug builds also accept `--hosts <path>`
//!   (tests); release builds do not. See `lampo_infra_web::hosts::helper`.
//! * `lampo-hosts-helper.exe --defender-exclusion add|remove [--path <folder>] [--process <exe>]...`:
//!   adds or removes the Microsoft Defender exclusions of Lampo's MySQL data folder of a user
//!   profile (path) and of the `php-cgi.exe`/`httpd.exe` of its packages (processes), and of
//!   nothing else; exits with `lampo_infra_web::defender::helper_exit` codes. See
//!   `lampo_infra_web::defender::helper`.

// No console window flashes when started through UAC.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    use lampo_infra_web::{defender, hosts};
    let args: Option<Vec<String>> = std::env::args_os().skip(1).map(|a| a.into_string().ok()).collect();
    let code = match args {
        Some(args) if defender::request::is_exclusion_request(&args) => defender::helper::run(&args),
        Some(args) => hosts::helper::run(&args),
        None => hosts::helper_exit::BAD_ARGUMENTS,
    };
    std::process::exit(code);
}
