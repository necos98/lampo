//! Live-network checks. Ignored by default; run with
//! `cargo test -p lampo-infra-packages --test live -- --ignored --nocapture --test-threads 1`.

use std::path::Path;
use std::process::Command;

use lampo_domain::package::{CatalogEntry, InstalledPackage, PackageKind, PackagePayload, latest, merge_catalogs, verify_checksum};
use lampo_domain::paths::LampoPaths;
use lampo_domain::php::PhpMinor;
use lampo_domain::ports::{ArchiveExtractor, InstalledPackageRegistry, NullProgressSink, PackageDownloader, VcRuntimeDetector};
use lampo_domain::progress::CancelFlag;
use lampo_infra_packages::catalog::default_sources;
use lampo_infra_packages::{EmbeddedCatalog, FsInstalledPackageRegistry, HttpDownloader, RegistryVcRuntimeDetector, ZipExtractor, hooks};

async fn all_entries(paths: &LampoPaths) -> Vec<CatalogEntry> {
    let client = lampo_infra_packages::http::build_client().unwrap();
    let mut lists = Vec::new();
    for source in default_sources(client, paths) {
        for kind in source.kinds() {
            match source.fetch(&kind).await {
                Ok(list) => lists.push(list),
                Err(e) => panic!("source {} failed for {kind}: {e}", source.name()),
            }
        }
    }
    let merged = merge_catalogs(lists);
    assert!(merged.conflicts.is_empty(), "live and pinned hashes disagree: {:?}", merged.conflicts);
    merged.entries
}

/// The install pipeline as the application runs it: download (hashed) → verify → extract/place → hook → record.
async fn install(entry: &CatalogEntry, registry: &FsInstalledPackageRegistry, downloads: &Path) -> InstalledPackage {
    let client = lampo_infra_packages::http::build_client().unwrap();
    let downloader = HttpDownloader::new(client);
    let cancel = CancelFlag::new();
    std::fs::create_dir_all(downloads).unwrap();
    let file = downloads.join(entry.url.file_name());
    let downloaded = downloader.download(&entry.url, &file, entry.max_download_bytes(), &NullProgressSink, &cancel).await.unwrap();
    println!("downloaded {} ({} bytes)", entry.url, downloaded.bytes);
    verify_checksum(entry.url.file_name(), entry.sha256.as_ref().expect("pinned"), &downloaded.sha256).unwrap();
    let dir = registry.install_dir(&entry.id);
    match &entry.payload {
        PackagePayload::Zip { strip_prefix } => {
            ZipExtractor.extract_zip(&file, &dir, strip_prefix.as_deref(), &NullProgressSink, &cancel).await.unwrap();
            std::fs::remove_file(&file).unwrap();
        }
        PackagePayload::SingleFile { file_name } => ZipExtractor.place_file(&file, &dir, file_name, &cancel).await.unwrap(),
    }
    let package = InstalledPackage::new(entry.id.clone(), dir, entry.vc_runtime);
    for hook in hooks::all().iter().filter(|h| h.kind() == entry.id.kind) {
        hook.after_install(&package).await.unwrap();
    }
    registry.record(&package).unwrap();
    package
}

fn run(exe: &Path, args: &[&str]) -> String {
    let out = Command::new(exe).args(args).output().unwrap_or_else(|e| panic!("{}: {e}", exe.display()));
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{} {args:?} failed: {text}", exe.display());
    text
}

#[tokio::test]
#[ignore = "network: downloads PHP 8.3 + extensions (~35 MB)"]
async fn installs_php_and_extensions_end_to_end() {
    let home = tempfile::tempdir().unwrap();
    let paths = LampoPaths::new(home.path().join("config"), home.path().join("data"));
    let registry = FsInstalledPackageRegistry::new(paths.clone());
    let entries = all_entries(&paths).await;
    let php83 = PhpMinor::parse("8.3").unwrap();

    let php = latest(&entries.iter().filter(|e| php83.contains(&e.id.version)).cloned().collect::<Vec<_>>(), &PackageKind::PHP, None).unwrap().clone();
    let vc = php.vc_runtime.unwrap();
    println!("PHP entry: {} via {:?}; runtime {vc} satisfied: {}", php.id, php.channel, vc.is_satisfied_by(&RegistryVcRuntimeDetector.installed().unwrap()));
    let installed = install(&php, &registry, &paths.downloads_dir()).await;
    let php_exe = installed.install_dir.join("php.exe");
    let version = run(&php_exe, &["-n", "-v"]);
    println!("{version}");
    assert!(version.contains(&format!("PHP {}", php.id.version.as_str().split('-').next().unwrap())));

    for kind in [PackageKind::XDEBUG, PackageKind::PHP_REDIS] {
        let ext = latest(&entries, &kind, Some(php83)).unwrap().clone();
        let pkg = install(&ext, &registry, &paths.downloads_dir()).await;
        let dll = std::fs::read_dir(&pkg.install_dir).unwrap().map(|e| e.unwrap().path()).find(|p| p.extension().is_some_and(|x| x == "dll")).unwrap();
        let flag = if kind == PackageKind::XDEBUG { "zend_extension" } else { "extension" };
        let modules = run(&php_exe, &["-n", "-d", &format!("{flag}={}", dll.display()), "-m"]);
        let expected = if kind == PackageKind::XDEBUG { "Xdebug" } else { "redis" };
        assert!(modules.contains(expected), "{kind} not loaded: {modules}");
        println!("{} loads in PHP {}", ext.id, php.id);
    }
    assert_eq!(registry.list().unwrap().len(), 3);
    println!("layout: {:?}", std::fs::read_dir(paths.packages_dir()).unwrap().map(|e| e.unwrap().file_name()).collect::<Vec<_>>());
}

#[tokio::test]
#[ignore = "network: downloads an older and a newer PHP 8.3 build (~70 MB)"]
async fn patch_update_replaces_the_minor_in_place() {
    let home = tempfile::tempdir().unwrap();
    let paths = LampoPaths::new(home.path().join("config"), home.path().join("data"));
    let registry = FsInstalledPackageRegistry::new(paths.clone());
    let newest = latest(&all_entries(&paths).await, &PackageKind::PHP, None).unwrap().clone();
    // An older patch of the same series, straight from the archives (hash computed on download).
    let mut older = newest.clone();
    let series = newest.id.series();
    let html = reqwest::get("https://downloads.php.net/~windows/releases/archives/").await.unwrap().text().await.unwrap();
    let file = html
        .split('"')
        .rfind(|s| s.starts_with(&format!("php-{series}.")) && s.contains("-nts-Win32-") && s.ends_with("-x64.zip") && !s.contains(newest.id.version.as_str()))
        .unwrap()
        .to_string();
    let version = file.trim_start_matches("php-").split("-nts").next().unwrap().to_string();
    older.id.version = lampo_domain::version::PackageVersion::parse(&version).unwrap();
    older.url = lampo_domain::package::DownloadUrl::new(format!("https://windows.php.net/downloads/releases/archives/{file}")).unwrap();
    let probe = paths.data_root.join("probe.zip");
    std::fs::create_dir_all(&paths.data_root).unwrap();
    let probed = HttpDownloader::new(lampo_infra_packages::http::build_client().unwrap()).download(&older.url, &probe, None, &NullProgressSink, &CancelFlag::new()).await.unwrap();
    older.sha256 = Some(probed.sha256);
    std::fs::remove_file(&probe).unwrap();

    let a = install(&older, &registry, &paths.downloads_dir()).await;
    let marker = paths.php_ini_dir(&"shop".parse::<lampo_domain::site::SiteId>().unwrap());
    std::fs::create_dir_all(&marker).unwrap();
    std::fs::write(marker.join("php.ini"), "memory_limit=512M\n").unwrap();
    let b = install(&newest, &registry, &paths.downloads_dir()).await;
    assert_eq!(a.install_dir, b.install_dir);
    let v = run(&b.install_dir.join("php.exe"), &["-n", "-v"]);
    println!("{} -> {}: {}", older.id, newest.id, v.lines().next().unwrap());
    assert!(v.contains(newest.id.version.as_str().split('-').next().unwrap()));
    assert_eq!(registry.list().unwrap().iter().map(|p| p.id.clone()).collect::<Vec<_>>(), std::slice::from_ref(&newest.id));
    assert!(marker.join("php.ini").exists(), "per-project php.ini untouched");
}

#[tokio::test]
#[ignore = "network: downloads Redis (~14 MB) and starts it on a random port"]
async fn redis_accepts_a_config_file() {
    let home = tempfile::tempdir().unwrap();
    let paths = LampoPaths::new(home.path().join("config"), home.path().join("data"));
    let registry = FsInstalledPackageRegistry::new(paths.clone());
    let entry = latest(EmbeddedCatalog::new().entries().unwrap(), &PackageKind::REDIS, None).unwrap().clone();
    let pkg = install(&entry, &registry, &paths.downloads_dir()).await;
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let redis = lampo_domain::service::ServiceId::new(lampo_domain::service::ServiceKind::REDIS, None);
    let conf_dir = paths.service_config_dir(&redis);
    let data_dir = paths.service_data_dir(&redis);
    std::fs::create_dir_all(&conf_dir).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();
    let fwd = |p: &Path| p.display().to_string().replace('\\', "/");
    let conf = conf_dir.join("redis.conf");
    std::fs::write(&conf, format!("bind 127.0.0.1\nport {port}\ndir \"{}\"\nsave \"\"\nappendonly no\n", fwd(&data_dir))).unwrap();
    // The MSYS2 runtime treats `C:\...` arguments as relative paths: pass the file name and run
    // from the config directory (a `/cygdrive/c/...` argument also works). Inside redis.conf,
    // Windows paths with forward slashes work.
    let mut server = Command::new(pkg.install_dir.join("redis-server.exe")).arg("redis.conf").current_dir(&conf_dir).stdout(std::process::Stdio::null()).spawn().unwrap();
    let cli = pkg.install_dir.join("redis-cli.exe");
    let mut pong = String::new();
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if let Ok(out) = Command::new(&cli).args(["-p", &port.to_string(), "ping"]).output() {
            pong = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if pong == "PONG" {
                break;
            }
        }
    }
    let dir = Command::new(&cli).args(["-p", &port.to_string(), "config", "get", "dir"]).output().unwrap();
    println!("config get dir: {}", String::from_utf8_lossy(&dir.stdout).replace('\n', " "));
    let saved = Command::new(&cli).args(["-p", &port.to_string(), "save"]).output().unwrap();
    println!("save: {}", String::from_utf8_lossy(&saved.stdout).trim());
    let _ = Command::new(&cli).args(["-p", &port.to_string(), "shutdown", "nosave"]).output();
    let _ = server.kill();
    let _ = server.wait();
    assert_eq!(pong, "PONG");
    assert!(data_dir.join("dump.rdb").exists(), "the `dir` from redis.conf is honoured");
}

#[tokio::test]
#[ignore = "network: requests the first byte of every catalog URL"]
async fn every_catalog_url_responds() {
    let home = tempfile::tempdir().unwrap();
    let paths = LampoPaths::new(home.path().join("config"), home.path().join("data"));
    let client = lampo_infra_packages::http::build_client().unwrap();
    let mut failures = Vec::new();
    let entries = all_entries(&paths).await;
    for e in &entries {
        assert!(e.sha256.is_some(), "{} has no checksum", e.id);
        let response = client.get(e.url.as_str()).header("Range", "bytes=0-0").send().await;
        let status = response.as_ref().map(|r| r.status().as_u16()).unwrap_or(0);
        println!("{status} {:<28} {:?} {}", e.id.to_string(), e.channel, e.url);
        if status != 206 && status != 200 {
            failures.push(format!("{} {}: {:?}", e.id, e.url, response.map(|r| r.status())));
        }
    }
    println!("{} entries checked", entries.len());
    assert!(failures.is_empty(), "{failures:#?}");
}
