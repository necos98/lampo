//! Named-pipe control channel: concurrency, streaming, disconnect → cancel, errors.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use lampo_domain::PortError;
use lampo_domain::ports::{ControlChannelClient, ControlChannelServer, ControlFrameSink, ControlRequestHandler};
use lampo_domain::progress::CancelFlag;
use lampo_infra_services::control_pipe::test_support::{create_lampo_server_pipe, create_server_pipe_with_sddl};
use lampo_infra_services::{NamedPipeControlClient, NamedPipeControlServer};
use tokio::io::AsyncWriteExt;
use tokio::task::JoinHandle;

fn unique_pipe() -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    format!(r"\\.\pipe\lampo-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
}

/// Request: `{"id":N,"events":K,"gap_ms":G,"final_delay_ms":D}` → K event frames then a final
/// frame echoing the id. `{"until_cancel":true}` streams until cancelled.
#[derive(Default)]
struct TestHandler {
    cancelled: AtomicUsize,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
}

#[async_trait]
impl ControlRequestHandler for TestHandler {
    async fn handle(&self, request_json: String, events: &dyn ControlFrameSink, cancel: &CancelFlag) -> String {
        let req: serde_json::Value = serde_json::from_str(&request_json).unwrap();
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_in_flight.fetch_max(now, Ordering::SeqCst);
        if req["until_cancel"].as_bool() == Some(true) {
            let mut i = 0;
            while !cancel.is_cancelled() {
                events.frame(format!("{{\"tick\":{i}}}"));
                i += 1;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            self.cancelled.fetch_add(1, Ordering::SeqCst);
        } else {
            let gap = Duration::from_millis(req["gap_ms"].as_u64().unwrap_or(0));
            for i in 0..req["events"].as_u64().unwrap_or(0) {
                // A pretty-printed frame: embedded newlines must not break the framing.
                events.frame(format!("{{\n  \"id\": {},\n  \"event\": {i}\n}}", req["id"]));
                tokio::time::sleep(gap).await;
            }
            tokio::time::sleep(Duration::from_millis(req["final_delay_ms"].as_u64().unwrap_or(0))).await;
        }
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        format!("{{\"done\":{}}}", req["id"])
    }
}

#[derive(Default)]
struct Collect(Mutex<Vec<(Instant, String)>>);

impl ControlFrameSink for Collect {
    fn frame(&self, json: String) {
        self.0.lock().unwrap().push((Instant::now(), json));
    }
}

async fn start_server(name: &str, handler: Arc<TestHandler>) -> JoinHandle<()> {
    let server = NamedPipeControlServer::new(name.to_string());
    let task = tokio::spawn(async move {
        let _ = server.serve(handler).await;
    });
    let client = NamedPipeControlClient::new(name.to_string());
    for _ in 0..100 {
        if client.is_server_running() {
            return task;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("server did not start");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serves_many_concurrent_clients() {
    let name = unique_pipe();
    let handler = Arc::new(TestHandler::default());
    let server = start_server(&name, handler.clone()).await;
    let mut clients = Vec::new();
    for id in 0..32u32 {
        let name = name.clone();
        clients.push(tokio::spawn(async move {
            let client = NamedPipeControlClient::new(name);
            let sink = Collect::default();
            let reply = client.exchange(format!(r#"{{"id":{id},"events":3,"gap_ms":30}}"#), &sink, Duration::from_secs(20)).await.unwrap();
            (id, reply, sink.0.into_inner().unwrap())
        }));
    }
    for c in clients {
        let (id, reply, events) = c.await.unwrap();
        assert_eq!(reply, format!(r#"{{"done":{id}}}"#));
        assert_eq!(events.len(), 3);
        for (i, (_, e)) in events.iter().enumerate() {
            let v: serde_json::Value = serde_json::from_str(e).unwrap();
            assert_eq!((v["id"].as_u64().unwrap(), v["event"].as_u64().unwrap()), (id as u64, i as u64));
        }
    }
    assert!(handler.max_in_flight.load(Ordering::SeqCst) > 1, "requests were not served concurrently");
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn streams_events_before_the_final_frame() {
    let name = unique_pipe();
    let server = start_server(&name, Arc::new(TestHandler::default())).await;
    let client = NamedPipeControlClient::new(name);
    let sink = Collect::default();
    let sent = Instant::now();
    let reply = client.exchange(r#"{"id":1,"events":1,"final_delay_ms":600}"#.into(), &sink, Duration::from_secs(10)).await.unwrap();
    let replied = Instant::now();
    assert_eq!(reply, r#"{"done":1}"#);
    let events = sink.0.into_inner().unwrap();
    assert_eq!(events.len(), 1);
    assert!(events[0].0 - sent < Duration::from_millis(300), "event was not delivered as it was produced");
    assert!(replied - events[0].0 >= Duration::from_millis(400));
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn client_disconnect_cancels_the_handler() {
    let name = unique_pipe();
    let handler = Arc::new(TestHandler::default());
    let server = start_server(&name, handler.clone()).await;
    let client = NamedPipeControlClient::new(name);
    let sink = Collect::default();
    // The exchange times out client-side (Ctrl-C equivalent: the connection is dropped).
    let err = client.exchange(r#"{"until_cancel":true}"#.into(), &sink, Duration::from_millis(300)).await.unwrap_err();
    assert!(matches!(err, PortError::Io { .. }), "{err}");
    assert!(!sink.0.lock().unwrap().is_empty(), "stream frames arrived before the disconnect");
    let deadline = Instant::now() + Duration::from_secs(3);
    while handler.cancelled.load(Ordering::SeqCst) == 0 {
        assert!(Instant::now() < deadline, "handler was not cancelled after the client went away");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn no_server_means_app_not_found() {
    let client = NamedPipeControlClient::new(unique_pipe());
    assert!(!client.is_server_running());
    let err = client.exchange("{}".into(), &Collect::default(), Duration::from_secs(2)).await.unwrap_err();
    assert!(matches!(err, PortError::NotFound { entity: "app", .. }), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_server_on_the_same_pipe_is_refused() {
    let name = unique_pipe();
    let server = start_server(&name, Arc::new(TestHandler::default())).await;
    let second = NamedPipeControlServer::new(name.clone());
    let err = tokio::time::timeout(Duration::from_secs(2), second.serve(Arc::new(TestHandler::default()))).await.expect("refused immediately").unwrap_err();
    assert!(matches!(err, PortError::AlreadyExists { .. }), "{err}");
    // The first server is unaffected.
    let reply = NamedPipeControlClient::new(name).exchange(r#"{"id":5}"#.into(), &Collect::default(), Duration::from_secs(5)).await.unwrap();
    assert_eq!(reply, r#"{"done":5}"#);
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn foreign_framing_is_handed_to_the_application_as_the_final_frame() {
    let name = unique_pipe();
    // Same security as the real server: default security would make the pipe owned by
    // BUILTIN\Administrators when the test runs elevated, and the client would refuse it.
    let mut raw = create_lampo_server_pipe(&name).unwrap();
    let server = tokio::spawn(async move {
        raw.connect().await.unwrap();
        raw.write_all(b"{\"legacy\":true}\n").await.unwrap();
        raw.flush().await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
    });
    let reply = NamedPipeControlClient::new(name).exchange("{}".into(), &Collect::default(), Duration::from_secs(5)).await.unwrap();
    assert_eq!(reply, r#"{"legacy":true}"#);
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pipe_owned_by_someone_else_is_refused() {
    const ERROR_INVALID_OWNER: i32 = 1307;
    let name = unique_pipe();
    let raw = match create_server_pipe_with_sddl(&name, "O:BAD:P(A;;GA;;;BU)") {
        Ok(raw) => raw,
        // Not elevated: the token cannot make Administrators the owner, so nobody without admin
        // rights can plant such a pipe either. The client-side check is covered by `win` unit tests.
        Err(e) if e.raw_os_error() == Some(ERROR_INVALID_OWNER) => return,
        Err(e) => panic!("creating the pipe: {e}"),
    };
    let server = tokio::spawn(async move {
        let _ = raw.connect().await;
        tokio::time::sleep(Duration::from_millis(500)).await;
    });
    let err = NamedPipeControlClient::new(name).exchange("{}".into(), &Collect::default(), Duration::from_secs(5)).await.unwrap_err();
    assert!(err.to_string().contains("not owned by the current user"), "{err}");
    server.abort();
}
