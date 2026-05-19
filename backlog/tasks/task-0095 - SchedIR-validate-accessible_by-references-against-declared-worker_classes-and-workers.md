---
id: TASK-0095
title: >-
  SchedIR: validate accessible_by references against declared worker_classes and
  workers
status: Done
assignee:
  - '@mped'
created_date: '2026-05-18 00:33'
updated_date: '2026-05-19 15:14'
labels:
  - M0
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
memory_region R { accessible_by = { name1, name2 } } currently passes through to the IR without checking whether name1/name2 are declared worker_class or worker names. Grammar sched.md sec.2 note 4 says "resolution is the linker's job" — but for accessible_by the resolution is purely schedule-internal and can be done in SchedIR lowering. Add validation.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 SchedIR lowering validates every name in 'memory_region R { accessible_by = { ... } }' is a declared worker_class or worker (schedule-internal resolution, done in lowering not deferred to linker)
- [x] #2 An undeclared accessible_by name produces a typed error (decision-0003: NOT panic) naming the offending name, surfaced via the driver channel
- [x] #3 Negative test pins rejection of an undeclared accessible_by name; valid schedules still lower; no e2e/determinism regression
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add SchedLowerError::UnknownAccessibleByName{region,name} + Display.
2. After pass-1 collects worker_classes + workers, validate each memory_region.accessible_by name is a declared worker_class OR worker name (schedule-internal, not deferred to linker). Do it after default-class synthesis and worker collection so forward refs are honoured (grammar 2 note 4 says forward refs rejected, but lowering uses 2-pass; consistent with existing UnknownWorkerClass placement).
3. Scope: plain undeclared-name typed error only. NO fuzzy did-you-mean (that is TASK-0096).
4. Negative test: accessible_by={ ghost }; positive: embedded_multimcu-style valid names lower.
5. Full gate.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented. SchedLowerError::UnknownAccessibleByName{region,name} + Display. Validation in lower_sched after pass-1 collects worker_classes (incl. synthetic default) and workers, before pass 2. Each name in memory_region.accessible_by must be a declared worker_class OR worker name — checked against ir.worker_classes / ir.workers (schedule-internal, NOT deferred to linker). Iterates the BTreeMaps (deterministic) so first-error report is stable.
Grounding: grammar-sched.md sec.2 note 4 says name resolution is "the linker job", but for accessible_by every legal target is declared in the same schedule, so resolution is purely schedule-internal — done in lowering as the task requires. Note 4 updated to record this.
SCOPE: plain undeclared-name typed error only. Did-you-mean / fuzzy-match suggestions are deliberately OUT of scope — that is TASK-0096. UnknownAccessibleByName is exactly where TASK-0096 will add the suggestion (forward-carried note).
Gate: just test 399/0. e2e 30/26/0/4 req-fail 0. determinism byte-identical 30/26/0/4. det-neg + xbackend-neg bite. clippy clean. ci exit 0.
E2E driver evidence: accessible_by = { ghost } -> nucleus: error: schedule lower error: `memory_region R` `accessible_by` lists `ghost`, which is not a declared `worker_class` or worker. No panic.
Regression-grep: only 14-hearing-aid/embedded_multimcu uses accessible_by — names fe_core/dsp_core/rf_core are all declared worker_class in that file; rule accepts it. Commit 07af8fc.

ORCHESTRATOR review-gate close (phase3-ralph): both reviewers GO, all three batch tasks genuinely Done & gate-substantiated. qa-test-runner: workspace 399/0 (sched_lower 43/43, 4 neg + 5 pos by name); e2e EXACTLY 30/26/0/4/0; determinism byte-identical + both negatives bite; clippy --all-targets clean; ci exit 0; 4 rejections proven end-to-end via the real driver (clean nucleus: error: lines, no panic/backtrace); regression INDEPENDENTLY grep-verified across all 20 example schedules (none trip a new rule; 14-hearing-aid accessible_by names confirmed declared worker_class). mped-architect: rules grounded EXACTLY in grammar EBNF/notes (single-valued set = AST LoopOption/TransferOption exhaustively, not over-broad); silent-grammar interpretation (bare reuse idempotent) documented in note 7 + code + positive test, not tribal; zero panic/unwrap/expect on any user-reachable check path (decision-0003 compliant); 0094 reject-as-hard-error sound + recorded x3 + ordering verified by inspection; grammar-sched.md notes 4/7/10 independently verified to match shipped code (no doc-lie); per-task Done honest, ACs map 1:1 to committed code, not retrofitted-loose; disclosed test-bug fix was correctly to the test (production code right); pre-existing limitations (no sched-AST spans; first-violation-only) correctly attributed as out-of-scope not regressions. Two reviewer findings BOTH explicitly optional/non-blocking/low-priority (ConflictingTransferMode message imprecise-but-disclosed on sync,sync path — adjudicated NOT a doc-lie; optional place-worker-ordering test) filed as TASK-0193 (dep TASK-0093) rather than scope-crept into this deep-context cycle. Done stands.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
SchedIR lowering now validates every name in `memory_region R { accessible_by = { ... } }` against the schedule-declared worker_class and worker names; an undeclared name is SchedLowerError::UnknownAccessibleByName (decision-0003: typed Result, no panic), surfaced via the driver as a clean nucleus: error: line.

Grounding & scope: grammar-sched.md sec.2 note 4 frames name resolution as the linker job, but accessible_by targets are all declared in the same schedule, so resolution is schedule-internal and done in lowering (not deferred). Note 4 updated. Scope is a plain undeclared-name error — did-you-mean fuzziness is explicitly left to TASK-0096, and this variant is where TASK-0096 will hook in (recorded for forward-carry).

Changes:
- ir.rs: + UnknownAccessibleByName{region,name} + Display.
- lower.rs: validation loop after pass-1 symbol collection (default class + workers in scope), deterministic BTreeMap iteration for stable first-error.
- sched_lower.rs: negative test (accessible_by = { ghost }) + positive test (class name + worker name both resolve).
- grammar-sched.md: note 4 records schedule-internal resolution.

Tests: just test 399/0; e2e 30/26/0/4; determinism byte-identical; clippy/ci clean. Only embedded_multimcu uses accessible_by; its names are declared worker_classes (regression-grepped, unaffected). Driver evidence captured.
<!-- SECTION:FINAL_SUMMARY:END -->
