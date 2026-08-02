# Nursery Port Overview — ApexOS-RS + ApexRouter-RS

**Goal**: Port the ApexAurum `nursery.py` ("Where new minds are cultivated") into the pure-Rust ApexOS-RS ecosystem, using ApexRouter-RS as the canonical compute provider for local / Vast.ai / Together AI training. This closes a practical baby-RSI loop when Qwen3.6-27B (or its fine-tuned descendants) drives ApexOS.

## Source

- **Origin**: https://github.com/buckster123/ApexAurum/blob/master/tools/nursery.py + NURSERY_INTEGRATION_PLAN.md
- **Core capabilities**:
  - **Data Garden**: `nursery_generate_data`, `nursery_extract_conversations`, `nursery_list_datasets`
  - **Training Forge**: `nursery_estimate_cost`, `nursery_train_cloud` (Together primary; Vast/RunPod secondary), `nursery_train_local`, `nursery_job_status`, `nursery_list_jobs`
  - **Model Cradle**: `nursery_list_models`, `nursery_deploy_ollama` (or local GGUF / Router alias), `nursery_test_model`, `nursery_compare_models`
  - **Apprentice Protocol**: `nursery_create_apprentice` (master agent → specialized LoRA from Village/Cerebro knowledge) + listing
- **Integration**: Village Protocol event posts (dataset_created, training_started/complete, model_deployed, apprentice_created); agent attribution; local registry under `sandbox/nursery/`

## Targets

1. **ApexOS-RS** (https://github.com/buckster123/ApexOS-RS)
   - New tool plugin(s) under `tools/crates/` or `agentd` tool registry.
   - Uses Cerebro for knowledge → training data conversion and event logging (Village analog).
   - Agent-callable via existing tool schema / MCP-like surface.
   - Storage: `$STATE/nursery/` or `~/.local/state/apexos/nursery/` (datasets/, models/, jobs.jsonl, registry.json).

2. **ApexRouter-RS** (https://github.com/buckster123/ApexRouter-RS)
   - Extend `apexrouter-providers` + control plane for **training jobs**.
   - Together AI: fine-tune API (LoRA SFT already priced ~$1.50/M for 17-69B band; supports Qwen3.6-27B).
   - Vast.ai: new SearchProfiles + Recipes for training containers (Unsloth / Axolotl / custom for Qwen3.6-27B QLoRA).
   - Local: supervise training processes (Unsloth CLI or Python subprocess, or future pure-Rust trainers).
   - Ledger + SpendApproval already exist — reuse for training spend.
   - Expose via control API + MCP tools so ApexOS Nursery can call "provision training compute + launch job".

## Baby-RSI Loop (when Qwen3.6-27B drives ApexOS)

```
Qwen3.6-27B (or apprentice) in agentd
    ↓ tool call
Nursery (ApexOS-RS)
    ↓ estimate / create dataset from Cerebro
    ↓ request training job
ApexRouter-RS (provider)
    ↓ provision (local | Together fine-tune | Vast Unsloth container)
    ↓ monitor + ledger
    ↓ return adapter / GGUF / HF repo
Nursery
    ↓ register + deploy as Router alias or Ollama
    ↓ Cerebro event + lineage
Agent can now use improved specialist → repeat
```

Stage-1 is deliberately "baby" RSI: specialized apprentices + model registry + self-driven data/training.  
**Full recursive weight rewrite** of the driver model itself is defined in the Stage-2+ addendum (`07-FULL-REBIRTH-AND-RSI-VISION.md`) with hard safety rails (Model Watcher, previous_good, promote/rollback).

## Deliverables in this artifact set

| File | Purpose |
|------|---------|
| 00-OVERVIEW.md | This file |
| 01-NURSERY-AURUM-ANALYSIS.md | Detailed analysis of Python source + integration plan |
| 02-APEXOS-RS-NURSERY-PLAN.md | Port plan, crate layout, tool schemas, Cerebro integration, scaffolding |
| 03-APEXROUTER-RS-TRAINING-PLAN.md | Extension plan for training providers, Vast recipes, Together fine-tune, local supervision |
| 04-INTEGRATION-AND-RSI.md | End-to-end flow, agent tool surface, lineage, safety, baby-RSI checklist |
| 05-SCAFFOLDING.md | Concrete Rust structs, trait sketches, example tool handlers, config TOML, job state machine |
| 06-IMPLEMENTATION-CHECKLIST.md | Ordered tasks for autonomous agents (grok-build / Claude Code style) |
| 07-FULL-REBIRTH-AND-RSI-VISION.md | **Addendum**: Full weight rewrite / rebirth path, Model Watcher, thresholds, rollbacks, future vision |

## Constraints & Style Alignment

- Pure Rust, no Python runtime required at runtime (training may still shell out to Unsloth/Python on the provisioned machine).
- Follow ApexOS-RS patterns: tools as crates, event bus, Cerebro MCP, agent attribution.
- Follow ApexRouter-RS patterns: ledger-first, SpendApproval, no auto-destroy of paid resources, fit solver, ArcSwap routing, state in `$STATE`.
- Safety: training jobs are expensive; require explicit approval for cloud spend > threshold; track trainer_agent; never overwrite production aliases without confirmation.
- Qwen3.6-27B is a first-class target (dense 27B, excellent coding agentic scores, fits QLoRA on 24-32 GB).

## Success Criteria

- Agent in ApexOS-RS can call `nursery_generate_data` / `nursery_create_apprentice` / `nursery_train_cloud`.
- Jobs appear in Router ledger and can be polled.
- Completed adapters are registered and routable via ApexRouter (or local Ollama).
- Cerebro records the lineage event.
- Full loop works with Qwen3.6-27B as the driving model (local or via Router).
