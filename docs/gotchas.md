# Gotchas — the invariant ledger

> **RULE: before modifying ANY subsystem, grep this file for it and read the matching
> entries.** These are load-bearing invariants — most were written after something broke
> on a live node, and many end with an explicit "don't do X" that a future change would
> otherwise walk straight into.
>
> **A newly discovered gotcha goes HERE**, not in CLAUDE.md. Docs travel with code —
> update this file in the same PR as the change that discovered or altered an invariant.
>
> Format: one bullet, **bold lead naming the invariant**, then the story, ending with the
> explicit don't. Cross-project version drift lives in
> `~/Projects/Launchpad-RS/docs/sharp-edges.md` instead.

- **A `ModelRecord` never stores liveness.** No `deployed`, no `live`, no `serving`, no
  `status`. Whether an alias actually answers is ApexRouter's truth — it depends on a process
  we do not supervise, on a box we did not rent (D2/D4), so anything we persist is a lie the
  moment Router restarts, the tunnel drops, or the box is parked. The record stores what
  Puerperium *did*: artifact, `alias_requested`, dataset hash, trainer, parent. A test asserts
  those words never appear in the serialized form. Same rule downstream: `ApprenticeRecord` has
  no `trained: bool` because that is `model.is_some()`. **Don't add a field that restates
  another field, and don't cache a remote system's state in a local record.**

- **The lineage walk needs its cycle guard.** Registry records are plain JSON that a human can
  edit, and nothing stops two models naming each other as parent. Without the `seen` set the
  walk hangs forever on `a → b → a`. The guard reports the cycle in `incomplete` rather than
  erroring, because a partially-broken registry should still answer as much as it can.
  **Don't remove the guard on the grounds that "records are generated" — they are also
  hand-fixable, which is the point of keeping them as readable JSON.**

- **A dataset hash mismatch is reported, never repaired.** If a model names a dataset whose
  bytes no longer hash to what the record says, that is precisely the situation lineage exists
  to catch — the model was trained on data that is no longer there. `dataset_hash_mismatch`
  surfaces it. **Don't "fix" it by refreshing the recorded hash from disk; that destroys the
  only evidence that the provenance is broken.**

- **A missing record is `RecordNotFound`, never a raw io error.** `dataset::read_meta` leaked
  an ENOENT chain to whoever referenced a deleted dataset, while `store::load` gave a clean
  named error for the same situation. Two shapes for one condition means callers handle one and
  not the other. **Don't return a bare `Error::Io` for a lookup that failed because the thing
  is simply not there.**

- **Every new `DatasetMeta` field must be `#[serde(default)]`.** Datasets are durable artifacts
  referenced by hash for the life of the registry, and their sidecars are read by binaries newer
  than the one that wrote them. Adding `framing` without a default made *every* previously-written
  dataset unreadable — `puerperium data list` died with `missing field \`framing\`` and there was
  no way back except regenerating, which changes the hash and therefore breaks lineage. Regression
  test: `dataset::tests::sidecar_written_before_a_field_existed_still_loads`. **Don't add a
  sidecar field without a default, and don't "clean up" an existing one by removing its default.**

- **A document title lifted from line 1 must be excluded from the section body.** `chunk()`
  extracts the first line as the doc title into every chunk's `heading_path`. Leaving that line in
  the body emitted a one-line preamble chunk which — being shorter than `min_section` — merged
  *forward* into the first real section and overwrote its heading path with the title-only one.
  Symptom: every sectioned document produced `["DOC"]` instead of `["DOC", "Section"]`, so every
  instruction read "Explain DOC." **Don't reintroduce the title line into the section stream; and
  note the deliberate asymmetry — unsectioned content *keeps* its first line, because there the
  body is the whole memory.**

- **A2A messages carry `msg` / `from:X` / `to:Y`, not `message`.** The first denylist had
  `message` and missed every real agent-to-agent memory in the store; they sailed through and
  became training examples like *"What do you know about msg, from:CLAUDE, and to:HERMES-KRKN?"*.
  Routing tags are now denied by **prefix** (`filter::is_routing_tag`), not by exact match.
  **Don't assume a tag vocabulary — check it against the real store before trusting a denylist.**

- **Tag-derived instructions are the weak half and must stay labelled.** 70 of 182 examples from
  the real store were framed from tags rather than headings, and before the topical-tag filter
  they read *"What do you know about phase-6, completion-summary, and session-notes?"* —
  grammatical and empty. Bookkeeping tags, routing tags and bare years are excluded from framing,
  and every example still records `instruction_kind` so a consumer can weight or filter.
  **Don't silently mix framing strategies, and don't invent a question ("Explain the following.")
  for a chunk that cannot be framed — count it `Unframeable` instead.**

- **The conversion pipeline stays pure and I/O-free.** `convert()` takes materialised
  `MemoryRecord`s and returns examples plus a ledger. That is what let the whole quality gate be
  tested against real captured content with no Cerebro running, and what made the three defects
  above findable in one afternoon. **Don't reach for Cerebro, the filesystem or the network from
  inside `convert/` — the source adapters belong outside it.**
