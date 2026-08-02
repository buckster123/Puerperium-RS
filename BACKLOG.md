# Puerperium-RS backlog — slice ledger

A row gets its ✅ when the slice is **merged and verified for real** — not when tests pass
(house doctrine #5). Notes carry the date and the evidence. Gates are in `docs/CHARTER.md`.

## v1 (an agent raises its first specialist)

- [x] **S0 — bootstrap**: charter (D1–D12), CLAUDE.md, workspace, CI, drafts triaged
      (2026-08-02 — Grok's drafts superseded on two structural points; see charter amendments)
- [ ] **S1 — dataset garden**: Cerebro query → instruction JSONL, synthetic templates,
      provenance stamp + `sha256` per dataset (D12). Pure convert/chunk fns unit-tested.
- [ ] **S2 — registry**: datasets · models · apprentices · lineage. Facts only, phase computed
      on read (D3). CRUD round-trips under `tempfile`.
- [ ] **S3 — job lifecycle**: submit → poll → artifact. **Together path first** — an API call
      with no provisioning exercises the whole lifecycle for the least risk. Fixture-driven,
      hermetic (D5). Poll timeout leaves the job recoverable by provider id, never orphaned.
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

**Ideas, unscheduled**

- `puerperium-api` face + a dashboard — no consumer yet
- DPO/ORPO/RFT; continued pre-training on a curated mixture
- Colony-level model evolution: parallel candidates across nodes, mesh benchmarks the winner
- Specialist → foundation promotion (distil a proven apprentice into the next base)
- Self-proposing eval cases, so the Watcher stays ahead of capability drift
- A trained adapter as a resident in ApexRouter's `GARDEN.md` model garden — the two designs
  meet here, and neither needs the other to ship
