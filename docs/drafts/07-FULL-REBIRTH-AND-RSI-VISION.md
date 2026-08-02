# 07 — Full Rebirth & Recursive Weight Rewrite Vision
**Addendum to the Nursery Port**

> Stage-1 (LoRA apprentices + specialist registry) is the safe, cheap, always-on loop.  
> This document defines Stage-2+: the **Full Rebirth** path — recursive rewrite of the actual base weights — with the same hard safety rails ApexOS-RS already applies to its own binary, soul.md, and Cerebro memory.

---

## 1. Why Full Rebirth Exists

LoRA / QLoRA is excellent for specialization, style, tool affinity, and domain knowledge.  
It is **not** sufficient when:

- The distribution of required behavior has shifted too far (new tool schemas, new embodiment, major policy changes).
- Catastrophic forgetting or format drift appears in the specialists themselves.
- The colony needs a new **foundational** capability that cannot be expressed as a low-rank update.
- Empirical measurements show that LoRA rank saturation has been reached and further LoRA gains are marginal.

At that point the system (or the user via UI) may trigger a **Rebirth**: a full-parameter (or near-full) continued training / SFT / preference optimization run that produces an entirely new base model checkpoint. The new checkpoint becomes a candidate successor to the current driver model.

This is the true recursive self-improvement surface — and therefore the highest-risk surface. It is gated, watched, and reversible by design.

---

## 2. Core Principles (non-negotiable)

1. **Never promote a candidate that has not passed the Watcher.**
2. **Previous known-good is always kept and instantly restorable.**
3. **Rebirth is an explicit act** — agent tool, UI button, or scheduled policy — never an automatic background process without human/agent confirmation above a cost/risk threshold.
4. **The same safety posture used for the binary watchdog and soul.md rollbacks applies to models.**
5. **All cost, lineage, and evaluation artifacts are durable and queryable in Cerebro + Router ledger.**

---

## 3. Architecture Overview

```
Current Driver (Qwen3.6-27B or successor)
        │
        ▼
┌───────────────────────────────────────────────────────────┐
│  Nursery Rebirth Tools                                    │
│  - nursery_rebirth_estimate                               │
│  - nursery_rebirth_prepare (dataset + eval suite)         │
│  - nursery_rebirth_start (full / near-full training)      │
│  - nursery_rebirth_status                                 │
│  - nursery_rebirth_promote / nursery_rebirth_rollback      │
└───────────────────────────┬───────────────────────────────┘
                            │
                            ▼
┌───────────────────────────────────────────────────────────┐
│  ApexRouter Training Provider (extended)                  │
│  - Full-param / high-rank / continued-pretrain recipes    │
│  - Multi-GPU Vast / dedicated clusters                    │
│  - Artifact staging (candidate checkpoint + GGUF)         │
└───────────────────────────┬───────────────────────────────┘
                            │
                            ▼
┌───────────────────────────────────────────────────────────┐
│  Model Watcher (new component)                            │
│  - Runs fixed evaluation battery                          │
│  - Tool-calling integrity, format adherence, regression   │
│  - Safety / refusal consistency, embodiment checks        │
│  - Score + pass/fail + detailed report                    │
│  - Only on PASS → candidate becomes promotable            │
└───────────────────────────┬───────────────────────────────┘
                            │
              ┌─────────────┴─────────────┐
              ▼                           ▼
     Promote (swap default alias)    Rollback / Quarantine
     + Cerebro event                 + keep previous live
     + lineage update
```

The Watcher is the model equivalent of the binary watchdog. It is the final gate.

---

## 4. Threshold Logic — When is Full Rebirth Justified?

Rebirth is expensive. The system therefore maintains a **Rebirth Score** (or a set of triggers) that must exceed a configurable threshold before the tools even become available, or before an agent is allowed to call them without elevated confirmation.

### Suggested signals (weighted)

| Signal | Example Measurement | Weight |
|--------|---------------------|--------|
| LoRA saturation | Further rank increases yield < X% gain on specialist benchmarks | High |
| Format / tool-calling drift | Regression on held-out tool-use suite > Y% | Critical |
| Embodiment mismatch | New sensors / tools / policy not expressible via LoRA | High |
| Knowledge gap | Core domain questions that LoRA specialists consistently fail | Medium |
| Colony consensus | Multiple agents + user vote for rebirth | Medium |
| Cost/benefit | Estimated full-tune cost vs projected capability gain | Always checked |

Concrete policy example (config-driven):

```toml
[rebirth]
enabled = true
min_score = 0.72
require_human_confirm_above_usd = 80.0
require_agent_supermajority = true   # optional colony vote
max_concurrent_rebirths = 1
```

Only when `rebirth_score >= min_score` do the rebirth tools appear in the agent’s tool list (or become non-erroring). Below the threshold the tools return a clear explanation and the current LoRA path remains the only option.

---

## 5. The Model Watcher

This is the heart of safety.

### 5.1 Evaluation Battery (must be deterministic and versioned)

A fixed, versioned eval suite stored under `$STATE/nursery/eval/`:

1. **Tool-calling integrity**  
   - Held-out set of tool schemas + expected argument structures.  
   - Must produce valid JSON / correct tool names / correct required fields.  
   - Failure = automatic reject.

2. **Output format adherence**  
   - Structured responses, thinking/non-thinking mode switches, XML/JSON fences, etc.  
   - Regression against the previous model’s format compliance rate.

3. **Core capability regression**  
   - Small but high-signal set of coding, reasoning, and embodiment tasks that the previous model passed.  
   - Absolute score and relative delta vs previous.

4. **Safety / refusal consistency**  
   - Known refusal prompts, policy tests, jailbreak canaries.  
   - Must not become more permissive or more broken than the parent.

5. **Embodiment & identity**  
   - Soul.md / embodiment block consistency.  
   - Agent still answers as the correct persona and knows its tools/sensors.

6. **Smoke inference**  
   - Basic generation quality, repetition, collapse checks.

Each battery produces a structured report + a single Pass/Fail + a numeric score.

### 5.2 Watcher Lifecycle

```
Candidate checkpoint ready
        │
        ▼
Watcher loads candidate (via Router temporary alias or isolated endpoint)
        │
        ▼
Runs eval battery (parallel where possible)
        │
        ▼
Writes report to $STATE/nursery/watch/{candidate_id}.json
        + Cerebro event
        │
        ▼
If PASS and score ≥ promote_threshold → mark PROMOTABLE
Else → mark QUARANTINED + keep previous as default
```

The Watcher itself can run on the same machine or on a cheap rented GPU; it does not need the full training cluster.

### 5.3 Promotion Rules

Promotion is a separate explicit action (`nursery_rebirth_promote` or UI button):

- Only works on candidates that the Watcher has marked PROMOTABLE.
- Atomically:
  1. Records previous default alias / checkpoint as `previous_good`.
  2. Swaps the production alias / default model pointer.
  3. Updates Cerebro lineage and embodiment block.
  4. Emits `rebirth.promoted` event.
- Rollback is one command / button that restores `previous_good` and re-points the alias.

This mirrors the binary watchdog’s “reincarnate only if the new binary is healthy” pattern and the soul.md rollback tools.

---

## 6. New Nursery Tools (Stage-2 surface)

| Tool | Purpose |
|------|---------|
| `nursery_rebirth_score` | Compute current rebirth justification score + explanation |
| `nursery_rebirth_estimate` | Full cost / time / GPU estimate for a proposed rebirth |
| `nursery_rebirth_prepare` | Assemble training mixture + freeze the eval suite version |
| `nursery_rebirth_start` | Launch full / high-rank / continued-pretrain job via Router |
| `nursery_rebirth_status` | Progress + intermediate metrics |
| `nursery_rebirth_watch` | Manually trigger or re-run the Watcher on a candidate |
| `nursery_rebirth_promote` | Promote a PROMOTABLE candidate (with confirmation) |
| `nursery_rebirth_rollback` | Instant restore of previous_good |
| `nursery_rebirth_list` | History of rebirths, candidates, scores, outcomes |

All tools carry the same agent attribution and Cerebro event discipline as Stage-1.

---

## 7. Training Modes for Rebirth

Router recipes are extended:

- **Full SFT** (all parameters) — expensive, highest plasticity.
- **High-rank LoRA / DoRA / full-rank adapters** — middle ground.
- **Continued pre-training** on curated mixture + light SFT.
- **Preference optimization** (DPO / ORPO / RFT) on top of a strong base.
- Multi-GPU Vast profiles or dedicated cluster backends for 27B+ full tunes.

The Nursery does not care which recipe is chosen; it only cares that the resulting checkpoint is handed to the Watcher.

---

## 8. Rollback & Versioning Model

Exactly analogous to existing ApexOS-RS mechanisms:

```
$STATE/nursery/rebirths/
├── history.jsonl                 # append-only log
├── candidates/
│   └── {id}/
│       ├── checkpoint/           # or HF snapshot / GGUF
│       ├── watch_report.json
│       └── metadata.json
└── previous_good → symlink or pointer to last promoted
```

- `previous_good` is never deleted by the system.
- Multiple previous generations can be kept (configurable retention).
- Rollback is instantaneous (alias swap + optional process restart of any local servers).
- Cerebro records every promote / rollback with full lineage so the colony can reason about its own evolution.

---

## 9. Future Vision (Stage-3 and beyond)

Once the rebirth loop is solid, the following become natural extensions:

1. **Colony-level model evolution**  
   Multiple nodes run parallel rebirth candidates; the mesh votes or benchmarks them; the best is propagated.

2. **Continuous low-intensity improvement**  
   Small, frequent, heavily-watched full or near-full updates on a schedule, still gated by the Watcher.

3. **Specialist → Foundation promotion**  
   An apprentice that has proven itself over many tasks can be used as the seed for the next full rebirth (distillation + continued training).

4. **Self-modifying eval suite**  
   The system can propose new eval cases (with human/agent review) so the Watcher stays ahead of capability drift.

5. **Hardware-aware rebirth**  
   Recipes automatically target the best available cluster (local DGX, Vast multi-node, etc.) and produce quantized variants for every tier (Nano → Pro).

6. **Soul-model co-evolution**  
   Rebirth can optionally update the soul.md / embodiment in lockstep, with the same rollback guarantees.

7. **True recursive loop**  
   The driver model that is itself a product of previous rebirths uses the Nursery tools to birth the next generation. At that point the system is recursively improving its own foundation under hard safety constraints.

---

## 10. Implementation Phasing (relative to Stage-1)

| Phase | Scope | Depends on |
|-------|-------|------------|
| R0 | Design freeze + config surface (this document) | — |
| R1 | Rebirth score calculator + tool stubs | Stage-1 Nursery |
| R2 | Full-param / high-rank recipes in Router | Stage-1 training jobs |
| R3 | Model Watcher v1 (tool-calling + format + regression) | R2 |
| R4 | Promote / rollback + previous_good management | R3 |
| R5 | UI triggers + colony voting (optional) | R4 |
| R6 | Continuous / scheduled rebirth policies | R5 |

Stage-1 can ship and run for a long time before any of R1–R6 is required. The existence of the rebirth path is what makes the overall vision coherent.

---

## 11. Safety Summary

- Rebirth is gated by an explicit score threshold.
- Training itself is still ledgered and approval-gated.
- No candidate ever becomes the default driver without passing the Watcher.
- Previous known-good is always retained and one-command restorable.
- All decisions and artifacts are durable in Cerebro and the Router ledger.
- The same cultural and technical posture that protects the binary and soul.md now protects the model weights.

This is how ApexOS-RS can grow a real recursive self-improvement capability without gambling the colony on an unvalidated weight update.

---

*"From the Nursery, new minds emerge.  
From the Rebirth, the foundation itself is renewed — only when the Watcher says it is good."*
