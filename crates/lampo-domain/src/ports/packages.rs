//! Package catalog, download and installation. Implemented by `lampo-infra-packages`.

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use crate::error::PortResult;
use crate::package::{CatalogEntry, DownloadUrl, DownloadedFile, InstalledPackage, PackageId, PackageKind};
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
    /// Writes to `destination` (creating its parent dir if missing) and returns the byte count
    /// with the SHA-256 computed while streaming (the comparison is the domain's
    /// `package::verify_checksum`). Refuses (`InvalidData`) a body announced or received larger
    /// than `max_bytes`. Reports `ProgressStage::Downloading`; honours `cancel` between chunks.
    async fn download(&self, url: &DownloadUrl, destination: &Path, max_bytes: Option<u64>, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<DownloadedFile>;
    /// Deletes a download made by [`PackageDownloader::download`] (complete or partial, with any
    /// resume metadata) once it is no longer needed. Ok if absent.
    async fn discard(&self, destination: &Path) -> PortResult<()>;
}

/// A package slot whose contents were just replaced, with its previous contents still kept aside:
/// [`ArchiveExtractor::commit`] makes the replacement final, [`ArchiveExtractor::roll_back`]
/// puts the previous contents back. Until either runs, a crash leaves both on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "a replacement must be committed or rolled back"]
pub struct SlotReplacement {
    /// The slot directory, now holding the new package.
    pub destination: PathBuf,
    /// Where the previous contents were moved; `None` if the slot was empty.
    pub previous: Option<PathBuf>,
}

/// Places verified payloads into a package directory, all-or-nothing: `destination` ends up
/// either as it was or as the complete new package, the previous contents kept aside until the
/// returned [`SlotReplacement`] is committed or rolled back.
#[async_trait]
pub trait ArchiveExtractor: Send + Sync {
    /// Extracts the zip `archive` into `destination` (created if missing), dropping the
    /// `strip_prefix` top-level folder when given. Rejects entries escaping `destination` or
    /// unsafe on Windows, and archives expanding beyond a bounded size.
    /// Reports `ProgressStage::Extracting`; honours `cancel`.
    async fn extract_zip(&self, archive: &Path, destination: &Path, strip_prefix: Option<&str>, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<SlotReplacement>;
    /// Moves the single-file payload `file` into a fresh `destination` as `file_name` (a plain
    /// file name), replacing the previous contents of `destination` in one rename.
    async fn place_file(&self, file: &Path, destination: &Path, file_name: &str, cancel: &CancelFlag) -> PortResult<SlotReplacement>;
    /// Makes `replacement` final: deletes the previous contents. A deletion that fails (a file
    /// still held) is logged and left to the next leftover sweep.
    async fn commit(&self, replacement: SlotReplacement);
    /// Undoes `replacement`: the slot gets its previous contents back, or is removed if it was
    /// empty before.
    async fn roll_back(&self, replacement: SlotReplacement) -> PortResult<()>;
}

/// Held while a package slot is reserved; dropping it releases the slot.
pub struct SlotGuard {
    /// Only held for its `Drop`.
    _held: Box<dyn Send + Sync>,
}

impl SlotGuard {
    /// A guard releasing the slot when `held` (the adapter's lock handles) is dropped.
    pub fn new(held: impl Send + Sync + 'static) -> Self {
        Self { _held: Box::new(held) }
    }
}

impl std::fmt::Debug for SlotGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SlotGuard")
    }
}

/// Mutual exclusion per package slot (kind + series + PHP minor), within the process and
/// across Lampo processes (GUI, CLI): two installs into one slot never download into the same
/// partial file or extract into the same directory at the same time.
#[async_trait]
pub trait PackageSlotLock: Send + Sync {
    /// Waits until nobody else holds the slot of `id`, then reserves it until the guard is
    /// dropped. Honours `cancel` while waiting (`Cancelled`).
    async fn acquire(&self, id: &PackageId, cancel: &CancelFlag) -> PortResult<SlotGuard>;
}

/// Removes what interrupted installs left behind (run at app start).
#[async_trait]
pub trait PackageLeftoverSweeper: Send + Sync {
    /// Restores slots whose replacement a crash interrupted, deletes the staging, previous and
    /// removed copies of processes that are gone, and stale partial downloads. Best effort:
    /// what cannot be deleted now is logged and retried at the next sweep.
    async fn sweep(&self);
}

/// The set of installed packages on disk.
#[async_trait]
pub trait InstalledPackageRegistry: Send + Sync {
    fn list(&self) -> PortResult<Vec<InstalledPackage>>;
    /// Where `id` is (or would be) installed.
    fn install_dir(&self, id: &PackageId) -> PathBuf;
    /// Records a package whose files are already in place.
    fn record(&self, package: &InstalledPackage) -> PortResult<()>;
    /// Deletes the package files and its record. Async: a package can be thousands of files,
    /// deleted off the async workers.
    async fn remove(&self, id: &PackageId) -> PortResult<()>;
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
