//! Embeds every file under `claude-plugin/skills/` (repository root) as text, so skills are
//! added or edited without touching Rust code. Generates `$OUT_DIR/claude_skills.rs`:
//! `&[(relative path with forward slashes, include_str!(file))]`, sorted by path.

use std::path::{Path, PathBuf};

fn collect(dir: &Path, root: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, root, out);
        } else if let Ok(relative) = path.strip_prefix(root) {
            let relative = relative.to_string_lossy().replace('\\', "/");
            out.push((relative, path));
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let skills = manifest.join("..").join("..").join("claude-plugin").join("skills");
    // A directory is scanned recursively by cargo: adding, removing or editing a file reruns this.
    println!("cargo:rerun-if-changed={}", skills.display());
    assert!(skills.is_dir(), "missing skills folder {}", skills.display());
    let mut files = Vec::new();
    collect(&skills, &skills, &mut files);
    files.sort();
    let mut code = String::from("&[\n");
    for (relative, path) in files {
        code.push_str(&format!("    ({relative:?}, include_str!({:?})),\n", path.display().to_string()));
    }
    code.push_str("]\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("claude_skills.rs");
    std::fs::write(out, code).expect("writing claude_skills.rs");
}
