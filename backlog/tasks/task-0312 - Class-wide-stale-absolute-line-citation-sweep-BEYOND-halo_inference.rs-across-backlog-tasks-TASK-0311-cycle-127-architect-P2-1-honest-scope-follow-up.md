---
id: TASK-0312
title: >-
  Class-wide stale absolute-line citation sweep BEYOND halo_inference.rs across
  backlog/tasks/ (TASK-0311 cycle-127 architect P2 #1 honest-scope follow-up)
status: To Do
assignee: []
created_date: '2026-05-25 06:25'
updated_date: '2026-05-25 06:25'
labels:
  - forward-carried-from-TASK-0311
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0311 (cycle 127) closed the class-wide stale `halo_inference.rs:[0-9]` absolute-line citation sweep across `backlog/tasks/`. The cycle-127 architect (`mped-architect`) review-gate P2 #1 flagged that the SAME DEFECT CLASS fires on OTHER production files cited by tracker md:

- `partition_workers.rs:40` — 10 hits across {task-0249, task-0258, task-0259}; verified STALE (line 40 today is spillover; the "all three PartitionKind variants now have consumers" comment those records describe lives at lines 60-66 today, refactored sometime after TASK-0258/0259 landed).
- `driver/src/main.rs:410` + `:413` — 7 hits across {task-0265, task-0271, task-0280, task-0274:69 (already self-corrected; the remaining 3 task records carry stale citations); `apply_reuse_inference` call site is now at line ~418 (verified via cycle-127 grep).
- Full corpus: `grep -rn '[a-zA-Z_]\+\.rs:[0-9]\+' backlog/tasks/` returns ~365 distinct citations across the project. The top-frequency offenders (partition_workers.rs:40 ×10, multi_worker_walker.rs:478 ×4, driver/main.rs:410 ×4 + :413 ×3) are the high-priority sweep targets.

## Why filed (silent-sibling recurrence at the meta level)

The TASK-0311 AC#1 narrowly enumerated halo_inference.rs; closing it left the structurally identical sibling files silently skipped. Same shape as the [[feedback-silent-sibling-defect]] cycle-127 update: when a "class-wide" sweep is scoped narrowly, the broader class becomes a silent sibling. Filing the broader-sweep follow-up keeps the thread tracked.

## Acceptance criteria

1. `grep -rn '[a-zA-Z_]\+\.rs:[0-9]\+' backlog/tasks/` returns hits ONLY inside intentional historical-lesson-preservation records (same cycle-126 charitable rule that TASK-0311 used).
2. Each migrated citation uses the cycle-122 symbolic-anchor convention.
3. The cycle-126 P1 + P2 #1 substitution-defect lessons MUST be applied during the substitution:
   - Atomic per-string Edit (never sed-batch).
   - Surrounding-context re-grep after each.
   - Greppability verification of every new symbolic anchor (`grep -rn '<anchor>' nucleus/` returns ≥1 hit).
   - No dangling articles, no duplicated articles, no AC inversion, no non-greppable descriptive coinage.
4. The cycle-125 heredoc-quoting discipline applies for any commit shell heredoc.
5. **Sweep ordering** (architect recommendation): start with the highest-frequency offenders (partition_workers.rs:40 ×10, driver/main.rs:410/413 ×7), spot-check freshness on the top-10, then sweep the long tail.

## Honest scope

LOW priority. These citations are reading aids — they drift over months but rarely block work. The cycle-127 cost was 1 cycle for 4 sites of one file's slice; the full corpus is ~365 hits across many files. A realistic single-cycle scope is maybe 30-50 sites (the verified-stale subset). The "tail" of historical-record hits stays under the charitable rule.

## Cross-references

- TASK-0311 cycle-127 architect P2 #1 finding.
- Memory: `feedback-sed-batch-tracker-md-substitution` (cycle-127 epilogue) — substitution discipline confirmed working.
- Memory: `feedback-silent-sibling-defect` (cycle-127 update) — the meta-level recurrence shape.

## Out of scope (deliberate)

- A `just check-tracker-line-citations` lint recipe: the cycle-127 architect P3 noted the cost/benefit is unfavourable as a CI gate (high false-positive rate from legitimate historical records). Do NOT add as part of this task.
<!-- SECTION:DESCRIPTION:END -->
