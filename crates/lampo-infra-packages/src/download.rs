//! HTTP downloads with progress, cooperative cancellation, retries and resume.
//!
//! * The body is streamed to `<destination>.part`; on success it is renamed to `destination`, so
//!   `destination` only ever holds a complete download.
//! * A sidecar `<destination>.part.json` remembers the URL and validator (`ETag`/`Last-Modified`)
//!   of the partial file. A later attempt resumes with `Range` + `If-Range`, so a file that changed
//!   on the server is downloaded again from scratch instead of being spliced.
//! * Transient failures (connection errors, 408/429/5xx) are retried with exponential backoff,
//!   honouring `Retry-After`; a 403 is retried once with a generic browser-like user agent;
//!   404/410 moves on to the official fallback locations of [`crate::mirrors`].
//! * Cancellation deletes the partial file. Other failures keep it so the next attempt resumes.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::StreamExt;
use lampo_domain::package::DownloadUrl;
use lampo_domain::ports::{PackageDownloader, ProgressSink};
use lampo_domain::progress::{CancelFlag, Progress, ProgressStage};
use lampo_domain::{PortError, PortResult};
use reqwest::StatusCode;
use reqwest::header::{CONTENT_RANGE, ETAG, IF_RANGE, LAST_MODIFIED, RANGE, RETRY_AFTER, USER_AGENT};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::http::{FALLBACK_USER_AGENT, is_retryable};
use crate::mirrors::{self, UrlFallback};

/// Retry/backoff knobs (tests shorten them).
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Attempts per URL, the first one included.
    pub attempts: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { attempts: 6, initial_backoff: Duration::from_secs(1), max_backoff: Duration::from_secs(30) }
    }
}

/// Streams to `<destination>.part`, renames on completion; throttles progress (~10/s).
pub struct HttpDownloader {
    client: reqwest::Client,
    retry: RetryPolicy,
    fallbacks: Vec<Box<dyn UrlFallback>>,
}

impl HttpDownloader {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client, retry: RetryPolicy::default(), fallbacks: mirrors::default_rules() }
    }

    pub fn with_retry_policy(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// Downloads `url` (any scheme the client accepts: tests use a local `http://` server).
    pub async fn download_str(&self, url: &str, destination: &Path, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<u64> {
        let part = sibling(destination, ".part");
        let meta = sibling(destination, ".part.json");
        if let Some(dir) = destination.parent().filter(|d| !d.as_os_str().is_empty()) {
            tokio::fs::create_dir_all(dir).await.map_err(|e| PortError::io(format!("creating {}", dir.display()), e))?;
        }
        let result = self.download_any(url, destination, &part, &meta, progress, cancel).await;
        match &result {
            Ok(_) => {}
            Err(PortError::Cancelled) => {
                remove_quietly(&part).await;
                remove_quietly(&meta).await;
            }
            Err(e) => tracing::warn!(url, error = %e, "download failed; partial file kept for resume"),
        }
        result
    }

    async fn download_any(&self, url: &str, destination: &Path, part: &Path, meta: &Path, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<u64> {
        let mut candidates = vec![url.to_string()];
        for rule in &self.fallbacks {
            candidates.extend(rule.alternatives(url));
        }
        let mut last = PortError::Network(format!("{url}: no attempt made"));
        for candidate in &candidates {
            match self.download_one(candidate, part, meta, progress, cancel).await {
                Ok(bytes) => {
                    finish(part, meta, destination).await?;
                    return Ok(bytes);
                }
                Err(Attempt::NotFound(e)) => last = e,
                Err(Attempt::Fatal(e)) => return Err(e),
            }
        }
        Err(last)
    }

    /// Downloads one URL into `part`, with retries and resume. `Ok` means `part` is complete.
    async fn download_one(&self, url: &str, part: &Path, meta: &Path, progress: &dyn ProgressSink, cancel: &CancelFlag) -> Result<u64, Attempt> {
        let mut backoff = self.retry.initial_backoff;
        let mut browser_ua = false;
        let mut last_error = String::new();
        let mut attempt = 0;
        while attempt < self.retry.attempts {
            attempt += 1;
            cancel.check().map_err(Attempt::Fatal)?;
            let resume = resumable_offset(url, part, meta).await;
            let mut request = self.client.get(url);
            if browser_ua {
                request = request.header(USER_AGENT, FALLBACK_USER_AGENT);
            }
            if let Some((offset, validator)) = &resume {
                request = request.header(RANGE, format!("bytes={offset}-")).header(IF_RANGE, validator.as_str());
            }
            let response = match request.send().await {
                Ok(r) => r,
                Err(e) => {
                    last_error = format!("{url}: {e}");
                    self.pause(&mut backoff, None, cancel).await?;
                    continue;
                }
            };
            let status = response.status();
            match status {
                StatusCode::OK | StatusCode::PARTIAL_CONTENT => {}
                StatusCode::NOT_FOUND | StatusCode::GONE => return Err(Attempt::NotFound(PortError::Network(format!("{url}: HTTP {status}")))),
                StatusCode::FORBIDDEN if !browser_ua => {
                    tracing::info!(url, "HTTP 403, retrying with a browser-like user agent");
                    browser_ua = true;
                    attempt -= 1;
                    continue;
                }
                StatusCode::RANGE_NOT_SATISFIABLE => {
                    // Stale partial file: start over.
                    remove_quietly(part).await;
                    remove_quietly(meta).await;
                    last_error = format!("{url}: HTTP {status}");
                    continue;
                }
                s if is_retryable(s) => {
                    last_error = format!("{url}: HTTP {status}");
                    let retry_after = retry_after(&response);
                    self.pause(&mut backoff, retry_after, cancel).await?;
                    continue;
                }
                s => return Err(Attempt::Fatal(PortError::Network(format!("{url}: HTTP {s}")))),
            }

            // Decide whether the server honoured the range request.
            let start = match (&resume, status) {
                (Some((offset, _)), StatusCode::PARTIAL_CONTENT) if content_range_start(&response) == Some(*offset) => *offset,
                (_, StatusCode::PARTIAL_CONTENT) => {
                    // A range we did not ask for (or a mismatching one): discard and retry cleanly.
                    remove_quietly(part).await;
                    remove_quietly(meta).await;
                    last_error = format!("{url}: unexpected partial response");
                    continue;
                }
                _ => 0,
            };
            let total = response.content_length().map(|len| len + start);
            if start == 0 {
                write_meta(meta, url, &response).await;
            }
            match stream_body(response, part, start, total, url, progress, cancel).await {
                Ok(written) => {
                    if let Some(total) = total {
                        if written != total {
                            last_error = format!("{url}: received {written} of {total} bytes");
                            continue;
                        }
                    }
                    return Ok(written);
                }
                Err(StreamError::Cancelled) => return Err(Attempt::Fatal(PortError::Cancelled)),
                Err(StreamError::Io(e)) => return Err(Attempt::Fatal(e)),
                Err(StreamError::Network(e)) => {
                    // Keep what we have; the next attempt resumes from there.
                    last_error = e;
                    self.pause(&mut backoff, None, cancel).await?;
                }
            }
        }
        Err(Attempt::Fatal(PortError::Network(format!("giving up after {} attempts: {last_error}", self.retry.attempts))))
    }

    async fn pause(&self, backoff: &mut Duration, retry_after: Option<Duration>, cancel: &CancelFlag) -> Result<(), Attempt> {
        let wait = retry_after.unwrap_or(*backoff).min(self.retry.max_backoff);
        *backoff = (*backoff * 2).min(self.retry.max_backoff);
        // Sleep in small steps so cancellation stays responsive.
        let deadline = Instant::now() + wait;
        while Instant::now() < deadline {
            cancel.check().map_err(Attempt::Fatal)?;
            tokio::time::sleep(Duration::from_millis(100).min(deadline - Instant::now())).await;
        }
        cancel.check().map_err(Attempt::Fatal)
    }
}

#[async_trait]
impl PackageDownloader for HttpDownloader {
    async fn download(&self, url: &DownloadUrl, destination: &Path, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<u64> {
        self.download_str(url.as_str(), destination, progress, cancel).await
    }

    async fn discard(&self, destination: &Path) -> PortResult<()> {
        for file in [destination.to_path_buf(), sibling(destination, ".part"), sibling(destination, ".part.json")] {
            match tokio::fs::remove_file(&file).await {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(PortError::io(format!("deleting {}", file.display()), e)),
                _ => {}
            }
        }
        Ok(())
    }
}

enum Attempt {
    /// 404/410: try the next official location.
    NotFound(PortError),
    Fatal(PortError),
}

enum StreamError {
    Cancelled,
    Io(PortError),
    Network(String),
}

#[derive(Debug, Serialize, Deserialize)]
struct PartMeta {
    url: String,
    /// `ETag` (strong only) or `Last-Modified`, sent back as `If-Range`.
    validator: Option<String>,
}

async fn write_meta(meta: &Path, url: &str, response: &reqwest::Response) {
    let header = |name| response.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
    let validator = header(ETAG).filter(|e| !e.starts_with("W/")).or_else(|| header(LAST_MODIFIED));
    let json = serde_json::to_vec(&PartMeta { url: url.to_string(), validator }).unwrap_or_default();
    if let Err(e) = tokio::fs::write(meta, json).await {
        tracing::debug!(error = %e, "cannot write download metadata; resume disabled");
    }
}

/// Offset and validator to resume `part`, if it belongs to `url` and the server gave a validator.
async fn resumable_offset(url: &str, part: &Path, meta: &Path) -> Option<(u64, String)> {
    let len = tokio::fs::metadata(part).await.ok()?.len();
    let meta: PartMeta = serde_json::from_slice(&tokio::fs::read(meta).await.ok()?).ok()?;
    if len == 0 || meta.url != url {
        return None;
    }
    meta.validator.map(|v| (len, v))
}

fn content_range_start(response: &reqwest::Response) -> Option<u64> {
    // `bytes 100-999/1000`
    let value = response.headers().get(CONTENT_RANGE)?.to_str().ok()?;
    let range = value.strip_prefix("bytes ")?;
    range.split('-').next()?.trim().parse().ok()
}

fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    let secs: u64 = response.headers().get(RETRY_AFTER)?.to_str().ok()?.trim().parse().ok()?;
    Some(Duration::from_secs(secs))
}

async fn stream_body(response: reqwest::Response, part: &Path, start: u64, total: Option<u64>, url: &str, progress: &dyn ProgressSink, cancel: &CancelFlag) -> Result<u64, StreamError> {
    let io = |e: std::io::Error| StreamError::Io(PortError::io(format!("writing {}", part.display()), e));
    let mut file = if start > 0 { tokio::fs::OpenOptions::new().append(true).open(part).await.map_err(io)? } else { tokio::fs::File::create(part).await.map_err(io)? };
    let subject = file_name_of(url);
    let mut written = start;
    let mut last_report = Instant::now() - Duration::from_secs(1);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if cancel.is_cancelled() {
            return Err(StreamError::Cancelled);
        }
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                file.flush().await.map_err(io)?;
                return Err(StreamError::Network(format!("{url}: {e}")));
            }
        };
        file.write_all(&chunk).await.map_err(io)?;
        written += chunk.len() as u64;
        if last_report.elapsed() >= Duration::from_millis(100) {
            progress.report(Progress::new(ProgressStage::Downloading, written, total).with_subject(subject.clone()));
            last_report = Instant::now();
        }
    }
    file.flush().await.map_err(io)?;
    file.sync_all().await.map_err(io)?;
    progress.report(Progress::new(ProgressStage::Downloading, written, total.or(Some(written))).with_subject(subject));
    Ok(written)
}

async fn finish(part: &Path, meta: &Path, destination: &Path) -> PortResult<()> {
    if tokio::fs::try_exists(destination).await.unwrap_or(false) {
        tokio::fs::remove_file(destination).await.map_err(|e| PortError::io(format!("replacing {}", destination.display()), e))?;
    }
    tokio::fs::rename(part, destination).await.map_err(|e| PortError::io(format!("moving download to {}", destination.display()), e))?;
    remove_quietly(meta).await;
    Ok(())
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

fn file_name_of(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or("download").to_string()
}

async fn remove_quietly(path: &Path) {
    let _ = tokio::fs::remove_file(path).await;
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::test_server::{Reply, TestServer};

    fn fast() -> RetryPolicy {
        RetryPolicy { attempts: 4, initial_backoff: Duration::from_millis(10), max_backoff: Duration::from_millis(50) }
    }

    fn downloader() -> HttpDownloader {
        HttpDownloader::new(reqwest::Client::new()).with_retry_policy(fast())
    }

    #[derive(Default)]
    struct Recorder(Mutex<Vec<Progress>>);
    impl ProgressSink for Recorder {
        fn report(&self, progress: Progress) {
            self.0.lock().unwrap().push(progress);
        }
    }

    fn body(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    #[tokio::test]
    async fn downloads_and_reports_progress() {
        let data = body(300_000);
        let server = TestServer::start({
            let data = data.clone();
            move |_req| Reply::ok(data.clone())
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("not-yet-created").join("a.zip");
        let rec = Recorder::default();
        let n = downloader().download_str(&server.url("/a.zip"), &dest, &rec, &CancelFlag::new()).await.unwrap();
        assert_eq!(n, data.len() as u64);
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert!(!sibling(&dest, ".part").exists());
        let last = rec.0.lock().unwrap().last().cloned().unwrap();
        assert_eq!((last.stage, last.done, last.total), (ProgressStage::Downloading, data.len() as u64, Some(data.len() as u64)));
        assert_eq!(last.subject.as_deref(), Some("a.zip"));

        downloader().discard(&dest).await.unwrap();
        assert!(!dest.exists());
        downloader().discard(&dest).await.unwrap();
    }

    #[tokio::test]
    async fn resumes_a_partial_download_with_range() {
        let data = body(10_000);
        let ranges = std::sync::Arc::new(Mutex::new(Vec::new()));
        let server = TestServer::start({
            let data = data.clone();
            let ranges = ranges.clone();
            move |req| {
                ranges.lock().unwrap().push((req.header("range"), req.header("if-range")));
                Reply::ranged(data.clone(), req.header("range"), "\"v1\"")
            }
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("a.zip");
        let url = server.url("/a.zip");
        // Simulate an interrupted earlier attempt.
        std::fs::write(sibling(&dest, ".part"), &data[..4000]).unwrap();
        std::fs::write(sibling(&dest, ".part.json"), serde_json::to_vec(&PartMeta { url: url.clone(), validator: Some("\"v1\"".into()) }).unwrap()).unwrap();
        downloader().download_str(&url, &dest, &lampo_domain::ports::NullProgressSink, &CancelFlag::new()).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert_eq!(ranges.lock().unwrap()[0], (Some("bytes=4000-".into()), Some("\"v1\"".into())));
        assert!(!sibling(&dest, ".part.json").exists());
    }

    #[tokio::test]
    async fn restarts_when_the_server_ignores_the_range() {
        let data = body(5000);
        let server = TestServer::start({
            let data = data.clone();
            move |_req| Reply::ok(data.clone())
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("a.zip");
        let url = server.url("/a.zip");
        std::fs::write(sibling(&dest, ".part"), b"garbage-from-another-version").unwrap();
        std::fs::write(sibling(&dest, ".part.json"), serde_json::to_vec(&PartMeta { url: url.clone(), validator: Some("x".into()) }).unwrap()).unwrap();
        downloader().download_str(&url, &dest, &lampo_domain::ports::NullProgressSink, &CancelFlag::new()).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
    }

    #[tokio::test]
    async fn retries_transient_errors_then_succeeds() {
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let server = TestServer::start({
            let calls = calls.clone();
            move |_req| match calls.fetch_add(1, Ordering::SeqCst) {
                0 => Reply::status(503),
                1 => Reply::status(429).with_header("Retry-After", "0"),
                _ => Reply::ok(b"done".to_vec()),
            }
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.bin");
        downloader().download_str(&server.url("/f.bin"), &dest, &lampo_domain::ports::NullProgressSink, &CancelFlag::new()).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"done");
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retries_403_with_a_browser_user_agent() {
        let server = TestServer::start(|req| if req.header("user-agent").as_deref() == Some(FALLBACK_USER_AGENT) { Reply::ok(b"ok".to_vec()) } else { Reply::status(403) }).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.bin");
        downloader().download_str(&server.url("/f.bin"), &dest, &lampo_domain::ports::NullProgressSink, &CancelFlag::new()).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"ok");
    }

    #[tokio::test]
    async fn not_found_is_a_network_error_and_leaves_nothing() {
        let server = TestServer::start(|_req| Reply::status(404)).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.bin");
        let err = downloader().download_str(&server.url("/f.bin"), &dest, &lampo_domain::ports::NullProgressSink, &CancelFlag::new()).await.unwrap_err();
        assert!(matches!(err, PortError::Network(m) if m.contains("404")));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn gives_up_after_the_configured_attempts() {
        let server = TestServer::start(|_req| Reply::status(500)).await;
        let dir = tempfile::tempdir().unwrap();
        let err = downloader().download_str(&server.url("/f"), &dir.path().join("f"), &lampo_domain::ports::NullProgressSink, &CancelFlag::new()).await.unwrap_err();
        assert!(matches!(err, PortError::Network(m) if m.contains("4 attempts")));
    }

    #[tokio::test]
    async fn cancellation_removes_the_partial_file() {
        let server = TestServer::start(|_req| Reply::ok(body(2_000_000)).slow()).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("big.zip");
        let cancel = CancelFlag::new();
        struct CancelOnFirst(CancelFlag);
        impl ProgressSink for CancelOnFirst {
            fn report(&self, _p: Progress) {
                self.0.cancel();
            }
        }
        let err = downloader().download_str(&server.url("/big.zip"), &dest, &CancelOnFirst(cancel.clone()), &cancel).await.unwrap_err();
        assert_eq!(err, PortError::Cancelled);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "no .part or destination left behind");
    }

    #[tokio::test]
    async fn already_cancelled_does_not_touch_the_network() {
        let server = TestServer::start(|_req| panic!("must not be called")).await;
        let dir = tempfile::tempdir().unwrap();
        let cancel = CancelFlag::new();
        cancel.cancel();
        let err = downloader().download_str(&server.url("/x"), &dir.path().join("x"), &lampo_domain::ports::NullProgressSink, &cancel).await.unwrap_err();
        assert_eq!(err, PortError::Cancelled);
    }

    #[test]
    fn production_client_refuses_plain_http() {
        let client = crate::http::build_client().unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let err = rt.block_on(client.get("http://127.0.0.1:9/").send()).unwrap_err();
        assert!(err.to_string().to_lowercase().contains("http"), "{err}");
    }
}
