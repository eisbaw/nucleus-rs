---
id: TASK-0410
title: >-
  Consider removing structurally-dead
  SchedLowerErrorKind::UnsupportedPartitionKind variant
status: To Do
assignee:
  - '@mark'
created_date: '2026-06-01 09:25'
labels:
  - hardening
  - dead-code-audit
  - cycle-236-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0407 audit confirmed (traced, not trusted) that SchedLowerErrorKind::UnsupportedPartitionKind (nucleus-compiler/src/sched/ir.rs:666) is STRUCTURALLY DEAD: lower_loop_option (sched/lower.rs:1104-1114) is exhaustive over all 3 PartitionKind variants (Workers/Rows/Blocks2d), each maps to a non-erroring ResolvedLoopOption, and each has a real consumer pass (partition_workers.rs:206, partition_rows.rs:236, partition_blocks2d.rs:283). The variant is constructed NOWHERE; only its declaration, Display arm (ir.rs:810), and explanatory comments exist. Became dead cycle-79c/80 when partition_rows + partition_blocks2d landed (memory: project-partition-silent-drop). The lower.rs:1095-1103 comment already documents it honestly as RESERVED/dead with the exhaustive match as the real exhaustiveness mechanism.

DECISION NEEDED (opacity-gate-rot, CLAUDE.md #4): keep as documented-defensive RESERVED shape (ready diagnostic for a future 4th PartitionKind) OR remove. Removing is a public-error-enum change: drop the variant + its Display arm + the lower.rs:1097 + partition_rows.rs:100 + lower.rs:1095-1103 doc references; verify exhaustive-match still compiles (it will — the variant is never matched in a non-Display context). LOW leverage; deferred from TASK-0407 because error-variant removal is wider than that audit cycles small-cleanup scope. NOTE: PRD §6.3.3 mandates compile-time rejection of bad loop-option combos; the exhaustive match in lower_loop_option already enforces decide-on-every-variant, so the variant adds no safety today.
<!-- SECTION:DESCRIPTION:END -->
