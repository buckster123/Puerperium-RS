# 03 — ApexRouter-RS Training Provider Extension Plan

## Current State (from ARCHITECTURE.md + README)

ApexRouter-RS already excels at **inference** provisioning:
- Local llama-server supervision + fit solver
- Vast.ai: search offers → rent → SSH tunnel → container launch (llama.cpp / vLLM)
- Together AI: managed OpenAI-compatible endpoint
- Ledger + SpendApproval + no auto-destroy
- Control plane + MCP + CLI

**Missing for Nursery**: a first-class **training job** abstraction that can:
1. Submit a fine-tune to Together AI (API).
2. Provision a Vast.ai instance with a training-oriented container (Unsloth / Axolotl / custom) and run a LoRA job.
3. Supervise a local training process.
4. Report progress / artifacts back to the caller (Nursery).
5. Register the resulting adapter as a new routable endpoint/alias.

## Recommended New Surface

### Control API additions

```
POST   /v1/training/jobs
GET    /v1/training/jobs
GET    /v1/training/jobs/{id}
POST   /v1/training/jobs/{id}/cancel
POST   /v1/training/jobs/{id}/artifacts/register   # turn finished adapter into a Router backend
GET    /v1/training/providers                   # capability matrix
```

Request body example (Together path):

```json
{
  "provider": "together",
  "base_model": "Qwen/Qwen3.6-27B",
  "dataset": { "type": "url", "url": "https://.../dataset.jsonl" },
  "method": "lora_sft",
  "hyperparams": {
    "n_epochs": 3,
    "learning_rate": 1e-5,
    "lora_rank": 16,
    "lora_alpha": 32
  },
  "budget_usd": 25.0,
  "trainer_agent": "AZOTH",
  "output_name": "azoth-tool-specialist-v1"
}
```

Vast path adds:

```json
{
  "provider": "vast",
  "recipe": "unsloth-qwen36-27b-qlora",
  "gpu_profile": "rtx4090-or-better",
  "max_dph": 0.8,
  ...
}
```

### Internal Modules

Extend `apexrouter-providers`:

```
providers/
├── together/
│   ├── inference.rs          # existing
│   └── fine_tune.rs          # NEW: create fine-tune job, poll, download adapter
├── vast/
│   ├── rent.rs               # existing
│   ├── tunnel.rs             # existing
│   └── training_recipe.rs    # NEW: launch training container, watch logs, pull artifacts
└── local/
    └── train_supervisor.rs   # NEW: spawn Unsloth / axolotl process, monitor
```

Reuse existing:
- Ledger (write `TrainingJobReserved` / `TrainingJobStarted` / `TrainingJobSucceeded` / cost rows **before** spend).
- SpendApproval gate.
- JobRecord state machine (already used for long-running provisioning).
- Fit solver (for local training VRAM checks).

## Together AI Fine-Tune Path (highest priority, lowest friction)

Together already prices LoRA SFT for 17–69B models (~$1.50 / 1M tokens). Qwen3.6-27B is supported.

Implementation sketch:
1. Upload dataset (or give Together a public URL / HF dataset).
2. Call Together fine-tune endpoint with LoRA config.
3. Poll job status.
4. On success, download adapter or obtain a Together-hosted endpoint; optionally convert to GGUF via local post-process.
5. Register as managed backend or local alias.

Cost is transparent and token-based — perfect for `nursery_estimate_cost`.

## Vast.ai Training Recipes

Define named recipes in config or code:

```toml
[[training.recipes]]
id = "unsloth-qwen36-27b-qlora"
image = "unsloth/unsloth:latest"          # or a pinned custom image
gpu_filters = ["RTX 4090", "RTX 5090", "A100", "H100"]
min_vram_gb = 24
onstart = """
  # download base model + dataset
  # run unsloth train script with injected env
  # on finish: write /workspace/adapter/ and signal done
"""
artifact_paths = ["/workspace/adapter", "/workspace/gguf"]
```

Flow:
1. Search offers with profile + VRAM filter.
2. SpendApproval + ledger write.
3. Rent + tunnel.
4. Launch container with recipe onstart + env (DATASET_URL, BASE_MODEL, LORA_RANK, EPOCHS, OUTPUT_NAME, ...).
5. Poll boot + training logs (via SSH or Vast API).
6. On completion, `scp` / rsync artifacts back to `$STATE/training/{job_id}/`.
7. Optionally destroy instance (explicit, never automatic) or keep for evaluation.
8. Register resulting adapter.

This reuses 90 % of the existing Vast rental + tunnel machinery.

## Local Training Supervisor

- Check hardware tier / available VRAM via existing rig snapshot.
- For models ≤ ~3–7B or heavily quantized QLoRA: allow.
- For Qwen3.6-27B QLoRA: require ≥ 24 GB (ideally 32 GB) and Unsloth.
- Spawn with `setsid` + ownership flock (same pattern as llama-server).
- Stream stdout/stderr into job log; update status.
- On finish, place adapter under `$STATE/models/` and optionally quantize.

## Cost Estimation Helper

Expose `POST /v1/training/estimate` that Nursery can call. Inputs: dataset size (tokens or MB), base_model, method (lora_sft / full / dpo), provider preference. Output: estimated tokens, hours, $ for each viable provider (reuse Together pricing tables + Vast offer sampling).

## Artifact Registration

Once a job succeeds:

```
POST /v1/training/jobs/{id}/artifacts/register
{
  "alias": "azoth-tool-specialist",
  "kind": "lora_adapter" | "gguf" | "hf_repo",
  "path_or_url": "...",
  "base_model": "Qwen/Qwen3.6-27B"
}
```

This creates a new backend entry and optional route, so the agent can immediately `chat` against the specialist via the usual OpenAI endpoint.

## Safety & Observability

- All cloud spend goes through SpendApproval + ledger (already battle-tested).
- Jobs are never auto-destroyed; human / agent must explicitly cancel or destroy.
- Trainer agent attribution is required and stored.
- Progress events can be pushed over the existing WebSocket control plane.
- Metrics: training tokens processed, wall time, final loss, adapter size.

## MCP / CLI Surface

Add tools so agents (and Nursery) can drive everything without raw HTTP:

- `apexrouter_training_estimate`
- `apexrouter_training_submit`
- `apexrouter_training_status`
- `apexrouter_training_list`
- `apexrouter_training_cancel`
- `apexrouter_training_register`

CLI mirrors the same verbs under `apexrouter training ...`.

## Implementation Order (for autonomous agents)

1. Together fine-tune client + estimate endpoint (lowest risk, high value for 27B).
2. Job state machine + persistence + control API skeleton.
3. Local train supervisor (Unsloth subprocess).
4. Vast training recipe + container launch path.
5. Artifact registration → routing table integration.
6. Full e2e with Qwen3.6-27B LoRA on a small synthetic set.
