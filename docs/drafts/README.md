# drafts/ — the originating brainstorm

**Status: provenance. Superseded by `../CHARTER.md`.** Kept because the analysis is good and
the reasoning is worth being able to retrace — not because it is current. Where a draft and the
charter disagree, **the charter wins**; where a draft and the code disagree, the code wins.

Authored by Grok (web, 2026-08-02) after an X post, with partial repo access. Triaged into the
charter the same day.

| File | What it is | Status |
|---|---|---|
| `00-OVERVIEW.md` | The pitch and the two-repo target | superseded (D1, D2) |
| `01-NURSERY-AURUM-ANALYSIS.md` | Analysis of ApexAurum `tools/nursery.py` | **accurate — still the reference** for the Python original's tool surface |
| `02-APEXOS-RS-NURSERY-PLAN.md` | Port plan as a crate inside ApexOS-RS | superseded (D1 — standalone sibling) |
| `03-APEXROUTER-RS-TRAINING-PLAN.md` | Training subsystem inside ApexRouter | superseded (D2 — Router's charter fences training out) |
| `04-INTEGRATION-AND-RSI.md` | End-to-end flow, safety rules, event taxonomy | partly live — the flow holds, the ownership doesn't |
| `05-SCAFFOLDING.md` | Rust sketches, types, config | reference sketches; types re-derived in `../design.md` |
| `06-IMPLEMENTATION-CHECKLIST.md` | Ordered phases | superseded by the charter's S0–S6 |
| `07-FULL-REBIRTH-AND-RSI-VISION.md` | Stage-2 weight-rewrite vision + Model Watcher | **the strongest draft** — carried forward into `../rebirth.md` |

## What the iteration changed, and why

Recorded so the corrections don't get re-argued, and so the drafts aren't read as current.

1. **Standalone sibling, not an ApexOS-RS crate** (D1). The drafts place the nursery at
   `ApexOS-RS/tools/crates/nursery` from day one, which pre-decides the assimilation question.
   The house pattern for a new capability is a standalone repo consumed over MCP.

2. **The training lifecycle lives here, not in ApexRouter** (D2). Draft 03 proposes a
   `/v1/training/*` subsystem in ApexRouter. That repo's `CHARTER.md` — a log its own CLAUDE.md
   calls binding — says *"Not a model zoo, **not a training tool**, not a quantiser."* Grok had
   the README and ARCHITECTURE from the web but evidently not the charter. Its `GARDEN.md`
   supplies a better mechanism anyway: a rented box tunnelled to `127.0.0.1:88xx` is already
   sanctioned, so Router needs no new concept.

3. **Facts, not status, on disk** (D3). Draft 06 persists `status: JobPhase` to `jobs.jsonl`.
   ApexRouter invariant 3: *"No `status: "running"` string ever goes to disk — it is a lie the
   moment someone types `kill`."*

4. **No live-network tests** (D5). Draft 06 plans "E2E test against real Together". The
   hermeticity rule next door exists *because* a suite once made live authenticated calls to
   `api.together.ai` with the real key.

5. **No agent-initiated paid boxes** (D4). Draft 06 Phase 6 plans rented training e2e runs.
   ApexRouter's money rule forbids agents calling any vast endpoint that creates, modifies or
   destroys an instance, with the credit balance pinned to the cent.

6. **`trainer_agent` is not `agent_id`** (D6). Drafts 02/04 want attribution "mandatory and
   immutable" in `agent_id`. agentd **overwrites** `agent_id` on every Cerebro call, so that
   attribution would silently become the stamping identity — wrong in a way that looks right.

7. **Tools never hide** (D8). Draft 07 §4 gates rebirth tools by making them appear in the tool
   list above a score. A vanishing tool is context divergence.

8. **Promotion gates on a *verified restorable* `previous_good`** (D11), not merely a retained
   one — matching ApexOS-RS's H4 snapshot gate.

## Verified while triaging

- **Qwen3.6-27B is real**: released 2026-04-22, Apache-2.0, dense 27B, 262k context, native
  vision, Unsloth-supported. It is also ApexRouter `GARDEN.md`'s designated "thinker".
- **Together LoRA pricing is right**: $0.48/1M ≤16B · **$1.50/1M 17–69B** · $2.90/1M 70–100B.
  The drafts miss that **hosting a tuned model is a separate ongoing charge** — see the charter's
  open questions.
- **Unverified**: whether Together currently accepts `Qwen/Qwen3.6-27B` specifically as a
  fine-tune base. Pricing band ≠ supported-base list. Confirm at S3.
