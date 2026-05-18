---
id: TASK-0130
title: 'Reduction examples with non-zero identities (min, max, product)'
status: To Do
assignee: []
created_date: '2026-05-18 03:09'
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
