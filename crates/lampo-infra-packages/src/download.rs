//! HTTP downloads with progress, cooperative cancellation, retries and resume.
//!
//! * The body is streamed to `<destination>.part`; on success it is renamed to `destination`, so
//!   `destination` only ever holds a complete download.
//! * A sidecar `<destination>.part.json` remembers the URL and validator (`ETag`/`Last-Modified`)
//!   of the partial file. A later attempt resumes with `Range` + `If-Range`, so a file that changed
//!   on the server is downloaded again from scratch instead of being spliced.
//! * Transient failures (connection errors, 408/429/5xx, a stale range answered with 416, a
//!   partial response for another range, a body shorter than announced) are retried with
//!   exponential backoff, honouring `Retry-After`; a 403 is retried once with a generic
//!   browser-like user agent; 404/410 moves on to the official fallback locations of
//!   [`crate::mirrors`].
//! * Cancellation deletes the partial file. Other failures keep it so the next attempt resumes.
//! * The SHA-256 is computed while the body is streamed; a resumed download reads the bytes
//!   already on disk once, at the start of the attempt.
//! * With a size limit, a body announced (`Content-Length`) or received larger than the limit is
//!   refused and its partial file deleted: nothing is retried, since the next attempt or official
//!   location would be asked for the same oversized package.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::StreamExt;
use lampo_domain::package::{DownloadUrl, DownloadedFile};
use lampo_domain::ports::{PackageDownloader, ProgressSink};
use lampo_domain::progress::{CancelFlag, Progress, ProgressStage};
use lampo_domain::{PortError, PortResult};
use reqwest::StatusCode;
use reqwest::header::{CONTENT_RANGE, ETAG, IF_RANGE, LAST_MODIFIED, RANGE, RETRY_AFTER, USER_AGENT};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::checksum::IncrementalSha256;
use crate::http::{FALLBACK_USER_AGENT, is_retryable};
use crate::mirrors::{self, UrlFallback};

/// Appended to the destination's name for the file being downloaded.
pub(crate) const PART_SUFFIX: &str = ".part";
/// Appended to the destination's name for the resume metadata of the partial file.
pub(crate) const PART_META_SUFFIX: &str = ".part.json";

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

    /// Downloads `url` (any scheme the client accepts: tests use a local `http://` server),
    /// refusing a body larger than `max_bytes`.
    pub async fn download_str(&self, url: &str, destination: &Path, max_bytes: Option<u64>, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<DownloadedFile> {
        let part = sibling(destination, PART_SUFFIX);
        let meta = sibling(destination, PART_META_SUFFIX);
        if let Some(dir) = destination.parent().filter(|d| !d.as_os_str().is_empty()) {
            tokio::fs::create_dir_all(dir).await.map_err(|e| PortError::io(format!("creating {}", dir.display()), e))?;
        }
        let target = Target { part: &part, meta: &meta, max_bytes };
        let result = self.download_any(url, destination, &target, progress, cancel).await;
        match &result {
            Ok(_) => {}
            Err(PortError::Cancelled) => {
                remove_partial(&target).await;
            }
            Err(e) => tracing::warn!(url, error = %e, "download failed; a partial file, if any, is kept for resume"),
        }
        result
    }

    async fn download_any(&self, url: &str, destination: &Path, target: &Target<'_>, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<DownloadedFile> {
        let mut candidates = vec![url.to_string()];
        for rule in &self.fallbacks {
            candidates.extend(rule.alternatives(url));
        }
        let mut last = PortError::Network(format!("{url}: no attempt made"));
        for candidate in &candidates {
            match self.download_one(candidate, target, progress, cancel).await {
                Ok(downloaded) => {
                    finish(target.part, target.meta, destination).await?;
                    return Ok(downloaded);
                }
                Err(Attempt::NotFound(e)) => last = e,
                Err(Attempt::Fatal(e)) => return Err(e),
            }
        }
        Err(last)
    }

    /// Downloads one URL into `part`, with retries and resume. `Ok` means `part` is complete.
    async fn download_one(&self, url: &str, target: &Target<'_>, progress: &dyn ProgressSink, cancel: &CancelFlag) -> Result<DownloadedFile, Attempt> {
        let (part, meta) = (target.part, target.meta);
        let mut backoff = self.retry.initial_backoff;
        let mut browser_ua = false;
        let mut last_error = String::new();
        // Hash of the bytes in `part`, carried from one attempt to the next.
        let mut hashed: Option<IncrementalSha256> = None;
        let mut attempt = 0;
        while attempt < self.retry.attempts {
            attempt += 1;
            cancel.check().map_err(Attempt::Fatal)?;
            let (resume, mut digest) = resume_state(url, part, meta, hashed.take()).await;
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
                    hashed = Some(digest);
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
                    hashed = Some(digest);
                    attempt -= 1;
                    continue;
                }
                StatusCode::RANGE_NOT_SATISFIABLE => {
                    // Stale partial file: start over.
                    remove_partial(target).await;
                    last_error = format!("{url}: HTTP {status}");
                    self.pause(&mut backoff, None, cancel).await?;
                    continue;
                }
                s if is_retryable(s) => {
                    last_error = format!("{url}: HTTP {status}");
                    hashed = Some(digest);
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
                    remove_partial(target).await;
                    last_error = format!("{url}: unexpected partial response");
                    self.pause(&mut backoff, None, cancel).await?;
                    continue;
                }
                _ => 0,
            };
            if start == 0 {
                digest = IncrementalSha256::default();
            }
            let total = response.content_length().map(|len| len + start);
            if let Some(err) = total.and_then(|total| oversize(url, total, target.max_bytes)) {
                remove_partial(target).await;
                return Err(Attempt::Fatal(err));
            }
            if start == 0 {
                write_meta(meta, url, &response).await;
            }
            let body = Body { url, part, total, max_bytes: target.max_bytes };
            match stream_body(response, &body, &mut digest, progress, cancel).await {
                Ok(written) => {
                    if let Some(total) = total {
                        if written != total {
                            last_error = format!("{url}: received {written} of {total} bytes");
                            hashed = Some(digest);
                            self.pause(&mut backoff, None, cancel).await?;
                            continue;
                        }
                    }
                    return Ok(DownloadedFile { bytes: written, sha256: digest.finish() });
                }
                Err(StreamError::Cancelled) => return Err(Attempt::Fatal(PortError::Cancelled)),
                Err(StreamError::Io(e)) => return Err(Attempt::Fatal(e)),
                Err(StreamError::TooLarge(e)) => {
                    remove_partial(target).await;
                    return Err(Attempt::Fatal(e));
                }
                Err(StreamError::Network(e)) => {
                    // Keep what we have; the next attempt resumes from there.
                    last_error = e;
                    hashed = Some(digest);
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
    async fn download(&self, url: &DownloadUrl, destination: &Path, max_bytes: Option<u64>, progress: &dyn ProgressSink, cancel: &CancelFlag) -> PortResult<DownloadedFile> {
        self.download_str(url.as_str(), destination, max_bytes, progress, cancel).await
    }

    async fn discard(&self, destination: &Path) -> PortResult<()> {
        for file in [destination.to_path_buf(), sibling(destination, PART_SUFFIX), sibling(destination, PART_META_SUFFIX)] {
            match tokio::fs::remove_file(&file).await {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(PortError::io(format!("deleting {}", file.display()), e)),
                _ => {}
            }
        }
        Ok(())
    }
}

/// The files of one download and its size limit.
struct Target<'a> {
    part: &'a Path,
    meta: &'a Path,
    max_bytes: Option<u64>,
}

/// One response body being streamed into `part`.
#[derive(Clone, Copy)]
struct Body<'a> {
    url: &'a str,
    part: &'a Path,
    /// Expected size of the whole file (resumed bytes included), when the server announced it.
    total: Option<u64>,
    max_bytes: Option<u64>,
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
    /// More bytes than the size limit.
    TooLarge(PortError),
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

/// What to resume `part` from and the hash of the bytes it already holds: the previous
/// attempt's hash when it covers exactly those bytes, else they are read once. A partial file
/// that cannot be read back is deleted and the download starts over.
async fn resume_state(url: &str, part: &Path, meta: &Path, previous: Option<IncrementalSha256>) -> (Option<(u64, String)>, IncrementalSha256) {
    let Some((offset, validator)) = resumable_offset(url, part, meta).await else { return (None, IncrementalSha256::default()) };
    if let Some(state) = previous.filter(|state| state.len() == offset) {
        return (Some((offset, validator)), state);
    }
    match IncrementalSha256::of_prefix(part, offset).await {
        Ok(state) => (Some((offset, validator)), state),
        Err(e) => {
            tracing::debug!(error = %e, "cannot hash the partial download; starting over");
            remove_quietly(part).await;
            remove_quietly(meta).await;
            (None, IncrementalSha256::default())
        }
    }
}

/// The refusal of a body of `size` bytes, if it exceeds `max_bytes`.
fn oversize(url: &str, size: u64, max_bytes: Option<u64>) -> Option<PortError> {
    let max = max_bytes.filter(|max| size > *max)?;
    Some(PortError::invalid_data(url, format!("the download is larger than expected ({size} bytes, at most {max} allowed for this package)")))
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

/// Appends the body to `part` (whose first `digest.len()` bytes `digest` already covers),
/// hashing each chunk as it is written. Returns the size of the whole file.
async fn stream_body(response: reqwest::Response, body: &Body<'_>, digest: &mut IncrementalSha256, progress: &dyn ProgressSink, cancel: &CancelFlag) -> Result<u64, StreamError> {
    let Body { url, part, total, max_bytes } = *body;
    let io = |e: std::io::Error| StreamError::Io(PortError::io(format!("writing {}", part.display()), e));
    let start = digest.len();
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
        if let Some(err) = oversize(url, written + chunk.len() as u64, max_bytes) {
            return Err(StreamError::TooLarge(err));
        }
        file.write_all(&chunk).await.map_err(io)?;
        digest.update(&chunk);
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

/// Deletes the partial file and its metadata, so the next attempt starts from scratch.
async fn remove_partial(target: &Target<'_>) {
    remove_quietly(target.part).await;
    remove_quietly(target.meta).await;
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

    fn sha256_of(data: &[u8]) -> lampo_domain::package::Sha256Digest {
        use sha2::Digest;
        lampo_domain::package::Sha256Digest::from_bytes(sha2::Sha256::digest(data).into())
    }

    #[tokio::test]
    async fn the_hash_of_a_resumed_download_covers_the_bytes_already_on_disk() {
        let data = body(10_000);
        let server = TestServer::start({
            let data = data.clone();
            move |req| Reply::ranged(data.clone(), req.header("range"), "\"v1\"")
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("a.zip");
        let url = server.url("/a.zip");
        std::fs::write(sibling(&dest, PART_SUFFIX), &data[..4000]).unwrap();
        std::fs::write(sibling(&dest, PART_META_SUFFIX), serde_json::to_vec(&PartMeta { url: url.clone(), validator: Some("\"v1\"".into()) }).unwrap()).unwrap();
        let downloaded = downloader().download_str(&url, &dest, None, &lampo_domain::ports::NullProgressSink, &CancelFlag::new()).await.unwrap();
        assert_eq!(downloaded.sha256, sha256_of(&data));
    }

    #[tokio::test]
    async fn the_hash_stays_right_when_a_dropped_connection_is_resumed() {
        let data = body(50_000);
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let server = TestServer::start({
            let data = data.clone();
            let calls = calls.clone();
            move |req| match calls.fetch_add(1, Ordering::SeqCst) {
                0 => Reply::ranged(data.clone(), None, "\"v1\"").cut_after(20_000),
                _ => Reply::ranged(data.clone(), req.header("range"), "\"v1\""),
            }
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("a.zip");
        let downloaded = downloader().download_str(&server.url("/a.zip"), &dest, Some(50_000), &lampo_domain::ports::NullProgressSink, &CancelFlag::new()).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2, "the second request resumed the first");
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert_eq!(downloaded, DownloadedFile { bytes: data.len() as u64, sha256: sha256_of(&data) });
    }

    #[tokio::test]
    async fn an_announced_size_over_the_limit_is_refused_before_writing() {
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let server = TestServer::start({
            let calls = calls.clone();
            move |_req| {
                calls.fetch_add(1, Ordering::SeqCst);
                Reply::ok(body(2000))
            }
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("a.zip");
        let err = downloader().download_str(&server.url("/a.zip"), &dest, Some(1999), &lampo_domain::ports::NullProgressSink, &CancelFlag::new()).await.unwrap_err();
        assert!(matches!(&err, PortError::InvalidData { message, .. } if message.contains("2000 bytes")), "{err:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "not retried");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "nothing written");
        downloader().download_str(&server.url("/a.zip"), &dest, Some(2000), &lampo_domain::ports::NullProgressSink, &CancelFlag::new()).await.unwrap();
    }

    #[tokio::test]
    async fn an_unannounced_body_over_the_limit_is_cut_off() {
        let server = TestServer::start(|_req| Reply::ok(body(300_000)).unannounced()).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("a.zip");
        let err = downloader().download_str(&server.url("/a.zip"), &dest, Some(100_000), &lampo_domain::ports::NullProgressSink, &CancelFlag::new()).await.unwrap_err();
        assert!(matches!(err, PortError::InvalidData { .. }), "{err:?}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "the partial file is deleted, not kept for resume");
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
        let downloaded = downloader().download_str(&server.url("/a.zip"), &dest, None, &rec, &CancelFlag::new()).await.unwrap();
        assert_eq!(downloaded, DownloadedFile { bytes: data.len() as u64, sha256: sha256_of(&data) }, "hashed while streamed");
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert!(!sibling(&dest, PART_SUFFIX).exists());
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
        std::fs::write(sibling(&dest, PART_SUFFIX), &data[..4000]).unwrap();
        std::fs::write(sibling(&dest, PART_META_SUFFIX), serde_json::to_vec(&PartMeta { url: url.clone(), validator: Some("\"v1\"".into()) }).unwrap()).unwrap();
        downloader().download_str(&url, &dest, None, &lampo_domain::ports::NullProgressSink, &CancelFlag::new()).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert_eq!(ranges.lock().unwrap()[0], (Some("bytes=4000-".into()), Some("\"v1\"".into())));
        assert!(!sibling(&dest, PART_META_SUFFIX).exists());
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
        std::fs::write(sibling(&dest, PART_SUFFIX), b"garbage-from-another-version").unwrap();
        std::fs::write(sibling(&dest, PART_META_SUFFIX), serde_json::to_vec(&PartMeta { url: url.clone(), validator: Some("x".into()) }).unwrap()).unwrap();
        downloader().download_str(&url, &dest, None, &lampo_domain::ports::NullProgressSink, &CancelFlag::new()).await.unwrap();
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
        downloader().download_str(&server.url("/f.bin"), &dest, None, &lampo_domain::ports::NullProgressSink, &CancelFlag::new()).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"done");
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retries_403_with_a_browser_user_agent() {
        let server = TestServer::start(|req| if req.header("user-agent").as_deref() == Some(FALLBACK_USER_AGENT) { Reply::ok(b"ok".to_vec()) } else { Reply::status(403) }).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.bin");
        downloader().download_str(&server.url("/f.bin"), &dest, None, &lampo_domain::ports::NullProgressSink, &CancelFlag::new()).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"ok");
    }

    #[tokio::test]
    async fn not_found_is_a_network_error_and_leaves_nothing() {
        let server = TestServer::start(|_req| Reply::status(404)).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.bin");
        let err = downloader().download_str(&server.url("/f.bin"), &dest, None, &lampo_domain::ports::NullProgressSink, &CancelFlag::new()).await.unwrap_err();
        assert!(matches!(err, PortError::Network(m) if m.contains("404")));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    /// A 416 (stale range) and a 206 for a range never asked for are retried like any transient
    /// failure: after a growing pause, never in a tight loop against the server.
    #[tokio::test]
    async fn range_failures_back_off_before_retrying() {
        for status in [416, 206] {
            let calls = std::sync::Arc::new(AtomicUsize::new(0));
            let server = TestServer::start({
                let calls = calls.clone();
                move |_req| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Reply::status(status)
                }
            })
            .await;
            let dir = tempfile::tempdir().unwrap();
            let policy = RetryPolicy { attempts: 3, initial_backoff: Duration::from_millis(100), max_backoff: Duration::from_millis(400) };
            let started = Instant::now();
            let err = HttpDownloader::new(reqwest::Client::new())
                .with_retry_policy(policy)
                .download_str(&server.url("/f"), &dir.path().join("f"), None, &lampo_domain::ports::NullProgressSink, &CancelFlag::new())
                .await
                .unwrap_err();
            assert!(matches!(err, PortError::Network(ref m) if m.contains("3 attempts")), "{status}: {err:?}");
            assert_eq!(calls.load(Ordering::SeqCst), 3, "{status}");
            assert!(started.elapsed() >= Duration::from_millis(100 + 200), "{status}: waited only {:?}", started.elapsed());
        }
    }

    #[tokio::test]
    async fn gives_up_after_the_configured_attempts() {
        let server = TestServer::start(|_req| Reply::status(500)).await;
        let dir = tempfile::tempdir().unwrap();
        let err = downloader().download_str(&server.url("/f"), &dir.path().join("f"), None, &lampo_domain::ports::NullProgressSink, &CancelFlag::new()).await.unwrap_err();
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
        let err = downloader().download_str(&server.url("/big.zip"), &dest, None, &CancelOnFirst(cancel.clone()), &cancel).await.unwrap_err();
        assert_eq!(err, PortError::Cancelled);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "no .part or destination left behind");
    }

    #[tokio::test]
    async fn already_cancelled_does_not_touch_the_network() {
        let server = TestServer::start(|_req| panic!("must not be called")).await;
        let dir = tempfile::tempdir().unwrap();
        let cancel = CancelFlag::new();
        cancel.cancel();
        let err = downloader().download_str(&server.url("/x"), &dir.path().join("x"), None, &lampo_domain::ports::NullProgressSink, &cancel).await.unwrap_err();
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
