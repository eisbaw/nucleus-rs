---
id: TASK-0369
title: >-
  Tier-1 fault-path differential cell: on_violation=count/log check-loop trips
  latency_max — zero tier-1 differential coverage today
status: Done
assignee: []
created_date: '2026-05-30 11:08'
updated_date: '2026-05-31 06:06'
labels:
  - e2e
  - runtime-check
  - robustness
  - M?
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-213 strategic-analysis finding (R3, robustness). VERIFIED: on_violation (count/log/panic) appears ONLY in embedded schedules (01-elementwise-add/embedded_check*, 14-hearing-aid/embedded_multimcu*), all req=0 in the tier-1 e2e matrix (validated solely by separate Renode recipes). So the entire runtime-assertion + fault-reporting surface is NEVER cross-backend differential-tested on tier-1 — the bit-identity invariant has zero coverage for the fault path. The inject_check_frames machinery already exists for tier-1 (TASK-0052). Add a tier-1 check-loop schedule that deliberately trips latency_max with on_violation=count (and/or log), producing a deterministic fault-report artifact, and promote it across the tier-1 backends.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A tier-1 (non-embedded) schedule exists with a check loop V : latency_max=T, on_violation=count (and/or log) where T is set so the violation deterministically FIRES
- [x] #2 The fault-report output (the count/log artifact) is bit-identical across the tier-1 backends where the schedule is capability-compatible, and the cell is promoted [[required]] on those backends
- [x] #3 Honest scoping: if on_violation=count/log output is inherently non-deterministic across backends (e.g. timing-derived), document precisely what IS pinned (e.g. the violation-count, not the latency value) and pin only that
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
FORWARD-CARRIED feasibility finding (cycle-214 orchestrator triage, NOT yet implemented). The fault-report artifacts are NON-trivial to differential-test, and AC#2 (bit-identical across backends) collides with the existing design. Evidence from backend-common/src/check_frame.rs: (1) BOTH the log branch (emit_log_branch :180) and the count Drop summary (emit_count_reporter_struct :106) write to STDERR, not stdout — the docstrings at :170-173 and :104-105 state this is DELIBERATE so the output.bin cross-backend differential stays stable/indifferent to check-loop presence. (2) The e2e harness compares output.bin only; it does NOT capture/compare stderr today. (3) The log message embeds _check_elapsed (actual wall-clock ns from monotonic_ns) -> inherently NON-deterministic across runs/backends. (4) count at latency_max=1ns is timing-dependent too: the compare is _check_elapsed > Nns, so an iteration measuring 0ns under coarse host clock resolution would NOT be counted -> count is not GUARANTEED bit-identical (this is exactly the 255-vs-256 band documented for the Renode embedded count fixture, but on tier-1 the work-per-iter makes >1ns near-certain, NOT certain). CONSEQUENCE: a standard output.bin differential cell would pass TRIVIALLY (fault goes to stderr, output.bin unchanged) WITHOUT testing the fault artifact at all. To honor AC#2 the harness must be extended to capture+compare stderr, and even then only a TIMING-INDEPENDENT invariant can be pinned (per AC#3): e.g. the count-summary LINE PRESENCE + loop-var name + threshold-ns echo, NOT the occurrence count or the elapsed ns. RECOMMENDED scoping for whoever picks this up: either (a) add a tier-1 e2e stderr-capture comparison mode + pin only the deterministic substring (presence + loop_var + threshold), explicitly NOT the count/ns; or (b) re-scope AC#2 to a single-backend golden-file pin of the fault line shape and document that cross-backend bit-identity of the count is not robustly achievable. This is the kind of design decision that belongs in the brief, not discovered mid-implementation.

ORCHESTRATOR SCOPING DECISION (cycle-222, pre-implementation — this is the design call the cycle-214 notes said 'belongs in the brief, not discovered mid-implementation'):

The fault path is TIMING-DERIVED by construction (`_check_elapsed = monotonic_ns()-start`), so the occurrence COUNT and the elapsed-NS are NOT robustly bit-identical across backends — CONFIRMED independently by the existing embedded_check_count fixture's own docstring (count = trip-count 'MODULO AT MOST ONE iteration' -> 255-OR-256 band; the timing-INDEPENDENT invariant is the line SHAPE, never an ns/count). Therefore:

CHOSEN = option (a) refined: extend the e2e harness with a fault-assertion stderr-SUBSTRING check, add a NEW tier-1 check-loop schedule, and pin ONLY the timing-independent substring `check loop `i` violated latency_max=1 ns:` (presence + loop_var + threshold echo). Do NOT pin the count or any ns figure.

AC#2 operationalised (NOT rewritten — consistent with AC#3 'pin only that'): 'bit-identical fault-report across backends' means the deterministic SUBSTRING is byte-identical across the backends where it is robustly observable; the count/ns are explicitly excluded.

FLOOR GUARANTEE: the 3 single-binary backends (pthreads-sync, pthreads-async, openmp-rs; transport=shared-memory) are exec'd directly by run_cell -> harness captures their stderr via Command::output() -> the count-summary line is GUARANTEED surfaced. The 4 multi-process backends (mp-tcp-bufsync, mp-tcp-event, mp-tcp-poll, mp-uds-event) emit via run.sh; whether the host-worker stderr reaches the harness must be VERIFIED empirically — honest [[skip]] (reason = observability limit, NOT a fake pass) for any that don't surface it. A >=3-backend differential on the fault line already discharges 'zero tier-1 coverage today'.

KEY CODE FACTS for the implementer: harness run.stderr is ALREADY captured (nucleus/e2e/src/main.rs:1704 `.output()`) but only used for failure tails — add a post-output.bin-diff fault-assert check. Reporter template = nucleus/backend-common/src/check_frame.rs::emit_count_reporter_struct (the eprintln! at :119 is the exact line: `check loop `{loop_var}` violated latency_max={threshold_ns} ns: {n} occurrence(s)`). Manifest schema = nucleus/e2e/src/main.rs:78 (add a new `[[fault_assert]]` table keyed by the (example,schedule,backend) triple, `#[serde(deny_unknown_fields)]` like Cell). New schedule = nucleus/examples/01-elementwise-add/schedules/ (copy naive.sched.nuc + `check loop i : latency_max = 1ns, on_violation = count;`).

PATH CORRECTION to the scoping note above: examples + schedules + matrix live under `nuc-nucleus/` (verified via Paths::example_dir = repo_root/nuc-nucleus/examples/<ex>, main.rs:746-748; manifest = nuc-nucleus/e2e-matrix.toml, main.rs:743). So the new schedule goes in `nuc-nucleus/examples/01-elementwise-add/schedules/`, NOT `nucleus/examples/...`. The Rust workspace (e2e harness, backends, backend-common) is the separate `nucleus/` tree.

LANDED cycle-222 (commit 1cc1684). Empirical sweep across all 7 tier-1 backends: each builds + runs rc=0, output.bin BIT-IDENTICAL to reference.bin, AND the fault line `check loop `i` violated latency_max=1 ns:` PRESENT on harness-captured stderr. The conservative '>=3 single-binary backends' floor in the scoping note was OVER-DELIVERED: all 7 surface it (the single-worker host-only schedule routes every backend through the shared single-worker renderer; the 4 multi-process backends use the single-process run.sh fallback whose stderr Command::output() captures — verified, run.sh has no stderr redirection). So no honest-skip was needed.

GOTCHAS / lessons for the next person:
- The fault report is on STDERR by design; run.stderr was ALREADY captured by run_cell's Command::output() (used only for failure tails) — the feature just adds a post-diff substring check, no new capture plumbing.
- COUNT is NOT pinnable: empirically 256 here, but timing-derived; an iteration measuring 0ns under coarse clock isn't counted (the 255-vs-256 band). Pin only the line-shape substring.
- mped-architect P2 (FIXED in-thread, same commit): a [[fault_assert]] on a [[skip]]'d cell is a SILENT no-op — the skip short-circuits run_cell before the artefact runs, so Phase-5 never fires, yet the cell IS in planned_set so a naive 'in planned ⇒ fine' orphan check waves it through. fault_assert_orphans now flags skip-shadowed fault_asserts too (the docstring that claimed 'rides along fine' was an active doc-lie; corrected). Not live today (all 7 cells are required), but the guard + a regression pin (fault_assert_on_skipped_cell_is_flagged) close it.
- Stale-citation correction (architect P3): the cycle-222 scoping note cited 'main.rs:1704 .output()' — that was correct PRE-edit; after the Phase::Fault + FaultAssert + fault_assert_table additions shifted run_cell down ~150 lines, the artefact-run .output() is now ~main.rs:1859/1886. The note's 1704 is a pre-implementation estimate, now stale (recurring stale-line-citation class).
- non_upper_case_globals warning on emitted NUC_CHECK_COUNT_i: pre-existing check_frame.rs wart, warning-only (generated crate has no -D warnings), newly visible on tier-1 -> filed TASK-0386.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
TASK-0369 DONE (cycle-222, commit 1cc1684). Added the FIRST tier-1 (non-embedded) cross-backend differential of the runtime-assertion / fault-reporting surface, closing the 'zero tier-1 coverage for the fault path' gap.

DELIVERED:
- nuc-nucleus/examples/01-elementwise-add/schedules/check_count.sched.nuc — naive placement + `check loop i : latency_max=1ns, on_violation=count` (AC#1: latency_max=1ns guarantees the violation FIRES; empirically 256 violations on this host).
- e2e harness `[[fault_assert]]` table (nucleus/e2e/src/main.rs): after the output.bin diff passes, every declared substring must appear in the run's STDERR. Struct FaultAssert (#[serde(deny_unknown_fields)]) + Manifest::fault_assert_table (rejects empty list / empty substring / duplicate triple) + Phase::Fault + pure unit-testable missing_fault_substring helper + the run_cell Phase-5 check + fault_assert_orphans (anti-silent-vanish guard, sibling of required_coverage_gaps). Cells without a declaration are byte-for-byte unaffected (#[serde(default)] + empty-default + is_empty guard).
- matrix: 7 [[required]] + 7 [[fault_assert]] for check_count × all 7 tier-1 backends (milestone M6).

AC#2 (operationalised per the pre-implementation scoping decision, consistent with AC#3 — NOT AC-gamed): the timing-INDEPENDENT fault-line substring `check loop `i` violated latency_max=1 ns:` is BIT-IDENTICAL across ALL 7 tier-1 backends (over-delivered vs the >=3-backend floor) and promoted [[required]] on all 7. AC#3 (honest scoping): the occurrence COUNT and elapsed-ns are timing-derived (_check_elapsed=monotonic_ns()-start) and NOT robustly bit-identical (the 255-vs-256 band) — they are EXPLICITLY not asserted; only presence+loop_var+threshold are pinned, documented in the schedule docstring, the FaultAssert docstring, and the matrix comment.

GATE (orchestrator-rerun this cycle): just e2e 336/279/0/57/0 -> 343/286/0/57/0 (+7, 0 fail, 0 required-fail); e2e unit tests 95 passed (12 new incl. skip-shadowing pin + shipped-manifest no-orphans canary); clippy/test/test-release green; determinism-check full + all 3 negative arms bit correctly; all structural fences (mega-files/textual-replace/include-str/narrative-doc-lie/doc-citation x2/doc-links) OK. NEGATIVE BITE PROVEN end-to-end: a wrong fault_assert substring -> FAIL/fault, required-fail=1.

REVIEW: mped-architect GO (no P1). P2 (skip-shadowing silent hole + an active doc-lie in fault_assert_orphans) FIXED in-thread (same commit) + regression pin. Follow-up TASK-0386 filed (emitted NUC_CHECK_COUNT_<lowercase> non_upper_case_globals warning, cosmetic).
<!-- SECTION:FINAL_SUMMARY:END -->
