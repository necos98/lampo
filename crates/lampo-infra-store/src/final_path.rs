//! Final paths of folders: `std::fs::canonicalize`, which on Windows opens the folder and asks
//! `GetFinalPathNameByHandleW` (volume drive letter instead of a `subst` alias, junctions and
//! symbolic links followed, long names instead of 8.3 ones).

use lampo_domain::ports::PathCanonicalizer;

#[derive(Debug, Default, Clone, Copy)]
pub struct FsPathCanonicalizer;

impl PathCanonicalizer for FsPathCanonicalizer {
    fn canonicalize(&self, folder: &str) -> Option<String> {
        if folder.trim().is_empty() {
            return None;
        }
        std::fs::canonicalize(folder).ok().map(|p| p.display().to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use lampo_domain::project::ProjectRoot;

    use super::*;

    fn root(path: &str) -> ProjectRoot {
        ProjectRoot::new(path).unwrap()
    }

    #[test]
    fn a_junction_resolves_to_its_target() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("real").join("shop");
        std::fs::create_dir_all(target.join("app")).unwrap();
        let link = tmp.path().join("link");
        let status = Command::new("cmd").arg("/C").arg("mklink").arg("/J").arg(&link).arg(&target).output().unwrap();
        assert!(status.status.success(), "mklink /J failed: {}", String::from_utf8_lossy(&status.stderr));

        let canonical = FsPathCanonicalizer.canonicalize(&link.join("app").display().to_string()).unwrap();
        let expected = FsPathCanonicalizer.canonicalize(&target.join("app").display().to_string()).unwrap();
        assert_eq!(root(&canonical), root(&expected));
        assert!(root(&FsPathCanonicalizer.canonicalize(&target.display().to_string()).unwrap()).contains(&canonical));
    }

    #[test]
    fn missing_or_blank_folders_have_no_final_path() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(FsPathCanonicalizer.canonicalize(&tmp.path().join("gone").display().to_string()), None);
        assert_eq!(FsPathCanonicalizer.canonicalize(""), None);
    }
}
