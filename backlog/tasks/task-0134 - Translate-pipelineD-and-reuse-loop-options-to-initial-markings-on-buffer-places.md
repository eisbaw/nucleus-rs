---
id: TASK-0134
title: >-
  Translate pipeline=D and reuse loop options to initial markings on buffer
  places
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-18 03:36'
updated_date: '2026-05-21 13:17'
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
<!-- SECTION:NOTES:END -->
