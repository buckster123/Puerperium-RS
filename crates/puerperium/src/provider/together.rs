//! The Together fine-tuning provider.
//!
//! Everything load-bearing here is a **pure function** — the request builder, the response
//! parsers, the status mapping. The HTTP glue is a thin shell around them, and no test in
//! this crate ever opens a socket (charter D5).
//!
//! Shapes are taken from Together's own SDK (`together-python`,
//! `src/together/types/finetune.py`), not from prose documentation, so the status vocabulary
//! and the LoRA discriminator match what the API actually sends.

use serde::Deserialize;

use crate::job::{Hyperparams, Phase};
use crate::provider::{ProviderError, ProviderStatus, SubmitRequest};

/// `POST /v1/fine-tunes` body.
///
/// Pure. `training_type` uses the SDK's `"Lora"` discriminator — not `"lora"`; the casing is
/// load-bearing and an upstream that does not recognise it would silently full-fine-tune,
/// which costs roughly ten times as much.
pub fn build_submit_body(req: &SubmitRequest) -> serde_json::Value {
    let Hyperparams {
        n_epochs,
        learning_rate,
        lora_r,
        lora_alpha,
        batch_size,
    } = &req.hyperparams;

    let mut body = serde_json::json!({
        "training_file": req.training_file_id,
        "model": req.base_model,
        "suffix": req.output_name,
        "n_epochs": n_epochs,
        "learning_rate": learning_rate,
        "training_type": {
            "type": "Lora",
            "lora_r": lora_r,
            "lora_alpha": lora_alpha,
        },
    });
    if let Some(bs) = batch_size {
        body["batch_size"] = serde_json::json!(bs);
    }
    body
}

#[derive(Deserialize)]
struct SubmitResponse {
    id: Option<String>,
    job_id: Option<String>,
}

/// Extract the upstream job id from a submit response.
///
/// The SDK carries both `id` and `job_id`; either is accepted, `id` first. A response with
/// neither is an error rather than a fabricated id — a job we cannot name later is a job we
/// cannot recover, and this is the money path.
pub fn parse_submit_response(body: &str) -> Result<String, ProviderError> {
    let parsed: SubmitResponse = serde_json::from_str(body)
        .map_err(|e| ProviderError::Malformed(format!("submit response was not JSON: {e}")))?;
    parsed
        .id
        .or(parsed.job_id)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ProviderError::Malformed(
                "submit response carried neither `id` nor `job_id` — the job cannot be tracked"
                    .into(),
            )
        })
}

#[derive(Deserialize)]
struct StatusResponse {
    status: Option<String>,
    #[serde(default)]
    adapter_output_name: Option<String>,
    #[serde(default)]
    model_output_name: Option<String>,
}

/// Map a poll response onto a phase, an artifact and an honest reason.
pub fn parse_status_response(body: &str) -> Result<ProviderStatus, ProviderError> {
    let parsed: StatusResponse = serde_json::from_str(body)
        .map_err(|e| ProviderError::Malformed(format!("status response was not JSON: {e}")))?;

    let raw = parsed.status.unwrap_or_default();
    let phase = map_status(&raw);

    Ok(ProviderStatus {
        phase,
        artifact: parsed.adapter_output_name.or(parsed.model_output_name),
        error: failure_reason(&raw),
        upstream_status: raw,
    })
}

/// Together's `FinetuneJobStatus` → our [`Phase`].
///
/// **An unrecognised status maps to [`Phase::Unknown`], never to `Running`.** Upstreams add
/// states; a parser that guesses turns an unknown into a confident lie, and this one governs
/// whether a paid job is treated as finished.
pub fn map_status(raw: &str) -> Phase {
    match raw {
        "pending" | "queued" => Phase::Submitted,
        "running" | "compressing" | "uploading" => Phase::Running,
        "cancel_requested" => Phase::Cancelling,
        "completed" => Phase::Succeeded,
        "error" | "user_error" => Phase::Failed,
        "cancelled" => Phase::Cancelled,
        _ => Phase::Unknown,
    }
}

/// Both failure states become `Failed`, but the distinction survives in the reason —
/// "your dataset was rejected" and "our trainer fell over" call for different actions.
fn failure_reason(raw: &str) -> Option<String> {
    match raw {
        "user_error" => Some(
            "upstream reported user_error — the request or dataset was rejected; \
             check the dataset format and the base model before resubmitting"
                .into(),
        ),
        "error" => Some("upstream reported error — the training run itself failed".into()),
        _ => None,
    }
}

// -------------------------------------------------------- pricing

/// Together LoRA fine-tuning, US dollars per million tokens, by parameter band.
///
/// Verified 2026-08-02. Bands, not a formula — a model just over a boundary costs a step more.
pub fn lora_price_per_mtok(params_b: f64) -> Option<f64> {
    match params_b {
        p if p <= 0.0 => None,
        p if p <= 16.0 => Some(0.48),
        p if p <= 69.0 => Some(1.50),
        p if p <= 100.0 => Some(2.90),
        // Above the published bands, and frontier architectures are priced separately
        // ($3–$40/Mtok with per-model minimums). Refusing to guess is the honest answer.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::Method;

    fn req() -> SubmitRequest {
        SubmitRequest {
            training_file_id: "file-abc".into(),
            base_model: "Qwen/Qwen3.6-27B".into(),
            output_name: "worker-v1".into(),
            method: Method::LoraSft,
            hyperparams: Hyperparams::default(),
        }
    }

    #[test]
    fn submit_body_uses_the_sdks_lora_discriminator_exactly() {
        let body = build_submit_body(&req());
        assert_eq!(
            body["training_type"]["type"], "Lora",
            "casing is load-bearing"
        );
        assert_eq!(body["training_type"]["lora_r"], 16);
        assert_eq!(body["training_type"]["lora_alpha"], 32);
        assert_eq!(body["model"], "Qwen/Qwen3.6-27B");
        assert_eq!(body["suffix"], "worker-v1");
        assert_eq!(body["training_file"], "file-abc");
        assert!(
            body.get("batch_size").is_none(),
            "unset optional must be omitted"
        );
    }

    #[test]
    fn submit_response_accepts_either_id_field() {
        assert_eq!(
            parse_submit_response(r#"{"id":"ft-1","status":"pending"}"#).expect("id"),
            "ft-1"
        );
        assert_eq!(
            parse_submit_response(r#"{"job_id":"ft-2","status":"pending"}"#).expect("job_id"),
            "ft-2"
        );
    }

    /// A job we cannot name later is a job we cannot recover — and this is the money path.
    #[test]
    fn submit_response_without_an_id_is_an_error_not_a_fabricated_one() {
        let err = parse_submit_response(r#"{"status":"pending"}"#).expect_err("must fail");
        assert!(err.to_string().contains("cannot be tracked"), "got {err}");
        assert!(parse_submit_response("not json").is_err());
    }

    /// The full status vocabulary, taken from the SDK enum.
    #[test]
    fn every_known_upstream_status_maps_deliberately() {
        let cases = [
            ("pending", Phase::Submitted),
            ("queued", Phase::Submitted),
            ("running", Phase::Running),
            ("compressing", Phase::Running),
            ("uploading", Phase::Running),
            ("cancel_requested", Phase::Cancelling),
            ("completed", Phase::Succeeded),
            ("error", Phase::Failed),
            ("user_error", Phase::Failed),
            ("cancelled", Phase::Cancelled),
        ];
        for (raw, want) in cases {
            assert_eq!(map_status(raw), want, "{raw}");
        }
    }

    /// The invariant that governs whether a paid job is treated as finished.
    #[test]
    fn an_unrecognised_status_is_unknown_never_running() {
        for raw in ["", "paused", "future_state_we_have_not_seen", "COMPLETED"] {
            assert_eq!(
                map_status(raw),
                Phase::Unknown,
                "{raw:?} must not be guessed"
            );
        }
        assert!(!Phase::Unknown.is_terminal());
    }

    #[test]
    fn status_response_carries_the_artifact_and_a_distinguishing_reason() {
        let done = parse_status_response(
            r#"{"status":"completed","adapter_output_name":"acct/worker-v1-adapter"}"#,
        )
        .expect("parse");
        assert_eq!(done.phase, Phase::Succeeded);
        assert_eq!(done.artifact.as_deref(), Some("acct/worker-v1-adapter"));
        assert_eq!(done.error, None);

        let user = parse_status_response(r#"{"status":"user_error"}"#).expect("parse");
        let sys = parse_status_response(r#"{"status":"error"}"#).expect("parse");
        assert_eq!(user.phase, Phase::Failed);
        assert_eq!(sys.phase, Phase::Failed);
        assert_ne!(
            user.error, sys.error,
            "the two failures must not read the same"
        );
        assert!(user.error.expect("reason").contains("dataset"));
    }

    #[test]
    fn status_response_falls_back_to_model_output_name() {
        let got = parse_status_response(r#"{"status":"completed","model_output_name":"acct/m"}"#)
            .expect("parse");
        assert_eq!(got.artifact.as_deref(), Some("acct/m"));
    }

    #[test]
    fn unknown_phase_preserves_the_raw_status_for_diagnosis() {
        let got = parse_status_response(r#"{"status":"brand_new_state"}"#).expect("parse");
        assert_eq!(got.phase, Phase::Unknown);
        assert_eq!(got.upstream_status, "brand_new_state");
    }

    #[test]
    fn pricing_bands_step_at_the_published_boundaries() {
        assert_eq!(lora_price_per_mtok(7.0), Some(0.48));
        assert_eq!(lora_price_per_mtok(16.0), Some(0.48));
        assert_eq!(lora_price_per_mtok(27.0), Some(1.50), "Qwen3.6-27B band");
        assert_eq!(lora_price_per_mtok(69.0), Some(1.50));
        assert_eq!(lora_price_per_mtok(70.0), Some(2.90));
    }

    #[test]
    fn pricing_refuses_to_guess_outside_the_published_bands() {
        assert_eq!(
            lora_price_per_mtok(400.0),
            None,
            "frontier tiers are priced separately"
        );
        assert_eq!(lora_price_per_mtok(0.0), None);
    }
}
