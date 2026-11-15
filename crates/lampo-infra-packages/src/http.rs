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

use std::time::Duration;

use lampo_domain::{PortError, PortResult};

/// Generic user agent (no personal data).
pub const USER_AGENT: &str = concat!("Lampo/", env!("CARGO_PKG_VERSION"), " (Windows; package downloader)");

/// Generic browser-like user agent, used only to retry a request a server refused with 403.
/// Contains no personal or machine-specific data.
pub const FALLBACK_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

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

/// GET `url` and return the body as text, with the downloader's retry policy for transient
/// failures; the last attempt uses the fallback user agent. Used by the live catalog sources.
pub(crate) async fn get_text(client: &reqwest::Client, url: &str, attempts: u32) -> PortResult<String> {
    let mut last_error = String::new();
    for attempt in 0..attempts.max(1) {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(500 * 2u64.pow(attempt))).await;
        }
        let mut request = client.get(url).timeout(Duration::from_secs(30));
        if attempt > 0 && attempt + 1 == attempts {
            request = request.header(reqwest::header::USER_AGENT, FALLBACK_USER_AGENT);
        }
        match request.send().await {
            Ok(response) if response.status().is_success() => {
                return response.text().await.map_err(|e| PortError::Network(format!("{url}: {e}")));
            }
            Ok(response) if is_retryable(response.status()) || response.status() == reqwest::StatusCode::FORBIDDEN => {
                last_error = format!("{url}: HTTP {}", response.status());
            }
            Ok(response) => return Err(PortError::Network(format!("{url}: HTTP {}", response.status()))),
            Err(e) => last_error = format!("{url}: {e}"),
        }
    }
    Err(PortError::Network(last_error))
}

/// Statuses worth retrying after a pause.
pub(crate) fn is_retryable(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS || status == reqwest::StatusCode::REQUEST_TIMEOUT || status.is_server_error()
}
