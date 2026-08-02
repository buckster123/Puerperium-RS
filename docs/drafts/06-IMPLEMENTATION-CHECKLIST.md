# 06 — Implementation Checklist for Autonomous Agents

Ordered, independently testable steps. Prefer small PRs. Update CLAUDE.md / docs in the same change.

## Phase 0 — Prep (both repos)

- [ ] Clone / open ApexOS-RS and ApexRouter-RS workspaces.
- [ ] Confirm current tool registration pattern in ApexOS-RS agentd.
- [ ] Confirm control-plane + ledger + Vast + Together patterns in ApexRouter-RS.
- [ ] Create feature branches: `feat/nursery` and `feat/training-jobs`.

## Phase 1 — ApexRouter-RS: Training Job Skeleton (highest leverage)

- [ ] Add `TrainingJobRequest` / `TrainingJobStatus` / `JobPhase` to protocol crate.
- [ ] Implement in-memory + durable (`$STATE/training/jobs.jsonl`) job store.
- [ ] Control API: `POST /v1/training/jobs`, `GET /v1/training/jobs`, `GET /v1/training/jobs/{id}`.
- [ ] Stub provider handler that returns "not implemented" for now.
- [ ] Ledger entries for training job lifecycle.
- [ ] MCP tools + CLI verbs for the above.
- [ ] Unit tests for state machine + serialization.
- [ ] Docs: update ARCHITECTURE.md + API.md.

**Exit**: `curl` can create and list a Pending job.

## Phase 2 — ApexRouter-RS: Together Fine-Tune Provider

- [ ] Together fine-tune client (upload dataset or URL, create LoRA job, poll).
- [ ] Map Together job states → JobPhase.
- [ ] Cost estimation helper using published $/1M rates (17-69B band).
- [ ] `POST /v1/training/estimate`.
- [ ] SpendApproval integration for jobs > threshold.
- [ ] On success: download adapter or note hosted endpoint; write artifact path.
- [ ] E2E test against real Together (small model + tiny dataset) or recorded fixtures.

**Exit**: Real (or mocked) Together LoRA job can be submitted and reaches Succeeded.

## Phase 3 — ApexOS-RS: Nursery Crate Skeleton

- [ ] Create `tools/crates/nursery` (or equivalent).
- [ ] Implement registry (datasets, jobs, models, apprentices) with atomic persistence under `$STATE/nursery/`.
- [ ] Implement `nursery_list_*` and pure data helpers.
- [ ] Tool schemas + registration in agentd.
- [ ] Cerebro event posting bridge (even if no-op initially).
- [ ] Unit tests for registry CRUD.

**Exit**: Agent can call `nursery_list_datasets` / `nursery_list_jobs` and get empty-but-valid responses.

## Phase 4 — ApexOS-RS ↔ Router Wiring

- [ ] Router client in Nursery (reqwest or existing NodeClient).
- [ ] `nursery_estimate_cost` → Router estimate endpoint.
- [ ] `nursery_train_cloud` → Router POST /v1/training/jobs (Together path).
- [ ] `nursery_job_status` / `list_jobs` → poll + merge local + remote state.
- [ ] Propagate trainer_agent and post Cerebro events.
- [ ] Integration test with mocked Router.

**Exit**: Full cloud train request from agent tool call reaches Router and is visible in both systems.

## Phase 5 — Local Training Path

- [ ] ApexRouter local train supervisor (Unsloth subprocess or equivalent).
- [ ] Hardware / VRAM gate (refuse Qwen3.6-27B on <24 GB).
- [ ] Nursery `nursery_train_local` wired to Router local provider.
- [ ] Progress log capture.
- [ ] Small-model e2e on developer machine.

## Phase 6 — Vast Training Recipe

- [ ] Define at least one recipe (`unsloth-qwen36-27b-qlora`).
- [ ] Extend Vast launch path for training containers + artifact pull.
- [ ] Wire into job system.
- [ ] Safety: explicit destroy only.
- [ ] Costly e2e only after SpendApproval tests pass.

## Phase 7 — Model Cradle + Apprentice Protocol

- [ ] `nursery_create_apprentice`: Cerebro knowledge → dataset → train (local or cloud).
- [ ] Lineage recording.
- [ ] `nursery_deploy` / artifact register → Router alias.
- [ ] `nursery_list_models`, `nursery_test_model`, `nursery_compare_models` (basic).
- [ ] Cerebro events for apprentice_created / model_deployed.

## Phase 8 — Polish & Baby-RSI Demo

- [ ] End-to-end script/demo: Qwen3.6-27B agent creates a small specialist, trains it (Together or local), deploys alias, uses it for a tool-heavy sub-task.
- [ ] UI card or tool-call visualization for jobs (optional).
- [ ] Docs: nursery.md, update CLAUDE.md, README mentions.
- [ ] Safety review: no auto-spend, no silent overwrite of aliases.
- [ ] Release notes.

## Notes for Autonomous Agents

- Prefer Together path first — it is the fastest way to get real 27B LoRA results without managing containers.
- Keep Python out of the runtime path; only shell out to Unsloth on the provisioned machine if needed.
- Always write ledger / job record **before** any paid API call.
- When in doubt, mirror existing Vast rental + tunnel patterns rather than inventing new lifecycle code.
- Test with tiny datasets and small base models until the job machine is solid; then scale to Qwen3.6-27B.
- Update both repos' documentation in the same PR that lands the code.

This checklist is intentionally sequential so each phase leaves the system in a working, testable state.
