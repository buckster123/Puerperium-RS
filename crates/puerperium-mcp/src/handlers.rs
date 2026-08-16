//! Sync tool handlers. Thin over the core library — no new capability lives here.

use std::collections::BTreeMap;
use std::path::Path;

use puerperium::apprentice::{self, Spec};
use puerperium::convert::filter::FilterConfig;
use puerperium::convert::instruct::InstructConfig;
use puerperium::convert::{convert, ConvertConfig, Converted};
use puerperium::dataset::{self, SourceSpec};
use puerperium::engine::{self, SubmitSpec};
use puerperium::estimate;
use puerperium::job::{self, ComputeRef, Hyperparams, Method, Phase, Provider};
use puerperium::memory::{MemoryRecord, MemoryType};
use puerperium::paths::Paths;
use puerperium::provider::together_http::TogetherClient;
use puerperium::provider::{ProviderError, SubmitRequest};
use puerperium::registry::{self, ApprenticeRecord};
use puerperium::source::cerebro_db;
use serde_json::{json, Value};

/// Why a `tools/call` did not return a payload.
#[derive(Debug)]
pub enum CallError {
    InvalidArgs(String),
    UnknownTool(String),
    /// The tool answered. The answer is no (D8).
    Refused {
        reason: String,
    },
    /// The tool tried and failed. The real reason, never a generic.
    Failed(String),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::InvalidArgs(s) | CallError::UnknownTool(s) | CallError::Failed(s) => {
                f.write_str(s)
            }
            CallError::Refused { reason } => f.write_str(reason),
        }
    }
}

/// One nursery process, one state directory.
pub struct Face {
    pub paths: Paths,
}

impl Face {
    pub fn call(&self, name: &str, args: &Value) -> Result<Value, CallError> {
        match name {
            "nursery_generate_data" => self.generate(args),
            "nursery_list_datasets" => self.list_datasets(),
            "nursery_inspect_dataset" => self.inspect_dataset(args),
            "nursery_estimate_cost" => self.estimate_cost(args),
            "nursery_quote" => self.quote(args),
            "nursery_upload" => self.upload(args),
            "nursery_train" => self.train(args),
            "nursery_job_status" => self.job_status(args),
            "nursery_list_jobs" => self.list_jobs(),
            "nursery_cancel_job" => self.cancel_job(args),
            "nursery_list_models" => self.list_models(),
            "nursery_register_model" => self.register_model(args),
            "nursery_test_model" => self.test_model(args),
            "nursery_create_apprentice" => self.create_apprentice(args),
            "nursery_list_apprentices" => self.list_apprentices(),
            "nursery_lineage" => self.lineage(args),
            other => Err(CallError::UnknownTool(format!("tool not found: {other}"))),
        }
    }

    fn generate(&self, args: &Value) -> Result<Value, CallError> {
        if args["synthetic"].as_bool() == Some(true) {
            return Err(CallError::Refused {
                reason: "synthetic templates are not built — nursery_generate_data takes a \
                         Cerebro snapshot (`db`) or a memories JSON export (`from`)"
                    .into(),
            });
        }

        let name = req_str(args, "name")?;
        let from = opt_str(args, "from");
        let db = opt_str(args, "db");
        let dry_run = bool_or(args, "dry_run", true);

        let (memories, source) = match (from, db) {
            (Some(path), None) => {
                let bytes = std::fs::read(path)
                    .map_err(|e| CallError::Failed(format!("reading {path}: {e}")))?;
                let memories: Vec<MemoryRecord> = serde_json::from_slice(&bytes).map_err(|e| {
                    CallError::Failed(format!("parsing {path} as a memory export: {e}"))
                })?;
                let n = memories.len();
                (
                    memories,
                    SourceSpec {
                        kind: "export_file".into(),
                        query: Some(path.to_string()),
                        agent_id: None,
                        memories_in: n,
                    },
                )
            }
            (None, Some(path)) => {
                let agent_id = req_str(args, "agent_id")?;
                let memories = mine_snapshot(
                    Path::new(path),
                    agent_id,
                    &string_list(args, "tags"),
                    opt_usize(args, "limit"),
                )?;
                let n = memories.len();
                (
                    memories,
                    SourceSpec {
                        kind: "cerebro_query".into(),
                        query: opt_str(args, "domain").map(str::to_string),
                        agent_id: Some(agent_id.to_string()),
                        memories_in: n,
                    },
                )
            }
            (Some(_), Some(_)) => {
                return Err(CallError::InvalidArgs(
                    "give `from` or `db`, not both".into(),
                ));
            }
            (None, None) => {
                return Err(CallError::InvalidArgs(
                    "`from` (memories JSON) or `db` (Cerebro snapshot) is required. \
                     Synthetic templates are not built and refuse honestly."
                        .into(),
                ));
            }
        };

        let cfg = convert_config(args)?;
        let out = convert(&memories, &cfg);
        let mut body = convert_summary(&out, memories.len());
        body["name"] = json!(name);
        body["dry_run"] = json!(dry_run);

        if dry_run {
            body["written"] = json!(false);
            return Ok(body);
        }

        let meta = dataset::write(&self.paths.datasets(), name, &out, source).map_err(lib_err)?;
        body["written"] = json!(true);
        body["sha256"] = json!(meta.sha256);
        body["path"] = json!(dataset::jsonl_path(&self.paths.datasets(), &meta.name)
            .map_err(lib_err)?
            .display()
            .to_string());
        Ok(body)
    }

    fn list_datasets(&self) -> Result<Value, CallError> {
        let all = dataset::list(&self.paths.datasets()).map_err(lib_err)?;
        Ok(json!({ "datasets": all, "count": all.len() }))
    }

    fn inspect_dataset(&self, args: &Value) -> Result<Value, CallError> {
        let name = req_str(args, "name")?;
        let head = opt_usize(args, "head").unwrap_or(3);
        let meta = dataset::read_meta(&self.paths.datasets(), name).map_err(lib_err)?;
        let path = dataset::jsonl_path(&self.paths.datasets(), name).map_err(lib_err)?;
        let text = std::fs::read_to_string(&path)
            .map_err(|e| CallError::Failed(format!("reading {}: {e}", path.display())))?;
        let examples: Vec<Value> = text
            .lines()
            .take(head)
            .map(|line| serde_json::from_str(line).map_err(|e| CallError::Failed(e.to_string())))
            .collect::<Result<_, _>>()?;
        Ok(json!({
            "meta": meta,
            "examples": examples,
            "head": examples.len(),
        }))
    }

    fn estimate_cost(&self, args: &Value) -> Result<Value, CallError> {
        let name = req_str(args, "dataset")?;
        let path = dataset::jsonl_path(&self.paths.datasets(), name).map_err(lib_err)?;
        let chars = std::fs::metadata(&path)
            .map_err(|e| CallError::Failed(format!("reading {}: {e}", path.display())))?
            .len();
        let epochs = u32_or(args, "epochs", 3);
        let params_b = f64_or(args, "params_b", 35.0);
        let est = estimate::together_lora(chars, epochs, params_b);
        serde_json::to_value(est).map_err(|e| CallError::Failed(e.to_string()))
    }

    fn quote(&self, args: &Value) -> Result<Value, CallError> {
        let file_id = req_str(args, "training_file_id")?;
        let base = together_base(opt_str(args, "base_model"));
        let epochs = u32_or(args, "epochs", 3);
        let lora_r = u32_or(args, "lora_r", 16);
        let lora_alpha = u32_or(args, "lora_alpha", 32);
        let params_b = f64_or(args, "params_b", 35.0);

        let client = together()?;
        let limits = client.limits(&base).map_err(provider_err)?;
        let est = client
            .estimate_price(
                file_id,
                &base,
                epochs,
                lora_r,
                lora_alpha,
                &limits.target_modules,
            )
            .map_err(provider_err)?;

        let mut note = None;
        if let Some(band) = puerperium::provider::together::lora_price_per_mtok(params_b) {
            let metered = (est.train_tokens as f64 / 1_000_000.0) * band;
            if est.total_usd > metered * 2.0 {
                note = Some(format!(
                    "a MINIMUM CHARGE dominates: metered tokens are only ~${metered:.2}"
                ));
            }
        }

        Ok(json!({
            "training_file_id": file_id,
            "base_model": base,
            "train_tokens": est.train_tokens,
            "total_usd": est.total_usd,
            "allowed_to_proceed": est.allowed_to_proceed,
            "note": note,
        }))
    }

    fn upload(&self, args: &Value) -> Result<Value, CallError> {
        let dataset = req_str(args, "dataset")?;
        let body = provider_bytes(&self.paths, dataset)?;
        let client = together()?;
        let file_id = client
            .upload_jsonl(&format!("{dataset}.jsonl"), body.as_bytes())
            .map_err(provider_err)?;
        let meta = dataset::read_meta(&self.paths.datasets(), dataset).map_err(lib_err)?;
        puerperium::upload::bind(
            &self.paths,
            file_id.clone(),
            meta.dataset_ref(),
            body.as_bytes(),
        )
        .map_err(lib_err)?;
        Ok(json!({
            "training_file_id": file_id,
            "dataset": meta.name,
            "sha256": meta.sha256,
            "lines": body.lines().count(),
            "bytes": body.len(),
        }))
    }

    fn train(&self, args: &Value) -> Result<Value, CallError> {
        let id = req_str(args, "id")?;
        let dataset_name = req_str(args, "dataset")?;
        let output_name = req_str(args, "output_name")?;
        let training_file_id = req_str(args, "training_file_id")?;
        let confirm = bool_or(args, "confirm", false);
        let dry_run = bool_or(args, "dry_run", true);

        // Spend gate first (D4): a missing dataset must not mask a missing confirm.
        if !confirm && !dry_run {
            return Err(CallError::Refused {
                reason: "nursery_train never spends unless confirm is true (D4)".into(),
            });
        }

        let dataset = dataset::read_meta(&self.paths.datasets(), dataset_name)
            .map_err(lib_err)?
            .dataset_ref();
        let hyperparams = Hyperparams {
            n_epochs: u32_or(args, "epochs", 3),
            lora_r: u32_or(args, "lora_r", 16),
            lora_alpha: u32_or(args, "lora_alpha", 32),
            ..Hyperparams::default()
        };
        let compute = match opt_str(args, "compute") {
            Some(name) => ComputeRef::Node {
                name: name.to_string(),
            },
            None => ComputeRef::Managed,
        };
        let base_model = together_base(opt_str(args, "base_model"));
        let trainer_agent = opt_str(args, "trainer_agent")
            .unwrap_or("FORGE")
            .to_string();
        let available = string_list(args, "available_compute");

        if !confirm {
            let req = SubmitRequest {
                training_file_id: training_file_id.to_string(),
                base_model,
                output_name: output_name.to_string(),
                method: Method::LoraSft,
                hyperparams,
            };
            let body = puerperium::provider::together::build_submit_body(&req);
            return Ok(json!({
                "dry_run": true,
                "unresolved": true,
                "note": "UNRESOLVED — not what submit sends. Live submit resolves \
                         batch_size / rank / modules against GET /v1/fine-tunes/models/limits first.",
                "would_post": "/v1/fine-tunes",
                "body": body,
                "id": id,
                "dataset": dataset,
                "trainer_agent": trainer_agent,
                "compute": compute,
            }));
        }

        engine::check_compute(&compute, &available).map_err(lib_err)?;
        let spec = SubmitSpec {
            id: id.to_string(),
            provider: Provider::Together,
            dataset,
            base_model,
            output_name: output_name.to_string(),
            method: Method::LoraSft,
            hyperparams,
            trainer_agent,
            compute,
            training_file_id: training_file_id.to_string(),
        };
        let record =
            engine::submit(&self.paths, &together()?, spec, &available).map_err(lib_err)?;
        let mut out =
            serde_json::to_value(&record).map_err(|e| CallError::Failed(e.to_string()))?;
        if let Some(t) = &record.terminal {
            out["submit_rejected"] = json!(true);
            out["reason"] = json!(t.error.as_deref().unwrap_or(t.outcome.as_str()));
        }
        Ok(out)
    }

    fn job_status(&self, args: &Value) -> Result<Value, CallError> {
        let id = req_str(args, "id")?;
        let record = job::load(self.paths.root(), id).map_err(lib_err)?;
        let (phase, poll_note) = match record.terminal_phase() {
            Some(p) => (p, None),
            None => match together() {
                Ok(client) => match engine::refresh(&self.paths, &client, id) {
                    Ok((_, p)) => (p, None),
                    Err(e) => (Phase::Unknown, Some(e.to_string())),
                },
                Err(e) => (Phase::Unknown, Some(e.to_string())),
            },
        };
        let record = job::load(self.paths.root(), id).map_err(lib_err)?;
        Ok(json!({
            "phase": phase.as_str(),
            "poll_note": poll_note,
            "job": record,
        }))
    }

    fn list_jobs(&self) -> Result<Value, CallError> {
        let log = job::load_log(self.paths.root()).map_err(lib_err)?;
        let live = log
            .jobs
            .iter()
            .any(|j| !j.is_terminal() && j.provider_job_id.is_some())
            .then(together);

        let mut jobs = Vec::new();
        let mut poll_note = None;
        for j in &log.jobs {
            let phase = match (j.terminal_phase(), &live) {
                (Some(p), _) => p,
                (None, Some(Ok(client))) => engine::refresh(&self.paths, client, &j.id)
                    .map(|(_, p)| p)
                    .unwrap_or(Phase::Unknown),
                (None, Some(Err(e))) => {
                    poll_note = Some(e.to_string());
                    Phase::Unknown
                }
                (None, None) => Phase::Unknown,
            };
            jobs.push(json!({
                "id": j.id,
                "phase": phase.as_str(),
                "job": j,
            }));
        }

        let skipped: Vec<Value> = log
            .skipped
            .iter()
            .map(|s| json!({ "line": s.line, "reason": s.reason }))
            .collect();

        Ok(json!({
            "jobs": jobs,
            "count": jobs.len(),
            "skipped": skipped,
            "poll_note": poll_note,
        }))
    }

    fn cancel_job(&self, args: &Value) -> Result<Value, CallError> {
        let id = req_str(args, "id")?;
        let record = engine::cancel(&self.paths, &together()?, id).map_err(lib_err)?;
        Ok(json!({
            "cancel_requested": true,
            "note": "nothing marked terminal until the upstream says so",
            "job": record,
        }))
    }

    fn list_models(&self) -> Result<Value, CallError> {
        let all = registry::list_models(&self.paths).map_err(lib_err)?;
        Ok(json!({ "models": all, "count": all.len() }))
    }

    fn register_model(&self, args: &Value) -> Result<Value, CallError> {
        let model = req_str(args, "model")?;
        let alias = req_str(args, "alias")?;
        let confirm = bool_or(args, "confirm", false);
        let dry_run = bool_or(args, "dry_run", true);
        let base_url = opt_str(args, "base_url").unwrap_or("https://api.together.xyz");
        let served = opt_str(args, "served_model");
        let credential_env = opt_str(args, "credential_env").unwrap_or("TOGETHER_API_KEY");

        let record = registry::load_model(&self.paths, model).map_err(lib_err)?;
        let spec = puerperium::router::node_spec(
            base_url,
            &format!("puerperium: {}", record.name),
            &served.map(|s| vec![s.to_string()]).unwrap_or_default(),
            Some(credential_env),
        );
        let route_preview = puerperium::router::model_route(alias, "<backend-id>", served);

        if !confirm {
            if !dry_run {
                return Err(CallError::Refused {
                    reason: "nursery_register_model never contacts Router unless confirm is true"
                        .into(),
                });
            }
            return Ok(json!({
                "dry_run": true,
                "would_post": "/v1/backends",
                "backend": spec,
                "would_put": format!("/v1/routes/{alias}"),
                "route": route_preview,
                "note": "nothing sent, nothing recorded. Live path records alias_requested, not liveness (D3).",
            }));
        }

        let client = puerperium::router::RouterClient::from_env().map_err(provider_err)?;
        let backend_id = match client
            .backend_for_base_url(base_url)
            .map_err(provider_err)?
        {
            Some(existing) => existing.id,
            None => client.register_backend(&spec).map_err(provider_err)?,
        };
        let route = puerperium::router::model_route(alias, &backend_id, served);
        client.upsert_route(alias, &route).map_err(provider_err)?;

        let mut updated = record.clone();
        updated.alias_requested = Some(alias.to_string());
        registry::save_model(&self.paths, &updated).map_err(lib_err)?;

        Ok(json!({
            "alias": alias,
            "backend_id": backend_id,
            "model": updated,
            "note": "alias_requested is what we asked for, not proof it is live (D3)",
        }))
    }

    fn test_model(&self, args: &Value) -> Result<Value, CallError> {
        let _model = req_str(args, "model")?;
        let _prompt = req_str(args, "prompt")?;
        Err(CallError::Refused {
            reason: "nursery_test_model is present so the agent can see it (D8), but \
                     evaluation is the Watcher's job (Stage 2) — this verb will not \
                     fake a score or start a dedicated endpoint"
                .into(),
        })
    }

    fn create_apprentice(&self, args: &Value) -> Result<Value, CallError> {
        let id = req_str(args, "id")?;
        let db = req_str(args, "db")?;
        let master = req_str(args, "master_agent")?;
        let name = req_str(args, "name")?;
        let specialization = req_str(args, "specialization")?;
        let dataset_name = req_str(args, "dataset_name")?;
        let dry_run = bool_or(args, "dry_run", true);
        let base_model = opt_str(args, "base_model")
            .unwrap_or("Qwen/Qwen3.6-27B")
            .to_string();

        let memories = mine_snapshot(
            Path::new(db),
            master,
            &string_list(args, "tags"),
            opt_usize(args, "limit"),
        )?;
        let cfg = convert_config(args)?;

        if dry_run {
            let out = convert(&memories, &cfg);
            let mut body = convert_summary(&out, memories.len());
            body["dry_run"] = json!(true);
            body["written"] = json!(false);
            body["id"] = json!(id);
            body["master_agent"] = json!(master);
            body["note"] =
                json!("untrained by design — training costs money and is a separate act");
            return Ok(body);
        }

        let spec = Spec {
            id: id.to_string(),
            master_agent: master.to_string(),
            name: name.to_string(),
            specialization: specialization.to_string(),
            base_model,
            dataset_name: dataset_name.to_string(),
        };
        let created = apprentice::create(&self.paths, spec, &memories, &cfg).map_err(lib_err)?;
        let mut body = convert_summary(&created.converted, created.memories_in);
        body["dry_run"] = json!(false);
        body["written"] = json!(true);
        body["apprentice"] = serde_json::to_value(&created.apprentice)
            .map_err(|e| CallError::Failed(e.to_string()))?;
        body["trained"] = json!(created.apprentice.is_trained());
        body["note"] = json!("untrained by design — training costs money and is a separate act");
        Ok(body)
    }

    fn list_apprentices(&self) -> Result<Value, CallError> {
        let all = registry::list_apprentices(&self.paths).map_err(lib_err)?;
        let rows: Vec<Value> = all
            .iter()
            .map(|a| {
                json!({
                    "id": a.id,
                    "master_agent": a.master_agent,
                    "name": a.name,
                    "specialization": a.specialization,
                    "trained": a.is_trained(),
                    "model": a.model,
                    "dataset": a.dataset,
                    "job_id": a.job_id,
                    "created_at": a.created_at,
                })
            })
            .collect();
        Ok(json!({ "apprentices": rows, "count": rows.len() }))
    }

    fn lineage(&self, args: &Value) -> Result<Value, CallError> {
        let name = req_str(args, "name")?;
        match registry::lineage(&self.paths, name) {
            Ok(lin) => Ok(serde_json::to_value(lin).map_err(|e| CallError::Failed(e.to_string()))?),
            Err(_) => {
                let apprentice = match registry::load_apprentice(&self.paths, name) {
                    Ok(a) => a,
                    Err(e) => return Err(lib_err(e)),
                };
                lineage_from_apprentice(&self.paths, apprentice)
            }
        }
    }
}

fn lineage_from_apprentice(
    paths: &Paths,
    apprentice: ApprenticeRecord,
) -> Result<Value, CallError> {
    match &apprentice.model {
        Some(model) => {
            let lin = registry::lineage(paths, model).map_err(lib_err)?;
            let mut out =
                serde_json::to_value(lin).map_err(|e| CallError::Failed(e.to_string()))?;
            out["apprentice"] =
                serde_json::to_value(&apprentice).map_err(|e| CallError::Failed(e.to_string()))?;
            Ok(out)
        }
        None => Ok(json!({
            "apprentice": apprentice,
            "trained": false,
            "entries": [],
            "note": "this apprentice has no model yet — there is nothing to walk. \
                     Training is a separate, explicit act (D4).",
        })),
    }
}

fn mine_snapshot(
    db: &Path,
    agent_id: &str,
    tags: &[String],
    limit: Option<usize>,
) -> Result<Vec<MemoryRecord>, CallError> {
    let query = cerebro_db::Query {
        agent_id: Some(agent_id.to_string()),
        any_tags: tags.to_vec(),
        limit,
    };
    let mut got = cerebro_db::read(db, &query).map_err(|e| CallError::Failed(e.to_string()))?;
    let stem = db.file_stem().and_then(|s| s.to_str()).unwrap_or("db");
    for m in &mut got {
        m.id = format!("{stem}:{}", m.id);
    }
    Ok(got)
}

fn convert_config(args: &Value) -> Result<ConvertConfig, CallError> {
    let mut cfg = ConvertConfig::new();
    let types = string_list(args, "include_types");
    if !types.is_empty() {
        cfg.filter.include_types = types
            .iter()
            .map(|s| parse_type(s))
            .collect::<Result<_, _>>()?;
    }
    cfg.filter.include_dream_derived = bool_or(args, "include_dream", false);
    if let Some(n) = opt_usize(args, "min_content") {
        cfg.filter = FilterConfig {
            min_content: n,
            ..cfg.filter
        };
    }
    cfg.instruct = InstructConfig {
        domain: opt_str(args, "domain").map(str::to_string),
        ..InstructConfig::new()
    };
    Ok(cfg)
}

fn convert_summary(out: &Converted, memories_in: usize) -> Value {
    let framing: BTreeMap<&str, usize> =
        out.framing.iter().map(|(k, n)| (k.as_str(), *n)).collect();
    json!({
        "memories_in": memories_in,
        "memories_used": out.memories_used,
        "examples": out.examples.len(),
        "rejected": out.rejections.total(),
        "rejections": out.rejections.counts(),
        "framing": framing,
    })
}

fn provider_bytes(paths: &Paths, name: &str) -> Result<String, CallError> {
    let path = dataset::jsonl_path(&paths.datasets(), name).map_err(lib_err)?;
    let stored = std::fs::read_to_string(&path)
        .map_err(|e| CallError::Failed(format!("reading {}: {e}", path.display())))?;
    puerperium::export::to_provider_jsonl(&stored, puerperium::export::ProviderFormat::Conversation)
        .map_err(|e| CallError::Failed(e.to_string()))
}

fn together() -> Result<TogetherClient, CallError> {
    TogetherClient::from_env().map_err(provider_err)
}

fn together_base(explicit: Option<&str>) -> String {
    explicit
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(puerperium::provider::together::default_base)
}

fn provider_err(e: ProviderError) -> CallError {
    match e {
        ProviderError::NoKey { .. } => CallError::Refused {
            reason: e.to_string(),
        },
        other => CallError::Failed(other.to_string()),
    }
}

fn lib_err(e: puerperium::Error) -> CallError {
    CallError::Failed(e.to_string())
}

fn parse_type(s: &str) -> Result<MemoryType, CallError> {
    serde_json::from_value(Value::String(s.trim().to_lowercase()))
        .map_err(|_| CallError::InvalidArgs(format!("unknown memory type {s:?}")))
}

fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, CallError> {
    args[key]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CallError::InvalidArgs(format!("{key} (non-empty string) required")))
}

fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args[key].as_str().map(str::trim).filter(|s| !s.is_empty())
}

fn bool_or(args: &Value, key: &str, default: bool) -> bool {
    args[key].as_bool().unwrap_or(default)
}

fn u32_or(args: &Value, key: &str, default: u32) -> u32 {
    args[key]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(default)
}

fn f64_or(args: &Value, key: &str, default: f64) -> f64 {
    args[key].as_f64().unwrap_or(default)
}

fn opt_usize(args: &Value, key: &str) -> Option<usize> {
    args[key].as_u64().and_then(|n| usize::try_from(n).ok())
}

fn string_list(args: &Value, key: &str) -> Vec<String> {
    match &args[key] {
        Value::Array(a) => a
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        Value::String(s) if !s.trim().is_empty() => s
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use puerperium::convert::ConvertConfig;
    use puerperium::memory::MemoryRecord;

    fn face() -> (tempfile::TempDir, Face) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let paths = Paths::new(dir.path());
        paths.ensure().expect("ensure");
        (dir, Face { paths })
    }

    fn sample_memories() -> Vec<MemoryRecord> {
        let doc = "DEPLOY REFERENCE\n\n## Building\n\nAlways build on the target board; an x86 \
                   binary gives Exec format error, which reads like a corrupt file rather than \
                   a wrong architecture.\n";
        vec![MemoryRecord {
            id: "m1".into(),
            content: doc.into(),
            memory_type: MemoryType::Procedural,
            tags: vec!["deploy".into()],
            agent_id: Some("FORGE".into()),
            salience: 0.9,
        }]
    }

    fn write_sample(face: &Face, name: &str) {
        let out = convert(&sample_memories(), &ConvertConfig::new());
        dataset::write(
            &face.paths.datasets(),
            name,
            &out,
            SourceSpec {
                kind: "export_file".into(),
                query: Some("test".into()),
                agent_id: None,
                memories_in: 1,
            },
        )
        .expect("write dataset");
    }

    #[test]
    fn list_datasets_empty_is_valid() {
        let (_d, face) = face();
        let v = face.call("nursery_list_datasets", &json!({})).expect("ok");
        assert_eq!(v["count"], 0);
        assert_eq!(v["datasets"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn generate_dry_run_writes_nothing() {
        let (_d, face) = face();
        let dir = tempfile::TempDir::new().expect("mems");
        let path = dir.path().join("mem.json");
        std::fs::write(&path, serde_json::to_vec(&sample_memories()).unwrap()).unwrap();

        let v = face
            .call(
                "nursery_generate_data",
                &json!({
                    "name": "set-a",
                    "from": path.display().to_string(),
                }),
            )
            .expect("ok");
        assert_eq!(v["dry_run"], true);
        assert_eq!(v["written"], false);
        assert!(v["examples"].as_u64().unwrap() >= 1);
        assert!(dataset::list(&face.paths.datasets()).unwrap().is_empty());
    }

    #[test]
    fn generate_can_write_when_dry_run_is_false() {
        let (_d, face) = face();
        let dir = tempfile::TempDir::new().expect("mems");
        let path = dir.path().join("mem.json");
        std::fs::write(&path, serde_json::to_vec(&sample_memories()).unwrap()).unwrap();

        let v = face
            .call(
                "nursery_generate_data",
                &json!({
                    "name": "set-a",
                    "from": path.display().to_string(),
                    "dry_run": false,
                }),
            )
            .expect("ok");
        assert_eq!(v["written"], true);
        assert_eq!(v["sha256"].as_str().unwrap().len(), 64);
    }

    #[test]
    fn synthetic_generate_refuses() {
        let (_d, face) = face();
        let err = face
            .call(
                "nursery_generate_data",
                &json!({ "name": "x", "synthetic": true }),
            )
            .expect_err("refuse");
        assert!(matches!(err, CallError::Refused { .. }));
        assert!(err.to_string().contains("synthetic"));
    }

    #[test]
    fn train_without_confirm_is_a_labelled_dry_run() {
        let (_d, face) = face();
        write_sample(&face, "set-a");
        let v = face
            .call(
                "nursery_train",
                &json!({
                    "id": "j1",
                    "dataset": "set-a",
                    "output_name": "worker-v1",
                    "training_file_id": "file-abc",
                }),
            )
            .expect("dry-run ok");
        assert_eq!(v["dry_run"], true);
        assert_eq!(v["unresolved"], true);
        assert!(v["note"].as_str().unwrap().contains("UNRESOLVED"));
    }

    #[test]
    fn train_dry_run_false_without_confirm_refuses() {
        let (_d, face) = face();
        write_sample(&face, "set-a");
        let err = face
            .call(
                "nursery_train",
                &json!({
                    "id": "j1",
                    "dataset": "set-a",
                    "output_name": "worker-v1",
                    "training_file_id": "file-abc",
                    "dry_run": false,
                }),
            )
            .expect_err("refuse");
        assert!(matches!(err, CallError::Refused { .. }));
        assert!(err.to_string().contains("confirm"));
    }

    #[test]
    fn test_model_refuses_honestly() {
        let (_d, face) = face();
        let err = face
            .call(
                "nursery_test_model",
                &json!({ "model": "worker-v1", "prompt": "hello" }),
            )
            .expect_err("refuse");
        assert!(matches!(err, CallError::Refused { .. }));
        assert!(err.to_string().contains("Watcher"));
    }

    #[test]
    fn unknown_tool_is_named() {
        let (_d, face) = face();
        let err = face
            .call("nursery_rebirth_score", &json!({}))
            .expect_err("no");
        assert!(matches!(err, CallError::UnknownTool(_)));
    }

    #[test]
    fn inspect_and_estimate_need_a_real_dataset() {
        let (_d, face) = face();
        write_sample(&face, "set-a");
        let inspect = face
            .call("nursery_inspect_dataset", &json!({ "name": "set-a" }))
            .expect("inspect");
        assert_eq!(inspect["meta"]["name"], "set-a");
        assert!(!inspect["examples"].as_array().unwrap().is_empty());

        let est = face
            .call("nursery_estimate_cost", &json!({ "dataset": "set-a" }))
            .expect("estimate");
        assert!(est["caveats"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| { c.as_str().unwrap().contains("MINIMUM CHARGE") }));
    }

    #[test]
    fn untrained_apprentice_lineage_says_there_is_nothing_to_walk() {
        let (_d, face) = face();
        let out = convert(&sample_memories(), &ConvertConfig::new());
        dataset::write(
            &face.paths.datasets(),
            "ap-data",
            &out,
            SourceSpec {
                kind: "cerebro_query".into(),
                query: Some("ops".into()),
                agent_id: Some("FORGE".into()),
                memories_in: 1,
            },
        )
        .unwrap();
        apprentice::create(
            &face.paths,
            Spec {
                id: "ap1".into(),
                master_agent: "FORGE".into(),
                name: "hand".into(),
                specialization: "ops".into(),
                base_model: "Qwen/Qwen3.6-27B".into(),
                dataset_name: "ap-data2".into(),
            },
            &sample_memories(),
            &ConvertConfig::new(),
        )
        .unwrap();

        let v = face
            .call("nursery_lineage", &json!({ "name": "ap1" }))
            .expect("lineage");
        assert_eq!(v["trained"], false);
        assert!(v["note"].as_str().unwrap().contains("no model"));
    }
}
