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
      *Not verified live* — no key, so the HTTP client is INSTALLED, not ACTIVE.
- [ ] **S4 — apprentice protocol**: `nursery_create_apprentice` — knowledge → dataset → train →
      lineage-complete record, from a real Cerebro query.
- [ ] **S5 — deploy + lineage**: hand back the adapter; register with Router as a separate
      explicit verb through its existing endpoints (D2). Cerebro carries the lineage event.
- [ ] **S6 — field**: one real adapter, end to end. A specialist trained from FORGE's own
      memory beats its base on a real task — **measured, not asserted**. This is the point of
      the whole thing; nothing above counts until this row is ticked.

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
- **Cerebro MCP source adapter** — S1 reads a JSON export. The live path (`export_memories` for
  bulk, `recall`/`find_by_tags` for targeted) lands with S2's registry, mirroring
  Prefrontal-RS's `core/cortex.rs` stdio-client pattern.
- **Episode → trajectory examples** — episodes carry ordered steps, which is the closest thing
  in the store to a real tool-use trajectory. Different converter, likely the highest-value
  data for a *worker* model. Excluded from S1 because episodic-as-prose is narrative; episodic-
  as-steps is not the same thing and deserves its own design.
- **Near-duplicate collapse** — Cerebro reinforces rather than re-mints at ≥0.86 cosine, but a
  dataset mined across months can still carry restatements of one lesson.

**S3 follow-ups**

- [x] **Credential loading** — env-file support (`~/.config/puerperium/env`, 0600) plus a
      `keys` verb that reports configuration without printing values. A real environment
      variable always wins, so a one-off override still works.

- [x] **Dataset upload to Together** — `data export` (offline projection + validation) and
      `job upload` (the three-step presigned flow). The loop is closed: dataset → export →
      upload → submit → poll → adapter. Found in the process: Together rejects extra columns,
      so the stored and uploaded files are deliberately different artifacts.
- **First live submission** — gated on a `TOGETHER_API_KEY` and André's explicit go (D4/D8).
  This is also where the charter's open question gets answered: whether Together accepts
  `Qwen/Qwen3.6-27B` as a fine-tune base. Run `job submit --dry-run` first; it prints the exact
  body without contacting anything.
- **Router compute discovery** — `--available-compute` is passed by hand today. Reading
  Router's control plane read-only (charter D2) replaces it, and lands naturally with S5.

**Ideas, unscheduled**

- `puerperium-api` face + a dashboard — no consumer yet
- DPO/ORPO/RFT; continued pre-training on a curated mixture
- Colony-level model evolution: parallel candidates across nodes, mesh benchmarks the winner
- Specialist → foundation promotion (distil a proven apprentice into the next base)
- Self-proposing eval cases, so the Watcher stays ahead of capability drift
- A trained adapter as a resident in ApexRouter's `GARDEN.md` model garden — the two designs
  meet here, and neither needs the other to ship
