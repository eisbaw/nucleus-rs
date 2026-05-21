---
id: TASK-0134
title: >-
  Translate pipeline=D and reuse loop options to initial markings on buffer
  places
status: To Do
assignee: []
created_date: '2026-05-18 03:36'
updated_date: '2026-05-21 13:10'
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
