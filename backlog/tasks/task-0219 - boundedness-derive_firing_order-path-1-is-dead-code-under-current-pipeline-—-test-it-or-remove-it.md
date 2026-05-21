---
id: TASK-0219
title: >-
  boundedness::derive_firing_order path-1 is dead code under current pipeline —
  test it or remove it
status: Done
assignee:
  - '@mped'
created_date: '2026-05-21 14:54'
updated_date: '2026-05-21 18:10'
labels:
  - compiler
  - boundedness
  - M4
  - tech-debt
dependencies:
  - TASK-0213
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architecture-review finding (TASK-0213 cycle): the marking-aware logic in derive_firing_order (path 1) is currently never exercised by any in-tree fixture, because path 2 (acfg_to_petri TtoP-arc elision) makes source-order legal on every existing schedule. The implementer characterised path 1 as 'defense-in-depth for nets with softer constraints', but it is currently dead code with no test. Dead-code-with-no-test is a maintenance cost.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Pick ONE: (a) add a synthetic-net unit test that constructs a 2-place net by hand where source-order isn't legal but a legal interleaving exists; assert derive_firing_order discovers it; AND a stuck-state-fallback test asserting check_bounded surfaces the violation via a leftover-trip; OR (b) remove the marking-aware logic and replace it with debug_assert!("source-order is always legal after acfg_to_petri elision").
- [ ] #2 Decision rationale documented in derive_firing_order's docstring or the path-1 module section; recurring-defect (dead code with no test) audit closes this loop.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Done — orchestrator-direct cycle

Two synthetic-net unit tests added to `nucleus/compiler/tests/boundedness.rs` exercising the previously-untested path-1 behaviours of `derive_firing_order`:

1. `derive_firing_order_reorders_under_initial_marking_pressure` — net with buf cap=1 pre-marked at 1; T1=produce (source idx 0, would overflow), T2=consume (idx 1). Asserts path-1 returns [consume, produce] (marking-aware reorder) AND check_bounded accepts that order.

2. `derive_firing_order_appends_stuck_leftovers_so_check_bounded_diagnoses` — net where T1=fill_partial fires successfully then T2=overfill would overflow buf. Asserts path-1 returns [T1, T2] (T2 appended in source order even though unfirable post-T1) AND check_bounded returns CapacityExceeded naming `overfill` + `buf`.

Choice resolution: AC#1 option (a) — keep the defensive logic, add the tests. Removing path-1 (option b) would punish external callers passing hand-built nets with soft constraints (no in-tree pipeline triggers it, but the function is `pub` and accepts any Net). The docstring rewrite explicitly names this rationale (Net is `pub`, callers may pass soft-constraint nets) AND cross-references the two test names so a future "is this still alive?" audit can find the pins.

Updated derive_firing_order docstring with the "Why this defensive layer is kept (TASK-0219)" subsection — accurate per code reading; matches what the tests pin.

Gate (orchestrator re-ran):
- cargo test workspace: 542 pass / 0 fail / 2 ignored (was 540/0/2; +2 new tests).
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- just e2e: 36 cells: 29 / 0 / 7 (baseline unchanged).
<!-- SECTION:NOTES:END -->
