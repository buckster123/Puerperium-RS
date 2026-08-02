<div align="center">

<img src="assets/banner.jpg" alt="Puerperium-RS" width="100%">

<h1>Puerperium-RS</h1>

<p><strong>A nursery for models.</strong><br>
Turn what an agent has already learned into a specialist that knows it — mined from its own
memory, trained, and registered with lineage you can actually trace.</p>

<p>
<img alt="license" src="https://img.shields.io/badge/license-MIT-blue">
<img alt="rust" src="https://img.shields.io/badge/rust-2021-orange?logo=rust&logoColor=white">
<img alt="ci" src="https://img.shields.io/github/actions/workflow/status/buckster123/Puerperium-RS/ci.yml?label=ci">
<img alt="status" src="https://img.shields.io/badge/status-v0.1%20%C2%B7%20charter%20locked-yellow">
</p>

</div>

---

> [!NOTE]
> **Puerperium makes models; it does not serve them, rent GPUs, or spend money on its own.**
> Those are deliberate scope fences, not gaps — see [`docs/CHARTER.md`](docs/CHARTER.md).
> Standalone is a first-class goal: any MCP or CLI consumer can use it with no ApexOS
> dependency at all.

## What it is

An agent that has been working for months knows things its base model doesn't — which tools
actually behave how, which approaches failed, what this particular codebase wants. That
knowledge lives in its memory and dies with its context window.

Puerperium turns it into a specialist. Query the agent's memory for what it learned about a
domain, convert that to instruction data, fine-tune a small adapter on it, and register the
result with enough lineage that *"why is this specialist like this?"* always has an answer.

The loop, deliberately scoped as "baby" RSI:

```
agent notices it needs a specialist
      ↓
dataset generated from its own remembered experience
      ↓
LoRA fine-tune  (on compute someone else rented, under someone else's ledger)
      ↓
adapter registered — lineage, dataset hash, trainer attribution
      ↓
work routed to the specialist → repeat
```

Rewriting the driver model's *own* weights is Stage 2 — designed in
[`docs/rebirth.md`](docs/rebirth.md), deliberately **not built**.

## Where it sits

Sibling to the rest of the Rust colony, each one standalone:
**Cerebro** remembers · **Occipital** reads · **Imaginarium** sees · **Sonus** hears ·
**Callosum** bridges — and **Puerperium raises**.

It reads from Cerebro, trains on compute ApexRouter provisioned, and exposes `nursery_*` tools
over MCP to any agent that wants them.

## Status

**Charter locked, no code yet.** S0 (bootstrap) is done; S1 (dataset garden) is next.
The slice ledger is [`BACKLOG.md`](BACKLOG.md).

## Install

```sh
git clone https://github.com/buckster123/Puerperium-RS
cd Puerperium-RS
cargo build --release --workspace
```

## Docs

| File | What's in it |
|------|--------------|
| [`docs/CHARTER.md`](docs/CHARTER.md) | Binding decisions D1–D12, phases, the scope fence |
| [`docs/design.md`](docs/design.md) | The contract — tool surface, types, job lifecycle |
| [`docs/rebirth.md`](docs/rebirth.md) | Stage 2 design freeze — the weight-rewrite path |
| [`BACKLOG.md`](BACKLOG.md) | Slice ledger — what's shipped, what's next |
| [`docs/drafts/`](docs/drafts/README.md) | The originating brainstorm — provenance, superseded |

## License

MIT — see [LICENSE](LICENSE).
