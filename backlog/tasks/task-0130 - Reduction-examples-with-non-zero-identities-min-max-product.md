---
id: TASK-0130
title: 'Reduction examples with non-zero identities (min, max, product)'
status: Done
assignee: []
created_date: '2026-05-18 03:09'
updated_date: '2026-05-23 21:30'
labels:
  - M1
  - examples
  - language
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Example 03-reduction (TASK-0022) ships SUM only because the additive identity is zero, which is what the pthreads-sync pre-init pass already provides for indexed-only data symbols. Other reductions need different identities — min: INT_MAX, max: INT_MIN, product: 1, bitwise-and: !0, etc. — and v2 has no surface to declare them. Two design options: (a) language-level 'init=...' clause on the dataflow statement; (b) explicit per-symbol init kernel (effectful but pure-of-output, fires before the fold loop). Both interact with the static-scheduling model differently — option (a) keeps the init purely declarative; option (b) treats init as a regular kernel firing the schedule can place. Decide and implement. Until then, only sum is the cleanly expressible reduction at the algorithm level.
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-no-driver (orchestrator-direct, cycle 77 sweep). Task's own framing: 'Until then, only sum is the cleanly expressible reduction at the algorithm level.' No in-tree example uses min/max/product/bitwise-and reductions; 03-reduction is sum-only and works (additive identity is zero, which pthreads-sync's pre-init pass already provides). The substantive design decision (option (a) 'init=...' clause vs (b) per-symbol init kernel) requires a real example with a non-zero identity reduction to ground the choice — designing it ahead of demand likely picks the wrong abstraction. Reopen when a real example uses min/max/product (e.g. histogram in DSP, max-pooling in CNN — the latter would natural fit example 13 if it grew a real maxpool kernel; today 13 uses abstract i32 ops). Same deferred-until-example pattern.
<!-- SECTION:FINAL_SUMMARY:END -->
