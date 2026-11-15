use lampo_domain::PortResult;
use lampo_domain::ports::VcRuntimeDetector;
use lampo_domain::vc_runtime::{InstalledVcRuntime, VcRedistributable};

/// Reads `HKLM\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64` (2015–2026) and
/// `HKLM\SOFTWARE\Microsoft\VisualStudio\11.0\VC\Runtimes\x64` (2012), WOW6432Node included
/// (the 2012 x64 redistributable registers itself only there).
#[derive(Debug, Default, Clone, Copy)]
pub struct RegistryVcRuntimeDetector;

/// `(registry path under HKLM, redistributable)`.
const KEYS: &[(&str, VcRedistributable)] = &[
    (r"SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64", VcRedistributable::Vs2015To2022X64),
    (r"SOFTWARE\WOW6432Node\Microsoft\VisualStudio\14.0\VC\Runtimes\x64", VcRedistributable::Vs2015To2022X64),
    (r"SOFTWARE\Microsoft\VisualStudio\11.0\VC\Runtimes\x64", VcRedistributable::Vs2012X64),
    (r"SOFTWARE\WOW6432Node\Microsoft\VisualStudio\11.0\VC\Runtimes\x64", VcRedistributable::Vs2012X64),
];

/// Values of one `Runtimes\x64` key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeKey {
    pub installed: u32,
    pub major: u32,
    pub minor: u32,
    pub build: u32,
}

/// Keeps installed entries, one per redistributable (the newest version).
pub fn merge(found: impl IntoIterator<Item = (VcRedistributable, RuntimeKey)>) -> Vec<InstalledVcRuntime> {
    let mut out: Vec<InstalledVcRuntime> = Vec::new();
    for (redistributable, key) in found {
        if key.installed != 1 {
            continue;
        }
        let (Ok(major), Ok(minor)) = (u16::try_from(key.major), u16::try_from(key.minor)) else { continue };
        let version = (major, minor, key.build);
        match out.iter_mut().find(|r| r.redistributable == redistributable) {
            Some(existing) if existing.version < version => existing.version = version,
            Some(_) => {}
            None => out.push(InstalledVcRuntime { redistributable, version }),
        }
    }
    out.sort_by_key(|r| r.redistributable);
    out
}

impl VcRuntimeDetector for RegistryVcRuntimeDetector {
    #[cfg(windows)]
    fn installed(&self) -> PortResult<Vec<InstalledVcRuntime>> {
        use winreg::RegKey;
        use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};

        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let mut found = Vec::new();
        for (path, redistributable) in KEYS {
            // Explicit 64-bit view, so the explicit WOW6432Node paths mean what they say.
            let key = match hklm.open_subkey_with_flags(path, KEY_READ | KEY_WOW64_64KEY) {
                Ok(k) => k,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    tracing::debug!(path, error = %e, "cannot read VC++ runtime key");
                    continue;
                }
            };
            let get = |name: &str| key.get_value::<u32, _>(name).unwrap_or(0);
            found.push((*redistributable, RuntimeKey { installed: get("Installed"), major: get("Major"), minor: get("Minor"), build: get("Bld") }));
        }
        Ok(merge(found))
    }

    #[cfg(not(windows))]
    fn installed(&self) -> PortResult<Vec<InstalledVcRuntime>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use lampo_domain::vc_runtime::VcRuntime;

    use super::*;

    #[test]
    fn merges_views_and_skips_uninstalled() {
        let k = |installed, major, minor, build| RuntimeKey { installed, major, minor, build };
        let merged =
            merge([(VcRedistributable::Vs2015To2022X64, k(1, 14, 40, 33810)), (VcRedistributable::Vs2015To2022X64, k(1, 14, 51, 36247)), (VcRedistributable::Vs2012X64, k(0, 11, 0, 61030))]);
        assert_eq!(merged, [InstalledVcRuntime { redistributable: VcRedistributable::Vs2015To2022X64, version: (14, 51, 36247) }]);
        assert!(VcRuntime::Vs18.is_satisfied_by(&merged));
        assert!(!VcRuntime::Vc11.is_satisfied_by(&merged));
    }

    #[cfg(windows)]
    #[test]
    fn reads_this_machine_without_error() {
        // Content depends on the machine; it must simply not fail.
        let found = RegistryVcRuntimeDetector.installed().unwrap();
        for r in found {
            assert!(r.version.0 == 11 || r.version.0 == 14, "{r:?}");
        }
    }
}
