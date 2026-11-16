//! Real MySQL smoke test (ignored by default). Needs an extracted MySQL zip:
//!
//! ```powershell
//! $env:LAMPO_TEST_MYSQL_DIR = "C:\path\to\mysql-8.4.x-winx64"
//! cargo test -p lampo-infra-services --test mysql_smoke -- --ignored --nocapture
//! ```
//!
//! Initialises a data dir in a temp folder, starts on a random high port, measures idle memory,
//! creates/imports/exports/clones a database, stops cleanly and restarts to prove InnoDB needed
//! no recovery.

use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lampo_domain::mysql::{DatabaseName, MysqlSeries};
use lampo_domain::ports::{DatabaseAdmin, DumpImportOptions, MysqlEndpoint, NullEventPublisher, NullProgressSink, ServiceRunner};
use lampo_domain::progress::CancelFlag;
use lampo_domain::service::{ServiceLaunch, ServiceState};
use lampo_domain::tcp_port::Port;
use lampo_infra_services::{JobObjectSupervisor, MysqlCliAdmin, MysqlRunner};

fn free_port() -> Port {
    Port::new(std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port()).unwrap()
}

fn memory_of(pid: u32) -> String {
    // mysqld 8 on Windows = monitor process + server child: report both.
    let script = format!(
        "$ids = @({pid}) + @((Get-CimInstance Win32_Process -Filter 'ParentProcessId={pid}').ProcessId); \
         Get-Process -Id $ids | ForEach-Object {{ '{{0}} pid {{1}}: {{2:N1}} MB working set, {{3:N1}} MB private' -f $_.Name, $_.Id, ($_.WorkingSet64/1MB), ($_.PrivateMemorySize64/1MB) }}"
    );
    std::process::Command::new("powershell.exe").args(["-NoProfile", "-Command", &script]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a MySQL zip: set LAMPO_TEST_MYSQL_DIR"]
async fn mysql_end_to_end() {
    let Some(install) = std::env::var_os("LAMPO_TEST_MYSQL_DIR") else { return };
    let series = std::env::var("LAMPO_TEST_MYSQL_SERIES").ok().map(|s| MysqlSeries::parse(&s).unwrap()).unwrap_or(MysqlSeries::DEFAULT);
    let root = tempfile::tempdir().unwrap();
    let launch = ServiceLaunch {
        service: series.service_id(),
        install_dir: install.into(),
        port: free_port(),
        ui_port: None,
        config_dir: root.path().join("generated").join("mysql"),
        data_dir: Some(root.path().join("mysql").join("data")),
        log_dir: root.path().join("logs").join("mysql"),
    };
    let sup = Arc::new(JobObjectSupervisor::new());
    let runner = MysqlRunner::new(sup.clone(), sup.clone(), Arc::new(NullEventPublisher));

    let t = Instant::now();
    runner.prepare(&launch).await.unwrap();
    println!("initialised in {:?}", t.elapsed());
    let t = Instant::now();
    runner.start(&launch).await.unwrap();
    println!("ready in {:?}", t.elapsed());
    let status = runner.status(&launch.service);
    assert_eq!(status.state, ServiceState::Running);
    let pid = status.pid.unwrap();

    tokio::time::sleep(Duration::from_secs(15)).await;
    println!("IDLE MEMORY after start: {}", memory_of(pid));

    let client_dir = launch.install_dir.join("bin");
    let endpoint = MysqlEndpoint { series, client_dir, port: launch.port, user: "root".into() };
    let admin = MysqlCliAdmin::new(sup.clone(), sup.clone());
    let shop = DatabaseName::new("shop").unwrap();
    admin.create(&endpoint, &shop).await.unwrap();
    assert!(matches!(admin.create(&endpoint, &shop).await, Err(lampo_domain::PortError::AlreadyExists { .. })));

    let sql = root.path().join("seed.sql");
    {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&sql).unwrap());
        writeln!(f, "CREATE TABLE items (id INT PRIMARY KEY, name VARCHAR(100), note TEXT) CHARACTER SET utf8mb4;").unwrap();
        for chunk in 0..200 {
            let rows: Vec<String> = (0..100).map(|i| format!("({}, 'item {} àèì 😀', REPEAT('x', 200))", chunk * 100 + i, chunk * 100 + i)).collect();
            writeln!(f, "INSERT INTO items VALUES {};", rows.join(",")).unwrap();
        }
    }
    let t = Instant::now();
    admin.import(&endpoint, &shop, &sql, DumpImportOptions::default(), &NullProgressSink, &CancelFlag::new()).await.unwrap();
    println!("imported {} bytes in {:?}", std::fs::metadata(&sql).unwrap().len(), t.elapsed());

    let dump = root.path().join("dump.sql");
    admin.export(&endpoint, &shop, &dump, &NullProgressSink, &CancelFlag::new()).await.unwrap();
    let dump_text = std::fs::read_to_string(&dump).unwrap();
    assert!(dump_text.contains("CREATE TABLE `items`"));
    assert!(dump_text.contains("item 19999 àèì 😀"));
    println!("exported {} bytes", dump_text.len());

    let copy = DatabaseName::new("shop_copy").unwrap();
    admin.clone_database(&endpoint, &shop, &copy, &NullProgressSink, &CancelFlag::new()).await.unwrap();
    let mut names = admin.list(&endpoint).await.unwrap();
    names.sort();
    assert_eq!(names, ["shop", "shop_copy"]);
    let copy_dump = root.path().join("copy.sql");
    admin.export(&endpoint, &copy, &copy_dump, &NullProgressSink, &CancelFlag::new()).await.unwrap();
    assert!(std::fs::read_to_string(&copy_dump).unwrap().contains("item 19999 àèì 😀"));
    admin.drop_database(&endpoint, &copy).await.unwrap();
    admin.drop_database(&endpoint, &copy).await.unwrap();
    println!("IDLE MEMORY after workload: {}", memory_of(pid));

    let t = Instant::now();
    runner.stop(&launch.service).await.unwrap();
    println!("stopped in {:?}", t.elapsed());
    assert_eq!(runner.status(&launch.service).state, ServiceState::Stopped);
    let log = std::fs::read_to_string(launch.log_dir.join("error.log")).unwrap();
    assert!(log.contains("Shutdown complete"), "no clean shutdown in the error log:\n{log}");
    // Stopped through the shutdown event, not the mysqladmin fallback.
    assert!(!log.contains("Received SHUTDOWN from user"), "the shutdown event was not used:\n{log}");

    // Restart: a clean shutdown means no crash recovery.
    let mark = log.len();
    runner.prepare(&launch).await.unwrap();
    runner.start(&launch).await.unwrap();
    runner.stop(&launch.service).await.unwrap();
    let log = std::fs::read_to_string(launch.log_dir.join("error.log")).unwrap();
    let second = &log[mark..];
    assert!(!second.to_ascii_lowercase().contains("crash recovery"), "recovery after a clean stop:\n{second}");
    println!("--- error.log of the second run ---\n{second}");
}
