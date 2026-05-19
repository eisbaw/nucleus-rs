---
id: TASK-0093
title: 'SchedIR: detect duplicate / mutually-exclusive options on a single directive'
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
TASK-0010 lowers loop/transfer/check options as a Vec preserving source order, but does not detect duplicates (e.g. `block=64, block=128` on one loop) or mutually-exclusive combinations (e.g. `sync, async` on one transfer). Grammar sched.md sec.2 notes 5 and 7 call these out as linker concerns. The information is preserved on the IR (Vec, ordered); add a pass.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 SchedIR lowering rejects a directive carrying a duplicate single-valued option (e.g. 'loop i : block=64, block=128') with a typed error (decision-0003: user-diagnosable -> Result, NOT panic) naming the directive and the duplicated option
- [x] #2 SchedIR lowering rejects mutually-exclusive option combinations on one directive (e.g. transfer '... : sync, async') per grammar-sched.md sec.2 notes 5 and 7, with a typed error
- [x] #3 All existing example schedules and test fixtures still lower successfully (no e2e/determinism regression); negative tests pin each rejection with a clear message
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add SchedLowerError::DuplicateLoopOption{var,option} + DuplicateTransferOption{data,option} + ConflictingTransferOptions{data} variants + Display.
2. In lower_loop: detect a single-valued key appearing >1x (block/vectorize/unroll/pipeline/partition single-valued; reuse is a flag, idempotent-ok but treat dup as dup too for consistency? -> single-valued = has-a-value keys; reuse repeat is harmless redundancy, NOT a conflict per note 7 which only calls out value conflicts -> only flag value-bearing keys appearing >1x).
3. In lower_transfer: detect sync+async coexisting (note 5) -> ConflictingTransferOptions; detect duplicate single-valued (buffer, notify, and sync/async repeated) keys.
4. Negative tests: dup block, sync+async, dup buffer; positive: reordered distinct options still lower.
5. Full gate.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented. SchedLowerError gains DuplicateLoopOption{var,option}, DuplicateTransferOption{data,option}, ConflictingTransferMode{data} (ir.rs) + Display. lower.rs: loop_option_keyword() helper + at-most-once set check in lower_loop; transfer mode-flag + buffer/notify at-most-once check in lower_transfer.
Grounding: grammar-sched.md sec.2 note 7 / sec.5.1 (option list is an unordered set; block=64,block=128 is a value conflict) -> value-bearing keys (block/vectorize/unroll/pipeline/partition; buffer/notify) at-most-once; note 5 / sec.5.3 (sync/async mutually exclusive). Interpretation recorded in code+spec: bare `reuse` is idempotent; note 7 targets VALUE conflicts only, so repeated `reuse` is NOT rejected (positive test pins it). sync,sync / async,async folded into ConflictingTransferMode (same user-error class — a transfer has one mode).
Gate: just test 399 passed / 0 failed (sched_lower 43/43). e2e 30/26/0/4 req-fail 0. determinism byte-identical 30/26/0/4. det-neg + xbackend-neg still bite. clippy --all-targets clean. ci exit 0.
E2E driver evidence: `loop i : block=64, block=128` -> nucleus: error: schedule lower error: loop `i` has more than one `block` option; each option may appear at most once. `transfer a : sync, async` -> ...transfer `a` is both `sync` and `async`; these options are mutually exclusive. No panic.
Regression-grep: all *.sched.nuc under nuc-nucleus/examples/ — no directive has a duplicate value-bearing option or sync+async. Commit 07af8fc.

ORCHESTRATOR review-gate close (phase3-ralph): both reviewers GO, all three batch tasks genuinely Done & gate-substantiated. qa-test-runner: workspace 399/0 (sched_lower 43/43, 4 neg + 5 pos by name); e2e EXACTLY 30/26/0/4/0; determinism byte-identical + both negatives bite; clippy --all-targets clean; ci exit 0; 4 rejections proven end-to-end via the real driver (clean nucleus: error: lines, no panic/backtrace); regression INDEPENDENTLY grep-verified across all 20 example schedules (none trip a new rule; 14-hearing-aid accessible_by names confirmed declared worker_class). mped-architect: rules grounded EXACTLY in grammar EBNF/notes (single-valued set = AST LoopOption/TransferOption exhaustively, not over-broad); silent-grammar interpretation (bare reuse idempotent) documented in note 7 + code + positive test, not tribal; zero panic/unwrap/expect on any user-reachable check path (decision-0003 compliant); 0094 reject-as-hard-error sound + recorded x3 + ordering verified by inspection; grammar-sched.md notes 4/7/10 independently verified to match shipped code (no doc-lie); per-task Done honest, ACs map 1:1 to committed code, not retrofitted-loose; disclosed test-bug fix was correctly to the test (production code right); pre-existing limitations (no sched-AST spans; first-violation-only) correctly attributed as out-of-scope not regressions. Two reviewer findings BOTH explicitly optional/non-blocking/low-priority (ConflictingTransferMode message imprecise-but-disclosed on sync,sync path — adjudicated NOT a doc-lie; optional place-worker-ordering test) filed as TASK-0193 (dep TASK-0093) rather than scope-crept into this deep-context cycle. Done stands.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
SchedIR lowering now rejects a directive carrying a duplicate single-valued option or a mutually-exclusive option pair, with typed SchedLowerError variants (decision-0003: user-diagnosable -> Result, no panic).

Changes:
- ir.rs: + DuplicateLoopOption{var,option}, DuplicateTransferOption{data,option}, ConflictingTransferMode{data} + Display messages naming the directive and the offending option.
- lower.rs: loop_option_keyword() + at-most-once set check (lower_loop); transfer mode-flag + buffer/notify at-most-once check (lower_transfer). Grounded in grammar-sched.md sec.2 notes 5 (sync/async exclusive) & 7 (option list is a set; value conflicts rejected) — both notes updated to record the lowering-enforced rule and the bare-`reuse`-is-not-a-conflict interpretation.
- sched_lower.rs: negative tests (dup block, sync+async, dup buffer) + positive tests (reordered distinct options lower; repeated `reuse` lowers).

User impact: malformed schedules fail fast with a clear `nucleus: error:` line instead of silently lowering an ambiguous option set.

Tests: just test 399/0; e2e 30/26/0/4; determinism byte-identical; clippy/ci clean. All existing example schedules unaffected (regression-grepped). 4-rejection driver evidence captured. Commit 07af8fc.
<!-- SECTION:FINAL_SUMMARY:END -->
