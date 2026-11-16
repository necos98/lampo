//! Shared HTTP client configuration.
//!
//! Site quirks (checked November 2026), which shape the defaults below:
//! * every package host used by the catalogs accepts this crate's plain `Lampo/<version>` user
//!   agent and supports `Range` requests;
//! * `dev.mysql.com` (Akamai) answers 403 to unknown *and* to browser-like user agents coming
//!   from non-browser TLS stacks, so catalog entries point at `cdn.mysql.com` instead;
//! * `windows.php.net` redirects (302) to `downloads.php.net/~windows`: redirects must be followed;
//! * DNS resolution and connections occasionally fail transiently: the downloader retries.
//!
//! No personal data is ever sent: the user agent is a fixed product token.

use std::time::{Duration, Instant};

use futures_util::StreamExt;
use lampo_domain::progress::CancelFlag;
use lampo_domain::{PortError, PortResult};

/// Generic user agent (no personal data).
pub const USER_AGENT: &str = concat!("Lampo/", env!("CARGO_PKG_VERSION"), " (Windows; package downloader)");

/// Generic browser-like user agent, used only to retry a request a server refused with 403.
/// Contains no personal or machine-specific data.
pub const FALLBACK_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// Largest body [`get_text`] accepts: catalog pages and checksum files are a few kilobytes.
pub(crate) const MAX_TEXT_BYTES: usize = 5 * 1024 * 1024;

/// Pause before retry number `n` (1, 2, …) of [`get_text`]: `BASE_BACKOFF * 2^n` (1 s, 2 s, …).
const BASE_BACKOFF: Duration = Duration::from_millis(500);
/// Granularity of [`sleep_cancellable`]: how quickly a cancellation interrupts a pause.
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// One client for the whole process (connection pooling). rustls, https only, redirects followed.
pub fn build_client() -> PortResult<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        // Per read, not per request: large downloads may take minutes but must not stall forever.
        .read_timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| PortError::Network(e.to_string()))
}

/// How hard [`get_text`] tries before giving up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FetchPolicy {
    /// Attempts, the first one included.
    pub attempts: u32,
    /// Whole-request timeout of each attempt.
    pub timeout: Duration,
}

impl FetchPolicy {
    /// Catalog lists: one quick attempt; the cache answers with what it has meanwhile.
    pub(crate) const LIST: Self = Self { attempts: 1, timeout: Duration::from_secs(5) };
    /// Checksum files: small, and without one a release is skipped, so worth a couple of retries.
    pub(crate) const CHECKSUM: Self = Self { attempts: 3, timeout: Duration::from_secs(10) };
    /// Tests against the local server: one attempt.
    #[cfg(test)]
    pub(crate) const ONCE: Self = Self { attempts: 1, timeout: Duration::from_secs(5) };
}

/// GET `url` and return the body as text (at most [`MAX_TEXT_BYTES`]), retrying transient
/// failures as `policy` says; the last of several attempts uses the fallback user agent. Once
/// `cancel` is set no new attempt starts and a pending pause ends with `Cancelled`. Used by the
/// live catalog sources.
pub(crate) async fn get_text(client: &reqwest::Client, url: &str, policy: FetchPolicy, cancel: &CancelFlag) -> PortResult<String> {
    let attempts = policy.attempts.max(1);
    let mut last_error = String::new();
    for attempt in 0..attempts {
        if attempt > 0 {
            sleep_cancellable(BASE_BACKOFF * 2u32.pow(attempt), cancel).await?;
        }
        cancel.check()?;
        let mut request = client.get(url).timeout(policy.timeout);
        if attempt > 0 && attempt + 1 == attempts {
            request = request.header(reqwest::header::USER_AGENT, FALLBACK_USER_AGENT);
        }
        match request.send().await {
            Ok(response) if response.status().is_success() => return read_text(url, response).await,
            Ok(response) if is_retryable(response.status()) || response.status() == reqwest::StatusCode::FORBIDDEN => {
                last_error = format!("{url}: HTTP {}", response.status());
            }
            Ok(response) => return Err(PortError::Network(format!("{url}: HTTP {}", response.status()))),
            Err(e) => last_error = format!("{url}: {e}"),
        }
    }
    Err(PortError::Network(last_error))
}

/// The body of `response` as UTF-8, refused (`InvalidData`) past [`MAX_TEXT_BYTES`], whether the
/// size is announced or not.
async fn read_text(url: &str, response: reqwest::Response) -> PortResult<String> {
    let too_large = || PortError::invalid_data(url, format!("response larger than {MAX_TEXT_BYTES} bytes"));
    if response.content_length().is_some_and(|len| len > MAX_TEXT_BYTES as u64) {
        return Err(too_large());
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| PortError::Network(format!("{url}: {e}")))?;
        if body.len() + chunk.len() > MAX_TEXT_BYTES {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|e| PortError::invalid_data(url, e))
}

/// Sleeps for `duration`, ending early with `Cancelled` (within [`CANCEL_POLL`]) once `cancel`
/// is set.
pub(crate) async fn sleep_cancellable(duration: Duration, cancel: &CancelFlag) -> PortResult<()> {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        cancel.check()?;
        tokio::time::sleep(CANCEL_POLL.min(deadline.saturating_duration_since(Instant::now()))).await;
    }
    cancel.check()
}

/// Statuses worth retrying after a pause.
pub(crate) fn is_retryable(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS || status == reqwest::StatusCode::REQUEST_TIMEOUT || status.is_server_error()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{Reply, TestServer};

    #[tokio::test]
    async fn reads_a_text_body() {
        let server = TestServer::start(|_| Reply::text("hello")).await;
        let text = get_text(&reqwest::Client::new(), &server.url("/a"), FetchPolicy::ONCE, &CancelFlag::new()).await.unwrap();
        assert_eq!(text, "hello");
    }

    #[tokio::test]
    async fn refuses_bodies_over_the_limit_announced_or_not() {
        let big = vec![b'a'; MAX_TEXT_BYTES + 1];
        let announced = big.clone();
        let server = TestServer::start(move |req| if req.path == "/announced" { Reply::ok(announced.clone()) } else { Reply::ok(big.clone()).unannounced() }).await;
        let client = reqwest::Client::new();
        for path in ["/announced", "/unannounced"] {
            let err = get_text(&client, &server.url(path), FetchPolicy::ONCE, &CancelFlag::new()).await.unwrap_err();
            assert!(matches!(err, PortError::InvalidData { .. }), "{path}: {err}");
        }
    }

    #[tokio::test]
    async fn cancellation_interrupts_the_backoff() {
        let server = TestServer::start(|_| Reply::status(503)).await;
        let cancel = CancelFlag::new();
        let canceller = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            canceller.cancel();
        });
        let started = Instant::now();
        let policy = FetchPolicy { attempts: 4, timeout: Duration::from_secs(5) };
        let err = get_text(&reqwest::Client::new(), &server.url("/busy"), policy, &cancel).await.unwrap_err();
        assert!(matches!(err, PortError::Cancelled), "{err}");
        assert!(started.elapsed() < Duration::from_secs(1), "the 1 s pause was cut short: {:?}", started.elapsed());
    }
}
