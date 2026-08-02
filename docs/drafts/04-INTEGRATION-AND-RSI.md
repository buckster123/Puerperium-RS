# 04 — Integration, Safety & Baby-RSI Loop

## End-to-End Happy Path

1. Agent (driven by Qwen3.6-27B via ApexRouter or local) decides it needs a specialist.
2. Calls `nursery_create_apprentice(master_agent="AZOTH", apprentice_name="tool_forge", specialization="tool calling and code editing", base_model="Qwen/Qwen3.6-27B", auto_train=true)`.
3. Nursery:
   - Queries Cerebro for AZOTH-relevant knowledge / tool traces.
   - Converts → JSONL dataset under `$STATE/nursery/datasets/`.
   - Posts Cerebro event `dataset_created`.
   - Calls ApexRouter `POST /v1/training/jobs` (Together preferred for cost transparency, or Vast if custom Unsloth needed).
4. Router:
   - Estimates / checks budget.
   - SpendApproval (if required).
   - Ledger write.
   - Submits to Together or provisions Vast container.
   - Returns `job_id`.
5. Nursery records local job, posts `training_started`.
6. Agent (or background watcher) polls `nursery_job_status` → Router status.
7. On success:
   - Router can auto-register or Nursery calls `/artifacts/register`.
   - New alias appears (e.g. `tool_forge`).
   - Nursery writes ApprenticeRecord + Cerebro `apprentice_created` / `training_complete` with lineage.
8. Agent can now route specialist traffic to the new alias for tool-heavy sub-tasks.

## Data Flow Diagram (textual)

```
Cerebro (knowledge / tool traces)
        │
        ▼
Nursery Data Garden ──► dataset.jsonl
        │
        ▼
Nursery Training Forge ──► ApexRouter /v1/training/jobs
        │                         │
        │                    (Together | Vast | Local)
        │                         │
        │                         ▼
        │                    adapter / GGUF
        │                         │
        ▼                         ▼
Model Cradle + Registry ◄── register alias
        │
        ▼
Cerebro event + lineage
        │
        ▼
Agent can use improved specialist
```

## Safety Rules (must be enforced)

- Cloud jobs require either:
  - Explicit `budget_usd` + SpendApproval, or
  - A pre-granted nursery training allowance.
- Never overwrite a live production alias without an extra confirmation flag.
- Trainer agent attribution is mandatory and immutable.
- Local training on low-tier hardware is refused with a clear message (point to cloud).
- Artifacts are stored under `$STATE`; paid Vast instances are never auto-destroyed.
- Dataset and model paths are validated; no arbitrary code execution from untrusted datasets.

## Baby-RSI Characteristics

This is deliberately **scoped** recursive self-improvement:

| Capability | Status after port |
|------------|-------------------|
| Agent generates its own training data from experience | Yes (extract + synthetic) |
| Agent trains specialized descendants | Yes (Apprentice Protocol) |
| Lineage + attribution | Yes (Cerebro + registry) |
| Automatic deployment of improved models | Yes (Router alias) |
| Driver model (Qwen3.6-27B) can use the specialists it created | Yes |
| Full recursive rewrite of the 27B weights themselves | No (out of scope for v1; higher risk / cost) |
| Continuous online learning / RL | Future (can build on job system) |

When the driving model is itself a fine-tuned Qwen3.6-27B (or an apprentice that grew strong enough), the loop becomes "the system is improving the specialists that the system uses".

## Cerebro Event Taxonomy

Recommended event types / tags:

- `nursery.dataset_created`
- `nursery.training_started`
- `nursery.training_complete`
- `nursery.training_failed`
- `nursery.apprentice_created`
- `nursery.model_deployed`
- `nursery.model_registered`

All carry `trainer_agent`, `job_id`, `base_model`, `output_name`, optional metrics.

## UI / Observability Hooks

- Agent chat can surface job status as tool-call cards (already supported by tool block UI).
- Optional Slint dashboard card: active jobs, recent apprentices, spend this month.
- CLI: `apexos nursery status`, `apexrouter training list`.

## Failure Modes & Recovery

- Together job fails → Nursery marks failed, posts event, leaves dataset intact for retry.
- Vast instance stalls → existing diagnose / restart-download paths; Nursery can cancel.
- Local OOM → job fails cleanly; suggest lower rank / shorter seq / cloud.
- Network partition → jobs are durable in both Router ledger and Nursery jobs.jsonl; resume by polling.

## Versioning & Lineage

Every ApprenticeRecord and Model entry stores:

```json
{
  "lineage": ["AZOTH", "tool_forge-v1", "tool_forge-v2"],
  "parent_job_id": "...",
  "dataset_hash": "sha256:...",
  "base_model": "Qwen/Qwen3.6-27B",
  "created_by": "AZOTH"
}
```

This enables future "which specialist is currently best for tool calling?" queries and A/B via `nursery_compare_models`.
