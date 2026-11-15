//! # lampo-infra-packages
//!
//! Everything about getting third-party binaries onto disk. One reason to change: *where
//! packages come from and how they are fetched/unpacked*.
//!
//! | Adapter                          | Ports                       |
//! |----------------------------------|-----------------------------|
//! | [`EmbeddedCatalog`]              | `PackageCatalogSource` (frozen/pinned entries compiled in) |
//! | [`PhpNetReleasesSource`]         | `PackageCatalogSource` (live windows.php.net list, cached) |
//! | [`ComposerSource`]               | `PackageCatalogSource` (live getcomposer.org) |
//! | [`ApacheLoungeSource`]           | `PackageCatalogSource` (live apachelounge.com) |
//! | [`HttpDownloader`]               | `PackageDownloader`         |
//! | [`Sha256Calculator`]             | `ChecksumCalculator`        |
//! | [`ZipExtractor`]                 | `ArchiveExtractor`          |
//! | [`FsInstalledPackageRegistry`]   | `InstalledPackageRegistry`  |
//! | [`RegistryVcRuntimeDetector`]    | `VcRuntimeDetector`         |
//! | [`FsPhpExtensionScanner`]        | `PhpExtensionScanner`       |
//! | [`hooks`]                        | `PackageInstallHook` (per kind) |
//!
//! [`catalog::default_sources`] builds every catalog source in the right order. The on-disk
//! layout (one directory per kind + series, replaced in place on patch updates) is documented in
//! [`registry`]; where each kind keeps its binaries (entry points by role) in [`layout`].
//!
//! Nothing is hosted by Lampo: every URL is the vendor's own official location, with a pinned or
//! vendor-published SHA-256.

pub mod catalog;
pub mod checksum;
pub mod download;
pub mod extract;
pub mod hooks;
pub mod http;
pub mod layout;
pub mod mirrors;
pub mod php_extensions;
pub mod registry;
pub mod vc_runtime;

#[cfg(test)]
mod test_server;

pub use catalog::apache_lounge::ApacheLoungeSource;
pub use catalog::composer::ComposerSource;
pub use catalog::embedded::EmbeddedCatalog;
pub use catalog::php_net::PhpNetReleasesSource;
pub use checksum::Sha256Calculator;
pub use download::HttpDownloader;
pub use extract::ZipExtractor;
pub use php_extensions::FsPhpExtensionScanner;
pub use registry::FsInstalledPackageRegistry;
pub use vc_runtime::RegistryVcRuntimeDetector;
