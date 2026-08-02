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

- **Together's API applies NO defaults — the SDK does, client-side.** Omitting a field is not
  "use the default", it is sending zero. An absent `batch_size` is refused with *"batch size is
  zero"*, an absent `n_checkpoints` with *"number of checkpoints is less than one"*. The body
  must carry the SDK's full default set explicitly. **Don't assume an optional-looking field is
  optional.**

- **`training_method` and `lr_scheduler` are OBJECTS, not strings.** `"training_method": "sft"`
  is refused with `Could not create the FineTune object (Binding)` — a body-binding type
  mismatch that names no field, so it reads like a model problem when it is a shape problem.
  Send `{"method":"sft"}` and
  `{"lr_scheduler_type":"cosine","lr_scheduler_args":{...}}`. **Don't debug a "(Binding)" error
  by changing the model.**

- **The upload-URL request is FORM-ENCODED.** Query params and a JSON body both return
  `400 "Unable to save the file - invalid purpose specified"` — the *same* message for every
  `purpose` value, because the server never sees the field at all. That identical message is
  the tell: it means "absent", not "wrong". **Don't chase the enum value; check the encoding.**

- **`batch_size: "max"` is resolved by the client, not the server.** The SDK looks up
  `GET /v1/fine-tunes/models/limits?model_name=…` and substitutes a number. Some models publish
  `min == max`, so there is exactly one legal value. That endpoint is **free** and is also the
  honest way to learn whether a base is fine-tunable at all — a model that is not answers with
  a `message` instead of limits. **Don't discover an unsupported base by submitting.**

- **`lora_trainable_modules: "all-linear"` is rejected by models that publish a specific list.**
  `Qwen/Qwen3.6-35B-A3B` accepts only `k_proj,o_proj,q_proj,v_proj`. Read `target_modules` from
  the limits call and send those. **Don't hard-code "all-linear".**

- **A `-Lora`/`-FP8` suffixed name is not a fine-tune base.** The limits endpoint refuses both
  `Qwen/Qwen3.6-35B-A3B-Lora` and `-FP8` as "not available for fine-tuning", while the
  unsuffixed `Qwen/Qwen3.6-35B-A3B` is accepted. The suffixed names are serving endpoints.

- **A Router backend row is configuration, not liveness.** `GET /v1/backends` returns
  `enabled: true` for a vast recipe whose box is **cold and unreachable** — enabled means
  "enabled in config", not "answering now". Our first `compute` output printed `enabled`, which
  reads as *available*, and that is the exact D3 failure mode (a status that is a lie the moment
  the box dies) reproduced in our own CLI. It now prints `configured` and says so explicitly.
  **Don't feed a backend listing straight into `--available-compute` as though it proved a box
  was up; probe first (`/v1/backends/{id}/probe`).**

- **The fine-tune base differs by provider — check the catalogue, don't assume.** Together
  carries no dense `Qwen/Qwen3.6-27B`, which was our default; its LoRA-capable Qwen3.6 base is
  `Qwen/Qwen3.6-35B-A3B`. The dense 27B is right for the local/vast path, where the garden node
  serves it today. `router::serves_model` and `router::lora_capable_bases` answer this **for
  free, locally**, off ApexRouter's existing backend listing. **Don't hard-code one base as
  "the" base, and don't discover an unsupported one by paying for a failed job.**

- **A `-Lora` suffix names a serving endpoint, not a base.** `Qwen/Qwen3.6-35B-A3B-Lora` is
  where adapters *of* `Qwen/Qwen3.6-35B-A3B` are served. Fine-tune the unsuffixed name.
  **Don't submit the suffixed one as a base.**

- **Never send `Origin` or `Sec-Fetch-Site` to ApexRouter.** Its mutation gate reads: if
  `Origin` is present it must be same-origin; if `Sec-Fetch-Site` is present it must be
  `same-origin` or `none`; **otherwise a bearer with `write` scope is required.** Non-browser
  clients send neither and pass unchanged, which is why our client works on loopback with no
  token. **Don't add either header "for completeness" — every mutation would start 403'ing
  unless a token happened to be configured.**

- **An ApexRouter `CredentialSource` is a pointer, never key material.** Its own schema says
  "A DESCRIPTION of where a credential lives." We send `{kind:"env", var:"TOGETHER_API_KEY"}`,
  so Router learns the variable's *name* and the key never leaves our process. **Don't put a
  key in a NodeSpec.**

- **Reuse a backend that already points at the URL.** Router already had a `together` backend;
  registering a second for the same base URL leaves two rows that disagree the moment either is
  edited. `backend_for_base_url` checks first. **Don't blind-register.**

- **A Cerebro database is opened `SQLITE_OPEN_READ_ONLY`, always.** It is another tool's state
  directory and usually a live daily driver — the same posture ApexRouter takes toward
  `~/.vastai-gguf/`: read it, never write it. Read-only open also means a typo'd path *fails*
  instead of creating an empty database that then reads as "no memories". A test asserts the
  file is not created. **Point this at a `.backup` snapshot, not a live file — copying a
  database being written is not safe, `.backup` is. Don't add a write path, a migration, or a
  `create_if_missing`.**

- **Mine procedural memories when quality matters.** On the real store, restricting to
  procedural gave **84% heading-framed** examples (96/18) against 62% (112/70) with semantic
  included — because the structured memories *are* the procedural ones (22 of 59 carry `##`
  sections, versus 3 of 78 semantic). Semantic widens coverage and dilutes framing. **Don't
  read a small example count as a bad run — check the framing split before widening the type
  filter.**

- **The stored dataset and the uploaded file are different artifacts.** Together's validator
  rejects unknown columns outright (`InvalidFileFormatError: Found extra column`), and our
  JSONL carries `provenance` and `instruction_kind` beside `messages` because lineage is the
  product (D12). Uploading a stored dataset verbatim would be refused. `export::to_provider_jsonl`
  projects down to `{"messages": [...]}`; the stored file is never mutated, so its hash — the
  lineage identity — survives. **Don't "simplify" by uploading the stored bytes, and don't
  strip provenance from the stored file to make them match.**

- **Do not follow redirects on the Together client.** The upload flow returns the presigned
  URL in a `Location` header and the file id in `X-Together-File-Id`. A client that follows
  redirects automatically consumes both, PUTs an empty body to storage, and reports success —
  an upload that silently uploaded nothing. `Policy::none()` is set at construction.
  **Don't remove it, and don't add a second client that defaults to following.**

- **The presigned PUT must not carry the bearer token.** The signature in the URL *is* the
  authorisation, and the target is third-party storage — attaching our API key would hand it
  to a host that has no business seeing it. **Don't add `.bearer_auth()` to the upload PUT
  "for consistency" with the other calls.**

- **An unrecognised upstream status is `Unknown`, never `Running`.** Together's status
  vocabulary has ten values today and will have more tomorrow. A parser that falls through to
  a plausible default turns a state it has never seen into a confident claim — and this
  particular mapping governs whether a *paid* job is treated as finished. `map_status` returns
  `Phase::Unknown` for anything unlisted, `Unknown` is not terminal, and the raw string is kept
  in `upstream_status` so the surprise is diagnosable. **Don't add a catch-all arm that guesses.**

- **"We could not ask" is not "they said no".** An unreachable provider leaves the job
  **non-terminal** — it may well be running and billing, and doctrine #9 says a paid run that
  outlives our patience is still running. A *rejected* submit is terminal, with the upstream's
  own words. Collapsing the two either orphans spend or invents a failure that never happened.
  **Don't map a transport error onto a job outcome.**

- **Check compute before building a provider.** The CLI evaluated `together()?` while
  assembling the call, so a missing `TOGETHER_API_KEY` surfaced *instead of* "that box does not
  exist" — the operator gets told to fix the wrong thing. `engine::check_compute` is public and
  free precisely so a caller can gate first; `submit` calls the same function, so there is one
  source of truth rather than a preview and a real check. **Don't reorder so that a credential
  lookup precedes a D4 refusal.**

- **`TogetherClient` implements `Debug` by hand, and must keep doing so.** A derived `Debug`
  prints the API key verbatim the first time anything formats the client — a log line, a panic
  message, an `{:?}` in a hurry. The manual impl reports length only, with a test asserting the
  value never appears. **Don't `#[derive(Debug)]` on anything holding a secret.**

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
