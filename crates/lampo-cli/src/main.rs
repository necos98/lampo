//! `lampo` - machine-friendly CLI front-end. See `cli.rs` for the command surface and
//! `exit_code.rs` for the stable exit codes.

fn main() {
    std::process::exit(lampo_cli::run(std::env::args_os().collect()));
}
