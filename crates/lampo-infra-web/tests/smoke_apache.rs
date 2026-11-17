//! End-to-end smoke test against real Apache Lounge + mod_fcgid + PHP builds. Ignored by
//! default; run with
//!
//! ```powershell
//! $env:LAMPO_SMOKE_APACHE = "...\Apache24"            # contains bin\httpd.exe
//! $env:LAMPO_SMOKE_FCGID  = "...\mod_fcgid.so"
//! $env:LAMPO_SMOKE_PHP    = "...\php-8.3"             # NTS x64, php-cgi.exe + php.ini-development
//! $env:LAMPO_SMOKE_PHP_OLD = "...\php-7.4"            # optional second PHP for the worktree site
//! cargo test -p lampo-infra-web --test smoke_apache -- --ignored --nocapture
//! ```
//!
//! The process ports are implemented here with plain `tokio::process` (test stand-ins for the
//! Job Object supervisor of `lampo-infra-services`). Everything is written under a temp dir
//! whose path contains spaces; httpd listens on free high loopback ports only.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::events::DomainEvent;
use lampo_domain::hostname::Hostname;
use lampo_domain::php::{ExtensionName, PhpMinor, php_ini_overrides};
use lampo_domain::ports::{
    ChildId, CommandOutput, CommandRunner, CommandSpec, EventPublisher, PhpIniWriter, ProcessObserver, ProcessSupervisor, ServiceRunner, SpawnedChild, WebServerConfigurator,
};
use lampo_domain::project::{ApacheDirectives, ProjectEnvVars, TlsMode};
use lampo_domain::service::{LogStream, ServiceId, ServiceLaunch, ServiceState};
use lampo_domain::site::SiteId;
use lampo_domain::tcp_port::Port;
use lampo_domain::web::{ExtensionLoad, PhpIniSpec, VirtualHost, WebServerConfig};
use lampo_domain::{PortError, PortResult};
use lampo_infra_web::{ApacheConfigurator, ApacheRunner, PhpIniRenderer, RenderOptions};
use tokio::io::{AsyncBufReadExt, BufReader};

// ---------------------------------------------------------------------------------------------
// Test stand-ins for the process ports
// ---------------------------------------------------------------------------------------------

struct TokioCommands;

#[async_trait]
impl CommandRunner for TokioCommands {
    async fn run(&self, spec: CommandSpec, timeout: Duration) -> PortResult<CommandOutput> {
        let mut cmd = tokio::process::Command::new(&spec.program);
        cmd.args(&spec.args).envs(spec.env.iter().cloned()).kill_on_drop(true);
        if let Some(dir) = &spec.working_dir {
            cmd.current_dir(dir);
        }
        let out = tokio::time::timeout(timeout, cmd.output()).await.map_err(|_| PortError::Cancelled)?.map_err(|e| PortError::process("cmd", e))?;
        Ok(CommandOutput { exit_code: out.status.code(), stdout: String::from_utf8_lossy(&out.stdout).into(), stderr: String::from_utf8_lossy(&out.stderr).into() })
    }
}

#[derive(Default)]
struct TokioSupervisor {
    next: AtomicU64,
    children: Mutex<HashMap<u64, Arc<tokio::sync::Mutex<tokio::process::Child>>>>,
    exited: Arc<Mutex<Vec<u64>>>,
}

#[async_trait]
impl ProcessSupervisor for TokioSupervisor {
    async fn spawn(&self, spec: CommandSpec, observer: Arc<dyn ProcessObserver>) -> PortResult<SpawnedChild> {
        let mut cmd = tokio::process::Command::new(&spec.program);
        cmd.args(&spec.args).envs(spec.env.iter().cloned()).stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
        if let Some(dir) = &spec.working_dir {
            cmd.current_dir(dir);
        }
        let mut child = cmd.spawn().map_err(|e| PortError::process("spawn", e))?;
        let pid = child.id().unwrap_or_default();
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        for (stream, reader) in [
            (LogStream::Stdout, child.stdout.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>)),
            (LogStream::Stderr, child.stderr.take().map(|s| Box::new(s) as _)),
        ] {
            let observer = observer.clone();
            if let Some(reader) = reader {
                tokio::spawn(async move {
                    let mut lines = BufReader::new(reader).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        observer.on_output(stream, &line);
                    }
                });
            }
        }
        let child = Arc::new(tokio::sync::Mutex::new(child));
        self.children.lock().unwrap().insert(id, child.clone());
        let exited = self.exited.clone();
        tokio::spawn(async move {
            loop {
                let status = child.lock().await.try_wait();
                match status {
                    Ok(Some(s)) => {
                        exited.lock().unwrap().push(id);
                        observer.on_exit(s.code());
                        break;
                    }
                    Ok(None) => tokio::time::sleep(Duration::from_millis(100)).await,
                    Err(_) => break,
                }
            }
        });
        Ok(SpawnedChild { id: ChildId(id), pid })
    }

    async fn terminate(&self, child: ChildId, grace: Duration) -> PortResult<()> {
        let Some(c) = self.children.lock().unwrap().get(&child.0).cloned() else { return Ok(()) };
        let deadline = tokio::time::Instant::now() + grace;
        while tokio::time::Instant::now() < deadline {
            if self.exited.lock().unwrap().contains(&child.0) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        println!("!! grace period expired: killing httpd");
        let _ = c.lock().await.kill().await;
        Ok(())
    }

    fn is_alive(&self, child: ChildId) -> bool {
        self.children.lock().unwrap().contains_key(&child.0) && !self.exited.lock().unwrap().contains(&child.0)
    }

    /// Stand-in: something accepts on the port while the child lives (the ports are free high
    /// ones picked by this test, so it is httpd).
    fn owns_listener(&self, child: ChildId, port: Port) -> bool {
        self.is_alive(child) && TcpStream::connect_timeout(&([127, 0, 0, 1], port.get()).into(), Duration::from_millis(200)).is_ok()
    }
}

struct PrintEvents;
impl EventPublisher for PrintEvents {
    fn publish(&self, event: DomainEvent) {
        match event {
            DomainEvent::ServiceStatusChanged(s) => println!("[status] {:?} pid={:?} {:?}", s.state, s.pid, s.message),
            DomainEvent::ServiceLog { line, .. } => println!("[httpd] {line}"),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// Minimal HTTP/1.0 GET: (status, body).
fn http_get(port: u16, host: &str, path: &str, extra: &str) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    write!(s, "GET {path} HTTP/1.0\r\nHost: {host}\r\n{extra}Connection: close\r\n\r\n").unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).unwrap();
    let text = String::from_utf8_lossy(&buf).into_owned();
    let status = text.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    (status, body)
}

/// Files a project keeps next to its code when the document root is the project root.
const PROJECT_INTERNALS: &[&str] = &[
    ".env",
    ".env.local",
    "admin/.env",
    ".htpasswd",
    ".gitignore",
    ".git/config",
    ".git/refs/heads/main",
    ".svn/entries",
    "storage/logs/laravel.log",
    "x.sql",
    "backup/dump.sql",
    "backup/Dump.SQL",
    "index.php.bak",
];
const WELL_KNOWN: &str = ".well-known/acme-challenge/token";

/// Custom directives that re-grant access in a section Apache merges after httpd.conf's
/// denials (`AuthMerging Off`, the default, replaces them). Every request of a local site comes
/// from 127.0.0.1, so each of them grants everything: none may re-expose the project's internal
/// files. One site (dedicated port) per directive, so one cannot hide another. A nested `<If>`
/// would merge after Lampo's barrier, however it is disguised (e.g. behind a `\ ` that Apache
/// does not read as a continuation, or a `\\` that Apache also continues over an empty line):
/// `ApacheDirectives::new` refuses it, so it cannot reach a vhost and the unit tests of
/// `apache_directives::sections` cover it.
const REGRANTS: &[(&str, &str)] = &[
    ("regrant-root", "<Location />\n    Require ip 127.0.0.1\n</Location>"),
    ("regrant-admin", "<Location /admin>\n    Require ip 127.0.0.1\n</Location>"),
    ("regrant-if", "<If \"true\">\n    Require local\n</If>"),
    ("regrant-or", "<Location />\n    AuthMerging Or\n    Require local\n</Location>"),
    ("regrant-location-if", "<Location />\n    <If \"true\">\n        Require local\n    </If>\n</Location>"),
];

fn write_project_internals(docroot: &Path) {
    for file in PROJECT_INTERNALS.iter().chain([&WELL_KNOWN]) {
        let path = docroot.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "secret").unwrap();
    }
}

fn assert_dotfiles_are_refused(port: u16, host: &str) {
    let case_variants = ["/.GIT/config", "/.Env"];
    for path in PROJECT_INTERNALS.iter().map(|f| format!("/{f}")).chain(case_variants.map(String::from)) {
        let (status, body) = http_get(port, host, &path, "");
        assert_eq!(status, 403, "{host}:{port}{path} must be refused: {body}");
    }
    for dir in ["/storage/logs/", "/.git/", "/backup/"] {
        let (status, body) = http_get(port, host, dir, "");
        assert_eq!(status, 403, "{host}:{port}{dir} must not be listed: {body}");
        assert!(!body.contains("Index of"), "{body}");
    }
    let (status, body) = http_get(port, host, &format!("/{WELL_KNOWN}"), "");
    assert_eq!((status, body.as_str()), (200, "secret"), "/.well-known/ stays reachable");
}

/// php-cgi.exe processes started from `php_dir`.
fn php_cgi_count(php_dir: &Path) -> usize {
    let script = format!("@(Get-Process php-cgi -ErrorAction SilentlyContinue | Where-Object {{ $_.Path -like '{}*' }}).Count", php_dir.display().to_string().replace('/', "\\"));
    let out = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &script]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0)
}

fn httpd_count(apache: &Path) -> usize {
    let script = format!("@(Get-Process httpd -ErrorAction SilentlyContinue | Where-Object {{ $_.Path -like '{}*' }}).Count", apache.display().to_string().replace('/', "\\"));
    let out = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &script]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0)
}

struct Site {
    vhost: VirtualHost,
    ini: PhpIniSpec,
}

fn site(data: &Path, id: &str, names: &[&str], port: Option<u16>, docroot: &Path, php_dir: &Path, php: &str) -> Site {
    let site: SiteId = id.parse().unwrap();
    let key = site.file_key();
    let phprc = data.join("generated").join("php").join(&key);
    let logs = data.join("logs").join("projects").join(&key);
    let ext = |n: &str| {
        let name = ExtensionName::new(n).unwrap();
        ExtensionLoad { zend: lampo_domain::web::is_zend_extension(&name), dll: None, name }
    };
    Site {
        vhost: VirtualHost {
            site: site.clone(),
            server_names: names.iter().map(|n| Hostname::new(n).unwrap()).collect(),
            loopback_port: port.map(Port::of),
            project_root: docroot.parent().unwrap().to_path_buf(),
            document_root: docroot.to_path_buf(),
            php_cgi: php_dir.join("php-cgi.exe"),
            phprc_dir: phprc.clone(),
            // Quotes and backslashes must reach `getenv()` verbatim through `SetEnv`.
            env_vars: ProjectEnvVars::from_pairs([("APP_NAME", r#"My "Shop" C:\data\"#)]).unwrap(),
            // A representative set of legitimate custom directives (valid under the modules
            // Lampo loads): real `httpd -t` must keep accepting them after the quoted-name and
            // re-grant hardening. `php_value` is left out on purpose — Lampo runs PHP through
            // mod_fcgid, not mod_php, so it is not a directive on this server.
            custom_directives: ApacheDirectives::new(concat!(
                "Header set X-Lampo-Site \"yes\"\n",
                "RewriteEngine On\n",
                "RewriteRule ^old$ /new [R=301,L]\n",
                "SetEnvIf Request_URI \"^/api\" IS_API=1\n",
                "<Location /admin>\n    Require ip 127.0.0.1\n</Location>\n",
                "ErrorDocument 404 /404.php",
            ))
            .unwrap(),
            tls: TlsMode::Disabled,
            error_log: logs.join("error.log"),
            access_log: logs.join("access.log"),
        },
        ini: PhpIniSpec {
            site,
            php: PhpMinor::parse(php).unwrap(),
            php_dir: php_dir.to_path_buf(),
            output_dir: phprc,
            extensions: vec![ext("mbstring"), ext("opcache"), ext("curl")],
            values: php_ini_overrides([("memory_limit", "300M")]).unwrap(),
            error_log: logs.join("php_errors.log"),
        },
    }
}

const INDEX_PHP: &str = r#"<?php
header('Content-Type: text/plain');
echo "php=", PHP_VERSION, "\n";
echo "ini=", str_replace('\\', '/', (string) php_ini_loaded_file()), "\n";
echo "uri=", $_SERVER['REQUEST_URI'], "\n";
echo "mbstring=", extension_loaded('mbstring') ? 'yes' : 'no', "\n";
echo "memory_limit=", ini_get('memory_limit'), "\n";
echo "auth=", isset($_SERVER['HTTP_AUTHORIZATION']) ? $_SERVER['HTTP_AUTHORIZATION'] : '-', "\n";
echo "temp=", str_replace('\\', '/', sys_get_temp_dir()), "\n";
echo "systemroot=", getenv('SystemRoot'), "\n";
echo "app_name=", getenv('APP_NAME'), "\n";
$srv = @stream_socket_server('tcp://127.0.0.1:0', $errno, $errstr);
$cli = $srv ? @stream_socket_client('tcp://' . stream_socket_get_name($srv, false), $errno, $errstr, 2) : false;
echo "socket=", $cli ? 'ok' : "fail $errno $errstr", "\n";
session_start(); $_SESSION['x'] = 1; session_write_close();
echo "session=", file_exists(session_save_path() . '/sess_' . session_id()) ? 'ok' : 'missing', "\n";
"#;

// ---------------------------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs real Apache/mod_fcgid/PHP binaries (see module docs)"]
async fn real_apache_serves_php_sites() {
    let (Some(apache), Some(fcgid), Some(php)) = (env_path("LAMPO_SMOKE_APACHE"), env_path("LAMPO_SMOKE_FCGID"), env_path("LAMPO_SMOKE_PHP")) else {
        panic!("set LAMPO_SMOKE_APACHE, LAMPO_SMOKE_FCGID and LAMPO_SMOKE_PHP");
    };
    let php_old = env_path("LAMPO_SMOKE_PHP_OLD");
    let tmp = tempfile::Builder::new().prefix("lampo smoke ").tempdir().unwrap();
    let data = tmp.path().join("Lampo Data");

    // A Laravel-like project: front controller + .htaccess rewrite.
    let docroot = tmp.path().join("My Projects").join("shop").join("public");
    std::fs::create_dir_all(&docroot).unwrap();
    std::fs::write(docroot.join("index.php"), INDEX_PHP).unwrap();
    std::fs::write(docroot.join(".htaccess"), "RewriteEngine On\nRewriteCond %{REQUEST_FILENAME} !-f\nRewriteCond %{REQUEST_FILENAME} !-d\nRewriteRule ^ index.php [L,QSA]\n").unwrap();
    std::fs::write(docroot.join("static.txt"), "static file").unwrap();
    write_project_internals(&docroot);

    let shared = free_port();
    let wt_port = free_port();
    let shop = site(&data, "shop", &["shop.test", "shop.localhost"], None, &docroot, &php, "8.3");
    let (old_dir, old_minor) = match &php_old {
        Some(p) => (p.clone(), "7.4"),
        None => (php.clone(), "8.3"),
    };
    let wt = site(&data, "shop@feature-x", &[], Some(wt_port), &docroot, &old_dir, old_minor);

    let regrants: Vec<(u16, Site)> = REGRANTS
        .iter()
        .map(|(id, directives)| {
            let port = free_port();
            let mut s = site(&data, id, &[], Some(port), &docroot, &php, "8.3");
            s.vhost.custom_directives = ApacheDirectives::new(*directives).unwrap();
            (port, s)
        })
        .collect();

    for s in [&shop, &wt].into_iter().chain(regrants.iter().map(|(_, s)| s)) {
        PhpIniRenderer.write(&s.ini).unwrap();
    }

    let mut options = RenderOptions::from_process_environment();
    options.fcgid.idle_timeout_secs = 3;
    options.fcgid.idle_scan_interval_secs = 1;
    let configurator = ApacheConfigurator::with_options(Arc::new(TokioCommands), options);
    let shop_access_log = shop.vhost.access_log.clone();
    let mut config = WebServerConfig {
        server_root: apache.clone(),
        mod_fcgid: fcgid.clone(),
        listen_port: Port::of(shared),
        config_dir: data.join("generated").join("apache"),
        log_dir: data.join("logs").join("apache"),
        vhosts: [&shop, &wt].into_iter().chain(regrants.iter().map(|(_, s)| s)).map(|s| s.vhost.clone()).collect(),
    };

    // 1. Validation with the real `httpd -t`: a broken site is rejected, nothing changes.
    let mut broken = config.clone();
    broken.vhosts[1].custom_directives = ApacheDirectives::new("ThisIsNotADirective on").unwrap();
    match configurator.apply(&broken).await {
        Err(PortError::ConfigRejected { output }) => {
            println!("--- rejection shown to the user ---\n{output}\n---");
            assert!(output.starts_with("Site `shop@feature-x`:"), "{output}");
            assert!(!output.contains(".staging"));
        }
        other => panic!("expected ConfigRejected, got {other:?}"),
    }
    assert!(!config.config_dir.exists(), "nothing promoted");

    configurator.apply(&config).await.expect("httpd -t accepts the generated config");
    println!("--- generated vhost ---\n{}", std::fs::read_to_string(config.config_dir.join("vhosts").join("shop@feature-x.conf")).unwrap());

    // 2. Start through the runner.
    let supervisor = Arc::new(TokioSupervisor::default());
    let runner = ApacheRunner::new(supervisor.clone(), Arc::new(PrintEvents));
    let launch = ServiceLaunch {
        service: ServiceId::apache(),
        install_dir: apache.clone(),
        port: Port::of(shared),
        ui_port: None,
        config_dir: config.config_dir.clone(),
        data_dir: None,
        log_dir: config.log_dir.clone(),
    };
    runner.prepare(&launch).await.unwrap();
    runner.start(&launch).await.expect("httpd starts");
    let pid = runner.status(&ServiceId::apache()).pid;
    assert_eq!(php_cgi_count(&php), 0, "no PHP process before the first request");

    let result = std::panic::AssertUnwindSafe(async {
        // 3. Name-based site, pretty URL through .htaccess, PHP 8.3 with its own php.ini.
        let (status, body) = http_get(shared, "shop.test", "/products/42?color=red", "Authorization: Bearer abc\r\n");
        println!("--- shop.test ---\n{body}");
        assert_eq!(status, 200, "{body}");
        assert!(body.contains("php=8.3."));
        assert!(body.contains("uri=/products/42?color=red"));
        assert!(body.contains("generated/php/shop/php.ini"));
        assert!(body.contains("mbstring=yes"));
        assert!(body.contains("memory_limit=300M"));
        assert!(body.contains("auth=Bearer abc"));
        assert!(body.contains("generated/php/shop/tmp"));
        assert!(body.contains("socket=ok"), "Winsock works (SystemRoot passed)");
        assert!(body.contains("session=ok"));
        assert!(body.contains(r#"app_name=My "Shop" C:\data\"#), "SetEnv values keep quotes and backslashes");
        let (status, body) = http_get(shared, "shop.localhost", "/static.txt", "");
        assert_eq!((status, body.as_str()), (200, "static file"));

        // 4. Worktree on its dedicated loopback port, other PHP version, other php.ini.
        let (status, body) = http_get(wt_port, &format!("127.0.0.1:{wt_port}"), "/feature/x", "");
        println!("--- worktree ---\n{body}");
        assert_eq!(status, 200, "{body}");
        assert!(body.contains(&format!("php={old_minor}.")));
        assert!(body.contains("generated/php/shop@feature-x/php.ini"));

        // 5. Unknown host → catch-all, on the shared port and on a dedicated one (a page that
        //    rebinds its own domain to 127.0.0.1 must not read the site same-origin).
        let (status, body) = http_get(shared, "nobody.test", "/", "");
        assert_eq!(status, 404);
        assert!(body.contains("no site is configured"));
        let (status, body) = http_get(wt_port, &format!("evil.example:{wt_port}"), "/static.txt", "");
        assert_eq!(status, 404, "foreign Host on a dedicated port: {body}");
        assert!(body.contains("no site is configured"));
        let (status, body) = http_get(wt_port, &format!("localhost:{wt_port}"), "/static.txt", "");
        assert_eq!((status, body.as_str()), (200, "static file"), "localhost reaches the dedicated port");

        // 5b. Document root = project root: VCS metadata, secrets, logs and dumps are refused,
        //     directories are not listed, `/.well-known/` stays reachable.
        assert_dotfiles_are_refused(shared, "shop.test");
        assert_dotfiles_are_refused(wt_port, "127.0.0.1");

        // 5c. No custom directive re-grants them, whatever section it merges in.
        for (port, s) in &regrants {
            println!("re-grant attempt on port {port}: {:?}", s.vhost.custom_directives.as_str());
            assert_dotfiles_are_refused(*port, "127.0.0.1");
        }

        // 6. Idle php-cgi processes are reaped.
        assert!(php_cgi_count(&php) >= 1);
        let mut reaped = false;
        for _ in 0..30 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if php_cgi_count(&php) == 0 && php_cgi_count(&old_dir) == 0 {
                reaped = true;
                break;
            }
        }
        assert!(reaped, "idle PHP processes were not reaped");
        println!("idle php-cgi processes reaped");

        // 7. Add a site on a new port; graceful restart picks it up without a new parent.
        let extra_port = free_port();
        let mut extra = site(&data, "blog", &[], Some(extra_port), &docroot, &php, "8.3");
        extra.vhost.custom_directives = ApacheDirectives::default();
        PhpIniRenderer.write(&extra.ini).unwrap();
        config.vhosts.insert(0, extra.vhost.clone());
        configurator.apply(&config).await.unwrap();
        runner.restart(&launch).await.unwrap();
        assert_eq!(runner.status(&ServiceId::apache()).pid, pid, "graceful restart keeps the parent");
        let (status, body) = http_get(extra_port, "127.0.0.1", "/", "");
        assert_eq!(status, 200, "{body}");
        assert!(body.contains("generated/php/blog/php.ini"));
        println!("graceful restart served the new port {extra_port}");
    });
    let outcome = futures_catch(result).await;

    // 8. Graceful stop: no httpd left behind (children included).
    runner.stop(&ServiceId::apache()).await.unwrap();
    assert_eq!(runner.status(&ServiceId::apache()).state, ServiceState::Stopped);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let left = httpd_count(&apache);
    if let Ok(log) = std::fs::read_to_string(config.log_dir.join("error.log")) {
        println!("--- Apache error.log (tail) ---");
        for line in log.lines().rev().take(12).collect::<Vec<_>>().into_iter().rev() {
            println!("{line}");
        }
    }
    println!("httpd processes left after stop: {left}; php-cgi left: {}", php_cgi_count(&php));
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
    assert_eq!(left, 0);
    // 9. httpd wrote the site's log directly into the file the log reader shows.
    let access = std::fs::read_to_string(&shop_access_log).expect("the site's access log exists");
    assert!(access.contains("\"GET /products/42?color=red HTTP/1.0\" 200 "), "access log not written:\n{access}");
}

/// Runs the future, capturing a panic so httpd is always stopped.
async fn futures_catch<F: std::future::Future<Output = ()>>(f: std::panic::AssertUnwindSafe<F>) -> Result<(), Box<dyn std::any::Any + Send>> {
    // Poll in place, catching unwinds from each poll.
    let mut f = Box::pin(f.0);
    std::future::poll_fn(move |cx| match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f.as_mut().poll(cx))) {
        Ok(std::task::Poll::Ready(())) => std::task::Poll::Ready(Ok(())),
        Ok(std::task::Poll::Pending) => std::task::Poll::Pending,
        Err(e) => std::task::Poll::Ready(Err(e)),
    })
    .await
}
