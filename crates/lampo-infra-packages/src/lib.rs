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
//! | [`ZipExtractor`]                 | `ArchiveExtractor`          |
//! | [`FsInstalledPackageRegistry`]   | `InstalledPackageRegistry`  |
//! | [`FsPackageSlotLock`]            | `PackageSlotLock` (in-process mutex + `<slot>.lock` file lock) |
//! | [`FsLeftoverSweeper`]            | `PackageLeftoverSweeper` (app start: interrupted installs, stale downloads) |
//! | [`RegistryVcRuntimeDetector`]    | `VcRuntimeDetector`         |
//! | [`FsPhpExtensionScanner`]        | `PhpExtensionScanner`       |
//! | [`hooks`]                        | `PackageInstallHook` (per kind) |
//!
//! [`catalog::default_sources`] builds every catalog source in the right order. The on-disk
//! layout (one directory per kind + series, replaced in place on patch updates) is documented in
//! [`registry`]; where each kind keeps its binaries (entry points by role) in [`layout`].
//! How a slot is replaced all-or-nothing is in [`slot`].
//!
//! Nothing is hosted by Lampo: every URL is the vendor's own official location, with a pinned or
//! vendor-published SHA-256.

pub mod catalog;
mod checksum;
pub mod download;
mod entry_name;
pub mod extract;
mod file_lock;
pub mod hooks;
pub mod http;
pub mod layout;
mod leftovers;
pub mod mirrors;
pub mod php_extensions;
mod pid;
mod recovery;
pub mod registry;
pub mod slot;
pub mod slot_lock;
pub mod sweep;
pub mod vc_runtime;

#[cfg(test)]
mod test_server;

pub use catalog::apache_lounge::ApacheLoungeSource;
pub use catalog::composer::ComposerSource;
pub use catalog::embedded::EmbeddedCatalog;
pub use catalog::php_net::PhpNetReleasesSource;
pub use download::HttpDownloader;
pub use extract::ZipExtractor;
pub use php_extensions::FsPhpExtensionScanner;
pub use registry::FsInstalledPackageRegistry;
pub use slot_lock::FsPackageSlotLock;
pub use sweep::FsLeftoverSweeper;
pub use vc_runtime::RegistryVcRuntimeDetector;
