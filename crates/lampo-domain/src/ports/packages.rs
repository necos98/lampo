//! Package catalog, download and installation. Implemented by `lampo-infra-packages`.

use std::path::Path;

use async_trait::async_trait;

use crate::error::PortResult;
use crate::package::{CatalogEntry, DownloadUrl, InstalledPackage, PackageId, PackageKind, Sha256Digest};
use crate::ports::events::ProgressSink;
use crate::progress::CancelFlag;
use crate::vc_runtime::InstalledVcRuntime;

/// A source of installable versions (embedded frozen catalog, windows.php.net live list, …).
/// Adding a package kind = adding a source that serves it (open-closed).
#[async_trait]
pub trait PackageCatalogSource: Send + Sync {
    /// Short name for logs and error messages (`embedded`, `windows.php.net`).
    fn name(&self) -> &str;
    /// Kinds this source can list.
    fn kinds(&self) -> Vec<PackageKind>;
    /// Entries of `kind` (empty if the source does not serve it). Live sources may cache.
    async fn fetch(&self, kind: &PackageKind) -> PortResult<Vec<CatalogEntry>>;
}

/// Downloads a URL to a file, resuming or restarting as the implementation sees fit.
#[async_trait]
pub trait PackageDownloader: Send + Sync {
    /// Writes to `destination` (creating its parent dir if missing) and returns the byte count.
    /// Reports `ProgressStage::Downloading`; honours `cancel` between chunks.
    async fn download(&self, url: &DownloadUrl, destination: &Path, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<u64>;
    /// Deletes a download made by [`PackageDownloader::download`] (complete or partial, with any
    /// resume metadata) once it is no longer needed. Ok if absent.
    async fn discard(&self, destination: &Path) -> PortResult<()>;
}

/// Computes file digests (the comparison is the domain's `package::verify_checksum`).
#[async_trait]
pub trait ChecksumCalculator: Send + Sync {
    async fn sha256(&self, file: &Path) -> PortResult<Sha256Digest>;
}

/// Extracts archives.
#[async_trait]
pub trait ArchiveExtractor: Send + Sync {
    /// Extracts the zip `archive` into `destination` (created if missing), dropping the
    /// `strip_prefix` top-level folder when given. Rejects entries escaping `destination`.
    /// Reports `ProgressStage::Extracting`; honours `cancel`.
    async fn extract_zip(&self, archive: &Path, destination: &Path, strip_prefix: Option<&str>, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<()>;
}

/// The set of installed packages on disk.
pub trait InstalledPackageRegistry: Send + Sync {
    fn list(&self) -> PortResult<Vec<InstalledPackage>>;
    /// Where `id` is (or would be) installed.
    fn install_dir(&self, id: &PackageId) -> std::path::PathBuf;
    /// Records a package whose files are already in place.
    fn record(&self, package: &InstalledPackage) -> PortResult<()>;
    /// Deletes the package files and its record.
    fn remove(&self, id: &PackageId) -> PortResult<()>;
}

/// Kind-specific work after files are in place (e.g. PHP: seed `php.ini-development`;
/// MySQL: nothing; Composer: write `composer.bat`). Registered per kind; kinds without a hook
/// need none (open-closed).
#[async_trait]
pub trait PackageInstallHook: Send + Sync {
    fn kind(&self) -> PackageKind;
    async fn after_install(&self, package: &InstalledPackage) -> PortResult<()>;
}

/// Lists the extensions an installed PHP build ships as loadable modules (`ext\php_<name>.dll`),
/// so front-ends offer exactly what that PHP minor can load.
pub trait PhpExtensionScanner: Send + Sync {
    /// Extension names, sorted. Empty if the folder has no `ext` dir.
    fn bundled_extensions(&self, php_dir: &Path) -> PortResult<Vec<crate::php::ExtensionName>>;
}

/// Reads which Visual C++ runtimes are installed (registry). Whether they are sufficient is
/// decided by `VcRuntime::is_satisfied_by`.
pub trait VcRuntimeDetector: Send + Sync {
    fn installed(&self) -> PortResult<Vec<InstalledVcRuntime>>;
}
