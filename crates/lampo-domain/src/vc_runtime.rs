//! Visual C++ runtime requirements of Windows builds (PHP, Apache Lounge, MySQL).
//!
//! The detection itself (registry lookup) is a port, [`crate::ports::VcRuntimeDetector`];
//! deciding whether what is installed satisfies a build is a domain rule, implemented here.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::php::PhpMinor;

/// The compiler toolset a binary was built with, as named by windows.php.net (`vs17`, `vc15`…).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VcRuntime {
    /// Visual Studio 2012 (PHP 5.5–5.6).
    Vc11,
    /// Visual Studio 2015 (PHP 7.0–7.1).
    Vc14,
    /// Visual Studio 2017 (PHP 7.2–7.4).
    Vc15,
    /// Visual Studio 2019 (PHP 8.0–8.3).
    Vs16,
    /// Visual Studio 2022 (PHP 8.4+, Apache Lounge builds up to 2.4.66).
    Vs17,
    /// Visual Studio 2026 (Apache Lounge builds since November 2026). Same 14.x redistributable,
    /// version 14.50 or later.
    Vs18,
}

/// The redistributable package a user installs to satisfy a [`VcRuntime`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VcRedistributable {
    /// "Visual C++ Redistributable for Visual Studio 2012 Update 4" (x64), runtime 11.0.
    Vs2012X64,
    /// "Visual C++ Redistributable 2015–2022" (x64), runtime 14.x. Backward compatible within 14.x.
    Vs2015To2022X64,
}

impl VcRedistributable {
    /// Official Microsoft download link shown to the user when the runtime is missing.
    pub fn download_url(self) -> &'static str {
        match self {
            Self::Vs2012X64 => "https://download.microsoft.com/download/1/6/B/16B06F60-3B20-4FF2-B699-5E9B7962F9AE/VSU_4/vcredist_x64.exe",
            Self::Vs2015To2022X64 => "https://aka.ms/vs/17/release/vc_redist.x64.exe",
        }
    }
}

impl fmt::Display for VcRedistributable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Vs2012X64 => "Visual C++ 2012 (x64)",
            Self::Vs2015To2022X64 => "Visual C++ 2015-2022 (x64)",
        })
    }
}

/// A runtime found on the machine (as reported by the detector).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledVcRuntime {
    pub redistributable: VcRedistributable,
    /// `(major, minor, build)`, e.g. `(14, 40, 33810)`.
    pub version: (u16, u16, u32),
}

impl VcRuntime {
    /// Toolset used by the official windows.php.net builds of a PHP minor.
    pub fn for_php(php: PhpMinor) -> Self {
        match (php.major(), php.minor()) {
            (5, _) => Self::Vc11,
            (7, 0 | 1) => Self::Vc14,
            (7, _) => Self::Vc15,
            (8, 0..=3) => Self::Vs16,
            _ => Self::Vs17,
        }
    }

    pub fn redistributable(self) -> VcRedistributable {
        match self {
            Self::Vc11 => VcRedistributable::Vs2012X64,
            _ => VcRedistributable::Vs2015To2022X64,
        }
    }

    /// Minimum `(major, minor)` runtime version that can load binaries built with this toolset.
    pub fn minimum_runtime(self) -> (u16, u16) {
        match self {
            Self::Vc11 => (11, 0),
            Self::Vc14 => (14, 0),
            Self::Vc15 => (14, 10),
            Self::Vs16 => (14, 20),
            Self::Vs17 => (14, 30),
            Self::Vs18 => (14, 50),
        }
    }

    /// Whether one of the installed runtimes can load binaries built with this toolset.
    pub fn is_satisfied_by(self, installed: &[InstalledVcRuntime]) -> bool {
        let needed = self.redistributable();
        let (min_major, min_minor) = self.minimum_runtime();
        installed.iter().any(|r| r.redistributable == needed && (r.version.0, r.version.1) >= (min_major, min_minor))
    }
}

impl fmt::Display for VcRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Vc11 => "vc11",
            Self::Vc14 => "vc14",
            Self::Vc15 => "vc15",
            Self::Vs16 => "vs16",
            Self::Vs17 => "vs17",
            Self::Vs18 => "vs18",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn php(s: &str) -> PhpMinor {
        PhpMinor::parse(s).unwrap()
    }

    #[test]
    fn maps_php_minors_to_toolsets() {
        assert_eq!(VcRuntime::for_php(php("5.6")), VcRuntime::Vc11);
        assert_eq!(VcRuntime::for_php(php("7.1")), VcRuntime::Vc14);
        assert_eq!(VcRuntime::for_php(php("7.4")), VcRuntime::Vc15);
        assert_eq!(VcRuntime::for_php(php("8.3")), VcRuntime::Vs16);
        assert_eq!(VcRuntime::for_php(php("8.4")), VcRuntime::Vs17);
    }

    #[test]
    fn checks_installed_runtime_versions() {
        let old14 = InstalledVcRuntime { redistributable: VcRedistributable::Vs2015To2022X64, version: (14, 16, 27012) };
        let new14 = InstalledVcRuntime { redistributable: VcRedistributable::Vs2015To2022X64, version: (14, 40, 33810) };
        let vc11 = InstalledVcRuntime { redistributable: VcRedistributable::Vs2012X64, version: (11, 0, 61030) };

        assert!(VcRuntime::Vc15.is_satisfied_by(&[old14]));
        assert!(!VcRuntime::Vs17.is_satisfied_by(&[old14]));
        assert!(VcRuntime::Vs17.is_satisfied_by(&[old14, new14]));
        assert!(!VcRuntime::Vc11.is_satisfied_by(&[new14]));
        assert!(VcRuntime::Vc11.is_satisfied_by(&[vc11]));
        assert!(!VcRuntime::Vs16.is_satisfied_by(&[]));
    }
}
