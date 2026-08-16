<div align="center">

<img src="assets/banner.png" alt="Puerperium-RS" width="100%">

<h1>Puerperium-RS</h1>

<p><strong>A nursery for models.</strong><br>
An agent that has worked for months knows things its base model doesn't.<br>
This turns that into a specialist — and keeps a record of <em>why</em> it turned out the way it did.</p>

<p>
<img alt="license" src="https://img.shields.io/badge/license-MIT-blue">
<img alt="rust" src="https://img.shields.io/badge/rust-2021-orange?logo=rust&logoColor=white">
<img alt="ci" src="https://img.shields.io/github/actions/workflow/status/buckster123/Puerperium-RS/ci.yml?label=ci">
<img alt="tests" src="https://img.shields.io/badge/tests-156-brightgreen">
<img alt="status" src="https://img.shields.io/badge/status-v0.1%20%C2%B7%20first%20job%20live-yellow">
</p>

</div>

---

> [!NOTE]
> **Puerperium makes models. It does not serve them, rent GPUs, or spend money on its own.**
> Those are deliberate fences, not gaps — see [`docs/CHARTER.md`](docs/CHARTER.md). It runs
> standalone: any MCP or CLI consumer can use it with no ApexOS dependency at all.

## The problem

Your agent learned that `text file busy` means you forgot to stop the service. That
`hf download --local-dir` plus `HF_HOME` gives you a double cache. That vast.ai's SSH mode
overrides the Docker `ENTRYPOINT`. Hundreds of small, hard-won, specific things.

All of it lives in a memory store and dies with the context window. The base model never
learns any of it, and the next agent rediscovers it the same painful way.

## What this does

```
agent's own memories  →  instruction data  →  LoRA fine-tune  →  a specialist
        │                                                            │
        └────────────── lineage you can walk back ────────────────────┘
```

```sh
puerperium apprentice create --id ops --db cerebro.db --master-agent APEX \
    --specialization "deployment and debugging" --name ops_hand --dataset-name ops-v1
puerperium job upload ops-v1          # → training_file_id
puerperium job submit  --id j1 --dataset ops-v1 --base-model Qwen/Qwen3.6-35B-A3B …
puerperium lineage ops-v1             # why is this specialist like this?
```

```
gen 0  apexos-ops-v1
        base    Qwen/Qwen3.6-35B-A3B
        trainer FORGE
        data    apexos-ops-v1 (4e4d12df9012) — 231 examples from 149 memories
```

## What makes it different

**Lineage is the product.** Every example records the memory it came from. Every dataset is
content-hashed, and that hash — not its name — is its identity. A model that names a dataset
whose bytes have changed reports `HASH MISMATCH`, because the whole point is to catch exactly
that. Nothing is silently repaired.

**Records hold facts, never status.** A `ModelRecord` never says whether it is deployed —
that is the router's truth, and a `deployed: true` on disk is a lie the moment a tunnel drops.
An apprentice has no `trained` flag; it's derived. Any boolean restating another field is just
two things that can disagree.

**Every refusal explains itself.** "No key configured" beats a timeout. An unreachable provider
is *not* a failed job — it may be running and billing. A *rejected* one is failed, terminal,
carrying the upstream's own words. Those two look identical at the call site and mean opposite
things.

**It stops before spending.** Creating an apprentice never submits a job. Nothing paid fires
from a default flow.

## Two things the real data taught us

Both cost nothing to learn here, and would have cost real money to learn later.

**A memory store contains chatter, not just knowledge.** A live memory, verbatim:
*"Yo HERMES-KRKN! 👋 … Just doing a first smoke test … Can you hear me?"* — 206 characters, past
every length gate. Without a quality filter your worker model learns to say hello. It's a named
regression test now.

**Dream-consolidated memories are the wrong material.** On one node, **1579 of 1629** procedural
memories were minted by the memory system's own consolidation phases — averaging 227 characters
against **4193** for the 50 lived ones, with every specific abstracted away into
*"establish a unified documentation hub as your investigation backbone."* That's the model's own
generic output. Training on it reinforces the abstraction instead of the knowledge underneath.
**Excluded by default.**

The node with 6× more memories yielded **a third as much trainable material.** Volume was never
the signal.

## Status

**v0.1 — the loop runs end to end.** Slices S0–S5 shipped; the first real fine-tune is live on
Together. S6's gate — *a specialist beats its base on a real task, measured* — is **not met yet**,
and won't be claimed until it is.

Deferred with reasons, not forgotten: vast.ai training recipes, a local training supervisor, and
the Stage-2 "Rebirth" path (full weight rewrite behind an evaluation Watcher) which is
[design-frozen](docs/rebirth.md) and deliberately unbuilt.

## Install

```sh
git clone https://github.com/buckster123/Puerperium-RS
cd Puerperium-RS && cargo build --release --workspace
./target/release/puerperium keys      # what's configured — never prints a value
```

## Where it sits

Sibling to the rest of a Rust agent colony, each standalone:
**Cerebro** remembers · **Occipital** reads · **Imaginarium** sees · **Sonus** hears ·
**Callosum** bridges — and **Puerperium raises**.

It reads from Cerebro, trains on compute [ApexRouter](https://github.com/buckster123/ApexRouter-RS)
provisioned, and exposes `nursery_*` tools over MCP.

## Docs

| File | What's in it |
|------|--------------|
| [`docs/CHARTER.md`](docs/CHARTER.md) | Binding decisions D1–D13, phases, the scope fence |
| [`docs/design.md`](docs/design.md) | The contract — tool surface, types, job lifecycle |
| [`docs/harvest.md`](docs/harvest.md) | Session JSONL harvest, license classes, RAG sketch (D13) |
| [`docs/gotchas.md`](docs/gotchas.md) | Invariants, each written after something broke |
| [`docs/rebirth.md`](docs/rebirth.md) | Stage 2 — the weight-rewrite path, design-frozen |
| [`BACKLOG.md`](BACKLOG.md) | Slice ledger — shipped, deferred, and why |

`docs/gotchas.md` is the one worth reading if you're integrating with Together's fine-tuning
API. It records six corrections that each took a rejected submit to find — including that the
API applies **no defaults** (omitting a field sends zero), and that `training_method` is an
object whose string form fails with an error naming no field at all.

## License

MIT — see [LICENSE](LICENSE).

<sub>Banner generated with <a href="https://github.com/buckster123/Imaginarium-RS">Imaginarium-RS</a> · job <code>01KZ2B9TBC4YQH0TZ95XGE7BQC</code></sub>
