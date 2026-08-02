# Puerperium-RS — Agent & Developer Guide

> A **nursery for models**: mines an agent's own remembered experience into training data,
> orchestrates LoRA fine-tuning, and keeps a lineage-complete registry of what it raised.
> One Rust workspace — a core lib behind an MCP face and a CLI. No daemon in v1.
> ApexOS-RS is the first consumer (via MCP, zero changes to it). **Standalone is a
> first-class goal** — assimilation is ApexOS-RS's decision, in its own thread.

Bootstrapped 2026-08-02. House conventions come from `~/Projects/Launchpad-RS/`
— load a doc from there when you need the detail behind a rule below.

**Read `docs/CHARTER.md` before any non-trivial change — its decisions log (D1–D12) is
binding.** Amend it with a dated entry when a decision changes, never silently. Where the
charter and this file disagree, the charter wins.

Siblings: `../CerebroCortex-RS` (the memory this mines — mirror its crate shape) ·
`../ApexOS-RS` (first consumer) · `../Occipital-RS`, `../Sonus-RS`, `../Callosum-RS` (same
sibling pattern). External: `buckster123/ApexRouter-RS` (compute + inference — **not checked
out locally**; clone read-only when you need to verify its surface).

Origin: `buckster123/ApexAurum` `tools/nursery.py` (Python, **do NOT modify**) — analysed in
`docs/drafts/01`, superseded by this charter.

---

## What this is

An agent that has been working for months knows things its base model doesn't. Puerperium
turns that into a specialist: query Cerebro for what the agent learned about a domain, convert
it to instruction data, fine-tune a small adapter on it, and register the result with enough
lineage that "why is this specialist like this?" always has an answer.

The deliberately-scoped RSI loop: **agent → dataset from its own memory → apprentice →
routed work → repeat.** Rewriting the driver model's own weights is Stage 2, designed in
`docs/rebirth.md` and not built (charter D10).

```
crates/
  puerperium/         # core lib — datasets, jobs, registry, lineage. No I/O glue.
  puerperium-mcp/     # MCP stdio server — the agent face (nursery_* tools)
  puerperium-cli/     # clap CLI — the human/ops face
docs/CHARTER.md       # binding decisions D1–D12, phases, scope fence
docs/design.md        # THE contract — tool surface, types, lifecycle
docs/rebirth.md       # Stage 2 design freeze (R0) — no code ships from this
docs/drafts/          # Grok's originating brainstorm — provenance, superseded
BACKLOG.md            # slice ledger S0–S6 + post-v1 parking
```

---

## Locked decisions

The load-bearing summary; **`docs/CHARTER.md` D1–D12 is the binding long form.**
**Locked means locked — do not re-litigate these mid-session; amend deliberately, with a date.**

- **Language**: Rust — one Cargo workspace, every binary in it
- **Shape**: standalone sibling, not an ApexOS-RS crate (D1)
- **Boundary**: Puerperium owns the training lifecycle; ApexRouter is used only through
  surfaces it already has — its charter fences training out, and we respect that (D2)
- **Records hold facts, never status** — phase is computed on read (D3)
- **Never initiates spend**; never calls a mutating vast.ai endpoint (D4)
- **Hermetic tests** — nothing connects beyond `127.0.0.x`; parsers test against fixtures (D5)
- **`trainer_agent` ≠ `agent_id`** — agentd stamps `agent_id`, so attribution needs its own
  field (D6)
- **Nano-first *refusal***: the nursery runs everywhere; only training refuses, honestly (D7)
- **Tools never hide** — a gated tool is present and explains its refusal (D8)
- **MCP**: hand-rolled newline-delimited JSON-RPC over stdio, protocol `2024-11-05`, no SDK
- **Storage**: JSONL + files under the state dir; SQLite only if querying demands it
- **HTTP**: `reqwest` (rustls) out; `clap` for CLI; `serde` everywhere
- **CI from commit 0**: fmt `--check` + clippy `-D warnings` + test + build
- **rustfmt-clean baseline from commit 0** — so `cargo fmt --all` is always safe here

---

## The playbook (the house method — read once, then live it)

Full rationale: `~/Projects/Launchpad-RS/docs/house-doctrine.md`. The nine, condensed:

1. **Contract first.** Pin it in `docs/design.md` before code. **Docs travel with code.**
2. **Slices, not marathons.** One branch = one slice off freshly-fetched `origin/main`.
3. **Honesty invariants.** Never a fake success. Degrades are *stated*. Failures carry the real
   reason. Check the body, not just the status. Never silently clamp what you can reject.
4. **Pure-fn test discipline.** Pure functions are the test surface; handlers are I/O glue.
   Upstream parsers get fixture tests from real captured JSON. Effectful tests skip *loudly*.
5. **Field truth beats green CI.** A slice is done when it ran for real — S6 is the whole point.
6. **Secrets hygiene.** Never print a key (lengths and heads only). No credentials in tracked
   files. `TOGETHER_API_KEY` lives in an env file, never here.
7. **Cerebro is the thread.** `session_recall` at start, `session_save` at milestones and end.
8. **Spend is gated.** Training costs real money. Nothing paid auto-fires (D4).
9. **Cost the failure, not the happy path.** A paid job that outlives its poll window is
   *pending*, not failed — resumable by provider job id, never orphaned.

---

## Git discipline

- **Never commit to `main`.** Feature branch off freshly-fetched `origin/main`: `feat/…`,
  `fix/…`, `chore/…`, `docs/…`. One branch = one slice.
- **Ship via PR** (`gh pr create`). **Do NOT merge it yourself** — André reviews and merges.
  (Pre-publication bootstrap commits are the sanctioned exception.)
- **Commit format:** imperative, lowercase. End with the `Co-Authored-By` trailer.
- **Never amend a pushed commit. Never force-push.**
- **Push after every commit.** If Cerebro is unavailable, repo + docs must be enough to
  reconstruct full project context.

---

## Cerebro session protocol (mandatory)

All Cerebro MCP calls use agent `FORGE` (`agent_id="FORGE"`) — memories stay isolated per
project. Full tool menu + grading discipline: `~/Projects/Launchpad-RS/docs/cerebro-protocol.md`.

**Session START** — before touching any code:
```
session_recall(query="Puerperium-RS build status slice progress", agent_id="FORGE")
```

**Session END** (and at milestones on long sessions):
```
session_save(session_summary="what was built, what broke, what was learned",
             key_discoveries=[...], unfinished_business=[...],
             agent_id="FORGE", priority="HIGH")
```
Then as needed: `store_procedure` · `record_procedure_outcome` (**grade every procedure you
exercised** — ungraded ones are invisible to the dream engine) · `store_intention` (parked
ideas, salience 0.8–0.95) · `episode_*` (multi-step sequences).

**Note the reflexivity:** Cerebro is both this project's session memory *and* its raw material.
Keep the two straight — `agent_id="FORGE"` writes are the build thread; a dataset mined from
Cerebro is product input and gets provenance-stamped (D12), never confused with a session save.

**The vaults:** CLAUDE.md = lean core + pointers · `docs/CHARTER.md` = binding decisions ·
`docs/gotchas.md` = invariants · Cerebro = session memory · git = code truth.

---

## Dev commands

```bash
cargo test --workspace
cargo fmt --all && cargo clippy --workspace -- -D warnings   # clippy-zero policy
cargo build --release --workspace

# Dataset garden (S1). Input is a JSON array of MemoryRecord — a Cerebro export.
./target/release/puerperium data generate --from memories.json --name my-set --domain ApexOS
./target/release/puerperium data generate --from memories.json --name x --dry-run  # no write
./target/release/puerperium data list
./target/release/puerperium data inspect my-set --head 5
./target/release/puerperium data verify my-set

# Registry (S2). Records hold facts only — never liveness (that is Router's truth).
./target/release/puerperium model add --name worker-v2 --base-model Qwen/Qwen3.6-27B \
    --parent worker-v1 --dataset my-set --trainer-agent FORGE
./target/release/puerperium model list
./target/release/puerperium apprentice agents /path/to/cerebro-snapshot.db
./target/release/puerperium apprentice create --id ap1 --db /path/to/snapshot.db \
    --master-agent CLAUDE --specialization "ApexOS ops" --name deploy_hand \
    --dataset-name ap1-data --include-types procedural --domain ApexOS --dry-run
./target/release/puerperium apprentice list
./target/release/puerperium lineage worker-v2          # --json for the full structure

# Deploy (S5). Read-only discovery; deploy needs a running ApexRouter on :2739.
./target/release/puerperium compute                    # backends + LoRA-capable bases
./target/release/puerperium deploy --model worker-v2 --alias apexos-worker \
    --served-model acct/worker-v2 --dry-run

# Jobs (S3). Together is a hosted API — no compute to provision.
./target/release/puerperium estimate --dataset my-set --params-b 27 --epochs 3
./target/release/puerperium job submit --id j1 --dataset my-set --output-name worker-v2 \
    --training-file-id file-abc --dry-run       # prints the body, contacts nothing
./target/release/puerperium data export my-set --to /tmp/upload.jsonl   # offline, validates
./target/release/puerperium job upload my-set          # -> training_file_id
./target/release/puerperium job list                   # polls non-terminal jobs
./target/release/puerperium job status j1
```

**Credentials.** `TOGETHER_API_KEY` is read from the process environment. A real environment
variable always wins; failing that, the first of `$PUERPERIUM_ENV_FILE`,
`~/.config/puerperium/env`, `/etc/puerperium/env` is loaded (`KEY=VALUE`, no `export`, `0600` —
the house format). `puerperium keys` reports what is configured **without ever printing a
value**. On this box the key already comes from `~/.bashrc`, so no file is needed.

**Nothing has been submitted to Together for real yet.** The client is built from their SDK
types and the parsers are tested against those shapes, but no request has been sent — it is
INSTALLED, not ACTIVE. First live use needs a key and is André's explicit call (D4/D8).

**Mining a Cerebro store:** `apprentice create --db` opens the file **read-only** and never
writes. Point it at a `.backup` snapshot, not a live database:
`sqlite3 /var/lib/cerebro/cerebro.db ".backup /tmp/snap.db"` — `.backup` is safe against
concurrent writes, copying the file is not.

**For a raw export instead:** copy the DB first, never query it in place —
`cp ~/.cerebro-cortex/cerebro.db /tmp/copy.db`, then dump with
`sqlite3 copy.db "SELECT json_group_array(json_object('id',id,'content',content,'memory_type',memory_type,'tags',json(COALESCE(tags,'[]')),'agent_id',agent_id,'salience',salience)) FROM memories WHERE deleted_at IS NULL;"`.
It is another tool's state directory: read it, never write it.

State dir: `~/.local/share/puerperium/` (`PUERPERIUM_STATE_DIR` overrides). **Nothing is ever
written into the repo directory.**

---

## Gotchas

Project invariants live in **`docs/gotchas.md`** — grep it for your subsystem **before**
modifying it. **A new gotcha goes THERE, not here.** Cross-project version drift is in
`~/Projects/Launchpad-RS/docs/sharp-edges.md`.

Three that bite here specifically:

- **MCP stdout is sacred.** All `tracing` output goes to **stderr**. A stray `println!`
  corrupts the JSON-RPC stream.
- **`agent_id` is stamped, not supplied.** agentd overwrites it on every Cerebro call. Anything
  you need attributed to a *trainer* goes in `trainer_agent` (D6).
- **Read the pinned crate's docs for the exact version** — not memory of an older API. Version
  drift gets recorded in a dated changelog line, never fought silently.

---

## Docs

Load only the relevant doc when entering a subsystem — do not load all of them.

| File | Load when working on |
|------|----------------------|
| `docs/CHARTER.md` | **Binding decisions D1–D12, phases, scope fence — before non-trivial work** |
| `docs/design.md` | **The contract** — tool surface, types, job lifecycle, env |
| `docs/gotchas.md` | **Any subsystem change — grep it first** |
| `docs/rebirth.md` | Stage 2 (full weight rewrite) — design freeze only, no code |
| `docs/drafts/` | Originating brainstorm (Grok) — provenance; superseded by the charter |
| `BACKLOG.md` | Outstanding work — slice ledger + parked items |

---

## Meta — when to update this file

- A locked decision changes → **`docs/CHARTER.md` first** (dated amendment), then the summary here
- A gotcha is discovered → **`docs/gotchas.md`**, not here
- A slice completes → tick it in `BACKLOG.md`
- A doc file is created → add a row to `## Docs`
- **Keep this file under ~250 lines / ~20 KB.** Fat goes to `docs/`; this file points.
- Before publishing, inline anything it truly depends on from `Launchpad-RS/` so the repo
  stands alone for outside readers.

### What never goes in CLAUDE.md or docs/*.md

- Task progress, session logs, completed-work summaries → Cerebro (`session_save`)
- Git SHAs, version pins → stale in days, belong in git history
- Commentary on what you just did → belongs in commit messages
- **Credentials of any kind** → env files (0600, root-owned), never a tracked file
