---
id: TASK-0134
title: >-
  Translate pipeline=D and reuse loop options to initial markings on buffer
  places
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-18 03:36'
updated_date: '2026-05-21 14:10'
labels:
  - M2
  - M4
  - compiler
  - ir
  - scheduling
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §8.2 maps "Initial marking on a place = Pipeline depth / latency-hiding head-start" and PRD §6.3.3 says `pipeline=D` "Software-pipeline with depth D (replaces double-buffering)". Currently `block_transform.rs:111` says pipeline= is a no-op; `acfg_to_petri.rs:80-89,120-123` documents the gap (no ACFG carrier; buffer-place initial_marking=0 unconditionally). This task is the IR/Petri layer of M4 — backend-agnostic, testable in isolation via boundedness + Petri inspection.

## Two-step decomposition (do both in this task; they are coupled)
1. Add a carrier from SchedIR `ResolvedLoopOption::Pipeline(d)` through ACFG → buffer-place-creation in `acfg_to_petri.rs`. Likely shape: an ACFG-level map `pipeline_depth_for_seq: BTreeMap<SeqTag, NonZeroU64>`, populated during `transfer_inject`/`block_transform` by walking the enclosing loop annotations of each Push/Wait pair. Reject `pipeline=0` and `pipeline=1` upstream (1 = no pipelining; pick exactly one of: hard error, or silent no-op — document the choice).
2. In `acfg_to_petri.rs` buffer-place creation: if the seq is in the pipeline-depth map, set `initial_marking = D`; otherwise stay at 0 (current behaviour).

## Constraint: D ≤ buffer=N
A `pipeline=D` on a loop whose downstream transfer is `buffer=N` requires `D ≤ N`. If `D > N`, the boundedness pass would trip — but the diagnostic should be a TYPED `LinkError`/`SchedLowerError` BEFORE the Petri net is built, naming the offending loop + transfer + the actual D and N. Acceptance criteria #4 below.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Carrier: ResolvedLoopOption::Pipeline(D) flows through transfer_inject (or a sibling pre-acfg_to_petri pass) into a deterministic BTreeMap<SeqTag, NonZeroU64> annotation on the ACFG (or carried alongside as a sidecar). Mechanism documented in the pass docstring; cite the file:line of the carrier definition.
- [ ] #2 Initial-marking: acfg_to_petri.rs buffer-place creation reads the pipeline-depth annotation; sets place.initial_marking = D for that SeqTag's buffer place; absence = 0 (existing behaviour preserved). Module docstring §'Initial markings' updated to reflect the new translated case (NOT 'not yet'). No doc-lie.
- [ ] #3 Constraint: pipeline=D with downstream buffer=N requires D <= N. A typed Result error (SchedLowerError or LinkError, judge: closer-to-source layer wins; rationale documented) names the offending {loop_var, transfer_data, D, N} BEFORE acfg_to_petri runs. Positive test: D=N passes; D=N-1 passes. Negative tests: D=N+1 hard-fails with exact span; D=0 fails upstream (parser rejects; verify with a negative parser test).
- [ ] #4 pipeline=1 is rejected as 'no-op pipelining; specify pipeline=D with D>=2 or omit'; OR documented as accepted-but-no-op. Pick one explicitly, justify, and put the test in. Do not leave the semantics ambiguous.
- [ ] #5 Petri test: lower a fixture with pipeline=3, buffer=3 transfer, sample 2-iteration loop body. Inspect Net via --emit-pn or unit test; assert buffer place initial_marking=3; assert boundedness pass still passes; assert deadlock pass still passes; assert determinism (build the net twice, structurally identical).
- [ ] #6 Existing fixtures regress unchanged: every example without pipeline= still has buffer place initial_marking=0; nucleus/e2e matrix bit-identical x2 vs determinism gate; clippy --workspace --all-targets clean; just ci exit 0.
- [ ] #7 Forward-carried lesson into TASK-0042 (and any pthreads-async sub-task once filed): when codegen lands, the ring-buffer must be pre-populated with D 'empty slots' (or producer-runs-ahead semantics matching the initial-marking) — the IR contract is now 'D producer tokens ready to fire before any consumer'. Do NOT defer-translate at codegen; the Petri net is the authoritative encoding.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Implementation plan (TASK-0134)

### Design choice (resolved)
- **Interpretation (a)**: every transfer inside a `pipeline=D` loop body gets initial_marking = D on its buffer place. Producer-runs-ahead. Rejected (b) (per-stage decremented marking) because (i) we have no stage-numbering metadata in the ACFG and synthesising it would couple this to a tropic-sort pass we don't have; (ii) the boundedness pass still polices each buffer place's `buffer=N` capacity, so an oversized (a) marking is caught upstream; (iii) (a) is a conservative upper bound — the kernels can fire fewer head-start producer iterations and still respect the contract, but (a) gives backends a clear "ring is pre-armed with D producer credits" semantics.
- **pipeline=1 policy**: HARD ERROR ("no-op pipelining; specify pipeline=D with D>=2 or omit"). Rationale: pipeline=1 has *no* meaningful semantics — it would mark the buffer place with 1 token, which is identical to no marking + the producer firing once before the consumer; that is just the default sequential flow. Accepting it would be a silent footgun (users would think they're pipelining). Rejecting it forces a clear schedule.
- **D ≤ N constraint**: rejected as `LinkError::PipelineExceedsBuffer { loop_var, data, depth, buffer }`. Closer-to-source layer (Link) wins because we have schedule directives & data-symbol names in source-friendly form at that layer; rejecting later (inside acfg_to_petri) would lose the loop_var name. (Variant rationale recorded in the link.rs docstring.)

### Carrier shape
- New ACFG sidecar `pipeline_depth_for_seq: BTreeMap<SeqTag, NonZeroU64>`, `#[cfg_attr(serde, default)]`, mirroring `inner_block_iter_vars` / `partition_worker_ranges` pattern (sidecar > struct field — leaves Repeat payload + every existing match unchanged).
- Populated in **transfer_inject**: when a Push/Wait pair is created, walk `enclosing_tile` against `linked.sched.loops`; if any enclosing loop carries `ResolvedLoopOption::Pipeline(D)`, insert seq -> D. (If multiple enclosing loops carry pipeline=D, choose the **innermost** — most-recently-entered; corresponds to "the stage producer/consumer pair lives in".)
- Carrier point: a single new helper `pipeline_depth_for_tile(tile, linked.sched) -> Option<NonZeroU64>`.

### Constraint check
- Lives in `link.rs` (`check_pipeline_constraints`), called from `link()` after the existing 6 checks. For each loop directive that has `Pipeline(D)`, for each transfer directive whose data is read by an Operation inside that loop, assert D <= buffer(transfer). Source of "which transfers are inside loop L" comes from the algo's `for VAR : ...` body — we can use the existing `collect_loop_vars` traversal extended to collect (loop_var -> data symbols read inside).

### acfg_to_petri changes
- `buffer_place_for` reads `acfg.pipeline_depth_for_seq.get(&x.seq)`; if present, pass D (clamped to u32::MAX, NonZeroU64 -> u32 conversion) as `initial_marking` to `add_place`; else 0.
- Module docstring "Initial markings" §: rewrite from "not yet" to the actual current behaviour.

### Tests
- Unit (acfg_to_petri.rs): synthetic ACFG with `pipeline_depth_for_seq` populated -> initial_marking on the buffer place == D.
- Unit (transfer_inject.rs): real LinkedIR with `loop n : pipeline=3` -> resulting ACFG's sidecar has one entry per Push/Wait pair seq.
- Unit (link.rs): pipeline=4 + buffer=3 -> LinkError::PipelineExceedsBuffer; pipeline=3 + buffer=3 -> ok; pipeline=2 + buffer=3 -> ok.
- Negative parser: pipeline=0 already rejected by `positive()` — confirm with a test (probably exists; if not, add one).
- Lower-sched: pipeline=1 -> SchedLowerError. New variant + display.
- Determinism: build the example-13 pipeline_parallel net twice; equal.

### Files to touch
- `nucleus/compiler/src/acfg.rs` — add sidecar field.
- `nucleus/compiler/src/passes/transfer_inject.rs` — populate sidecar; thread it through.
- `nucleus/compiler/src/passes/acfg_to_petri.rs` — read sidecar; update docstring.
- `nucleus/compiler/src/link.rs` — add `PipelineExceedsBuffer` variant; add `check_pipeline_constraints`.
- `nucleus/compiler/src/sched/lower.rs` — reject pipeline=1.
- Tests as above.

## Final implementation summary (TASK-0134)

### Commit hashes
- 08c3540 — pipeline=D loop options lower to buffer-place initial markings (core IR/passes/link/sched)
- 478f57c — tests covering AC#1-#6 (AC#5 boundedness `#[ignore]`d -> TASK-0213)
- 918741c — backlog: TASK-0134 progress + TASK-0213 follow-up

### Gate numbers (actual, end of cycle)
- `just build`: clean.
- `just test`: 503 passed, 0 failed, 3 ignored (one new ignore is the TASK-0213-deferred boundedness assertion).
- `just e2e`: 36 cells: 29 pass, 0 fail, 7 skipped (capability mismatches + open distributed-placement tasks — unchanged set).
- `just determinism-check`: 36 cells: 29 pass, 0 fail, 7 skipped — byte-identical x2 across runs.
- `just determinism-check-negative`: 29 of 36 cells correctly perturbed (negative gate bit).
- `just ci`: exit 0.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.

### Per-AC status

- **AC#1 carrier (DONE)**: `ACFG::pipeline_depth_for_seq: BTreeMap<SeqTag, NonZeroU64>` declared in acfg.rs (~line 627). Populated by `transfer_inject::annotate_pipeline_depth_for_seq` (transfer_inject.rs ~line 408) as a post-pass over the FINAL ACFG, reading each `Xfer::tile`. Post-pass placement was load-bearing: the at-build-time `enclosing_tile` is stale after `hoist_invariant_waits` moves a Wait out of the pipelined loop.
- **AC#2 initial-marking (DONE)**: `acfg_to_petri.buffer_place_for` reads the sidecar and passes `D` to `add_place` as `initial_marking_u32`. Module docstring §"Initial markings" rewritten — the prior "not yet" wording is gone; no doc-lie. Honest-limitation bullet retired for pipeline, kept for `reuse` (different carrier shape).
- **AC#3 constraint (DONE)**: `link::check_pipeline_buffer_constraints` rejects `D > buffer(N)` with `LinkError::PipelineExceedsBuffer { loop_var, data, depth, buffer }`. Fires only when the data symbol is BOTH produced and consumed inside the loop — mirrors the IR-level hoist semantics so the link diagnostic and the IR contract agree. Positive tests for D=N and D=N-1; negative tests for D=N+1 and default-buffer-1 vs D=3. `pipeline=0` already rejected by `positive()` (`SchedLowerErrorKind::ZeroLoopOption`).
- **AC#4 pipeline=1 (DONE)**: `SchedLowerErrorKind::UnitPipelineOption` introduced; sched_lower.rs:868 rejects `pipeline=1` with a message naming the loop var and suggesting `D >= 2 or omit`. Tested both negative (pipeline=1 -> error) and positive (pipeline=2 -> OK).
- **AC#5 Petri test (DONE except boundedness/deadlock — DEFERRED to TASK-0213)**: real fixture (example 13 pipeline_parallel) asserts buffer place initial_marking=3 for feat1/feat2, 0 for input/output (hoist semantics). Determinism (two lowerings of the same input -> structurally-equal nets) passes. Boundedness assertion deferred: with `initial_marking=D=N`, the source-order firing trips on the first Push (buffer full at startup, no room for one more deposit). Two clean resolution paths documented in TASK-0213.
- **AC#6 regression (DONE)**: 503 unit tests pass; e2e 36 cells unchanged set (29 pass, 7 skipped — same as before); determinism gate byte-identical; clippy clean. The fixture-update sweep (sidecar field added to test ACFG hand-built constructors) is mechanical — no behavioural delta in those tests.
- **AC#7 forward-carry (DONE)**: noted in acfg_to_petri.rs docstring + the next note below for TASK-0042.01.

### Follow-ups filed
- TASK-0213 — Boundedness pass must be initial-marking-aware for pipelined nets. Blocks AC#5's boundedness/deadlock assertion.

### Design choice resolution

Interpretation (a) of PRD §8.2 — every transfer inside a `pipeline=D` loop body gets `initial_marking = D` independently. Rationale:
- Conservative upper bound: in steady state the buffer holds D head-start credits per pair.
- The link-step `D <= N` check makes (a) consistent with the buffer's declared capacity.
- (b) — stage-decremented markings — was rejected because the ACFG has no stage-numbering metadata; synthesising it would require an extra tropic-sort pass orthogonal to TASK-0134's scope.
- The boundedness/deadlock interaction (AC#5 deferral) is documented as a known limitation of (a) under source-order firing: TASK-0213 will resolve it by either marking-aware firing-order generation or a structural acfg_to_petri rewrite.

### Honest limitations / gotchas / rejected approaches

1. **Boundedness pass tension**. With interpretation (a) and `D = N = capacity`, the buffer place is FULL at startup. Source-order firing then trips the first Push (would-be=D+1, capacity=D). The current `derive_firing_order` is marking-blind. TASK-0213 captures the precise fix space.

2. **Hoist invariance changes the annotation lifecycle**. The first implementation registered pipeline depth at `fresh_seq()` time (during the recursive walk). That was wrong: `hoist_invariant_waits` later moves a Wait OUT of the loop body and REWRITES its `tile` to the new enclosing context (transfer_inject.rs:733). Annotating at fresh_seq leaves a STALE D for the now-hoisted seq, which would over-fill the buffer. The post-pass approach (walk the final ACFG, use each Xfer's final `tile`) avoids this — but the lesson is general: any IR annotation derived from "at-build-time enclosing context" must be recomputed AFTER all the structural rewrites complete.

3. **Link-step check mirrors hoist semantics**. `input` in example 13 has producer (load_input) outside the loop, consumer (conv_block_1) inside. transfer_inject hoists the Wait OUT (loop-invariant whole-symbol transfer). So the IR-level pipeline_depth_for_seq has no entry for input's seq. The link-step check must also skip this case — otherwise the link error and the IR contract would disagree (link rejects D=3 vs buffer=1, but the IR would never set initial_marking=3 for input anyway). Mirror by requiring "BOTH producer and consumer inside the loop" at link time.

4. **`pipeline=1` is a hard error, not silent-accept**. The PRD didn't pin the semantics. Hard-error chosen because (i) initial_marking=1 is equivalent to the default sequential producer-then-consumer pattern; (ii) accepting silently would hide a user error (they probably meant `pipeline=2+` or `omit`); (iii) the error message tells them how to fix.

5. **The `output` transfer in example 13 pipeline_parallel**. The schedule writes `transfer output : sync;` (default buffer=1). With pipeline=3, naively this looks like a buffer-too-small bug — `output[n]` is written inside the loop. But `save_output(output)` is OUTSIDE the loop, and `save_output` reads `output` as a whole array (no per-iteration consumer side). transfer_inject hoists the Wait out (Push is at top level too, by splice_pushes_global). The buffer place stays at initial_marking=0. So buffer=1 (default) is fine — the link check correctly skips it.

6. **block_transform interaction**: when both `block=` and `pipeline=` apply to the same loop var, `block_transform` runs first, splitting V into V (inner, intra-tile) and V__tile (outer). The pipeline depth is then looked up against the INNER iter_var's id (block_transform keeps the original IterVar id for the inner loop). This means pipeline applies to the intra-tile loop. Not exercised by any current example; documented in code as the lookup-by-IterVar-id semantic.

### Files touched (absolute paths)
- /home/mpedersen/topics/mark_thesis/nucleus/compiler/src/acfg.rs
- /home/mpedersen/topics/mark_thesis/nucleus/compiler/src/passes/acfg_to_petri.rs
- /home/mpedersen/topics/mark_thesis/nucleus/compiler/src/passes/transfer_inject.rs
- /home/mpedersen/topics/mark_thesis/nucleus/compiler/src/passes/sync_inject.rs
- /home/mpedersen/topics/mark_thesis/nucleus/compiler/src/passes/partition_workers.rs
- /home/mpedersen/topics/mark_thesis/nucleus/compiler/src/passes/block_transform.rs
- /home/mpedersen/topics/mark_thesis/nucleus/compiler/src/sched/ir.rs
- /home/mpedersen/topics/mark_thesis/nucleus/compiler/src/sched/lower.rs
- /home/mpedersen/topics/mark_thesis/nucleus/compiler/src/link.rs
- /home/mpedersen/topics/mark_thesis/nuc-nucleus/examples/13-cnn-inference/schedules/pipeline_parallel.sched.nuc (doc-only comment update)
- /home/mpedersen/topics/mark_thesis/nucleus/compiler/tests/{acfg_to_petri.rs, link.rs, sched_lower.rs, transfer_inject.rs} (new tests for ACs)
- /home/mpedersen/topics/mark_thesis/nucleus/compiler/tests/{partition_workers.rs, petri_to_events.rs, sync_inject.rs, transfer_inject_hoist.rs} (mechanical sidecar field add)

## Cycle outcome: HONEST-PARTIAL (TASK-0213 deferral on AC#5 boundedness)

6 of 7 ACs fully DONE (AC#1, AC#2, AC#3, AC#4, AC#6, AC#7). AC#5 split:
- DONE: initial_marking emission, determinism gate, test fixture lowering.
- DEFERRED via TASK-0213: boundedness/deadlock assertion. With `initial_marking=D=N=capacity`, source-order firing trips on the first Push. derive_firing_order is currently marking-blind. TASK-0213 specifies two clean resolutions: marking-aware firing-order generation, or structural acfg_to_petri rewrite that elides D pre-fired Push transitions. The IR contract (sidecar + initial_marking) is in place; the firing-order generation is the missing piece.

Recommended state: leave TASK-0134 In Progress until TASK-0213 lands and the `#[ignore]`d boundedness assertion goes green. The IR-layer M4 piece (per the task brief "IR/Petri-layer load-bearing piece of M4") is complete and downstream (TASK-0042.01) can build on the contract. The boundedness gap is downstream-analysis-only; it does NOT block codegen, which reads `pipeline_depth_for_seq` directly.

Review-gate hardening (cycle close):

mped-architect review (read-only, this cycle): GO with conditions.
qa-test-runner review (read-only, this cycle): GO clean.

Findings folded in-thread:
- transfer_inject.rs annotate_pipeline_depth_for_seq docstring: "innermost wins"
  now explicitly cites IterTile::bounds outer-to-inner convention as
  load-bearing; "Wait wins" overclaim corrected to "last-visited wins"
  with the source-order rationale.
- event.rs IterTile::bounds docstring strengthened — the outer-to-inner
  convention is now flagged as load-bearing for downstream passes,
  with explicit pointer to the partition-rewrite caveat (TASK-0216).
- link.rs check_pipeline_buffer_constraints docstring softened: now
  states it mirrors specifically hoist_invariant_waits's semantic,
  with an explicit Caveat block calling out the same-worker
  src==dst skip gap (TASK-0214).
- block_transform.rs module doc now points at where pipeline=D IS
  consumed (transfer_inject post-pass + acfg_to_petri) and flags
  block= + pipeline= as untested (TASK-0215).
- New test: pipeline_exceeds_buffer_coexists_with_other_link_errors
  (link.rs tests) — pins cascade-safety claim that the new error
  rides the independent-error path and surfaces alongside an
  UnknownLoop in one pass.

Follow-ups filed for the medium-severity findings that did not fit
in-thread (each with the precise root cause + acceptance criteria):
- TASK-0214: same-worker transfer directive vs PipelineExceedsBuffer.
- TASK-0215: block=N + pipeline=D combination semantics.
- TASK-0216: partition=workers + pipeline=D coverage; partition-rewrite
  path's IterVar-id-order vs nest-order tile bounds.

Post-hardening gate (re-run in nix develop):
- cargo test workspace: 504 pass / 0 fail / 3 ignored (added 1 test).
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- just e2e: 36 cells / 29 pass / 0 fail / 7 skipped / 0 required-fail.

AC#5 stays PARTIAL (boundedness/deadlock under pipelined initial
markings → TASK-0213). Task remains In Progress with that honest
deferral; the rest of the AC set is verified by the (now reviewed)
implementation.
<!-- SECTION:NOTES:END -->
