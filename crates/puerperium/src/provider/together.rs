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
/// What a model's fine-tuning limits say. Parsed from
/// `GET /v1/fine-tunes/models/limits?model_name=…`, which is **free** and is the honest way
/// to learn whether a base is fine-tunable at all.
#[derive(Debug, Clone, PartialEq)]
pub struct Limits {
    pub max_batch_size: u32,
    pub min_batch_size: u32,
    pub max_rank: u32,
    /// The ONLY modules this model accepts. `"all-linear"` is rejected by models that
    /// publish a specific list.
    pub target_modules: Vec<String>,
    pub max_num_epochs: u32,
}

/// Parse a limits response.
///
/// A model that is not fine-tunable answers with a `message` instead of limits — that is a
/// clean, free "no" and is reported as such rather than as a parse failure.
pub fn parse_limits(body: &str) -> Result<Limits, ProviderError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| ProviderError::Malformed(format!("limits response: {e}")))?;

    if let Some(msg) = v.get("message").and_then(|m| m.as_str()) {
        return Err(ProviderError::Rejected(msg.to_string()));
    }
    let lora = v.get("lora_training").ok_or_else(|| {
        ProviderError::Rejected(
            "model publishes no lora_training limits — it is not LoRA fine-tunable".into(),
        )
    })?;

    let num = |k: &str| lora.get(k).and_then(serde_json::Value::as_u64).unwrap_or(0) as u32;
    Ok(Limits {
        max_batch_size: num("max_batch_size"),
        min_batch_size: num("min_batch_size"),
        max_rank: num("max_rank"),
        target_modules: lora
            .get("target_modules")
            .and_then(|t| t.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        max_num_epochs: v
            .get("max_num_epochs")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u32,
    })
}

/// Resolve requested hyperparameters against a model's published limits.
///
/// Pure. This is what the SDK does client-side, and skipping it is why a hand-built body gets
/// `"Could not create the FineTune object (Binding)"` — an opaque refusal that names nothing.
/// Doing it here turns a class of paid-path 400s into local arithmetic.
pub fn resolve(mut hp: Hyperparams, limits: &Limits) -> Hyperparams {
    // `"max"` is resolved BY THE CLIENT; the API wants a number. min == max on some models,
    // so there is exactly one legal value.
    hp.batch_size = Some(match hp.batch_size {
        Some(b) => b.clamp(limits.min_batch_size.max(1), limits.max_batch_size.max(1)),
        None => limits.max_batch_size.max(1),
    });
    if limits.max_rank > 0 && hp.lora_r > limits.max_rank {
        hp.lora_r = limits.max_rank;
    }
    if limits.max_num_epochs > 0 && hp.n_epochs > limits.max_num_epochs {
        hp.n_epochs = limits.max_num_epochs;
    }
    hp
}

/// `POST /v1/fine-tunes` body, with the model's own target modules.
pub fn build_submit_body_with(req: &SubmitRequest, target_modules: &[String]) -> serde_json::Value {
    let mut body = build_submit_body(req);
    // A model publishing a specific list rejects "all-linear".
    if !target_modules.is_empty() {
        body["training_type"]["lora_trainable_modules"] =
            serde_json::json!(target_modules.join(","));
    }
    body
}

/// `POST /v1/fine-tunes` body.
///
/// Pure. `training_type` uses the SDK's `"Lora"` discriminator — not `"lora"`; the casing is
/// load-bearing and an upstream that does not recognise it would silently full-fine-tune,
/// at roughly ten times the cost.
pub fn build_submit_body(req: &SubmitRequest) -> serde_json::Value {
    let Hyperparams {
        n_epochs,
        learning_rate,
        lora_r,
        lora_alpha,
        batch_size,
    } = &req.hyperparams;

    // THE API APPLIES NO DEFAULTS — the SDK does, client-side. Omitting a field is not
    // "use the default", it is sending zero: an absent `batch_size` is rejected with
    // "batch size is zero", an absent `n_checkpoints` with "number of checkpoints is less
    // than one". So the body carries the SDK's full default set explicitly. Verified against
    // the live API 2026-08-03, one rejection at a time.
    serde_json::json!({
        "training_file": req.training_file_id,
        "model": req.base_model,
        "suffix": req.output_name,
        "n_epochs": n_epochs,
        "learning_rate": learning_rate,
        "batch_size": match batch_size {
            Some(bs) => serde_json::json!(bs),
            None => serde_json::json!("max"),
        },
        "training_type": {
            "type": "Lora",
            "lora_r": lora_r,
            "lora_alpha": lora_alpha,
            "lora_dropout": 0.0,
            "lora_trainable_modules": "all-linear",
        },
        // `training_method` and `lr_scheduler` are OBJECTS, not strings. Sending
        // `"training_method": "sft"` is refused with the opaque
        // "Could not create the FineTune object (Binding)" — a body-binding type mismatch
        // that names no field. The SDK builds TrainingMethodSFT / CosineLRScheduler.
        "training_method": { "method": "sft" },
        "lr_scheduler": {
            "lr_scheduler_type": "cosine",
            "lr_scheduler_args": { "min_lr_ratio": 0.0, "num_cycles": 0.5 },
        },
        "n_checkpoints": 1,
        "n_evals": 0,
        "validation_file": "",
        "warmup_ratio": 0.0,
        "max_grad_norm": 1.0,
        "weight_decay": 0.0,
    })
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

/// What Together's own estimator says. Free, and **authoritative** — unlike a local
/// heuristic it knows the tokenizer and, decisively, the **minimum charge**.
#[derive(Debug, Clone, PartialEq)]
pub struct PriceEstimate {
    pub total_usd: f64,
    pub train_tokens: u64,
    pub allowed_to_proceed: bool,
}

/// Build the `POST /v1/fine-tunes/estimate-price` body. Pure.
pub fn build_estimate_body(
    training_file_id: &str,
    base_model: &str,
    n_epochs: u32,
    lora_r: u32,
    lora_alpha: u32,
    target_modules: &[String],
) -> serde_json::Value {
    let modules = if target_modules.is_empty() {
        "all-linear".to_string()
    } else {
        target_modules.join(",")
    };
    serde_json::json!({
        "training_file": training_file_id,
        "model": base_model,
        "n_epochs": n_epochs,
        "n_evals": 0,
        "training_type": {
            "type": "Lora",
            "lora_r": lora_r,
            "lora_alpha": lora_alpha,
            "lora_dropout": 0.0,
            "lora_trainable_modules": modules,
        },
        "training_method": { "method": "sft" },
    })
}

pub fn parse_price_estimate(body: &str) -> Result<PriceEstimate, ProviderError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| ProviderError::Malformed(format!("estimate response: {e}")))?;
    if let Some(msg) = v.get("message").and_then(|m| m.as_str()) {
        return Err(ProviderError::Rejected(msg.to_string()));
    }
    Ok(PriceEstimate {
        total_usd: v
            .get("estimated_total_price")
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| ProviderError::Malformed("no estimated_total_price".into()))?,
        train_tokens: v
            .get("estimated_train_token_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        allowed_to_proceed: v
            .get("allowed_to_proceed")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true),
    })
}

/// `total_price` on a job record is in **nano-dollars**, not dollars.
///
/// A completed job reporting `4000000000` cost **$4.00**. Reading it as dollars would be off
/// by a factor of a billion in the reassuring direction.
pub fn nanodollars_to_usd(nano: u64) -> f64 {
    nano as f64 / 1e9
}

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
        // Omitting it is rejected upstream with "batch size is zero".
        assert_eq!(body["batch_size"], "max", "must always be sent");
        // The API applies no defaults; every one of these is required in the body.
        assert_eq!(body["n_checkpoints"], 1, "absent = 'less than one'");
        assert_eq!(
            body["training_method"]["method"], "sft",
            "object, not a string"
        );
        assert_eq!(body["lr_scheduler"]["lr_scheduler_type"], "cosine");
        assert_eq!(body["n_evals"], 0);
        assert_eq!(body["max_grad_norm"], 1.0);
        assert_eq!(
            body["training_type"]["lora_trainable_modules"],
            "all-linear"
        );
    }

    #[test]
    fn an_explicit_batch_size_overrides_the_max_default() {
        let mut r = req();
        r.hyperparams.batch_size = Some(8);
        assert_eq!(build_submit_body(&r)["batch_size"], 8);
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

    /// The correction that matters: a job reporting 4000000000 cost $4.00, not $4bn and not
    /// 4 cents. Reading it as dollars is wrong by a billion in the reassuring direction.
    #[test]
    fn total_price_is_nanodollars() {
        assert!((nanodollars_to_usd(4_000_000_000) - 4.0).abs() < 1e-9);
        assert_eq!(nanodollars_to_usd(0), 0.0);
    }

    /// Real response from the live endpoint for the first shipped job.
    #[test]
    fn parses_the_authoritative_estimate() {
        let body = r#"{"allowed_to_proceed":true,"credit_limit":0,
            "estimated_eval_token_count":0,"estimated_total_price":4,
            "estimated_train_token_count":50319,"estimation_available":true}"#;
        let got = parse_price_estimate(body).expect("parse");
        assert_eq!(got.total_usd, 4.0);
        assert_eq!(got.train_tokens, 50319);
        assert!(got.allowed_to_proceed);
    }

    /// 50319 tokens at $1.50/Mtok is $0.075 — the charge was $4.00. A MINIMUM dominates
    /// small datasets, and no token-based local heuristic can see it.
    #[test]
    fn the_local_heuristic_cannot_see_the_minimum_charge() {
        let metered = (50_319.0 / 1_000_000.0) * lora_price_per_mtok(35.0).expect("band");
        assert!(metered < 0.10, "metered cost is trivial: {metered}");
        assert!(4.0 / metered > 50.0, "the minimum dominates by >50x");
    }

    #[test]
    fn estimate_body_uses_the_models_own_target_modules() {
        let b = build_estimate_body(
            "file-x",
            "Qwen/Qwen3.6-35B-A3B",
            3,
            16,
            32,
            &["k_proj".into(), "v_proj".into()],
        );
        assert_eq!(
            b["training_type"]["lora_trainable_modules"],
            "k_proj,v_proj"
        );
        assert_eq!(b["training_method"]["method"], "sft");
        assert_eq!(b["n_epochs"], 3);
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
