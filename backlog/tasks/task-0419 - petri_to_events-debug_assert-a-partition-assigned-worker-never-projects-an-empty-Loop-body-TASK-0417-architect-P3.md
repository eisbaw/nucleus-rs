---
id: TASK-0419
title: >-
  petri_to_events: debug_assert a partition-assigned worker never projects an
  empty Loop body (TASK-0417 architect P3)
status: Done
assignee:
  - '@mped'
created_date: '2026-06-01 23:06'
updated_date: '2026-06-02 00:48'
labels:
  - hardening
  - defense-in-depth
  - silent-drop
  - cycle-239-followup
dependencies:
  - TASK-0417
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect P3 from the TASK-0417 silent-drop audit. petri_to_events.rs:331 `if body_events.is_empty() { continue; }` drops a per-worker Event::Loop when a worker projects zero body events. This is INTENTIONAL and correct (petri_to_events.rs:304-308: a worker that does nothing in a loop gets no Loop, not an empty-bodied one) — it is NOT the build_dataflow silent-statement-drop class. BUT it is silent-by-design and is the one site a future regression of that class could hide behind (an upstream pass failing to populate a worker body would be swallowed silently).

PROPOSED (architect): add a debug_assert (or nuc_trace) that fires if a worker which IS present in the loop`s per_worker_override (partition_ranges[iter_var], i.e. partition=workers assigned it an exclusive slice) projects an EMPTY body — because such a worker should contribute body events. Stripped in release (no e2e/release behavior change), catches the upstream-population bug loudly in dev/test.

MANDATORY PRECONDITION before adding the assert: empirically VERIFY there is NO legitimate case where a partition-assigned worker projects an empty body (trace host-relay / halo-strip / cumulative / reuse interactions). A false-firing debug_assert is itself a panic-on-valid-input defect (the exact class this project rejects). If a legitimate empty case exists, narrow the predicate or keep it as a nuc_trace! diagnostic only. Add a bite test (a synthetic ACFG where a partition-assigned worker has an empty body must trip the assert/trace).

Pointer: nucleus/nucleus-compiler/src/passes/petri_to_events.rs ~309-345 (the walk-scratch + per_worker_override + empty-body continue).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Empirical precondition DISCHARGED: a documented trace establishes whether any LEGITIMATE case exists where a worker present in per_worker_override (partition=workers exclusive slice via partition_worker_ranges[iter_var]) projects an EMPTY body. Trace host-relay, halo-strip, cumulative, reuse, blocks2d (two-entries-per-data) interactions. Finding recorded in task notes.
- [x] #2 Guard added at petri_to_events.rs ~331 empty-body continue: when a worker present in per_worker_override projects an empty body it fires loudly in dev/test (debug_assert!, stripped in release) OR a nuc_trace! diagnostic if a legitimate empty case exists (predicate narrowed accordingly). Release/e2e behavior byte-unchanged.
- [x] #3 Bite test: a synthetic ACFG where a partition-assigned worker has an empty Loop body trips the guard; test demonstrably FAILS if the guard is deleted (prove-the-check-bites). If guard is nuc_trace-only, the bite test asserts the trace fires under NUC_TRACE.
- [x] #4 Gate green: nix develop -c just build clippy test test-release e2e all pass; e2e baseline 385/328/0/57/0 HELD; cargo doc warning count unchanged (10) if any doc-linked symbol is touched.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Empirical precondition (AC#1): instrument the empty-body continue site in petri_to_events::walk with a nuc_trace! that fires ONLY when a worker present in per_worker_override projects an empty body. Run the full partition=workers e2e corpus (03/05/08/16 distributed + blocks2d if any) under NUC_TRACE=1. Reason structurally too: per_worker_override is keyed by iter_var and only contains workers that appear in collect_op_workers(body) (the partition passes assign slices only to body Operation.workers), and emit_operation pushes a Fire for every op.workers member into scratch -> so a partition-assigned worker must project >=1 body event. Record verbatim finding in notes.
2. Guard (AC#2): if no legitimate empty case, add debug_assert! at the empty-body continue mirroring lines 236-242 style; message names wid + iter_var + says partition-assigned worker projected empty body (upstream-population bug). Release byte-identical.
3. Bite test (AC#3): synthetic ACFG in tests/petri_to_events.rs: Repeat with partition_worker_ranges assigning a worker an exclusive slice but body projects NO events for that worker -> #[should_panic] gated with #[cfg(debug_assertions)] (TASK-0291 dev-vs-release trap). Confirm test FAILS (no panic) if guard line deleted.
4. Gate (AC#4): nix develop -c just build clippy test test-release e2e; hold e2e 385/328/0/57/0; run e2e 2x for non-flake.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
AC#1 EMPIRICAL FINDING (verbatim). Instrumented the empty-body continue site in petri_to_events::walk with a NUC_TRACE probe, in two forms: (1) fires only when a partition-assigned worker (present in per_worker_override = partition_worker_ranges[iter_var]) projects an empty body; (2) unconditional positive-control firing on ANY empty-body worker, logging an assigned=true/false flag. Ran under NUC_TRACE=1: (a) full e2e matrix 385 cells; (b) direct-driver sweep of all 18 partition=workers/blocks2d (example,schedule) pairs (07-matmul distributed/-2d/3/8, 17-spmv distributed*, 05-stencil distributed/-2d, 11-game-of-life pipelined, 03-reduction distributed, 06-separable-filter distributed/2, 16-jacobi distributed, 08-histogram distributed/.scatter, 15-transpose distributed-rows, 13-cnn batch_parallel) x 7 tier-1 backends; (c) full 1243-test workspace suite. OBSERVED: ZERO probe hits in every corpus, for BOTH the assigned=true and assigned=false branches. CONCLUSION: not only is there no LEGITIMATE partition-assigned empty-body case, body_events.is_empty() is never true at all on shipping input. STRUCTURAL ROOT CAUSE: scratch is populated solely by emit_operation/emit_sync/emit_xfer, each of which does out.entry(wid).or_default().push(event) (every or_default immediately followed by a push), and the nested Repeat arm itself continues rather than inserting an empty vec. So a worker is a scratch key iff it has >=1 event -> the empty-body continue is effectively unreachable. A debug_assert! is therefore SAFE (cannot false-fire = no panic-on-valid-input) AND meaningful (catches a FUTURE regression where an upstream pass newly inserts an empty-bodied scratch entry for a partition-assigned worker). DECISION: debug_assert! (not nuc_trace). NOTE re host-relay/halo-strip/cumulative/reuse/blocks2d interactions named in the task: all exercised in the sweep corpus (05-stencil distributed = halo; 16-jacobi = cumulative band-slice; 06-separable = reuse; 07-matmul distributed-2d = blocks2d two-entries-per-data; event backends = host-relay) — none produced an empty partition-assigned body.

FINAL SUMMARY (TASK-0419). LANDED: (AC#2) added private fn debug_assert_partition_assigned_nonempty(wid, per_worker_override, iter_var) called at the empty-body continue in petri_to_events::walk; debug_assert! asserts per_worker_override.get(wid).is_none() (a partition-assigned worker must NOT reach an empty body). Mirrors the validate_event_lists_strict_per_worker debug_assert! precedent (lines 236-242). Release: compiled out, byte-identical e2e. (AC#3) Bite test partition_assigned_worker_with_empty_body_trips_guard + negative control non_partition_assigned_worker_with_empty_body_does_not_trip. PROVE-THE-CHECK-BITES verified: neutralizing the assert predicate to true made the bite test FAIL with note: test did not panic as expected; restored. DEVIATION FROM AC#3 WORDING: placed the bite test in an INLINE #[cfg(test)] mod in src/passes/petri_to_events.rs, NOT in tests/petri_to_events.rs as the AC text suggested. REASON (root, not workaround): the guard fires only on an empty-but-present scratch entry for a partition-assigned worker, a state UNREACHABLE through the public acfg_to_events API (a worker is a scratch key iff it has >=1 event — see AC#1 finding). The test must call the private guard fn directly, which only an in-crate test can do. The test exercises the REAL guard fn (not a copy). TASK-0291 trap handled: bite test gated #[cfg(debug_assertions)] so test-release stays green (debug_assert! stripped in release). GATE NUMBERS OBSERVED: just build clean; just clippy clean (-D warnings, re-run independently); just test = 0 failed all crates (incl 3 petri_to_events lib tests); just test-release = 0 failed all crates (bite test correctly excluded, no did-not-panic failure); e2e run 1 = 385/328/0/57/0, run 2 = 385/328/0/57/0 (non-flake); cargo doc = 10 rustdoc warnings (baseline held, new intra-doc links resolve). LIMITATIONS: (1) the guard is for a currently-unreachable code path — it is pure forward-looking defense-in-depth, adds zero behavior today; (2) e2e cannot observe driver nuc_trace output (harness captures child stderr via .output() and discards on success) — the AC#1 probe was run via DIRECT driver invocation + the in-process test suite, NOT through the e2e harness child process; documented so a future reviewer does not retry the e2e-NUC_TRACE path expecting trace lines.

No follow-up tasks filed: no stubs/shortcuts/new gaps discovered. The guard is complete as scoped.

ORCHESTRATOR REVIEW GATE (phase3-ralph, parallel read-only, commit 5e95def) — both GO, zero blocking findings. qa-test-runner INDEPENDENTLY RE-RAN: build+clippy clean (forced recompile, observed not claimed); just test 1245 passed/0 failed/3 ignored; just test-release 1243 passed/0 failed/3 ignored (the -2 delta = the two cfg(debug_assertions)-gated should_panic tests absent in release, both accounted for incl this task new bite test + pre-existing host_election guard — TASK-0291 trap respected); just e2e 385/328/0/57/0 x2 byte-identical non-flake; both new tests present+passing in dev; tree clean, only src + task-md staged. mped-architect INDEPENDENTLY VERIFIED the load-bearing AC#1 panic-on-valid-input claim is provably SOUND: traced collect_op_workers (partition_workers.rs:298-304 + partition_blocks2d.rs:443-451 key per_worker from body op-workers, recursing nested Repeat/Sequence) -> emit_operation pushes a Fire for every op.workers member -> any per_worker_override key gets >=1 scratch event; nested-Repeat false-fire risk traced and excluded (inner loop projects Event::Loop into parent scratch even for degenerate range). All 3 doc citations verified accurate (no doc-lie); silent-sibling grep CLEAR (petri_to_events.rs:331 is the only is_empty()-continue per-worker drop of this class; host_data_relay_inject only references the rule, no copy; reuse/halo or_default sites end in insert not empty-vec drop); dev-vs-release gating correct+complete; bite test calls the REAL fn. RECORDED NUMBERS ARE REVIEWER-RE-RUN, not implementer-claimed. No fold-back required.
<!-- SECTION:NOTES:END -->
