# Harvest, traces, and session RAG

> **Design freeze, not a slice.** Charter D13 holds the rules; this document is the
> contract those rules need. No converter, no RAG index, no ApexOS or ApexRouter code
> ships from this file. A later slice implements against it. A PR that changes behaviour
> updates this doc in the same commit.
>
> Bound by `CHARTER.md` D1–D13. Where this doc and the charter disagree, the charter wins.

The first Together run trained on Cerebro *lessons*. The next healthy dataset has to come
from lived *turns*. Those turns already exist — in ApexOS session JSONL — and two of the
four pipes that would make them usable are currently holes, not knobs.

## Why it exists

An agent that has been working for months has two kinds of residue:

- **Lessons** — procedures, decisions, gotchas. Cerebro stores these. Puerperium already
  mines them (`cerebro_query` / `export_file`). Dream consolidation then abstracts them.
  Measured: 1579 of 1629 procedural memories on a real node were dream-derived (~227 chars
  vs ~4193 for the lived 50). That is the wrong training material, and excluding dreams
  leaves too little for a second paid run.
- **Turns** — user text, tool calls, tool results, the assistant's actual moves. ApexOS
  already appends these to `<AGENTD_LOG>/sessions/<id>.jsonl`. Cerebro `session_save` and
  `POST /api/sessions/{id}/consolidate` *summarise* them on purpose. Episodes are
  `{description, memory_id?}` stubs, not tool rounds.

The first mine framed lessons. A worker model needs trajectories. This doc is how those
trajectories are harvested, licensed, retrieved, and converted — without dumping them into
Cerebro, without training on closed-API chain-of-thought, and without Puerperium sitting
on the request path.

## What is already true

The garden has three stores. They are not interchangeable.

```
model ──► ApexOS agentd ──► sessions/<id>.jsonl     full transcript (primary tap)
                 │
                 ├── session_save / consolidate ──► Cerebro     summary / lesson
                 └── chat completions ──► ApexRouter
                                              └── usage.jsonl   metadata only
```

- **ApexOS session JSONL** is the transcript. `SessionStore` appends every `Message`
  (user, assistant, tool_use, tool_result, and — on the Anthropic path only —
  `ContentBlock::Thinking`). Worker sessions persist; spawn sessions do not. Export
  already exists: `POST /api/sessions/export` writes
  `<workspace>/exports/session-<id>.{md,jsonl}`.
- **Markdown export strips thinking** on purpose (`render_session_markdown`, test-locked).
  JSONL export keeps the raw `Message`. That split stays.
- **Cerebro `session_save` is a summary**, not a transcript. Consolidate is the lossy
  path: one LLM turn → `{summary, key_discoveries}` → `session_save` before
  archive/delete, 48k-char head+tail. Dream extraction then abstracts further. That is
  why the first mine was thin.
- **Cerebro episodes are not trajectories.** `EpisodeStep` is a prose stub. The old
  BACKLOG item "episode → trajectory" is retired; session JSONL already *is* the
  sequence. Episodes stay narrative.
- **ApexRouter `[router] capture_bodies` is a dead knob.** Parsed, default `false`,
  shown in the UI, never applied (Router F037). `usage.jsonl` is tokens / alias /
  request id — no prompts. Wiring it is a Router product decision, not a Puerperium
  feature.
- **OAI thinking is dropped on both sides.** ApexOS `oai.rs` does not parse
  `delta.reasoning_content` and `build_body` skips `ContentBlock::Thinking`. Router's
  OpenAI path is a transparent proxy; Anthropic ingress records `reasoning_content`
  and does **not** map it onto `thinking`. A thinking Qwen that answers in
  `reasoning_content` is therefore invisible to harvest today.

Puerperium already reserved the analogue: `nursery_extract_conversations` is **not** a
separate verb. Session JSONL is a source kind inside `nursery_generate_data`. See
`design.md`.

## D13 — restated

The binding long form is `CHARTER.md`. The operational reading:

1. **Puerperium converts snapshots. It does not harvest on the request path.** Taps live
   in ApexOS (session owner) and, later, ApexRouter (optional capture). Those changes
   are those repos' threads (D1). This repo defines the snapshot contract and the
   converters.
2. **Three license classes, explicit allowlist, never guessed** — see below.
3. **Traces do not become Cerebro memories.** Cerebro is the lesson store. FORGE
   `session_save` is the *build thread*; a mined trace is *product input* and is
   provenance-stamped (D12), never confused with a session note.
4. **Opt-in, local, secret-scanned before anything leaves the box.** Router capture
   stays off by default. ApexOS session persist is the revive substrate, not a new tap.
   A mine copies a snapshot out — same discipline as `sqlite3 … ".backup"`. Secret
   scan refuses a round *before* `job upload`.

No spend (D4). Hermetic fixture tests against captured session lines (D5). Gated tools
stay visible and refuse (D8).

## License classes

A round carries exactly one class. The classifier is an **explicit model-id allowlist**,
not "we parsed a reasoning field so we may train on it."

| Class | What it is | Persist thinking | RAG thinking | Train on thinking |
|---|---|---|---|---|
| `open_reasoning` | Self-hosted weights (local Qwen, llama.cpp `--reasoning-format`) and APIs whose *current* terms allow persisting and training on served reasoning | yes | only if a node flag is on (default off) | yes |
| `closed_hidden` | Anthropic thinking+signature, OpenAI hidden CoT, xAI encrypted/hidden reasoning | live JSONL **only** when the provider requires replay (Anthropic signature) | **strip** | **strip** — never distill |
| `answer_only` | No reasoning channel | n/a | n/a | n/a |

Markdown export, RAG chunks, and training examples **always** strip `closed_hidden`
thinking text. The signature may remain in the live session file so Anthropic replay
does not 400. That file is not a training corpus.

### Allowlist (empty until verified)

Pin model-id prefixes here on the day of the first mine, after reading the provider's
current terms. Do not guess from memory of an older ToS.

```
# open_reasoning  — verified YYYY-MM-DD against <url>
#   (none yet)
#
# closed_hidden   — treat as closed unless moved above
#   claude-*
#   gpt-*  o1-*  o3-*  o4-*
#   grok-*
#
# everything else is answer_only until classified
```

Alibaba-style and Together-hosted Qwen that *serve* raw reasoning are candidates for
`open_reasoning` **only after that day's terms say so**. Self-hosted weights of an
Apache/Qwen-licensed checkpoint are the expected first entries.

Unknown model id → `answer_only`. A thinking block on an unknown id is stripped, not
promoted. Promotion is an allowlist edit, dated in this file.

## Snapshot contract

Puerperium never opens a live `AGENTD_LOG`. The operator (or ApexOS export) produces a
directory of JSONL files; Puerperium reads that directory.

Preferred input: `<workspace>/exports/session-*.jsonl` from `POST /api/sessions/export`
with `format: "jsonl"`. Acceptable: a copy of `sessions/` + `sessions/archive/`. A live
path is refused if we can detect it (same inode as `$AGENTD_LOG/sessions` on this host);
when we cannot detect it, the docs still say "copy, then mine."

### Round

The shared unit for RAG and for the mine. One pure split, two consumers.

A **round** is: one user message plus every following assistant / tool-bearing message
until the next user line (or end of file). Tool results that ApexOS stores on the
*next* user message (OAI ordering) belong to the round that issued the tool_use.

Skip, and count, before a round is born:

| Skip | Reason |
|---|---|
| session id `0` | sensor / scheduler funnel, not a conversation |
| spawn-range ids (`>= 1<<63`) | not persisted; should not appear in an export |
| image-only rounds | payloads are not instructions |
| empty-assistant rounds | today's Qwen hole — `reasoning_content` never landed |

Worker-range sessions (`1<<62 .. 1<<63`) **are** in scope — they are persisted tasks.

### Trace bundle (sidecar, optional)

A directory of raw `session-*.jsonl` is enough to mine. A sidecar, when present, saves
the classifier from guessing:

```json
{
  "node_id": "apex1",
  "exported_at": "2026-08-16T20:00:00Z",
  "sessions": [
    {
      "session_id": 22,
      "agent_id": "APEX",
      "model": "studio-llm",
      "license_class": "open_reasoning"
    }
  ]
}
```

Without a sidecar: `node_id` from the directory name or `--node`; `agent_id` unknown
unless the filename or a later ApexOS stamp carries it; `license_class` from the
allowlist against `model` if known, else `answer_only`. Never invent a class to keep
a thinking block.

### Provenance (D12)

Every mined example records:

```rust
Provenance::SessionTurn {
    node_id: String,
    session_id: u64,
    turn_index: u32,          // 0-based round in that file
    agent_id: Option<String>, // whose session — not the trainer (D6)
    license_class: LicenseClass,
    model: Option<String>,
}
```

`SourceSpec.kind` for the dataset sidecar is `"session_jsonl"`. `memories_in` is
reused as `rounds_in` in the accounting sense: `rounds_in = used + rejected`. Do not
introduce a second total that can drift.

Parked, not designed further here: `Provenance::RouterRequest { request_id, alias, backend }`
and `SourceSpec.kind = "router_capture"` — only if Router wires `capture_bodies` under
its own charter.

## Layer 1 — RAG over session history

**Problem.** The live agent sees the current session plus Cerebro *summaries*. It cannot
retrieve "the turn where we fixed LAN pairing" from last week's JSONL. Consolidate is
the opposite of this: it throws the transcript away on purpose.

**Do not** embed session turns as Cerebro memories. That would feed the dream engine
the thing we just measured as poison. **Do** give ApexOS its own session index, under
`AGENTD_LOG`, queried at turn start. That index is an ApexOS thread.

Sketch (sibling slice, later):

- **Unit:** a round, as above.
- **What is indexed:** `answer_only` text + tool names + truncated tool results.
  `closed_hidden` thinking is dropped. `open_reasoning` thinking is included only when
  a node-level flag is on (default **off** — thinking is large and rarely what priming
  wants).
- **Embedder:** call Cerebro's existing embed path as a *service* (vectors in, no
  `remember`). Do not take a second `ort` into agentd — the Kokoro / `fastembed` pin
  fight is already a gotcha.
- **Query:** last user prompt → top-k rounds from *other* sessions. Inject into the
  priming layer with a hard token budget (same composition slot as boot priming).
  The current session is already in history — do not retrieve it.
- **Honesty:** if the index is cold or embed is down, skip and log; do not invent
  context.

Puerperium's interest: the round-splitter is the mine unit. Specify it here; implement
it first in `puerperium::harvest` (fixture-tested). ApexOS may depend or copy the
*contract*, not the crate, until assimilation is a real decision (D1).

## Layer 2 — Plumbing for full traces (open reasoning)

The missing pipe is ApexOS OAI, not a new store. Sibling slice, later.

Today Qwen / Alibaba-style `reasoning_content` dies in `sse_to_chunks`. Anthropic
thinking is kept because the API rejects a continuation without the signature.

When ApexOS wires it:

- Parse `delta.reasoning_content` → `Chunk::ThinkingDelta` / `ThinkingBlock`.
- Persist as `ContentBlock::Thinking { thinking, signature: "" }` for open models
  (empty signature = not Anthropic-replay).
- **Do not** send those blocks back upstream on the OAI path (`build_body` already
  drops them — keep that). OAI replay does not want a fake `thinking` field.
- Classify the model id at persist time and stamp it on a session sidecar so a later
  mine does not guess.
- The UI thinking rail lighting up for studio-llm is a side effect, not the goal.

The license gate is the classifier, not the parser.

## Layer 3 — ApexRouter harvest potential

`[router] capture_bodies` is the right *name* and the wrong *belief*. It does not
harvest.

**Leave it dead** until Router amends its own charter. Their config comment already
calls inventing request-path body capture a product decision (what is stored, where,
under which lock, how it is redacted).

If and when they wire it:

- Default remains `false`.
- Store under the Router state dir, `0600`, join to `usage.jsonl` by `request_id`.
- Redact `Authorization`, cookies, `sk-…`, and 64-hex bearers before disk.
- Files survive restart (unlike in-memory LAN pair); needs a retention cap.
- This tap is for **non-ApexOS clients** (Claude Code, curl, other agents). ApexOS
  does not need it. Do not turn it on on the studio box to get a node's traces —
  export the session JSONL.

## Layer 4 — Session JSONL as the primary mine

This is the high-value data the first Together run did not have. Puerperium slice,
later; contracted here.

**Input:** a directory of exported `session-*.jsonl` (see Snapshot contract).

**Converter** (new, next to `convert/`; not the prose chunker):

1. Split into rounds (shared with RAG).
2. Strip `closed_hidden` thinking. Keep `open_reasoning` thinking only when the
   allowlist says so.
3. Reject, counted by reason:
   - greetings / a2a chatter — reuse the existing quality gate
   - image-only
   - empty assistant (the Qwen hole)
   - `secret` — key-shaped tokens (see below)
   - tool-result over the char cap (default 4000) — refuse the *round*, do not
     silently truncate the result into a half-lesson
4. Emit sharegpt-style `messages`: user → assistant (text + optional open thinking)
   → tool_use / tool_result sequences. That is a trajectory.
5. Stamp `Provenance::SessionTurn`. `SourceSpec.kind = "session_jsonl"`.
6. Sidecar accounting stays total: `rounds_in = used + rejected`.

Images stay out (a `[image]` marker is not an example). A 200k `cat` of a log is not
an instruction.

`nursery_generate_data` gains `sessions` (path) as a **third exclusive** source
beside `db` and `from`. `dry_run` still defaults true. Synthetic still refuses
honestly. **No new MCP verb.**

Until this slice is built, a caller asking for a sessions path is an honest
refusal naming `harvest.md`, not a silent ignore.

### Secret scan

Pure, fixture-tested, **before** `job upload`. A round that matches is
`rejected.secret`, never uploaded.

Minimum patterns (extend in code, not here, when a live export finds a new shape):

- `sk-` / `sk-ant-` / `sk-or-` prefixes
- `TOGETHER_API_KEY` / `OPENAI_API_KEY` / `OAI_API_KEY` / `ANTHROPIC_API_KEY` assignments
- 64-hex tokens in `Authorization: Bearer` or `token=` context
- PEM / `BEGIN` key blocks

Lengths and heads may be logged; values are never printed (house secrets hygiene).
`job upload` today has no scan — this converter is the gate, not a later filter
on Together's side.

## What this is not

- Not another Together run on the laptop FORGE mine.
- Not putting traces into Cerebro so `session_recall` magically gets smarter.
- Not wiring Router `capture_bodies` from this repo.
- Not training on Anthropic / OpenAI hidden reasoning because it happens to sit
  in JSONL for API replay.
- Not a new MCP verb.
- Not opening a live Cerebro DB or a live `AGENTD_LOG` from Puerperium.

## Sibling slices (parked)

Implementation is one branch = one slice, in the repo that owns the tap.

| Slice | Repo | What |
|---|---|---|
| Session JSONL source + trajectory converter + secret scan | **Puerperium-RS** | `puerperium::harvest`, `sessions` source, `Provenance::SessionTurn` |
| Persist OAI `reasoning_content` + license sidecar | ApexOS-RS | `oai.rs` parse; do not send thinking back on OAI |
| Session RAG index at turn start | ApexOS-RS | own index under `AGENTD_LOG`; Cerebro embed as a service |
| Wire `capture_bodies` | ApexRouter-RS | only after *their* charter amendment |

No slice spends. The S6 measurement still waits on a healthy dataset; this doc is
how that dataset gets a real source.
