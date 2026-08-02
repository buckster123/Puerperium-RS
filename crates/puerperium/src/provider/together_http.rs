//! The HTTP shell around the pure Together functions.
//!
//! Deliberately thin: every decision — what to send, what a response means, whether a status
//! is terminal — lives in [`super::together`] and is unit-tested there. This file only moves
//! bytes.
//!
//! > **Unverified against the live API.** Every shape here comes from Together's own SDK
//! > types, and the parsers are tested against those shapes, but no request has ever been
//! > sent. Charter D5 forbids live calls from tests, and D4/D8 make the first real submission
//! > André's explicit, counted act. Until then this is INSTALLED, not ACTIVE — and it says so
//! > rather than pretending otherwise.

use std::time::Duration;

use crate::provider::{together, ProviderError, ProviderStatus, SubmitRequest, TrainingProvider};

const DEFAULT_BASE_URL: &str = "https://api.together.xyz/v1";
const API_KEY_ENV: &str = "TOGETHER_API_KEY";

/// Long, because a fine-tune submission is not a fast call and an impatient client would
/// turn a live job into an unconfirmed one (doctrine #9).
const TIMEOUT: Duration = Duration::from_secs(120);

pub struct TogetherClient {
    base_url: String,
    api_key: String,
    http: reqwest::blocking::Client,
}

/// Hand-written so the key can never reach a log, a panic message or a `{:?}`.
///
/// Deriving `Debug` here would print it verbatim the first time anything formatted the
/// client — house rule: lengths and heads only, never the value (doctrine #6).
impl std::fmt::Debug for TogetherClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TogetherClient")
            .field("base_url", &self.base_url)
            .field(
                "api_key",
                &format_args!("<redacted, {} chars>", self.api_key.len()),
            )
            .finish_non_exhaustive()
    }
}

impl TogetherClient {
    /// Build from the environment.
    ///
    /// A missing key is [`ProviderError::NoKey`] naming the variable — "no key configured"
    /// beats a timeout, every time (doctrine #3).
    pub fn from_env() -> Result<Self, ProviderError> {
        let api_key = std::env::var(API_KEY_ENV)
            .ok()
            .filter(|k| !k.trim().is_empty())
            .ok_or(ProviderError::NoKey {
                provider: "together",
                env_var: API_KEY_ENV,
            })?;
        let base_url =
            std::env::var("TOGETHER_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());

        let http = reqwest::blocking::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .map_err(|e| ProviderError::Unreachable(format!("could not build http client: {e}")))?;

        Ok(Self {
            base_url,
            api_key,
            http,
        })
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }
}

/// Read a response into `(status, body)`, mapping transport failure to `Unreachable`.
///
/// The distinction is load-bearing: transport failure means *we could not ask*, which is not
/// a job failure, while an HTTP error status means *they answered and refused*.
fn read(resp: reqwest::Result<reqwest::blocking::Response>) -> Result<String, ProviderError> {
    let resp = resp.map_err(|e| ProviderError::Unreachable(e.to_string()))?;
    let status = resp.status();
    let body = resp
        .text()
        .map_err(|e| ProviderError::Unreachable(format!("could not read body: {e}")))?;

    if status.is_success() {
        Ok(body)
    } else {
        // Carry the upstream's own words — a bare status code costs the next session an hour.
        Err(ProviderError::Rejected(format!(
            "HTTP {status}: {}",
            body.trim()
        )))
    }
}

impl TrainingProvider for TogetherClient {
    fn submit(&self, req: &SubmitRequest) -> Result<String, ProviderError> {
        let body = together::build_submit_body(req);
        let resp = self
            .http
            .post(self.url("fine-tunes"))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send();
        together::parse_submit_response(&read(resp)?)
    }

    fn poll(&self, provider_job_id: &str) -> Result<ProviderStatus, ProviderError> {
        let resp = self
            .http
            .get(self.url(&format!("fine-tunes/{provider_job_id}")))
            .bearer_auth(&self.api_key)
            .send();
        together::parse_status_response(&read(resp)?)
    }

    fn cancel(&self, provider_job_id: &str) -> Result<(), ProviderError> {
        let resp = self
            .http
            .post(self.url(&format!("fine-tunes/{provider_job_id}/cancel")))
            .bearer_auth(&self.api_key)
            .send();
        read(resp).map(|_| ())
    }

    fn name(&self) -> &'static str {
        "together"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hermetic: constructs nothing that talks, only checks the no-key degrade and URL join.
    #[test]
    fn a_missing_key_is_named_not_a_timeout() {
        // SAFETY-ish: single-threaded test, restores nothing it did not set.
        let saved = std::env::var(API_KEY_ENV).ok();
        std::env::remove_var(API_KEY_ENV);

        let err = TogetherClient::from_env().expect_err("must refuse without a key");
        assert!(matches!(err, ProviderError::NoKey { .. }), "got {err:?}");
        assert!(
            err.to_string().contains(API_KEY_ENV),
            "must name the variable"
        );

        if let Some(v) = saved {
            std::env::set_var(API_KEY_ENV, v);
        }
    }

    /// Deriving `Debug` would leak the key the first time anything formatted the client.
    #[test]
    fn debug_never_prints_the_key() {
        let c = TogetherClient {
            base_url: "https://example.test/v1".into(),
            api_key: "sk-super-secret-value".into(),
            http: reqwest::blocking::Client::new(),
        };
        let rendered = format!("{c:?}");
        assert!(!rendered.contains("super-secret"), "leaked: {rendered}");
        assert!(rendered.contains("redacted"));
        assert!(
            rendered.contains("21 chars"),
            "lengths are fine, values are not"
        );
    }

    #[test]
    fn url_join_survives_a_trailing_slash_on_the_base() {
        let c = TogetherClient {
            base_url: "https://example.test/v1/".into(),
            api_key: "x".into(),
            http: reqwest::blocking::Client::new(),
        };
        assert_eq!(c.url("fine-tunes"), "https://example.test/v1/fine-tunes");
        assert_eq!(c.url("/fine-tunes"), "https://example.test/v1/fine-tunes");
    }
}
