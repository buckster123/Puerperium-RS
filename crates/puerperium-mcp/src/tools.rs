//! The advertised `nursery_*` surface (D9). Every name is always listed (D8).

use serde_json::{json, Value};

/// Stable order. A missing name here is a missing capability the agent will invent.
pub const TOOL_NAMES: &[&str] = &[
    "nursery_generate_data",
    "nursery_list_datasets",
    "nursery_inspect_dataset",
    "nursery_estimate_cost",
    "nursery_quote",
    "nursery_upload",
    "nursery_train",
    "nursery_job_status",
    "nursery_list_jobs",
    "nursery_cancel_job",
    "nursery_list_models",
    "nursery_register_model",
    "nursery_test_model",
    "nursery_create_apprentice",
    "nursery_list_apprentices",
    "nursery_lineage",
];

pub fn all_tool_schemas() -> Vec<Value> {
    TOOL_NAMES.iter().map(|&name| tool_schema(name)).collect()
}

fn tool_schema(name: &str) -> Value {
    match name {
        "nursery_generate_data" => json!({
            "name": name,
            "description": "Build an instruction dataset from a Cerebro snapshot (--db) or a memories JSON export (--from). Writes JSONL + sidecar, hashes the file. dry_run (default true) reports the conversion and writes nothing. Synthetic templates are not built — that path refuses honestly.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Dataset name. Immutable once written." },
                    "db": { "type": "string", "description": "Cerebro snapshot path. Opened read-only; prefer a .backup, not a live db." },
                    "from": { "type": "string", "description": "Path to a JSON array of MemoryRecord (Cerebro export)." },
                    "agent_id": { "type": "string", "description": "Whose memory space to mine when using db. Not the trainer (D6)." },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Keep memories carrying any of these tags." },
                    "limit": { "type": "integer", "description": "Cap memories, highest salience first." },
                    "domain": { "type": "string", "description": "Optional domain for the tag-fallback instruction, e.g. ApexOS." },
                    "include_dream": { "type": "boolean", "description": "Admit dream-engine memories. Off by default — they are abstractions, not lived experience." },
                    "include_types": { "type": "array", "items": { "type": "string" }, "description": "Memory types. Default procedural, semantic, schematic." },
                    "dry_run": { "type": "boolean", "description": "Report only; write nothing. Default true." }
                },
                "required": ["name"]
            }
        }),
        "nursery_list_datasets" => json!({
            "name": name,
            "description": "Datasets with example counts, source kind, sha256, creation time. Empty is valid.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        "nursery_inspect_dataset" => json!({
            "name": name,
            "description": "Sidecar (provenance histogram, hash, rejections) plus the first N examples.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "head": { "type": "integer", "description": "Examples to show (default 3)." }
                },
                "required": ["name"]
            }
        }),
        "nursery_estimate_cost" => json!({
            "name": name,
            "description": "Local heuristic for a dataset × size × epochs. FREE. Ignores Together's minimum charge — do not use this for a spend decision. Use nursery_quote on an uploaded file for the real number.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "dataset": { "type": "string" },
                    "params_b": { "type": "number", "description": "Base parameter count in billions (default 35)." },
                    "epochs": { "type": "integer", "description": "Default 3." }
                },
                "required": ["dataset"]
            }
        }),
        "nursery_quote" => json!({
            "name": name,
            "description": "Together's own POST /v1/fine-tunes/estimate-price. FREE and authoritative — includes the minimum charge. Needs TOGETHER_API_KEY and a training_file_id from nursery_upload.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "training_file_id": { "type": "string" },
                    "base_model": { "type": "string", "description": "Default $PUERPERIUM_DEFAULT_BASE or Qwen/Qwen3.6-35B-A3B." },
                    "epochs": { "type": "integer", "description": "Default 3." },
                    "lora_r": { "type": "integer", "description": "Default 16." },
                    "lora_alpha": { "type": "integer", "description": "Default 32." },
                    "params_b": { "type": "number", "description": "For the metered-vs-floor note only. Default 35." }
                },
                "required": ["training_file_id"]
            }
        }),
        "nursery_upload" => json!({
            "name": name,
            "description": "Project a dataset to the provider schema and upload it. Returns training_file_id and binds it to the dataset hash. Costs nothing. Needs TOGETHER_API_KEY.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "dataset": { "type": "string" }
                },
                "required": ["dataset"]
            }
        }),
        "nursery_train" => json!({
            "name": name,
            "description": "Submit a LoRA job. Never spends unless confirm is true (D4). dry_run (default true) prints the unresolved body and contacts nothing. Requires a training_file_id from nursery_upload.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Job id. Yours; reused ids with a live provider id are refused." },
                    "dataset": { "type": "string" },
                    "output_name": { "type": "string" },
                    "training_file_id": { "type": "string" },
                    "base_model": { "type": "string" },
                    "trainer_agent": { "type": "string", "description": "Who ordered it. Never agent_id (D6). Default FORGE." },
                    "epochs": { "type": "integer" },
                    "lora_r": { "type": "integer" },
                    "lora_alpha": { "type": "integer" },
                    "compute": { "type": "string", "description": "Router-known node. Omit for Together (managed)." },
                    "available_compute": { "type": "array", "items": { "type": "string" } },
                    "dry_run": { "type": "boolean", "description": "Default true." },
                    "confirm": { "type": "boolean", "description": "Required to actually submit. Default false." }
                },
                "required": ["id", "dataset", "output_name", "training_file_id"]
            }
        }),
        "nursery_job_status" => json!({
            "name": name,
            "description": "One job: facts from the record, phase computed live (D3). Unreachable provider is Unknown, not Failed.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }
        }),
        "nursery_list_jobs" => json!({
            "name": name,
            "description": "All jobs, newest first, phases computed. Unreadable snapshots are reported, not hidden.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        "nursery_cancel_job" => json!({
            "name": name,
            "description": "Ask the upstream to stop. Records cancel_requested_at; does not write local Cancelled.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }
        }),
        "nursery_list_models" => json!({
            "name": name,
            "description": "Registered adapters: base, dataset hash, trainer, artifact. No liveness field — that is Router's truth.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        "nursery_register_model" => json!({
            "name": name,
            "description": "Hand a finished model to Router as a backend/alias. dry_run (default true) prints the bodies. confirm required to send. Records alias_requested, not liveness.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "model": { "type": "string" },
                    "alias": { "type": "string" },
                    "base_url": { "type": "string", "description": "Default https://api.together.xyz" },
                    "served_model": { "type": "string" },
                    "credential_env": { "type": "string", "description": "Env var name Router stores. Default TOGETHER_API_KEY." },
                    "dry_run": { "type": "boolean" },
                    "confirm": { "type": "boolean" }
                },
                "required": ["model", "alias"]
            }
        }),
        "nursery_test_model" => json!({
            "name": name,
            "description": "Send a prompt at a registered model. Present so the agent can see it (D8). Evaluation is the Watcher's job (Stage 2) — this verb currently refuses honestly rather than faking a score.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "model": { "type": "string" },
                    "prompt": { "type": "string" }
                },
                "required": ["model", "prompt"]
            }
        }),
        "nursery_create_apprentice" => json!({
            "name": name,
            "description": "Mine a Cerebro snapshot → dataset → untrained ApprenticeRecord. Never trains (D4). dry_run (default true) reports the conversion and writes nothing. Point db at a .backup snapshot.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "db": { "type": "string", "description": "Cerebro snapshot path (read-only)." },
                    "master_agent": { "type": "string", "description": "Whose memories to mine. Not the trainer." },
                    "name": { "type": "string" },
                    "specialization": { "type": "string" },
                    "dataset_name": { "type": "string" },
                    "base_model": { "type": "string", "description": "Default Qwen/Qwen3.6-27B (local/vast). Together training uses 35B-A3B." },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "limit": { "type": "integer" },
                    "domain": { "type": "string" },
                    "include_dream": { "type": "boolean" },
                    "include_types": { "type": "array", "items": { "type": "string" } },
                    "dry_run": { "type": "boolean" }
                },
                "required": ["id", "db", "master_agent", "name", "specialization", "dataset_name"]
            }
        }),
        "nursery_list_apprentices" => json!({
            "name": name,
            "description": "Apprentices with master, specialization, trained-or-not (derived from model.is_some()).",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        "nursery_lineage" => json!({
            "name": name,
            "description": "Walk a model (or a trained apprentice) back through datasets, jobs and ancestors. This is the product.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Model name, or apprentice id." }
                },
                "required": ["name"]
            }
        }),
        other => json!({
            "name": other,
            "description": "unknown tool — this is a bug in tools.rs",
            "inputSchema": { "type": "object", "properties": {} }
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_advertised_name_has_a_real_schema() {
        let schemas = all_tool_schemas();
        assert_eq!(schemas.len(), TOOL_NAMES.len());
        for (name, schema) in TOOL_NAMES.iter().zip(&schemas) {
            assert_eq!(schema["name"], *name);
            assert!(
                schema["description"].as_str().unwrap().len() > 20,
                "{name} needs a real description"
            );
            assert_eq!(schema["inputSchema"]["type"], "object");
            assert!(
                !schema["description"]
                    .as_str()
                    .unwrap()
                    .contains("unknown tool"),
                "{name} fell through to the catch-all"
            );
        }
    }

    #[test]
    fn the_surface_is_capability_named() {
        for name in TOOL_NAMES {
            assert!(
                name.starts_with("nursery_"),
                "{name} must be capability-named (D9)"
            );
        }
    }
}
