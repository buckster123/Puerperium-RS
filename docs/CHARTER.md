# Puerperium-RS — charter

> **The decisions log below is BINDING.** Amend it with a dated entry; never silently.
> Where this document and the code disagree, one of them is a bug — say which.
> Where a later doc and D1–D13 disagree, **D1–D13 win**.

## What this is

A standalone Rust **nursery for models**: it mines an agent's own remembered experience into
training data, orchestrates LoRA fine-tuning jobs, keeps a registry of the resulting adapters
with full lineage, and gates every promotion behind evaluation.

*Puerperium* is the period of care after a birth. Cerebro remembers, Occipital reads,
Imaginarium sees, Sonus hears — **Puerperium raises**.

The loop it closes, deliberately scoped as "baby" RSI: an agent notices it needs a specialist,
generates the dataset from what it already knows, trains a descendant, records the lineage, and
routes work to it. The system improves the specialists the system uses. Rewriting the driver
model's own weights is Stage 2 — designed (`rebirth.md`), not built (D10).

## What it is not

The scope fence. Each line has a reason, so a future session can tell whether a proposed
feature is in or out without re-arguing it.

- **Not an inference server.** ApexRouter-RS serves models; Puerperium makes them. Puerperium
  never binds a proxy, never routes chat traffic, never holds a routing table.
- **Not a GPU rental manager.** Renting, tunnelling and ledgering a paid box are ApexRouter's
  verbs, under ApexRouter's `SpendApproval`. Puerperium never calls a vast.ai endpoint that
  creates, modifies or destroys an instance (D4).
- **Not a quantiser.** We do not run `llama-quantize`. If a GGUF is wanted it is produced on
  the training box as part of that job's recipe, or not at all. (Inherited from ApexRouter's
  own fence — the garden should not grow two quantisers.)
- **Not a Python runtime.** Training shells out to Unsloth/Axolotl **on the provisioned box**.
  No Python in Puerperium's own runtime path, ever.
- **Not an ApexOS-RS subsystem.** Standalone is a first-class goal (D1). Whether any of this is
  eventually assimilated into ApexOS-RS is **that repo's decision, in its own thread** — not
  assumed here.
- **Not an autonomous spender.** No default flow reaches a paid operation (D4).
- **Not a request-path harvester.** Session persistence is ApexOS's; optional body
  capture is ApexRouter's. Puerperium converts snapshots (D13).

## Decisions

- **D1 — Standalone, four-face, sibling-not-subsystem.** One Cargo workspace: `puerperium`
  (lib) · `puerperium-mcp` (agent face) · `puerperium-cli` (human face). A `-api` face is
  deferred, not designed out. ApexOS-RS consumes via MCP with **zero ApexOS-RS changes**.
  *Reason:* the proven Occipital/Imaginarium/Sonus/Callosum pattern; it keeps assimilation a
  later, evidence-based decision instead of a bootstrap assumption.

- **D2 — Puerperium owns the training-job lifecycle; ApexRouter is used only through the
  surfaces it already has.** Router rents the box, tunnels it, ledgers the dollar, and serves
  inference against the result. Puerperium owns dataset → job → adapter → registry → lineage.
  *Reason:* ApexRouter's `CHARTER.md` fences this out in its binding log — *"Not a model zoo,
  **not a training tool**, not a quantiser."* Respecting that fence lets Puerperium ship
  standalone with no cross-repo charter amendment. Its `GARDEN.md` supplies the mechanism: a
  rented box tunnelled to `127.0.0.1:88xx` is already the sanctioned pattern.

- **D3 — Persisted records hold facts, never status.** A job record stores the provider job id,
  submission time, ledger references, dataset hash, argv. Phase is **computed on read** by
  asking the provider. No `status: "running"` string is ever written to disk.
  *Reason:* adopted verbatim from ApexRouter invariant 3 — a persisted status is a lie the
  moment the box dies, and these records outlive boxes by design.

- **D4 — Puerperium never initiates spend.** It refuses to start a job whose compute does not
  already exist, naming what is missing. It never calls a mutating vast.ai endpoint. Live
  training runs are André's explicit keystroke, counted.
  *Reason:* the garden's money rule. The failure mode — a GPU billing overnight with no local
  record — has already happened once in this ecosystem.

- **D5 — Hermetic tests.** No test connects anywhere but `127.0.0.x`. Upstream response parsers
  are tested against **captured fixture JSON**, never a live call. Effectful tests skip loudly.
  *Reason:* ApexRouter's suite once made live authenticated calls to `api.together.ai` with the
  real key. Inherit the fix, not the lesson.

- **D6 — `trainer_agent` is a distinct field, never `agent_id`.** Attribution travels in its own
  key, like `target_agent_id` elsewhere.
  *Reason:* under agentd, `dispatch_tool` **overwrites** `agent_id` on every Cerebro call — the
  model cannot supply it. Attribution carried in `agent_id` would silently become whatever
  identity stamped the call, which is worse than no attribution because it looks correct.

- **D7 — Nano-first refusal, not Nano-first execution.** Training is inherently a Pro-tier act.
  On a small node Puerperium still runs fully — registry, datasets, lineage, job status — and
  refuses *training* with an honest message naming the missing capability.
  *Reason:* the house Nano rule applied honestly. The right behaviour on a Pi Zero is a working
  nursery that cannot train, not an absent binary.

- **D8 — Tools are always present; gates return honest refusals.** No dynamic tool-list hiding,
  ever, at any stage.
  *Reason:* a tool that silently vanishes is context divergence — precisely what ApexOS-RS's
  welfare seams exist to prevent. A refusal that explains itself is cheaper than a capability
  the agent cannot see and will confabulate around.

- **D9 — Tools are capability-named (`nursery_*`), not repo-prefixed.**
  *Reason:* house convention — Cerebro exposes `remember`/`recall`, Occipital exposes
  `web_search`/`web_fetch`. The repo is Puerperium; the capability is the nursery.

- **D10 — Stage 2 (Rebirth) is design-frozen, not built.** `rebirth.md` is the R0 freeze; R1–R6
  sit in `BACKLOG.md` post-v1 parking. No rebirth tool ships in v1.
  *Reason:* the full-weight-rewrite path has to exist on paper for the Stage-1 architecture to
  be shaped right — but shipping its tool surface before the Watcher exists would be exactly the
  unvalidated-weight-update gamble the design is meant to prevent.

- **D11 — Promotion requires a `previous_good` that has been verified restorable**, not merely
  retained.
  *Reason:* ApexOS-RS's H4 snapshot gate — `update_system_prompt` refuses to apply unless the
  undo exists *and* is durably persisted first, because a node lived the near-miss of rewriting
  its identity with no recoverable undo. Weights deserve the same gate.

- **D12 — Datasets are provenance-stamped and content-hashed.** Every example records where it
  came from; every dataset gets a `sha256`; every model record names the dataset hash it was
  trained on.
  *Reason:* lineage *is* the product. An unattributable dataset produces an unattributable
  model, and the registry stops being able to answer "why is this specialist like this?"

- **D13 — Harvest is licensed, opted-in, and never a Cerebro dump.** Puerperium converts
  snapshots; it does not sit on the request path. Taps live in ApexOS (session owner)
  and, later, ApexRouter (optional capture) — those changes are those repos' threads
  (D1). Three license classes (`open_reasoning` / `closed_hidden` / `answer_only`) are
  an explicit model-id allowlist, never guessed: closed-API chain-of-thought may stay
  in a live session file for provider replay and is **stripped** from markdown, RAG,
  and training examples. Traces do not become Cerebro memories — Cerebro is the lesson
  store; a mined trace is product input and is provenance-stamped (D12). A mine reads
  a copied export, same discipline as `sqlite3 … ".backup"`. Secret-scan before
  anything leaves the box. Long form: `docs/harvest.md`.
  *Reason:* the first Together run trained on remembered lessons and came up thin;
  dream extraction had already abstracted the specifics. The lived turns sit in
  ApexOS session JSONL. Dumping those into Cerebro would feed the dream engine the
  thing we just measured as poison, and training on Anthropic/OpenAI hidden reasoning
  because it happens to sit in JSONL for API replay would be a ToS and honesty failure.

## Phases

v1 ships the Stage-1 loop. Each gate is checkable, not aspirational.

| Slice | Scope | Done when |
|-------|-------|-----------|
| **S0** | Bootstrap: charter, contract, workspace, CI | clippy `-D warnings` clean; `design.md` pins the tool surface |
| **S1** | Dataset garden: Cerebro → JSONL, synthetic templates, provenance + hash | `nursery_generate_data` writes a hashed, provenance-stamped dataset; pure convert fns unit-tested |
| **S2** | Registry: datasets, models, apprentices, lineage (facts, not status) | `nursery_list_*` return valid empty *and* populated shapes; CRUD round-trips under `tempfile` |
| **S3** | Job lifecycle: submit → poll → artifact, Together path first | a fixture-driven job reaches a computed `Succeeded` and lands an artifact path; no network in tests |
| **S4** | Apprentice protocol: knowledge → dataset → train → record | `nursery_create_apprentice` produces a lineage-complete record from a real Cerebro query |
| **S5** | Deploy + lineage: register the adapter through Router's existing endpoints | a trained adapter is reachable as a Router alias; Cerebro carries the lineage event |
| **S6** | Field: one real adapter, end to end | a specialist trained from FORGE's own memory beats its base on a real task — measured, not asserted |

## Deliberately out of v1

**Permanently out**

- Inference serving, routing, proxying — ApexRouter's job, forever.
- Renting/destroying paid compute — ApexRouter's job, under its ledger.
- Quantisation in Puerperium's own process.
- Python anywhere in Puerperium's runtime path.
- EEG, and anything else inherited by association from the ApexAurum original.

**Out of v1, honestly deferred**

- **Vast training recipes** — the container/artifact-pull path is designed but needs a live paid
  box, which is André's keystroke (D4). Together goes first: an API call with no provisioning
  exercises the whole lifecycle for the least risk.
- **Local training supervisor** — wants fit-solver/VRAM gate work, and the laptop (24 GB
  unified, shared iGPU) cannot train a 27B anyway, so it would ship untested.
- **Model Watcher v1** — the eval battery is the safety heart and deserves building well rather
  than early. Stage-1 apprentices are additive and rollback-free; the Watcher becomes
  load-bearing when weights are.
- **`puerperium-api` face and any UI** — no consumer yet.
- **DPO/ORPO/RFT, continued pre-training, colony-level evolution** — Stage 2+.

## Open questions

- ~~**Does Together accept `Qwen/Qwen3.6-27B` as a fine-tune base?**~~ **Answered 2026-08-03:
  no — Together does not carry that model at all.** Read from their live catalogue (270 models,
  via ApexRouter's `together` backend): the Qwen3.6 entries are `Qwen3.6-Plus`,
  `Qwen3.6-35B-A3B-FP8` and `Qwen3.6-35B-A3B-Lora`. **The Together LoRA base is
  `Qwen/Qwen3.6-35B-A3B`** — the MoE, which `GARDEN.md` calls the throughput-first alternate.
  The dense 27B remains right for the *local/vast* path (the garden node serves
  `Qwen3.6-27B-Q6_K.gguf` today), so **the base differs by provider** and neither default is
  wrong everywhere. `puerperium compute` now lists the LoRA-capable bases each backend
  advertises, so the choice is checkable for free before anything is submitted.
- ~~**Hosting is a second, ongoing charge.**~~ **Confirmed concretely 2026-08-03.** A tuned
  adapter is **not serverless**: calling it returns *"Unable to access non-serverless model …
  create and start a new dedicated endpoint"*. So evaluating a specialist costs **hourly**,
  independent of the one-off training charge. Two consequences: the estimator must never imply
  it covers serving, and **the S6 measurement itself has a running cost** — which makes it an
  explicit, counted act rather than a step the pipeline takes on its own.
  Still open: whether to quote N days of hosting alongside training, or only name it.
- **Artifact handoff shape.** Does Puerperium register the adapter with Router itself, or hand
  back a path and let the operator route it? Leaning: hand back, then offer registration as a
  separate explicit verb — it keeps D2's boundary clean.
- ~~**Does `nursery_extract_conversations` survive?**~~ **Collapsed 2026-08-16.** Not a
  separate verb. Session JSONL is a source kind inside `nursery_generate_data`
  (`session_jsonl`); Cerebro episodes stay narrative. See D13 and `docs/harvest.md`.

---

## Amendments

- **2026-08-16** — **Harvest is licensed, opted-in, and never a Cerebro dump (D13).**
  The first paid run trained on Cerebro lessons and came up thin; the lived turns
  already sit in ApexOS session JSONL. This amendment pins the snapshot contract
  (`docs/harvest.md`) and retires the open question about
  `nursery_extract_conversations`. No converter, RAG index, or sibling-repo tap
  ships from this entry — those are later slices in the repo that owns the tap.
  D1–D12 stand; D4/D5/D8 apply to harvest the same way they apply to training.
  Router `capture_bodies` stays a dead knob until *their* charter says otherwise.

- **2026-08-16** — **`job download` is the missing link, and it shipped.** A finished
  Together job is now `GET /v1/finetune/download?checkpoint=adapter` (never omit
  `checkpoint` — the API default is `merged`), a `.tar.zst` extract with a path-escape
  guard, and epoch-mean loss from `trainer_state.json`. Recovery via `--provider-job-id`
  when there is no local record. No decision reversed; the 2026-08-03 serving amendment
  is now implementable.

- **2026-08-03** — **Together is a TRAINING provider only; serving happens on vast or local
  compute.** Their dedicated endpoints are priced for the hardware they run on (B200 / dual
  H100 class), which is absurd overkill for a small LoRA adapter. Parked until further notice.
  *Consequence the pipeline has to answer:* a Together fine-tune currently leaves its artifact
  as a **model name hosted on their side**, not as weights we hold. Serving on vast/local means
  the adapter must be **downloaded** — `job download` is now the missing link between a
  finished job and a servable model, and it is the top of the backlog. Until it exists, a
  completed job produces something we can name in lineage but cannot yet run.

- **2026-08-03** — **spend estimates come from the provider, not from us** (S6 finding). The
  first shipped job metered $0.076 of tokens and was billed **$4.00**: a minimum charge no
  token-based local heuristic can see, on top of a 3.6x token overestimate. `job quote` now
  calls Together's free `estimate-price` endpoint, and the local `estimate` says loudly that it
  ignores the floor. No decision reversed; D4 is *strengthened* — "spend is gated" is only
  meaningful if the number shown to the operator is the real one.

- **2026-08-03** — **the fine-tune base differs by provider** (S5 finding, resolves an open
  question). Together carries no dense `Qwen3.6-27B`; its LoRA-capable Qwen3.6 base is
  `Qwen/Qwen3.6-35B-A3B`. The local/vast path keeps the dense 27B. No decision reversed —
  D1–D12 stand — but any doc or default that read "the base model" as a single global value
  was wrong, and `compute` now surfaces the per-backend truth.

- **2026-08-02** — charter adopted. Supersedes `drafts/` (Grok, 2026-08-02) on two structural
  points: the nursery is a **standalone sibling**, not a crate inside ApexOS-RS (D1); and the
  training-job lifecycle lives **here**, not in an extended ApexRouter (D2 — after finding that
  ApexRouter's binding charter fences training out). Drafts retained as provenance; see
  `drafts/README.md`.
