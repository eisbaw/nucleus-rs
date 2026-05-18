---
id: TASK-0151
title: 'transfer_inject: cross-scope finalisation gate is whole-program coarse'
status: In Progress
assignee:
  - '@mark'
created_date: '2026-05-18 08:32'
updated_date: '2026-05-18 09:27'
labels:
  - M2
  - compiler
  - tech-debt
dependencies:
  - TASK-0154
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Pass A/B (TASK-0136) are gated on inner_block_iter_vars.is_empty() — a whole-PROGRAM switch. A program mixing one block=N loop with an unrelated non-blocked cross-scope whole-symbol transfer gets ZERO cross-scope finalisation, silently reintroducing the original deadlock for the non-blocked part. No example hits this today (single-schedule programs). Tighten to per-subtree scoping, and add a log::debug! on the skipped branch so the deferral is traceable instead of invisible. Raised by mped-architect review of TASK-0136.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Gate decision is per-Repeat-subtree, not whole-program
- [ ] #2 Skipped-finalisation branch logs a traceable debug message naming the deferred symbol/seq
- [x] #3 Test: mixed block + non-block program still pairs the non-block cross-scope transfer
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
AC#1 (per-subtree gate) + AC#3 (mixed block/non-block test) DONE. The gate moved from a whole-program inner_block_iter_vars.is_empty() switch to a per-subtree contains_block_inner() check: Pass A treats any Repeat nest containing a block-inner loop as opaque; Pass B excludes Waits inside such nests via a block-aware collect_waits. Non-block cross-scope transfers elsewhere in the same program are still finalised. Strictly more precise than before; identical behaviour when no block transform is active (05/07-blocked stay green).

AC#2 (traceable debug message) NOT done and deliberately not faked: the compiler crate has no logging facade and adding log+env_logger for one line contradicts the minimal-dep / no-spam project ethos. The deferral is documented in-code with TASK-0149/0151 references. Filed the logging-facility decision as a separate task; AC#2 closes when that lands.

Review follow-up (mped-architect Findings 2+3, non-negotiable honesty gap): documented the "block-entangled non-block transfers are stranded" over-approximation in transfer_inject module docs (Honest limitations), and pinned it with block_nested_in_plain_loop_strands_the_invariant_wait (asserts current conservative behaviour; flips when TASK-0149/0150 makes classification per-Wait). Added mixed_block_nonblock_tree_is_structurally_idempotent locking idempotence on the mixed tree. QA GO (333 tests, 7/7 e2e, clippy clean).
<!-- SECTION:NOTES:END -->
