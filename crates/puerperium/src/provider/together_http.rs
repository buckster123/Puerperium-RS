//! The HTTP shell around the pure Together functions.
//!
//! Deliberately thin: every decision — what to send, what a response means, whether a status
//! is terminal — lives in [`super::together`] and is unit-tested there. This file only moves
//! bytes.
//!
//! > **ACTIVE for one live job** (`ft-da39441f-d088`, 2026-08-03). Shapes still come from
//! > Together's SDK types and parsers stay fixture-tested (D5: no live calls from CI). The
//! > S6 *measurement* — a specialist beats its base on a real task — is still unmet, and
//! > further paid submits remain André's explicit, counted act (D4/D8).

use std::path::Path;
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
        // Faces that skip the CLI still pick up ~/.config/puerperium/env. A real
        // environment variable already set wins (secrets::load never overwrites).
        let _ = crate::secrets::load();
        let api_key = std::env::var(API_KEY_ENV)
            .ok()
            .filter(|k| !k.trim().is_empty())
            .ok_or(ProviderError::NoKey {
                provider: "together",
                env_var: API_KEY_ENV,
            })?;
        let base_url =
            std::env::var("TOGETHER_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());

        // Redirects are NOT followed. The upload flow hands back a presigned URL in the
        // `Location` header of a redirect response, and the file id in `X-Together-File-Id`.
        // A client that follows the redirect automatically consumes both and PUTs an empty
        // body to storage — the upload silently succeeds at nothing.
        let http = reqwest::blocking::Client::builder()
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
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

/// Classify an HTTP status + body.
///
/// - 2xx → the body, for the caller to parse
/// - 401/403 and other 4xx (except 408/429) → [`ProviderError::Rejected`] (they said no)
/// - 408 / 429 / 5xx → [`ProviderError::Unreachable`] (we could not get a stable answer;
///   a paid job may still be running — this must not become a local `Failed`)
pub fn classify_http(status: u16, body: &str) -> Result<String, ProviderError> {
    let msg = format!("HTTP {status}: {}", body.trim());
    match status {
        200..=299 => Ok(body.to_string()),
        401 | 403 => Err(ProviderError::Rejected(msg)),
        408 | 429 => Err(ProviderError::Unreachable(msg)),
        400..=499 => Err(ProviderError::Rejected(msg)),
        _ => Err(ProviderError::Unreachable(msg)),
    }
}

/// Read a response, mapping transport failure to `Unreachable` and classifying the status.
fn read(resp: reqwest::Result<reqwest::blocking::Response>) -> Result<String, ProviderError> {
    let resp = resp.map_err(|e| ProviderError::Unreachable(e.to_string()))?;
    let status = resp.status().as_u16();
    let body = resp
        .text()
        .map_err(|e| ProviderError::Unreachable(format!("could not read body: {e}")))?;
    classify_http(status, &body)
}

/// Header carrying the new file's id on the upload-URL response.
const FILE_ID_HEADER: &str = "X-Together-File-Id";

impl TogetherClient {
    /// Upload provider-ready JSONL, returning the file id `submit` needs.
    ///
    /// Three steps, per the SDK's own upload manager:
    ///
    /// 1. `POST /files` with `purpose`/`file_name`/`file_type` → a redirect whose `Location`
    ///    is a presigned URL and whose `X-Together-File-Id` is the id.
    /// 2. `PUT` the raw bytes to that presigned URL. **No auth header** — the signature *is*
    ///    the authorisation, and sending a bearer token to third-party storage would leak it.
    /// 3. `POST /files/{id}/preprocess` to finalise.
    ///
    /// `bytes` must already be projected to the provider's schema — see
    /// [`crate::export::to_provider_jsonl`]. Uploading a stored dataset verbatim is refused
    /// upstream with "Found extra column".
    pub fn upload_jsonl(&self, file_name: &str, bytes: &[u8]) -> Result<String, ProviderError> {
        // Step 1
        let resp = self
            .http
            .post(self.url("files"))
            .bearer_auth(&self.api_key)
            // FORM-ENCODED, not query params and not JSON. Both of those return
            // 400 "Unable to save the file - invalid purpose specified" — the same message
            // whatever `purpose` value you send, because the server never sees the field at
            // all. Verified against the live API 2026-08-03; the SDK's `params=` is
            // form-encoded for this call.
            .form(&[
                ("purpose", "fine-tune"),
                ("file_name", file_name),
                ("file_type", "jsonl"),
            ])
            .send()
            .map_err(|e| ProviderError::Unreachable(e.to_string()))?;

        let status = resp.status();
        if !(status.is_success() || status.is_redirection()) {
            let body = resp.text().unwrap_or_default();
            return Err(match classify_http(status.as_u16(), &body) {
                Err(e) => e,
                Ok(_) => ProviderError::Rejected(format!(
                    "HTTP {status} asking for an upload URL: {}",
                    body.trim()
                )),
            });
        }

        let header = |name: &str| -> Option<String> {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let file_id = header(FILE_ID_HEADER).ok_or_else(|| {
            ProviderError::Malformed(format!(
                "upload-URL response carried no {FILE_ID_HEADER} — the file cannot be referenced"
            ))
        })?;
        let presigned = header("location").ok_or_else(|| {
            ProviderError::Malformed("upload-URL response carried no Location".into())
        })?;

        // Step 2 — presigned; deliberately unauthenticated.
        let put = self
            .http
            .put(&presigned)
            .body(bytes.to_vec())
            .send()
            .map_err(|e| ProviderError::Unreachable(format!("uploading bytes: {e}")))?;
        if !put.status().is_success() {
            return Err(ProviderError::Rejected(format!(
                "HTTP {} storing the file",
                put.status()
            )));
        }

        // Step 3
        let confirm = self
            .http
            .post(self.url(&format!("files/{file_id}/preprocess")))
            .bearer_auth(&self.api_key)
            .send();
        read(confirm)?;

        Ok(file_id)
    }
}

impl TogetherClient {
    /// A model's published fine-tuning limits. Free, and the honest way to learn whether a
    /// base is fine-tunable before paying to find out.
    pub fn limits(&self, model: &str) -> Result<together::Limits, ProviderError> {
        let resp = self
            .http
            .get(self.url("fine-tunes/models/limits"))
            .bearer_auth(&self.api_key)
            .query(&[("model_name", model)])
            .send()
            .map_err(|e| ProviderError::Unreachable(e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp
            .text()
            .map_err(|e| ProviderError::Unreachable(e.to_string()))?;
        together::parse_limits(&classify_http(status, &body)?)
    }
}

impl TogetherClient {
    /// Together's own price estimate for an uploaded file. Free, and authoritative — it
    /// knows the real tokenizer *and* the minimum charge, neither of which a local heuristic
    /// can guess.
    pub fn estimate_price(
        &self,
        training_file_id: &str,
        base_model: &str,
        n_epochs: u32,
        lora_r: u32,
        lora_alpha: u32,
        target_modules: &[String],
    ) -> Result<together::PriceEstimate, ProviderError> {
        let body = together::build_estimate_body(
            training_file_id,
            base_model,
            n_epochs,
            lora_r,
            lora_alpha,
            target_modules,
        );
        let resp = self
            .http
            .post(self.url("fine-tunes/estimate-price"))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send();
        together::parse_price_estimate(&read(resp)?)
    }
}

impl TogetherClient {
    /// `GET /v1/finetune/download` → a `.tar.zst` written to `dest`.
    ///
    /// Free. Follows a redirect **without** the bearer token — a presigned URL is
    /// the authorisation, and sending the key to third-party storage would leak it
    /// (same rule as the upload PUT).
    pub fn download_checkpoint_to(
        &self,
        ft_id: &str,
        checkpoint: together::Checkpoint,
        dest: &Path,
    ) -> Result<String, ProviderError> {
        let url = self.url(&together::download_path(ft_id, checkpoint));
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.api_key)
            .header(reqwest::header::ACCEPT, "application/octet-stream")
            .send()
            .map_err(|e| ProviderError::Unreachable(e.to_string()))?;

        let status = resp.status();
        if status.is_redirection() {
            let loc = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| {
                    ProviderError::Malformed("download redirect carried no Location".into())
                })?
                .to_string();
            let filename = together::filename_from_disposition(
                resp.headers()
                    .get(reqwest::header::CONTENT_DISPOSITION)
                    .and_then(|v| v.to_str().ok()),
                checkpoint,
            );
            let body =
                self.http.get(&loc).send().map_err(|e| {
                    ProviderError::Unreachable(format!("following download URL: {e}"))
                })?;
            if !body.status().is_success() {
                return Err(ProviderError::Rejected(format!(
                    "HTTP {} fetching the archive",
                    body.status()
                )));
            }
            write_body(body, dest)?;
            return Ok(filename);
        }

        if !status.is_success() {
            let body = resp
                .text()
                .map_err(|e| ProviderError::Unreachable(e.to_string()))?;
            return Err(match classify_http(status.as_u16(), &body) {
                Err(e) => e,
                Ok(_) => ProviderError::Rejected(format!("HTTP {status}: {}", body.trim())),
            });
        }

        let filename = together::filename_from_disposition(
            resp.headers()
                .get(reqwest::header::CONTENT_DISPOSITION)
                .and_then(|v| v.to_str().ok()),
            checkpoint,
        );
        write_body(resp, dest)?;
        Ok(filename)
    }
}

fn write_body(mut resp: reqwest::blocking::Response, dest: &Path) -> Result<(), ProviderError> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            ProviderError::Unreachable(format!("creating {}: {e}", parent.display()))
        })?;
    }
    let tmp = dest.with_extension("part");
    {
        let mut f = std::fs::File::create(&tmp)
            .map_err(|e| ProviderError::Unreachable(format!("writing {}: {e}", tmp.display())))?;
        std::io::copy(&mut resp, &mut f)
            .map_err(|e| ProviderError::Unreachable(format!("writing {}: {e}", tmp.display())))?;
        f.sync_all()
            .map_err(|e| ProviderError::Unreachable(format!("syncing {}: {e}", tmp.display())))?;
    }
    std::fs::rename(&tmp, dest)
        .map_err(|e| ProviderError::Unreachable(format!("renaming {}: {e}", dest.display())))?;
    Ok(())
}

impl TrainingProvider for TogetherClient {
    fn submit(&self, req: &SubmitRequest) -> Result<String, ProviderError> {
        // Resolve against the model's own limits first. Free, and it converts an opaque
        // "(Binding)" refusal into either a clean local fix or an honest "not fine-tunable".
        let limits = self.limits(&req.base_model)?;
        let resolved = SubmitRequest {
            hyperparams: together::resolve(req.hyperparams.clone(), &limits)?,
            ..req.clone()
        };
        let body = together::build_submit_body_with(&resolved, &limits.target_modules);
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
        // Isolate from ~/.config/puerperium/env — from_env now calls secrets::load().
        let empty = tempfile::NamedTempFile::new().expect("tmp");
        let saved_key = std::env::var(API_KEY_ENV).ok();
        let saved_file = std::env::var("PUERPERIUM_ENV_FILE").ok();
        std::env::set_var("PUERPERIUM_ENV_FILE", empty.path());
        std::env::remove_var(API_KEY_ENV);

        let err = TogetherClient::from_env().expect_err("must refuse without a key");
        assert!(matches!(err, ProviderError::NoKey { .. }), "got {err:?}");
        assert!(
            err.to_string().contains(API_KEY_ENV),
            "must name the variable"
        );

        match saved_key {
            Some(v) => std::env::set_var(API_KEY_ENV, v),
            None => std::env::remove_var(API_KEY_ENV),
        }
        match saved_file {
            Some(v) => std::env::set_var("PUERPERIUM_ENV_FILE", v),
            None => std::env::remove_var("PUERPERIUM_ENV_FILE"),
        }
    }

    #[test]
    fn classify_http_keeps_retryable_statuses_non_terminal() {
        for code in [408, 429, 500, 502, 503] {
            let err = classify_http(code, "try again").expect_err("retryable");
            assert!(
                matches!(err, ProviderError::Unreachable(_)),
                "{code} must not become Rejected: {err}"
            );
        }
        assert!(matches!(
            classify_http(400, "bad base").expect_err("4xx"),
            ProviderError::Rejected(_)
        ));
        assert!(matches!(
            classify_http(401, "no").expect_err("auth"),
            ProviderError::Rejected(_)
        ));
        assert_eq!(classify_http(200, "ok").expect("2xx"), "ok");
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
