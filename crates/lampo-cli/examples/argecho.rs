//! Test executable for the shim tests (not shipped): prints its arguments as one JSON array on
//! stdout, reports the size of stdin and the `PHPRC` variable on stderr, and exits with the code
//! in `ARGECHO_EXIT` (default 0).

use std::io::Read;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    println!("{}", serde_json::to_string(&args).unwrap());
    let mut stdin = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut stdin);
    eprintln!("stdin={} phprc={}", stdin.len(), std::env::var("PHPRC").unwrap_or_default());
    std::process::exit(std::env::var("ARGECHO_EXIT").ok().and_then(|v| v.parse().ok()).unwrap_or(0));
}
