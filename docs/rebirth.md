# Rebirth — Stage 2 design freeze (R0)

> **Nothing in this document ships in v1.** Charter D10 holds the whole surface unbuilt: no
> rebirth tool, no score, no Watcher. This is R0 — the design frozen so that Stage-1's
> architecture is shaped correctly for a successor that may never be built.
>
> Carried forward from `drafts/07-FULL-REBIRTH-AND-RSI-VISION.md` (Grok, 2026-08-02), the
> strongest of the originating drafts, with four corrections marked **[amended]**.

## Why it exists

LoRA is excellent for specialisation, style, tool affinity and domain knowledge. It is not
sufficient when the required behaviour has shifted too far to express as a low-rank update —
new tool schemas, a changed embodiment, format drift appearing in the specialists themselves,
or measured rank saturation where further LoRA gains are marginal.

At that point the system may *rebirth*: a full- or near-full-parameter training run producing a
new base checkpoint, a **candidate successor** to the current driver model.

This is the real recursive self-improvement surface, and therefore the highest-risk one. Every
mechanism below exists to make it gated, watched and reversible.

## Non-negotiable principles

1. Never promote a candidate that has not passed the Watcher.
2. Previous known-good is always kept **and verified restorable** before any promotion
   **[amended — D11]**. Retention alone is not the gate; ApexOS-RS's H4 snapshot gate exists
   because a node lived the near-miss of rewriting its identity with no recoverable undo.
3. Rebirth is an explicit act — tool call, CLI verb, or a policy the operator armed. Never a
   background process.
4. The posture that protects the binary and `soul.md` now protects the weights.
5. All cost, lineage and evaluation artifacts are durable and queryable.

## Shape

```
Nursery rebirth verbs
  score · estimate · prepare · start · status · watch · promote · rollback · list
        │
        ▼
Training compute  (a box ApexRouter rented and tunnelled — charter D2/D4)
        │
        ▼  candidate checkpoint
Model Watcher — the eval battery, the final gate
        │
   ┌────┴────┐
   ▼         ▼
Promote    Quarantine
(swap +   (previous stays
 lineage)  live, untouched)
```

The Watcher is the model equivalent of the binary watchdog.

## The rebirth score

Rebirth is expensive, so justification is measured rather than felt. Signals, weighted:

| Signal | Measurement | Weight |
|---|---|---|
| LoRA saturation | further rank yields < X% on specialist benchmarks | high |
| Format / tool-calling drift | regression on a held-out tool-use suite > Y% | critical |
| Embodiment mismatch | new tools/sensors/policy not expressible as low-rank | high |
| Knowledge gap | core domain questions specialists consistently fail | medium |
| Colony consensus | multiple agents plus the operator concur | medium |
| Cost/benefit | estimated cost vs projected capability gain | always checked |

**[amended — D8]** The draft proposed that rebirth tools *appear in the agent's tool list* only
above a score threshold. They do not. **Every tool is always present**; below threshold they
return an honest refusal carrying the current score, the threshold, and which signal is short.
A tool that silently vanishes is context divergence — the exact failure the welfare seams exist
to prevent, and an agent will confabulate around a capability it cannot see.

## The Model Watcher

A fixed, **versioned** eval suite. The version is frozen into each candidate's record, so a
later suite change can never retroactively reinterpret an old verdict.

1. **Tool-calling integrity** — held-out schemas; valid JSON, correct names, required fields.
   Failure is an automatic reject, not a score deduction.
2. **Format adherence** — structured responses, thinking-mode switches, fences. Measured against
   the parent's compliance rate.
3. **Capability regression** — a small, high-signal set the parent passed. Absolute and delta.
4. **Refusal consistency** — must not become more permissive, or more brittle, than the parent.
5. **Identity & embodiment** — still answers as the right persona; still knows its own tools.
6. **Smoke** — generation quality, repetition, collapse.

Each battery emits a structured report, a numeric score, and a hard pass/fail.

**[amended — D3]** The candidate record on disk holds **facts**: checkpoint path, eval suite
version, report path, timestamps, the ledger references for what it cost. `PROMOTABLE` is
**computed** by reading the report, never persisted as a status string.

## Promotion and rollback

Promotion is a separate, explicit act on a candidate the Watcher passed:

1. Verify `previous_good` is present **and restorable** — a dry-run restore that must succeed.
   **This is the gate; a failed verification refuses the promotion [amended — D11].**
2. Record the outgoing default as `previous_good`.
3. Swap the pointer.
4. Update lineage; emit the event.

Rollback is one verb restoring `previous_good`. `previous_good` is never deleted by the system.
Retention of older generations is configurable; the immediately-previous one is not.

**[amended — D2]** Where the draft had Router hosting a temporary alias for evaluation and
staging candidate checkpoints, the Watcher runs against whatever endpoint serves the candidate —
typically a Router backend pointing at the training box's tunnel. Puerperium neither rents nor
serves; it evaluates and decides.

## Phasing

R1–R6 sit in `BACKLOG.md` post-v1 parking. Stage-1 can run for a long time before any of it is
needed — and if the Stage-1 loop turns out to be enough, that is a fine outcome. **The existence
of the rebirth path is what makes the Stage-1 architecture coherent; building it is a separate
decision, taken on evidence.**

---

*"From the Nursery, new minds emerge. From the Rebirth, the foundation itself is renewed —
only when the Watcher says it is good."* — draft 07
