//! Real Mailpit smoke test (ignored by default). Needs the extracted release zip:
//!
//! ```powershell
//! $env:LAMPO_TEST_MAILPIT_DIR = "C:\path\to\mailpit-windows-amd64"   # holds mailpit.exe
//! cargo test -p lampo-infra-services --test mailpit_smoke -- --ignored --nocapture
//! ```
//!
//! Starts Mailpit through `MailpitRunner` on two random ports with a temp data dir, sends one
//! message over raw SMTP, reads it back from the HTTP API, stops it, starts it again and checks
//! the message survived the restart (the `--database` file), then stops it.

use std::sync::Arc;
use std::time::Duration;

use lampo_domain::ports::{NullEventPublisher, ServiceRunner};
use lampo_domain::service::{ServiceId, ServiceKind, ServiceLaunch, ServiceState};
use lampo_domain::tcp_port::Port;
use lampo_infra_services::{JobObjectSupervisor, MailpitRunner};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

fn free_port() -> Port {
    Port::new(std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port()).unwrap()
}

/// One SMTP exchange: each command waits for its reply, whose code must match.
async fn send_mail(port: Port, subject: &str) {
    let stream = TcpStream::connect(("127.0.0.1", port.get())).await.unwrap();
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    let mut expect = async |code: &str| loop {
        let line = lines.next_line().await.unwrap().expect("SMTP reply");
        assert!(line.starts_with(code), "expected {code}, got {line}");
        // Multi-line replies use `250-`; the last line has a space after the code.
        if line.as_bytes().get(3) != Some(&b'-') {
            break;
        }
    };
    expect("220").await;
    for (command, code) in [("EHLO lampo.test", "250"), ("MAIL FROM:<shop@localhost>", "250"), ("RCPT TO:<customer@example.com>", "250"), ("DATA", "354")] {
        write.write_all(format!("{command}\r\n").as_bytes()).await.unwrap();
        expect(code).await;
    }
    let body = format!("From: shop@localhost\r\nTo: customer@example.com\r\nSubject: {subject}\r\n\r\nHello from the Lampo smoke test.\r\n.\r\n");
    write.write_all(body.as_bytes()).await.unwrap();
    expect("250").await;
    write.write_all(b"QUIT\r\n").await.unwrap();
}

/// `GET <path>` over plain HTTP/1.0 (no client crate needed); the body.
async fn http_get(port: Port, path: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port.get())).await.unwrap();
    s.write_all(format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes()).await.unwrap();
    let mut response = String::new();
    s.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.0 200") || response.starts_with("HTTP/1.1 200"), "{response}");
    response.split_once("\r\n\r\n").map(|(_, body)| body.to_string()).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs mailpit.exe: set LAMPO_TEST_MAILPIT_DIR"]
async fn mailpit_end_to_end() {
    let Some(install) = std::env::var_os("LAMPO_TEST_MAILPIT_DIR") else { return };
    let root = tempfile::tempdir().unwrap();
    let launch = ServiceLaunch {
        service: ServiceId::new(ServiceKind::MAILPIT, None),
        install_dir: install.into(),
        port: free_port(),
        ui_port: Some(free_port()),
        config_dir: root.path().join("generated").join("mailpit"),
        data_dir: Some(root.path().join("mailpit").join("data")),
        log_dir: root.path().join("logs").join("mailpit"),
    };
    let ui = launch.ui_port.unwrap();
    println!("SMTP {} / UI {ui}", launch.port);
    let sup = Arc::new(JobObjectSupervisor::new());
    let runner = MailpitRunner::new(sup, Arc::new(NullEventPublisher));

    runner.prepare(&launch).await.unwrap();
    runner.start(&launch).await.unwrap();
    assert_eq!(runner.status(&launch.service).state, ServiceState::Running);

    send_mail(launch.port, "Order 1001 confirmed").await;
    let mut messages = String::new();
    for _ in 0..20 {
        messages = http_get(ui, "/api/v1/messages").await;
        if messages.contains("Order 1001 confirmed") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    println!("API: {}", &messages[..messages.len().min(300)]);
    assert!(messages.contains("Order 1001 confirmed"), "{messages}");
    assert!(messages.contains("customer@example.com"), "{messages}");

    runner.stop(&launch.service).await.unwrap();
    assert_eq!(runner.status(&launch.service).state, ServiceState::Stopped);
    assert!(launch.data_dir.as_ref().unwrap().join("mailpit.db").is_file(), "the database lives in the data dir");

    // Messages survive a restart.
    runner.start(&launch).await.unwrap();
    let again = http_get(ui, "/api/v1/messages").await;
    assert!(again.contains("Order 1001 confirmed"), "{again}");
    runner.stop(&launch.service).await.unwrap();

    let log = std::fs::read_to_string(launch.log_dir.join("output.log")).unwrap();
    println!("output.log:\n{log}");
    assert!(!log.is_empty(), "Mailpit's output is captured");
}
