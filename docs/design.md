# Puerperium-RS — the contract

> **Contract first** (house doctrine #1). This document is pinned **before** the code it
> describes. Code follows this doc; a PR that changes behaviour updates this doc in the same
> commit. When the two disagree, that is a bug in one of them — find out which, don't guess.
>
> Bound by `CHARTER.md` D1–D13. Where this doc and the charter disagree, the charter wins.

## Scope

Covers: the `nursery_*` MCP tool surface, the record types and their serialized form, the job
lifecycle, the compute-discovery contract with ApexRouter, the Cerebro mining contract, the
session-JSONL harvest contract (D13 / `harvest.md`), and the environment.

Does not cover: Stage 2 / rebirth (`rebirth.md`, design-frozen), inference serving, GPU
provisioning, or quantisation — all fenced out by the charter. Request-path harvest
(ApexOS persist, Router `capture_bodies`) is specified in `harvest.md` and built in
those repos, not here.

---

## Tool surface (MCP)

Capability-named per D9. Every tool is always present; a tool that cannot act returns an honest
refusal naming what is missing (D8), never an absence and never a fake success.

### Data garden

| Tool | Purpose |
|---|---|
| `nursery_generate_data` | Build an instruction dataset. Source is a **Cerebro snapshot** (`db` + `agent_id`, opened read-only) or a **memories JSON export** (`from`). A third exclusive source — **exported ApexOS session JSONL** (`sessions`) — is contracted in `harvest.md` (D13) and **not wired**; asking for it is an honest refusal until that slice. Writes JSONL, stamps provenance per example, hashes the file (D12). `dry_run` defaults **true** (datasets are immutable). **Synthetic templates are not built** — that path refuses honestly rather than inventing examples. |
| `nursery_list_datasets` | Datasets with example counts, source kind, `sha256`, creation time. |
| `nursery_inspect_dataset` | First N examples plus the provenance histogram — *where did this data come from?* answered without opening the file. |

`nursery_extract_conversations` from the Python original is **not** a separate verb.
Session JSONL is source kind `session_jsonl` inside `nursery_generate_data` (D13 /
`harvest.md`). Cerebro episodes stay narrative — they are not trajectories.

### Training forge

| Tool | Purpose |
|---|---|
| `nursery_estimate_cost` | Local heuristic for a dataset × size × epochs. **FREE.** Ignores Together's minimum charge — do not use this for a spend decision. Honest `base_not_supported` rather than a guessed number. |
| `nursery_quote` | Together's own `POST /v1/fine-tunes/estimate-price`. **FREE and authoritative** — includes the minimum charge. Needs `TOGETHER_API_KEY` and a `training_file_id` from `nursery_upload`. |
| `nursery_upload` | Project a dataset to the provider schema and upload it. Returns `training_file_id` and binds it to the dataset hash. Costs nothing. Needs `TOGETHER_API_KEY`. |
| `nursery_train` | Submit a LoRA job. **Never spends unless `confirm` is true** (D4). `dry_run` defaults true and prints the unresolved body (labelled as such). Requires a `training_file_id` from `nursery_upload`. Compute, if named, must **already exist**. Returns a job id immediately; never blocks. |
| `nursery_job_status` | One job: facts from the record, phase computed live from the provider (D3). Unreachable provider is `Unknown`, not `Failed`. |
| `nursery_list_jobs` | All jobs, newest first, phases computed. Unreadable snapshots are reported, not hidden. |
| `nursery_cancel_job` | Ask the upstream to stop. Records `cancel_requested_at`; does not write local `Cancelled`. |
| `nursery_download` | Pull a finished Together adapter home (`.tar.zst`) and read `trainer_state.json`. **FREE.** Default checkpoint is `adapter`. Omitting `checkpoint` on Together's wire fetches `merged` (the full model) — we never omit it. Compare **epoch means**, never first-step vs last-step. |

### Model cradle

| Tool | Purpose |
|---|---|
| `nursery_list_models` | Registered adapters: base, dataset hash, trainer, artifact path, lineage. |
| `nursery_register_model` | The **explicit** handoff verb: take a finished artifact and register it as a Router backend/alias. Separate from training on purpose — it keeps D2's boundary visible. `dry_run` defaults true; live needs `confirm: true`. Records `alias_requested`, never liveness (D3). |
| `nursery_test_model` | Present so the agent can see it (D8). Evaluation is the Watcher's job (Stage 2) — this verb currently **refuses honestly** rather than faking a score or starting a dedicated endpoint. |

### Apprentice protocol

| Tool | Purpose |
|---|---|
| `nursery_create_apprentice` | The headline verb: master agent + specialization → Cerebro snapshot (read-only) → dataset → untrained `ApprenticeRecord`. Composes the three groups above; adds no new capability of its own. **Never trains** (D4). `dry_run` defaults true. |
| `nursery_list_apprentices` | Apprentices with master, specialization, trained-or-not, lineage. |
| `nursery_lineage` | Trace any model or apprentice back through datasets, jobs and ancestors to the memories it came from. **This is the product** — everything else exists so this can answer. |

**Parked for Stage 2:** `nursery_compare_models` (evaluation belongs with the Watcher) and every
`nursery_rebirth_*` verb (D10).

---

## Types

Serialized shapes are load-bearing — a representation change must be proven equivalent on the
wire.

```rust
/// A job record. FACTS ONLY (D3) — no phase, no status string, ever.
pub struct JobRecord {
    pub id: JobId,
    pub provider: Provider,              // Together | Vast | Local
    pub provider_job_id: Option<String>, // None until the upstream accepts it
    pub dataset: DatasetRef,             // name + sha256 — the hash is the real identity
    pub base_model: String,
    pub output_name: String,
    pub method: Method,                  // LoraSft (v1); FullSft/Dpo are Stage 2
    pub hyperparams: Hyperparams,
    pub trainer_agent: String,           // D6 — NEVER named agent_id
    pub compute: ComputeRef,             // what it ran on, and who provisioned it
    pub submitted_at: DateTime<Utc>,
    pub terminal: Option<Terminal>,      // written ONCE, when observed terminal
    pub cancel_requested_at: Option<DateTime<Utc>>, // the ask; not an outcome
    pub total_price_nanodollars: Option<u64>, // Together; nano-dollars
    pub ledger_refs: Vec<String>,        // ApexRouter ledger rows, for cost attribution
}

/// The only status ever persisted: an observed, immutable end state.
pub struct Terminal {
    pub outcome: Outcome,                // Succeeded | Failed | Cancelled
    pub observed_at: DateTime<Utc>,
    pub artifact: Option<PathBuf>,
    pub error: Option<String>,           // the real reason, never a generic
}

/// Computed on read, never stored.
pub enum Phase {
    Submitted, Provisioning, Running,
    Cancelling,                          // cancel asked for, upstream still working
    Succeeded, Failed, Cancelled,
    Unknown,
}
```

`Phase::Unknown` is a first-class, honest answer: the provider was unreachable **or returned a
state we do not recognise**, so we do not know. It is never silently rendered as `Running`.
Upstreams add states; a parser that guesses turns an unknown into a confident lie.

`Cancelling` was added at S3 (contract amended): Together has a real `cancel_requested` state,
and collapsing it into `Running` would tell an operator their cancel had not registered.

### Submitting: the API applies no defaults

The request body must carry Together's **full** parameter set — every field the SDK defaults
client-side. Omission is read as zero, not as "use the default". `submit` first calls the free
`GET /v1/fine-tunes/models/limits?model_name=…` and resolves against it:

- `batch_size` — `"max"` is a *client-side* token; an omitted value is filled with the
  model's published max. An **explicit** value outside `min..=max` is refused, never clamped.
- `lora_r` / `n_epochs` — refused when they exceed the published max, never silently reduced
- `lora_trainable_modules` — the model's own `target_modules`, not `"all-linear"`

That call is also the honest base check: a model that is not fine-tunable answers with a
`message` instead of limits, for free, before anything is submitted.

### Together status mapping

Taken from Together's own SDK (`FinetuneJobStatus`), not from prose docs:

| Upstream | Phase | Terminal? |
|---|---|---|
| `pending`, `queued` | `Submitted` | no |
| `running`, `compressing`, `uploading` | `Running` | no |
| `cancel_requested` | `Cancelling` | no |
| `completed` | `Succeeded` | **yes** |
| `error`, `user_error` | `Failed` | **yes** |
| `cancelled` | `Cancelled` | **yes** |
| anything else | `Unknown` | no |

`error` and `user_error` both map to `Failed`, but the **distinction is preserved in the
reason** — "your dataset was rejected" and "our trainer fell over" call for different actions.

```rust
pub struct ApprenticeRecord {
    pub id: String,
    pub master_agent: String,
    pub name: String,
    pub specialization: String,
    pub base_model: String,
    pub dataset: Option<DatasetRef>,
    pub job_id: Option<JobId>,
    pub artifact: Option<PathBuf>,
    pub lineage: Vec<LineageEdge>,       // ancestors, datasets, jobs
    pub created_at: DateTime<Utc>,
}

/// Every example carries where it came from (D12).
pub struct Example {
    pub messages: Vec<Message>,          // sharegpt-style
    pub provenance: Provenance,          // CerebroMemory | Synthetic | SessionTurn (D13)
}

/// What produced a dataset. `kind` is a closed string set, not free prose.
pub struct SourceSpec {
    pub kind: String,                    // cerebro_query | export_file | synthetic
                                         // | session_jsonl (D13, not wired)
                                         // | router_capture (parked)
    pub query: Option<String>,
    pub agent_id: Option<String>,        // whose space — not the trainer (D6)
    pub memories_in: usize,              // memories or rounds, matching kind
}

/// Origin of one example (D12). SessionTurn is contracted (D13), not in the crate yet.
pub enum Provenance {
    CerebroMemory { memory_id: String, agent_id: Option<String>, heading_path: Vec<String> },
    Synthetic { template: String },
    SessionTurn {
        node_id: String,
        session_id: u64,
        turn_index: u32,
        agent_id: Option<String>,
        license_class: LicenseClass,     // open_reasoning | closed_hidden | answer_only
        model: Option<String>,
    },
}
```

---

## Registry contract (S2)

Datasets, models, apprentices, and the lineage that joins them. **Facts only** (D3).

### What a record may not contain

The rule is sharper for models than for jobs, and it is worth stating explicitly because it
is easy to get wrong:

> **A `ModelRecord` never stores whether the model is deployed, live, or serving.**

Whether an alias actually answers is **ApexRouter's truth**, not ours — it depends on a
process we do not supervise, on a box we did not rent (D2/D4). A `deployed: true` on disk is
a lie the moment Router restarts, the tunnel drops, or the box is parked. What the record
stores is what Puerperium *did*: the artifact path, the alias it **requested**, the dataset
hash, the trainer, the parent. Liveness is **computed** by asking Router, and until that
client exists (S5) the honest answer is that we do not know.

Same shape elsewhere: an `ApprenticeRecord` has no `trained: bool` — that is `model.is_some()`,
derived on read. Any boolean that restates another field is a chance for the two to disagree.

### Records

```rust
/// A registered adapter. FACTS ONLY — no liveness, no deployment status.
pub struct ModelRecord {
    pub name: String,                    // registry key and Router alias candidate
    pub base_model: String,
    pub dataset: Option<DatasetRef>,     // name + sha256 — the hash is the identity
    pub job_id: Option<String>,
    pub trainer_agent: String,           // D6 — never `agent_id`
    pub artifact: Option<PathBuf>,
    pub parent: Option<String>,          // the model this one was trained from
    pub alias_requested: Option<String>, // what we asked Router for; NOT proof it is live
    pub created_at: DateTime<Utc>,
}

pub struct ApprenticeRecord {
    pub id: String,
    pub master_agent: String,            // whose knowledge it was raised on
    pub name: String,
    pub specialization: String,
    pub base_model: String,
    pub dataset: Option<DatasetRef>,
    pub job_id: Option<String>,
    pub model: Option<String>,           // the ModelRecord once trained; None = not yet
    pub created_at: DateTime<Utc>,
}
```

### Lineage — the product

`nursery_lineage` walks a model back through its ancestors, and at each generation names the
dataset (by hash) and how many memories fed it. Everything else in the registry exists so
this can answer *"why is this specialist like this?"*.

```rust
pub struct Lineage {
    pub entries: Vec<LineageEntry>,      // generation 0 = the model asked about
    /// Why the walk stopped early. `None` means it reached a root honestly.
    pub incomplete: Option<String>,
}
```

**Degrades honestly rather than erroring.** A dataset referenced by a record may have been
deleted; a parent may be missing; a hand-edited file may create a parent cycle. None of those
are errors — the walk records what it found, marks what it could not resolve, and stops.
A lineage that silently omits a broken link is worse than one that says the link is broken.
The cycle guard is not optional: records are plain JSON on disk and nothing stops a human
from pointing two models at each other.

### Storage

```
datasets/<name>.jsonl + <name>.meta.json
models/<name>.json                       # the record
models/<name>/                           # artifacts, when we hold them
apprentices/<id>.json
```

All records: atomic `tmp → fsync → rename`, name-validated against path escape, and
**every new field `#[serde(default)]`** — the same durability rule datasets already earned.

---

## Job lifecycle

```
nursery_train
   │  refuse unless compute exists (D4) ─────────► honest refusal, nothing written
   ▼
write JobRecord (facts, no phase)  ◄── before any upstream call
   ▼
submit upstream ──► store provider_job_id
   ▼
[ phase is computed from here on — poll the provider on every read ]
   ▼
observed terminal ──► write Terminal ONCE (immutable)
```

**Rules:**

- The record is written **before** the upstream call. A crash between write and submit leaves a
  job with no `provider_job_id` — recoverable and visible, which is the point.
- A job id is unique once it has a `provider_job_id` or a terminal. Resubmitting would
  last-write-wins the fold and orphan a paid run. A crash-row (id exists, neither set) is
  retried in place with the original facts; a different spec under the same id is refused.
- A poll timeout does **not** fail the job. A paid run that outlives our patience is still
  running; it stays non-terminal and resumable by `provider_job_id` (doctrine #9).
- `Terminal` is written once. A terminal job is never re-polled.
- Cancel records `cancel_requested_at` (a fact about the ask) and does **not** write local
  `Cancelled` — only an observed upstream `cancelled` is an outcome.
- Unreadable snapshots are **reported**, never silently dropped. A schema bump that fails to
  default must not hide a paid run. Every new `JobRecord` / `Hyperparams` field is
  `#[serde(default)]`.
- A rejected submit writes a terminal record **and** the CLI exits non-zero. The record is
  the fact; the exit status is the operator signal.
- HTTP 408 / 429 / 5xx are **Unreachable**, not Rejected — a rate-limit or trainer blip
  must not stop polling a job that is still billing.
- `training_file_id` is bound to the projected dataset hash at upload. Submit refuses an
  unbound or mismatched file — the record names a dataset, the upstream trains on a file,
  and those two must be the same bytes.
- `--dry-run` prints the *unresolved* body and says so. Resolving needs the limits
  endpoint; dry-run contacts nothing.
- Every failure path carries the real reason. No job can sit non-terminal *because* nothing
  looked at it — `nursery_list_jobs` polls.

### Storage: append-only

`jobs.jsonl` is **append-only**; each mutation appends a full record snapshot and current state
is a **fold by id, last write wins**. Datasets and registry records are single files, but jobs
are the money-adjacent ones — this mirrors ApexRouter's `ledger.jsonl`, where "active" is a
query rather than a stored flag. The progression (submitted → provider id assigned → terminal)
stays legible after the fact, and no rewrite can lose the fact that a job was ever submitted.

### The compute gate

`ComputeRef` distinguishes what needs provisioning from what does not:

- **`Managed`** — Together. A hosted API call; no box, so nothing to check. This is why the
  Together path lands first (S3): it exercises the whole lifecycle without touching D4's
  spend surface.
- **`Node { name }`** — a Router-known backend or tunnel, for the vast/local paths. `submit`
  **refuses before writing anything** when the named compute does not already exist, listing
  what does and naming the `apexrouter` verb that would create more. Puerperium never creates
  compute (D4).

---

## Export contract (what actually gets uploaded)

The stored dataset and the uploaded file are **different artifacts**, deliberately.

Together's validator rejects unknown columns. Our stored JSONL carries `provenance` and
`instruction_kind` beside `messages` (D12), so it would be refused verbatim.
`export::to_provider_jsonl` projects each line down to the accepted shape:

```
stored    {"messages":[…],"provenance":{…},"instruction_kind":"templated_heading"}
uploaded  {"messages":[{"role":"user","content":…},{"role":"assistant","content":…}]}
```

The stored file is never mutated, so its `sha256` — the lineage identity — is unaffected, and
the projection is reproducible from it at any time. **Lineage references the stored hash, not
the uploaded bytes.**

Validation mirrors the upstream's own rules (roles, non-empty content, at least one assistant
turn) so a bad dataset fails **locally with a line number**, rather than costing a round trip
to be told "Found extra column" with no idea where.

### Upload flow (Together)

Three steps, from the SDK's upload manager:

1. `POST /files` with `purpose`/`file_name`/`file_type` → a redirect carrying the presigned URL
   in `Location` and the id in `X-Together-File-Id`.
2. `PUT` the bytes to the presigned URL — **unauthenticated**; the signature is the
   authorisation and the target is third-party storage.
3. `POST /files/{id}/preprocess` to finalise.

Redirects are **not followed** (`Policy::none()`): following step 1 automatically consumes both
headers and PUTs an empty body, an upload that silently uploads nothing.

## Compute contract (with ApexRouter)

Charter D2/D4. Puerperium **discovers** compute; it never creates it.

- Read-only against Router's control plane (`127.0.0.1:2739`) to enumerate what already exists —
  live backends, rented instances, tunnels.
- `nursery_train` requires a `compute` argument naming one of those, or refuses with the list of
  what *is* available and the exact `apexrouter` verb André would run to get more.
- **Puerperium never calls a vast.ai endpoint at all**, mutating or otherwise. Router owns that
  relationship, its ledger and its `SpendApproval`.
- Ledger rows produced by Router are referenced (`ledger_refs`), never duplicated. Router remains
  the single source of truth for what money happened.

The Together path has no compute prerequisite — it is a managed API call, which is why it lands
first (S3).

---

## Conversion contract (S1)

How a memory becomes training data. **Every rule here was set against the real store**
(349 memories, `~/.cerebro-cortex/cerebro.db`, read-only copy, 2026-08-02) — not from a guess
about what memories look like.

### What the store actually contains

| Type | n | avg chars | `##` sections | Verdict |
|---|---|---|---|---|
| episodic | 194 | 1443 | 0 | **off by default** — session narrative |
| semantic | 78 | 511 | 3 | on — facts and decisions |
| procedural | 59 | 1009 | 22 | **on — the best material** |
| prospective | 12 | 870 | 0 | off — intentions, not knowledge |
| affective | 3 | 539 | 0 | off |
| schematic | 3 | 478 | 0 | on — derived architecture insight |

**Episodic is excluded by default and this is deliberate.** It is the largest class, and it is
session history. Training on it teaches a model to recite what happened rather than to do the
work. `include_types` can override; the default may not.

### The quality gate (`filter.rs`)

The store holds **messages and chatter as well as knowledge** — A2A pings, smoke tests,
greetings. A live example, verbatim: *"Yo HERMES-KRKN! 👋 ... Just doing a first smoke test on
the A2A messaging system. Can you hear me?"* That is a real semantic memory and it must never
become an instruction pair.

A candidate is rejected if any holds. Every rejection is **counted by reason** and reported —
a filter that silently eats data is worse than no filter.

1. `content.len() < MIN_CONTENT` (default 120) — too short to teach anything.
2. Conversational-artifact markers: greeting openers on the first line, and direct-address
   phrases ("can you hear me", "testing 123", "first smoke test"). Deliberately **not**
   decisive on its own: the bare phrase "smoke test", which appears in perfectly good
   procedures ("run the smoke test before deploying").
3. Tag denylist: `a2a`, `msg`, `message`, `test`, `smoke`, `chatter`, `ping`, plus any
   **routing-prefixed** tag (`from:`, `to:`). A2A messages in the real store carry
   `msg`/`from:CLAUDE`/`to:HERMES-KRKN` — a bare `message` entry missed all of them.
4. `salience < MIN_SALIENCE` (default 0.3) — Cerebro already decided it was marginal.
5. Content that is mostly a single URL, or mostly non-prose punctuation.

The filter is **pure and unit-tested against real captured content**, including the greeting
above as a named regression case.

### Chunking (`chunk.rs`)

- **Markdown-sectioned content** (`##`/`###`, 22 of 59 procedural memories): one chunk per
  section, carrying its heading path (`Doc Title › ## Section › ### Subsection`). Sections
  under `MIN_CONTENT` merge forward into the next.
- **Document title**: the first line, when it is a short banner (≤120 chars, not a
  sentence) **or** a labelled banner (`PROCEDURE — …`, `ARCHITECTURE DECISION …`,
  `LABEL — rest` with the dash in the first 50 characters). FORGE's lived procedures
  write the title as a long sentence; those are titles, not prose openers. An em-dash
  used as list punctuation later in a how-to sentence is not.
- **Unsectioned content**: one chunk, whole. No paragraph splitting — a lesson split mid-thought
  produces two half-lessons, which is worse than one long one.
- Chunks over `MAX_CHUNK` (default 6000 chars) split at paragraph boundaries, never mid-line.

### Instruction synthesis (`instruct.rs`) — templates only in S1

Deterministic, free, no LLM. The instruction is derived from what the memory already carries:

| Heading shape | Instruction form |
|---|---|
| Short noun phrase (≤4 words, ≤40 chars) under a doc title | *"Explain Essential Flags, in the context of VLLM SERVING REFERENCE."* |
| **Statement** heading (longer, or containing `, `) | *"In PROCEDURE — Deploying vLLM to vast.ai, explain: Vast SSH-mode OVERRIDES Docker ENTRYPOINT"* |
| Title only, noun phrase | *"Explain Deploy procedure."* |
| Title only, statement | *"Explain: When integrating NPU with CC, use the zero-code approach"* |
| No heading trail | topical tags: *"What do you know about mesh and federation in ApexOS?"* |
| No heading trail, no topical tags | **`Unframeable`** — counted, never invented |

The statement/phrase split exists because real memories use whole claims as section headings.
Inlining one gives *"Explain Vast SSH-mode OVERRIDES Docker ENTRYPOINT, in the context of…"*,
which is gibberish. The detection is deliberately **conservative**: the clause form reads fine
for a noun phrase too, so a false positive costs verbosity while a false negative costs a
broken sentence.

**Tags used for framing must be topical.** Routing metadata (`from:`, `to:`), bare years
(`2024`, `2024-2026`) and record bookkeeping (`session-notes`, `completion-summary`, `status`,
`wip`…) are excluded — framing from those produced *"What do you know about phase-6,
completion-summary, and session-notes?"*, which is grammatical and empty.

The response is the chunk body, verbatim. **Puerperium never rewrites the knowledge** — it
frames it.

**Every example records which strategy framed it** (`templated_heading` vs `templated_tag`),
and the sidecar reports the split. On the real store the ratio is 112 : 70 — the tag-framed
third is materially weaker, and that fact is data the consumer can act on rather than a
footnote. `llm_assisted` joins the same enum when that pass lands.

**LLM-assisted question generation is deliberately deferred** (post-v1): it costs tokens, so it
is gated by D4, and template mode has to be the honest floor anyway — a dataset must be
buildable on a node with no key and no budget.

### Output

JSONL, one `Example` per line, sharegpt-style `messages`. Alongside it a `<name>.meta.json`:
`sha256` of the JSONL bytes, example count, memories used, source spec, **per-reason rejection
counts**, the **framing split**, and the tool version. The hash is the dataset's real identity
(D12) — `DatasetRef` carries it everywhere.

Datasets are **immutable**: rebuilding under an existing name is an error, not an overwrite,
because a job record pointing at a silently-changed dataset is a lineage lie.

**Every new sidecar field must be `#[serde(default)]`.** Datasets are durable and referenced by
hash for the life of the registry; adding `framing` without a default broke `data list` on every
previously-written dataset, and that regression now has a test.

Accounting is **total**: `memories_used + rejected == memories_in`, always. A memory can never
vanish silently.

---

## Apprentice protocol (S4)

`nursery_create_apprentice` **adds no capability**. It composes mine → convert → dataset →
record, and its whole value is that the composition leaves a traceable record.

**It stops before spending.** Creating an apprentice does *not* submit a training job. The
apprentice exists in an honestly untrained state; `attach_job` and `attach_model` record what
happens to it later, and `is_trained()` stays derived from `model.is_some()` (D3).

Refusals, each because the alternative looks like progress and is not:

| Situation | Why it is refused |
|---|---|
| Conversion yields no examples | an apprentice with no data behind it is worse than none |
| Apprentice id already exists | silently rebuilding would orphan the first dataset |
| `attach_model` names an unregistered model | a dangling reference breaks the lineage walk at the generation being asked about |

## Cerebro mining contract

- Read via the Cerebro MCP surface: `recall` / `memory_search` / `find_by_tags` /
  `get_thread_memories`, scoped by `agent_id` and tags.
- **`agent_id` selects whose memories to mine. `trainer_agent` records who ordered the
  training.** They are different fields and often different values (D6) — agentd stamps the
  former on every call, so overloading it would silently mislabel every dataset.
- Every mined example keeps `Provenance::CerebroMemory { id, agent_id }`. A dataset can always
  answer which memories produced it.
- Writing back: lineage events are stored as ordinary tagged memories
  (`nursery`, `nursery:<event>`, `job:<id>`). No new Cerebro schema, no Cerebro changes.
- **Traces do not become Cerebro memories** (D13). `session_save` is a summary. A mined
  session JSONL is product input, provenance-stamped, never a session note.

## Session-JSONL mining contract (D13)

Long form: `harvest.md`. Contracted here so the wire shapes cannot drift when the
converter lands.

- Source kind `session_jsonl`. Input is a **copied export** of ApexOS
  `session-*.jsonl`, never a live `AGENTD_LOG`. Exclusive with `db` / `from`.
- Unit is a **round** (user + following assistant/tool messages). Session 0, spawn
  ids, image-only rounds, and empty-assistant rounds are skipped and counted.
- `closed_hidden` thinking is stripped before an example is born. `open_reasoning`
  thinking follows the allowlist in `harvest.md`, empty until verified on the day
  of the first mine.
- Every example keeps `Provenance::SessionTurn`. Accounting is total:
  `rounds_in = used + rejected`.
- Secret scan is part of convert, **before** `job upload`. A key-shaped round is
  `rejected.secret`, never uploaded.
- Until the converter is built, `nursery_generate_data` refuses a sessions path
  honestly, naming this section. No new MCP verb.
- `router_capture` is parked until ApexRouter wires `capture_bodies` under its
  own charter.

---

## Storage

```
~/.local/share/puerperium/          # $PUERPERIUM_STATE_DIR overrides
├── datasets/<name>.jsonl           # + <name>.meta.json (sha256, provenance histogram)
├── models/<output_name>/           # adapter artifacts
├── apprentices/<id>.json
├── jobs.jsonl                      # append-only; facts only
├── uploads/<file-id>.json          # training_file_id → dataset hash
└── fixtures/                       # captured upstream JSON for hermetic tests (D5)
```

Atomic writes (`tmp → fsync → rename`). State directories are `0700`; dataset, job, and
registry files are `0600`. **Nothing is ever written into the repo directory.**

---

## Environment

| Var | Default | Purpose |
|-----|---------|---------|
| `PUERPERIUM_STATE_DIR` | `~/.local/share/puerperium` | state root |
| `PUERPERIUM_ROUTER_URL` | `http://127.0.0.1:2739` | Router control plane, **read-only use** |
| `PUERPERIUM_DEFAULT_BASE` | `Qwen/Qwen3.6-35B-A3B` | default Together fine-tune base |
| `PUERPERIUM_TRAINER_AGENT` | `FORGE` | fallback when a caller supplies none |
| `TOGETHER_API_KEY` | unset | required for the Together path; `from_env` loads the house env file |
| `RUST_LOG` | `info` | tracing filter — **stderr only**, stdout is JSON-RPC |

All knobs are env-only in v1; if a config file arrives later, its precedence gets stated here
before it ships.

---

## Invariants

Each of these is a future `gotchas.md` entry waiting to happen.

1. **No phase on disk.** If you are about to serialize a status string, stop (D3).
2. **The record precedes the upstream call.** Always.
3. **`trainer_agent` is never `agent_id`** (D6).
4. **`Phase::Unknown` is a real answer.** Never coerce it to something more confident.
5. **A poll timeout is not a failure** (doctrine #9).
6. **Puerperium never creates compute** (D4).
7. **stdout is JSON-RPC only.** All logging to stderr.
8. **Every example has provenance; every dataset has a hash** (D12).
9. **Harvest is a snapshot, never a live log, and never a Cerebro dump** (D13).

---

## Honest degrades

| Situation | Response |
|---|---|
| No `TOGETHER_API_KEY` | `no_key_configured` naming the env var — never a timeout |
| Router unreachable | `compute_unavailable`, saying why — never an empty list implying "none exist" |
| No compute for the requested provider | Refusal listing what *is* available + the `apexrouter` verb to get more |
| Base model unsupported by provider | `base_not_supported` with the provider's supported list — never a guessed estimate |
| Node too small to train | `insufficient_hardware` naming the requirement (D7). Every other verb still works. |
| Provider unreachable mid-job | `Phase::Unknown`, job stays non-terminal, resumable |

---

## Open questions

Tracked in `CHARTER.md`; restated here where they bind the contract.

- ~~**Does Together accept `Qwen/Qwen3.6-27B` as a fine-tune base?**~~ **No** (2026-08-03).
  The LoRA base is `Qwen/Qwen3.6-35B-A3B`. The dense 27B remains the local/vast default.
- **Hosting is a second, ongoing charge.** `nursery_estimate_cost` should return training and
  hosting as separate labelled figures. A single blended number would be the dishonest kind of
  simplification.
- **Dataset format**: sharegpt-style `messages` assumed. Confirm against Together's expected
  schema at S3 and pin it here.
- ~~**Does `nursery_extract_conversations` survive?**~~ **No** (2026-08-16, D13).
  Source kind `session_jsonl` inside `nursery_generate_data`. See `harvest.md`.
