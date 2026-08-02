# 02 — ApexOS-RS Nursery Port Plan

## Placement in Workspace

Recommended layout (follow existing `tools/crates/` and `agentd` patterns):

```
ApexOS-RS/
├── tools/
│   └── crates/
│       └── nursery/                  # new crate
│           ├── Cargo.toml
│           ├── src/
│           │   ├── lib.rs
│           │   ├── data_garden.rs    # generate / extract / list
│           │   ├── training_forge.rs # estimate / train_* / job_*
│           │   ├── model_cradle.rs   # list / deploy / test / compare
│           │   ├── apprentice.rs     # create / list
│           │   ├── registry.rs       # local model + job persistence
│           │   ├── cerebro_bridge.rs # event posting + knowledge→data
│           │   └── schema.rs         # tool JSON schemas for agentd
│           └── tests/
├── agentd/
│   └── ... (register nursery tools in tool registry / MCP surface)
└── docs/
    └── nursery.md                    # user + agent docs
```

Alternative: if tools are flatter, put under `agentd/crates/tools-nursery` and re-export.

## Core Responsibilities of the `nursery` Crate

1. **Dataset lifecycle** — generate synthetic tool-use data (template or LLM-assisted via agentd), extract from Cerebro conversation history / tool traces, list, version.
2. **Job orchestration** — estimate cost (delegate to Router or local heuristics), submit training requests to ApexRouter-RS control plane, poll status, record local job history.
3. **Model & Apprentice registry** — local JSONL / SQLite under `$STATE/nursery/`, lineage (master_agent → apprentice → base_model → dataset), deploy hooks that call Router `route set` / `swap` or Ollama.
4. **Cerebro integration** — every significant event becomes a Cerebro memory item with tags `nursery`, `training_started`, `apprentice_created`, etc., plus agent_id attribution. Knowledge → training data converter for the Apprentice Protocol.
5. **Agent tool surface** — pure functions returning `serde_json::Value` (or typed Result structs) that match the original schema style so agentd can register them identically.

## Key Types (sketch)

```rust
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NurseryJob {
    pub job_id: String,
    pub provider: ProviderKind,       // Local | Together | Vast | ...
    pub dataset: String,
    pub base_model: String,
    pub output_name: String,
    pub status: JobStatus,            // Pending | Running | Succeeded | Failed | Cancelled
    pub trainer_agent: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub config: TrainingConfig,
    pub router_job_id: Option<String>, // ID returned by ApexRouter
    pub metrics: Option<TrainingMetrics>,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ApprenticeRecord {
    pub id: String,
    pub master_agent: String,
    pub name: String,
    pub specialization: String,
    pub base_model: String,
    pub dataset_id: Option<String>,
    pub model_path_or_alias: Option<String>,
    pub lineage: Vec<String>,
    pub trained: bool,
    pub created_at: DateTime<Utc>,
}
```

## Tool Registration

Mirror the Python schemas exactly where possible so existing prompts / agent skills keep working. Register via agentd's tool registry (see existing tool crates for pattern). Prefer typed handlers that return `Result<Value, NurseryError>`.

Example surface (agent-visible names stay the same):

- `nursery_generate_data`
- `nursery_extract_conversations`
- `nursery_list_datasets`
- `nursery_estimate_cost`
- `nursery_train_cloud`
- `nursery_train_local`
- `nursery_job_status`
- `nursery_list_jobs`
- `nursery_list_models`
- `nursery_deploy` (generalize beyond Ollama → Router alias + optional Ollama)
- `nursery_test_model`
- `nursery_create_apprentice`
- `nursery_list_apprentices`

## Storage Layout

```
$STATE/nursery/   # or ~/.local/state/apexos/nursery/
├── datasets/
│   └── {name}.jsonl
├── models/
│   ├── {output_name}/          # adapter files or HF snapshot
│   └── apprentices/
│       └── {id}.json
├── jobs.jsonl                  # append-only job history
└── registry.json               # quick index of models + aliases
```

Use `atomicwrites` or ApexOS existing state helpers. Jobs should also be mirrored into Cerebro for searchability.

## Cerebro Bridge

- On every event: call Cerebro MCP (or internal API) with agent_id = trainer / NURSERY_KEEPER.
- For Apprentice Protocol: `session_recall` or knowledge search filtered by master_agent + specialization → convert to sharegpt / alpaca style JSONL.
- Reuse existing Cerebro agent "FORGE" patterns where appropriate, or introduce NURSERY_KEEPER as a first-class agent_id.

## Local Training Path

- Prefer spawning a supervised process (Unsloth CLI or a small Python driver) rather than embedding full PEFT in Rust for v1.
- On capable hardware (Pro tier), allow direct training; on Nano/Micro, refuse with clear error and suggest cloud.
- Progress: write status to job record + optional websocket / event bus updates for UI.

## Cloud Path (via ApexRouter)

Nursery never talks to Together / Vast directly. It calls ApexRouter control API:

```
POST /v1/training/jobs
{
  "provider": "together" | "vast" | "local",
  "base_model": "Qwen/Qwen3.6-27B",
  "dataset_url_or_path": "...",
  "config": { "epochs": 3, "lora_rank": 16, ... },
  "budget_usd": 50.0,
  "trainer_agent": "AZOTH"
}
```

Router handles provisioning, ledger, approval, monitoring, and returns a job handle. Nursery polls `/v1/training/jobs/{id}` or receives webhook / WS events.

## UI / Observability (post-v1 or parallel)

- Slint panel or home-dashboard card for active Nursery jobs + recent apprentices.
- Lineage graph (Mermaid or simple tree) in settings or a dedicated window.

## Testing Strategy

- Unit: pure data generation, cost estimation heuristics, registry CRUD.
- Integration: mock ApexRouter control plane; real Cerebro for event posting.
- E2E: small TinyLlama or Qwen-0.8B local train on a 1k-example synthetic set.
- Safety: refuse jobs without SpendApproval when cloud; never overwrite live production alias.
