//! The ApexRouter client: discover compute, hand back a trained adapter.
//!
//! Charter D2 — Puerperium uses Router **only through surfaces it already has**. Two of them:
//!
//! - `GET /v1/backends` — read-only discovery of what compute exists (D4: we never create it).
//! - `POST /v1/backends` — *"Register a URL something else is running. No lifecycle is taken
//!   over."* Exactly the handoff this needs: Router routes to the adapter, and does not
//!   supervise it.
//! - `PUT /v1/routes/{alias}` — upsert the alias so the adapter is reachable by name.
//!
//! # Two invariants inherited from Router's own design
//!
//! **Never send an `Origin` or `Sec-Fetch-Site` header.** Router's mutation gate reads: *"If
//! `Origin` is present it must be same-origin; if `Sec-Fetch-Site` is present it must be
//! `same-origin` or `none`. Otherwise a bearer with `write` scope is required."* Non-browser
//! clients send neither and pass unchanged. Adding one "for completeness" would turn every
//! mutation into a 403 unless a token happened to be configured.
//!
//! **A `CredentialSource` is a description of where a credential lives — never key material.**
//! Router's own schema says so. We pass `{kind: "env", var: "TOGETHER_API_KEY"}`, so the key
//! stays in our environment and Router learns only its name.

use serde::{Deserialize, Serialize};

use crate::provider::ProviderError;

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:2739";

/// A backend Router already knows about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Backend {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub models: Vec<BackendModel>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendModel {
    pub id: String,
}

/// Build the `NodeSpec` body for `POST /v1/backends`.
///
/// Pure. `base_url` is stored **without** a trailing `/v1` — Router's schema is explicit
/// about that, and a doubled `/v1` produces 404s that look like the backend is down.
pub fn node_spec(
    base_url: &str,
    label: &str,
    declared_models: &[String],
    credential_env: Option<&str>,
) -> serde_json::Value {
    let credential = match credential_env {
        // A DESCRIPTION of where the key lives. Never the key.
        Some(var) => serde_json::json!({ "kind": "env", "var": var }),
        None => serde_json::json!({ "kind": "none" }),
    };
    serde_json::json!({
        "base_url": base_url.trim_end_matches('/').trim_end_matches("/v1"),
        "label": label,
        "declared_models": declared_models,
        "credential": credential,
    })
}

/// Build the `ModelRoute` body for `PUT /v1/routes/{alias}`.
///
/// Pure. `model` is the name the backend actually serves; `None` passes the alias through
/// unchanged, which is only right when the backend serves it under that exact name.
pub fn model_route(alias: &str, backend_id: &str, model: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "alias": alias,
        "strategy": "first_healthy",
        "targets": [{
            "backend": { "sel": "id", "value": backend_id },
            "model": model,
        }],
    })
}

/// Parse `GET /v1/backends`.
pub fn parse_backends(body: &str) -> Result<Vec<Backend>, ProviderError> {
    serde_json::from_str(body)
        .map_err(|e| ProviderError::Malformed(format!("backends response: {e}")))
}

/// Extract the backend id from a registration response.
pub fn parse_backend_id(body: &str) -> Result<String, ProviderError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| ProviderError::Malformed(format!("register response: {e}")))?;
    v.get("id")
        .and_then(|i| i.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            ProviderError::Malformed(
                "register response carried no `id` — the backend cannot be routed to".into(),
            )
        })
}

/// Which base models a backend advertises LoRA support for.
///
/// Together names them with a `-Lora` suffix (`Qwen/Qwen3.6-35B-A3B-Lora`). The base to
/// fine-tune is that name **without** the suffix — the suffixed entry is the serving
/// endpoint for adapters of it.
pub fn lora_capable_bases(backend: &Backend) -> Vec<String> {
    backend
        .models
        .iter()
        .filter_map(|m| {
            let id = &m.id;
            let lower = id.to_lowercase();
            lower
                .strip_suffix("-lora")
                .map(|_| id[..id.len() - 5].to_string())
        })
        .collect()
}

/// Is `base` something this backend actually serves?
///
/// Free, local, and *before* a paid submission. Our default was `Qwen/Qwen3.6-27B`, which
/// Together does not carry at all — the round trip to discover that costs a failed job.
pub fn serves_model(backend: &Backend, base: &str) -> bool {
    backend.models.iter().any(|m| m.id == base)
}

pub struct RouterClient {
    base_url: String,
    token: Option<String>,
    http: reqwest::blocking::Client,
}

impl std::fmt::Debug for RouterClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RouterClient")
            .field("base_url", &self.base_url)
            .field(
                "token",
                &self
                    .token
                    .as_ref()
                    .map(|t| format!("<redacted, {} chars>", t.len())),
            )
            .finish_non_exhaustive()
    }
}

impl RouterClient {
    pub fn from_env() -> Result<Self, ProviderError> {
        let base_url =
            std::env::var("PUERPERIUM_ROUTER_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        // Optional: a loopback control plane accepts non-browser clients without one.
        let token = std::env::var("APEXROUTER_TOKEN")
            .ok()
            .filter(|t| !t.trim().is_empty());
        let http = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| ProviderError::Unreachable(e.to_string()))?;
        Ok(Self {
            base_url,
            token,
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

    fn auth(&self, rb: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
        // NB: no Origin, no Sec-Fetch-Site — see the module docs.
        match &self.token {
            Some(t) => rb.bearer_auth(t),
            None => rb,
        }
    }

    /// What compute Router already has. Read-only (D4).
    pub fn backends(&self) -> Result<Vec<Backend>, ProviderError> {
        let resp = self.auth(self.http.get(self.url("v1/backends"))).send();
        parse_backends(&read(resp)?)
    }

    /// Register a URL something else is running. Returns the new backend id.
    pub fn register_backend(&self, spec: &serde_json::Value) -> Result<String, ProviderError> {
        let resp = self
            .auth(self.http.post(self.url("v1/backends")))
            .json(spec)
            .send();
        parse_backend_id(&read(resp)?)
    }

    /// An existing backend already pointing at `base_url`, if there is one.
    ///
    /// Registering a second backend for a URL Router already knows leaves two rows that
    /// disagree the moment one is edited. Reuse is the honest default.
    pub fn backend_for_base_url(&self, base_url: &str) -> Result<Option<Backend>, ProviderError> {
        let want = base_url.trim_end_matches('/').trim_end_matches("/v1");
        Ok(self
            .backends()?
            .into_iter()
            .find(|b| b.base_url.trim_end_matches('/') == want))
    }

    /// Upsert the alias so the adapter is reachable by name.
    pub fn upsert_route(
        &self,
        alias: &str,
        route: &serde_json::Value,
    ) -> Result<(), ProviderError> {
        let resp = self
            .auth(self.http.put(self.url(&format!("v1/routes/{alias}"))))
            .json(route)
            .send();
        read(resp).map(|_| ())
    }
}

fn read(resp: reqwest::Result<reqwest::blocking::Response>) -> Result<String, ProviderError> {
    let resp = resp.map_err(|e| ProviderError::Unreachable(e.to_string()))?;
    let status = resp.status();
    let body = resp
        .text()
        .map_err(|e| ProviderError::Unreachable(e.to_string()))?;
    if status.is_success() {
        Ok(body)
    } else {
        Err(ProviderError::Rejected(format!(
            "HTTP {status}: {}",
            body.trim()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Router's schema: "Stored WITHOUT /v1." A doubled `/v1` produces 404s that look
    /// exactly like the backend being down.
    #[test]
    fn node_spec_strips_a_trailing_v1_and_slash() {
        for input in [
            "https://api.together.xyz/v1",
            "https://api.together.xyz/v1/",
            "https://api.together.xyz/",
            "https://api.together.xyz",
        ] {
            let spec = node_spec(input, "l", &[], None);
            assert_eq!(spec["base_url"], "https://api.together.xyz", "from {input}");
        }
    }

    /// Router's own words: a CredentialSource is "A DESCRIPTION of where a credential lives.
    /// Never key material."
    #[test]
    fn credential_is_a_pointer_not_a_secret() {
        let spec = node_spec("https://x", "l", &[], Some("TOGETHER_API_KEY"));
        assert_eq!(spec["credential"]["kind"], "env");
        assert_eq!(spec["credential"]["var"], "TOGETHER_API_KEY");

        let rendered = spec.to_string();
        assert!(
            !rendered.contains("tgp_"),
            "no key material may appear: {rendered}"
        );
    }

    #[test]
    fn no_credential_declares_none_rather_than_omitting_it() {
        let spec = node_spec("https://x", "l", &[], None);
        assert_eq!(spec["credential"]["kind"], "none");
    }

    #[test]
    fn declared_models_ride_along_so_the_prober_need_not_guess() {
        let spec = node_spec("https://x", "l", &["acct/adapter-v1".to_string()], None);
        assert_eq!(spec["declared_models"][0], "acct/adapter-v1");
    }

    #[test]
    fn route_selects_the_backend_by_id_with_a_shipped_strategy() {
        let route = model_route("apexos-worker", "node-abc", Some("acct/adapter-v1"));
        assert_eq!(route["alias"], "apexos-worker");
        assert_eq!(route["targets"][0]["backend"]["sel"], "id");
        assert_eq!(route["targets"][0]["backend"]["value"], "node-abc");
        assert_eq!(route["targets"][0]["model"], "acct/adapter-v1");
        // mk1 ships exactly the strategies it implements; this is one of them.
        assert_eq!(route["strategy"], "first_healthy");
    }

    #[test]
    fn a_null_model_passes_the_alias_through() {
        let route = model_route("a", "b", None);
        assert!(route["targets"][0]["model"].is_null());
    }

    /// Shaped from a real response off the running Router.
    #[test]
    fn parses_a_real_backends_listing() {
        let body = r#"[{"id":"node-127.0.0.1","kind":"node","protocol":"open_ai",
            "label":"garden-r1 2x3090 vast","base_url":"http://127.0.0.1:8800",
            "credential":{"kind":"none"},"tags":["node"],"enabled":true,
            "models":[{"id":"Qwen3.6-27B-Q6_K.gguf","ctx":262144}]}]"#;
        let got = parse_backends(body).expect("parse");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "node-127.0.0.1");
        assert_eq!(got[0].label, "garden-r1 2x3090 vast");
        assert_eq!(got[0].models[0].id, "Qwen3.6-27B-Q6_K.gguf");
    }

    #[test]
    fn a_registration_without_an_id_is_an_error_not_a_guess() {
        assert_eq!(
            parse_backend_id(r#"{"id":"node-x"}"#).expect("id"),
            "node-x"
        );
        let err = parse_backend_id(r#"{"label":"no id here"}"#).expect_err("must fail");
        assert!(err.to_string().contains("cannot be routed to"), "got {err}");
    }

    fn together_like() -> Backend {
        Backend {
            id: "together".into(),
            label: "together (via /switch)".into(),
            base_url: "https://api.together.xyz".into(),
            enabled: true,
            models: vec![
                BackendModel {
                    id: "Qwen/Qwen3.6-35B-A3B-FP8".into(),
                },
                BackendModel {
                    id: "Qwen/Qwen3.6-35B-A3B-Lora".into(),
                },
                BackendModel {
                    id: "Qwen/Qwen3-8B-Lora".into(),
                },
                BackendModel {
                    id: "Qwen/Qwen3.6-Plus".into(),
                },
            ],
        }
    }

    /// Read off the live catalogue: Together carries no dense Qwen3.6-27B, which was our
    /// default base. Discovering that upstream costs a failed job; here it costs nothing.
    #[test]
    fn base_support_is_checkable_locally() {
        let b = together_like();
        assert!(
            !serves_model(&b, "Qwen/Qwen3.6-27B"),
            "not in the catalogue"
        );
        assert!(serves_model(&b, "Qwen/Qwen3.6-35B-A3B-FP8"));
    }

    #[test]
    fn lora_capable_bases_strip_the_serving_suffix() {
        let got = lora_capable_bases(&together_like());
        assert!(got.contains(&"Qwen/Qwen3.6-35B-A3B".to_string()));
        assert!(got.contains(&"Qwen/Qwen3-8B".to_string()));
        assert!(
            !got.iter().any(|m| m.ends_with("-Lora")),
            "suffix must be gone: {got:?}"
        );
        assert!(
            !got.contains(&"Qwen/Qwen3.6-Plus".to_string()),
            "no -Lora entry, not listed"
        );
    }

    #[test]
    fn debug_never_prints_the_token() {
        let c = RouterClient {
            base_url: "http://127.0.0.1:2739".into(),
            token: Some("super-secret-token".into()),
            http: reqwest::blocking::Client::new(),
        };
        let rendered = format!("{c:?}");
        assert!(!rendered.contains("super-secret"), "leaked: {rendered}");
        assert!(rendered.contains("redacted"));
    }
}
