# Puerperium-RS backlog — slice ledger

A row gets its ✅ when the slice is **merged and verified for real** — not when tests pass
(house doctrine #5). Notes carry the date and the evidence. Gates are in `docs/CHARTER.md`.

## v1 (an agent raises its first specialist)

- [x] **S0 — bootstrap**: charter (D1–D12), CLAUDE.md, workspace, CI, drafts triaged
      (2026-08-02 — Grok's drafts superseded on two structural points; see charter amendments)
- [x] **S1 — dataset garden**: memories → instruction JSONL, provenance-stamped and hashed
      (D12). Pure pipeline (filter → chunk → frame), 45 tests, `puerperium data
      generate|list|inspect|verify`. **Proven on the real store** (2026-08-02): 347 memories →
      182 examples from 100, accounting total, framing split 112 heading / 70 tag. Three
      defects the real data found and the synthetic tests could not: A2A chatter passing the
      gate, routing tags (`msg`, `from:`, `to:`) missing from the denylist, and statement
      headings producing broken instructions. Each has a named regression test.
      *Not yet done here:* LLM-assisted framing (deferred, D4) and synthetic tool-schema
      templates — see parking below.
- [x] **S2 — registry**: models · apprentices · lineage, facts only (D3). 63 tests.
      `puerperium model add|list|show`, `apprentice list|show`, `lineage <model>`.
      Sharpened D3 for models: a `ModelRecord` stores **no liveness** — whether an alias
      answers is Router's truth, so the record holds `alias_requested` (what we did) and
      nothing about what is live. `ApprenticeRecord` has no `trained` flag; it is derived.
      Lineage degrades honestly — missing dataset, hash mismatch, missing parent and
      hand-edited parent cycles are each reported, never fatal, never silently skipped.
      Shared `store.rs`/`paths.rs` extracted; `dataset.rs` refactored onto them.
- [x] **S3 — job lifecycle**: submit → poll → artifact, Together first. 100 tests, hermetic
      (D5) — nothing opens a socket. Append-only `jobs.jsonl` folded last-write-wins, mirroring
      ApexRouter's ledger: jobs are the money-adjacent records. Five invariants, each with the
      failure it prevents: record before upstream call · unreachable ≠ failure · rejected *is*
      failure, terminal, with their reason · terminal written once, never re-polled · compute
      gated before anything is written. Status mapping taken from Together's own SDK enum;
      unrecognised states are `Unknown`, never `Running`.
      *Verified live 2026-08-03* — job `ft-da39441f-d088` reached terminal once. Client is
      ACTIVE. S6 measurement (beats the base) is still unmet.
- [x] **S4 — apprentice protocol**: `apprentice create` mines a Cerebro snapshot (read-only)
      → dataset → lineage-complete record; `attach-job`/`attach-model` walk it to trained.
      134 tests. Proven on the real store: 300 memories mined → 114 examples from 39, dataset
      hashed and resolving, lineage tracing back to the memories. Stops before spending by
      design — training stays a separate explicit act (D4).
- [x] **S5 — deploy + lineage**: `compute` (read-only discovery) and `deploy` (register the
      adapter as a Router backend + alias, then write a Cerebro lineage event). 152 tests.
      Verified against the **live** Router: discovery reads real backends, dry-run prints the
      exact NodeSpec/ModelRoute. Reuses an existing backend rather than duplicating; credential
      is sent as a *pointer* (`{kind:env, var:…}`), never key material. Resolved a charter open
      question along the way — see the 2026-08-03 amendment.
- [~] **S6 — field**: pipeline proven end to end on real data and real money (2026-08-03) —
      231 examples → uploaded → trained on Together → terminal written once → registered →
      lineage resolving. **The measurement is deliberately unmet**: evaluating needs a
      dedicated endpoint (hourly, B200-class pricing) and serving is going to vast/local
      instead. Final gate awaits a proper run on a healthy dataset. Original text below.
- [ ] **S6 gate — one real adapter, end to end.** A specialist trained from FORGE's own
      memory beats its base on a real task — **measured, not asserted**. This is the point of
      the whole thing; nothing above counts until this row is ticked.

- [x] **MCP face (D1 groundwork, 2026-08-16)**: `puerperium-mcp` — sixteen `nursery_*` tools
      over newline-delimited JSON-RPC `2024-11-05`. Thin over the lib; spend verbs default
      to dry-run and need `confirm: true`. `nursery_test_model` and synthetic generate
      refuse honestly (D8). S6 measurement still unmet. `-api` remains deferred.

## Post-v1 parking

**Training paths deferred with reasons** (charter §Deliberately out of v1)

- **Vast training recipes** — container launch + artifact pull. Designed; needs a live paid box,
  which is André's keystroke (D4).
- **Local training supervisor** — Unsloth subprocess + VRAM gate. The laptop can't train a 27B,
  so it would ship untested; wants the fit-solver work first.
- **GGUF conversion** — out of scope in-process (charter). If wanted, it happens on the
  training box as part of that job's recipe.

**Stage 2 — Rebirth** (`docs/rebirth.md` is the R0 design freeze; D10 keeps all of this unbuilt)

- **R1** — rebirth score calculator + honest-refusal tool stubs (D8: present, never hidden)
- **R2** — full-param / high-rank recipes
- **R3** — Model Watcher v1: tool-calling integrity · format adherence · regression vs parent ·
  refusal consistency · identity check
- **R4** — promote / rollback with a **verified-restorable** `previous_good` (D11)
- **R5** — UI triggers, optional colony vote
- **R6** — continuous / scheduled rebirth policies

**Dataset garden follow-ups** (S1 shipped without these, deliberately)

- **LLM-assisted instruction framing** — the template floor is honest but plain, and 70 of 182
  examples framed from tags alone. A model writing the questions would lift those most. Costs
  tokens, so charter D4 gates it; `InstructionKind::LlmAssisted` already exists for it.
- **Synthetic generation from tool schemas** — the Python original's `SyntheticGenerator`.
  Needs a tool-schema source; worth doing when there is one to point at.
- **Cerebro MCP source adapter** — S4 reads a snapshot file directly (read-only SQLite), which
  is what the apex1 workflow actually needs. An MCP path (`export_memories` for bulk,
  `recall`/`find_by_tags` for targeted) would let it mine a *remote* Cerebro without a file
  copy; mirrors Prefrontal-RS's `core/cortex.rs` stdio-client pattern. Not needed until
  something wants to mine over the wire.
- **Session JSONL → trajectory examples** (D13 / `docs/harvest.md`, designed 2026-08-16,
  not built). ApexOS `sessions/<id>.jsonl` is the real tool-use sequence; Cerebro
  episodes stay narrative stubs. Converter is a new path next to `convert/` (round
  split, license strip, secret scan, `Provenance::SessionTurn`).
  `nursery_generate_data` gains exclusive `sessions` beside `db` / `from`. No new
  MCP verb. Do not dump traces into Cerebro. Sibling taps (OAI `reasoning_content`
  persist, session RAG index, Router `capture_bodies`) are those repos' threads.
- **Near-duplicate collapse** — Cerebro reinforces rather than re-mints at ≥0.86 cosine, but a
  dataset mined across months can still carry restatements of one lesson.

**Next up — the gap the first real run exposed**

- [x] **`data generate --db`** (2026-08-16) — the CLI matches the contract and
      MCP: a Cerebro snapshot (`--db` + `--agent`) or a JSON export (`--from`),
      not both. Mining prefixes ids with the file stem so two nodes cannot
      collide. Does not train.

- [x] **labelled banner titles** (2026-08-16) — first-line `PROCEDURE —` /
      `ARCHITECTURE DECISION` / `LABEL — rest` is a document title even when
      long or sentence-shaped. FORGE's lived procedures were 21% heading-framed
      because the 120-char / no-period rule dropped them. Does not invent
      examples; it upgrades the frame. A healthy *count* is still the S6 gap.

- [x] **convert Unframeable accounting** (2026-08-16) — a memory that splits into N
      unframeable chunks is one rejection, not N. `unframeable_chunks` is the chunk
      tally; `memories_used + rejections.total()` equals the input length again.

- [x] **`job download`** (2026-08-16) — `GET /v1/finetune/download?checkpoint=adapter`
      (never omit checkpoint — the API default is `merged`). Writes `.tar.zst`, extracts
      with a path-escape guard, reads `trainer_state.json` epoch means. Recovery via
      `--provider-job-id` when there is no local record. Free. MCP: `nursery_download`.
- **A healthy dataset** — 231 examples was a PoC. More *lived* material (not more epochs, not
  dream-derived) is what the final S6 run needs. The source for that material is
  session JSONL (`docs/harvest.md`), not another pass over the laptop FORGE mine.
  And with a minimum charge dominating, one larger run costs the same as several
  small ones.

**Harvest follow-ups** (D13 designed 2026-08-16; no code)

- [x] **Puerperium: `session_jsonl` source + trajectory converter + secret scan**
      (2026-08-16) — `puerperium::harvest`, `--sessions` / MCP `sessions`,
      `Provenance::SessionTurn`, secret scan before write. Live `AGENTD_LOG`
      refused. Open-reasoning allowlist still empty (verify ToS on first mine).
- **ApexOS: persist OAI `reasoning_content`** — sibling thread, **not needed
  for LoRA**. Today's Qwen thinking dies in `oai.rs`; that is fine for S6.
  Revisit if Stage 2 wants the thinking channel. Empty signature = not
  Anthropic-replay; do not send thinking back on the OAI path.
- **ApexOS: session RAG index** — sibling thread. Own index under `AGENTD_LOG`;
  Cerebro embed as a service, never `remember`. Query other sessions at turn start.
- **ApexRouter: wire `capture_bodies`** — sibling thread, and only after *their*
  charter amendment. Dead knob today (F037). For non-ApexOS clients; ApexOS
  already has a better store.

**S3 follow-ups**

- [x] **Credential loading** — env-file support (`~/.config/puerperium/env`, 0600) plus a
      `keys` verb that reports configuration without printing values. A real environment
      variable always wins, so a one-off override still works.

- [x] **Dataset upload to Together** — `data export` (offline projection + validation) and
      `job upload` (the three-step presigned flow). The loop is closed: dataset → export →
      upload → submit → poll → adapter. Found in the process: Together rejects extra columns,
      so the stored and uploaded files are deliberately different artifacts.
- [x] **First live submission** — `ft-da39441f-d088` (2026-08-03). Together does **not**
      accept `Qwen/Qwen3.6-27B`; the LoRA base is `Qwen/Qwen3.6-35B-A3B`. Further submits
      stay André's explicit go (D4/D8). `job quote` is the spend number; local `estimate`
      ignores the minimum charge.
- [x] **Router compute discovery** — `puerperium compute` reads Router's backends read-only.
      Still to wire: `job submit --available-compute` should default to a **probed** listing —
      a backend row is configuration, not liveness (a vast recipe reads `enabled` while cold),
      so `/v1/backends/{id}/probe` has to gate it or the D4 check would pass on a dead box.

**Ideas, unscheduled**

- `puerperium-api` face + a dashboard — no consumer yet
- DPO/ORPO/RFT; continued pre-training on a curated mixture
- Colony-level model evolution: parallel candidates across nodes, mesh benchmarks the winner
- Specialist → foundation promotion (distil a proven apprentice into the next base)
- Self-proposing eval cases, so the Watcher stays ahead of capability drift
- A trained adapter as a resident in ApexRouter's `GARDEN.md` model garden — the two designs
  meet here, and neither needs the other to ship
