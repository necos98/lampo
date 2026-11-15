//! Minimal local HTTP/1.1 server for unit tests (no network access needed).

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub struct Request {
    pub path: String,
    headers: Vec<(String, String)>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone())
    }
}

pub struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    slow: bool,
    /// Omit `Content-Length`: the body ends when the connection closes.
    unannounced: bool,
    /// Close the connection after this many body bytes (a dropped connection).
    cut_after: Option<usize>,
}

impl Reply {
    pub fn ok(body: Vec<u8>) -> Self {
        Self { status: 200, headers: Vec::new(), body, slow: false, unannounced: false, cut_after: None }
    }

    pub fn text(body: &str) -> Self {
        Self::ok(body.as_bytes().to_vec())
    }

    pub fn status(status: u16) -> Self {
        Self { status, ..Self::ok(Vec::new()) }
    }

    /// Honours `Range: bytes=N-` when `If-Range` is absent or equals `etag`.
    pub fn ranged(body: Vec<u8>, range: Option<String>, etag: &str) -> Self {
        let start = range.and_then(|r| r.strip_prefix("bytes=").and_then(|r| r.strip_suffix('-')).and_then(|n| n.parse::<usize>().ok()));
        match start {
            Some(start) if start < body.len() => {
                let total = body.len();
                Self {
                    status: 206,
                    headers: vec![("Content-Range".into(), format!("bytes {start}-{}/{total}", total - 1)), ("ETag".into(), etag.into())],
                    ..Self::ok(body[start..].to_vec())
                }
            }
            _ => Self::ok(body).with_header("ETag", etag),
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Sends the body in small pieces with pauses (to test cancellation).
    pub fn slow(mut self) -> Self {
        self.slow = true;
        self
    }

    /// Sends no `Content-Length` (a body delimited by the connection closing).
    pub fn unannounced(mut self) -> Self {
        self.unannounced = true;
        self
    }

    /// Announces the whole body but drops the connection after `bytes` of it.
    pub fn cut_after(mut self, bytes: usize) -> Self {
        self.cut_after = Some(bytes);
        self
    }
}

pub struct TestServer {
    port: u16,
}

impl TestServer {
    pub async fn start(handler: impl Fn(Request) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handler = Arc::new(handler);
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                let handler = handler.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        match socket.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let text = String::from_utf8_lossy(&buf).to_string();
                    let mut lines = text.split("\r\n");
                    let path = lines.next().unwrap_or("").split(' ').nth(1).unwrap_or("/").to_string();
                    let headers = lines.take_while(|l| !l.is_empty()).filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
                    let reply = handler(Request { path, headers });
                    let mut head = format!("HTTP/1.1 {} X\r\nConnection: close\r\n", reply.status);
                    if !reply.unannounced {
                        head.push_str(&format!("Content-Length: {}\r\n", reply.body.len()));
                    }
                    for (k, v) in &reply.headers {
                        head.push_str(&format!("{k}: {v}\r\n"));
                    }
                    head.push_str("\r\n");
                    if socket.write_all(head.as_bytes()).await.is_err() {
                        return;
                    }
                    let body = &reply.body[..reply.cut_after.unwrap_or(reply.body.len()).min(reply.body.len())];
                    if reply.slow {
                        for piece in body.chunks(16 * 1024) {
                            if socket.write_all(piece).await.is_err() {
                                return;
                            }
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                    } else {
                        let _ = socket.write_all(body).await;
                    }
                    let _ = socket.shutdown().await;
                });
            }
        });
        Self { port }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
}
