---
id: TASK-0152
title: 'transfer_inject: Wait with no producer must hard-error for real ACFGs'
status: Done
assignee:
  - '@mark'
created_date: '2026-05-18 08:32'
updated_date: '2026-05-18 09:32'
labels:
  - M2
  - compiler
  - robustness
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Pass B (splice_pushes_global, TASK-0136) silently continues when producer_repeat_path returns None, tolerating partial synthetic test ACFGs. For a real LinkedIR-derived ACFG a cross-worker Wait with no producer anywhere is a compiler-invariant violation (single source of truth: the producer MUST exist). It is currently caught only implicitly downstream by check_deadlock_free. Distinguish synthetic-partial from real input and panic/hard-error with context (which symbol, which seq) for real input, per acfg.rs fail-fast precedent. Raised by mped-architect review of TASK-0136.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Real-ACFG path hard-errors with symbol+seq context on a producerless cross-worker Wait
- [x] #2 A producerless cross-worker Wait is malformed regardless of origin (a Wait is only emitted when the schedule records a producer); hard-fail is universal, no opt-in escape needed. Pinned by should_panic test.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED. The actual silent-failure site was NOT Pass B continue (a producerless Wait bubbles to the root of Pass A and was silently dropped before Pass B). Fixed at the true site: inject_transfers now panics with full context (data name, id, seq, src->dst) when hoist_invariant_waits returns a non-empty escaped_at_root; Pass B keeps a defense-in-depth panic on producer_repeat_path None. AC#2 premise corrected: a cross-worker Wait is only emitted when build_waits_for_op finds a recorded producer, so a producerless cross-worker Wait is malformed regardless of test/real origin — universal hard-fail is correct, no opt-in needed. Pinned by should_panic test malformed_acfg_wait_without_producer_op_panics. Verified: full suite + e2e 7/7 + strict clippy green; no legitimate synthetic test regressed.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
transfer_inject: fail loud on a producerless cross-worker Wait.

Previously such a Wait was silently dropped (it bubbles to the root of the Pass A whole-symbol hoist and was discarded), handing downstream analysis nothing instead of an error. Now inject_transfers hard-errors with full context (symbol, id, seq, src->dst) at that root boundary, with a defense-in-depth panic in Pass B. A cross-worker Wait is only emitted when the schedule records a producer, so its absence is a malformed-ACFG/compiler-bug invariant violation, not a tolerable partial input — the hard-fail is universal. Pinned by a should_panic test. No legitimate synthetic test regressed; full suite + e2e + strict clippy green.
<!-- SECTION:FINAL_SUMMARY:END -->
