# 05 — Scaffolding & Code Sketches

Autonomous implementers can copy-paste / adapt these.

## 1. Nursery Crate Cargo.toml (ApexOS-RS)

```toml
[package]
name = "apexos-nursery"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4", "serde"] }
thiserror = "1"
anyhow = "1"
reqwest = { version = "0.12", features = ["json"] }
tracing = "0.1"
# link to apexos-core / cerebro client as needed
```

## 2. Core Error & Config

```rust
#[derive(thiserror::Error, Debug)]
pub enum NurseryError {
    #[error("dataset not found: {0}")]
    DatasetNotFound(String),
    #[error("job not found: {0}")]
    JobNotFound(String),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("insufficient hardware for local train")]
    InsufficientHardware,
    #[error("spend approval required")]
    SpendApprovalRequired,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, NurseryError>;
```

## 3. Job & Apprentice Structs

(See 02-APEXOS-RS-NURSERY-PLAN.md for full sketches; expand with `serde` derives and `Default`.)

## 4. Tool Handler Skeleton (agentd integration)

```rust
// In agentd tool registry
pub async fn handle_nursery_train_cloud(
    args: serde_json::Value,
    ctx: &ToolContext,          // contains agent_id, cerebro handle, router client
) -> Result<serde_json::Value> {
    let dataset_name = args["dataset_name"].as_str().ok_or(...)?;
    let base_model = args["base_model"].as_str().unwrap_or("Qwen/Qwen3.6-27B");
    let output_name = args["output_name"].as_str().ok_or(...)?;
    let provider = args["provider"].as_str().unwrap_or("together");
    let epochs = args["epochs"].as_u64().unwrap_or(3) as u32;
    let lora_rank = args["lora_rank"].as_u64().unwrap_or(16) as u32;
    let trainer_agent = ctx.agent_id.clone().unwrap_or_else(|| "NURSERY_KEEPER".into());

    // 1. locate dataset
    // 2. call router client
    let router_resp = ctx.router
        .post_training_job(TrainingJobRequest { ... })
        .await?;

    // 3. persist local job
    // 4. cerebro event
    // 5. return { success, job_id, message, ... }
}
```

## 5. ApexRouter Training Request / Response Types

```rust
// in apexrouter-protocol or providers
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TrainingJobRequest {
    pub provider: String,               // "together" | "vast" | "local"
    pub base_model: String,
    pub dataset: DatasetSpec,           // Path | Url | Inline
    pub method: TrainingMethod,         // LoraSft | FullSft | Dpo
    pub hyperparams: TrainingHyperparams,
    pub budget_usd: Option<f64>,
    pub trainer_agent: String,
    pub output_name: String,
    pub recipe: Option<String>,         // for Vast
    pub gpu_profile: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TrainingJobStatus {
    pub id: String,
    pub status: JobPhase,               // Pending | Provisioning | Running | Succeeded | Failed
    pub provider: String,
    pub progress_pct: Option<f32>,
    pub metrics: Option<serde_json::Value>,
    pub artifact_path: Option<String>,
    pub error: Option<String>,
    pub ledger_entry_ids: Vec<String>,
}
```

## 6. Config Snippet (ApexRouter)

```toml
[training]
enabled = true
default_provider = "together"
max_budget_usd_per_job = 50.0
require_approval_above_usd = 5.0

[training.together]
api_key_env = "TOGETHER_API_KEY"
# pricing tables can live in code or here

[[training.recipes]]
id = "unsloth-qwen36-27b-qlora"
description = "QLoRA fine-tune of Qwen3.6-27B via Unsloth"
image = "ghcr.io/unslothai/unsloth:latest"
min_vram_gb = 24
gpu_filters = ["RTX 4090", "RTX 5090", "A100 SXM", "H100"]
# onstart script template with placeholders
```

## 7. Synthetic Data Generator (minimal Rust port)

For v1, keep generation simple (template-based) or call the driving LLM via Router to expand examples. Full Python SyntheticGenerator logic can be reimplemented later or kept as an optional sidecar.

## 8. Cerebro Event Helper

```rust
async fn post_nursery_event(
    cerebro: &CerebroClient,
    event_type: &str,
    content: &str,
    agent_id: &str,
    metadata: serde_json::Value,
) -> Result<()> {
    // map to existing session_recall / knowledge write APIs
    // tag with "nursery" + event_type
}
```

## 9. Example End-to-End Test Outline

```rust
#[tokio::test]
async fn e2e_small_local_train() {
    // 1. generate tiny dataset
    // 2. submit local job for TinyLlama or Qwen-0.8B
    // 3. poll until Succeeded
    // 4. assert adapter dir exists
    // 5. register alias
    // 6. chat against alias and check response shape
}
```
