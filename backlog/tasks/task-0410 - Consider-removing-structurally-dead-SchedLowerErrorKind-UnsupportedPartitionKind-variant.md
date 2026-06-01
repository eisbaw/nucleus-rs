---
id: TASK-0410
title: >-
  Consider removing structurally-dead
  SchedLowerErrorKind::UnsupportedPartitionKind variant
status: Done
assignee:
  - '@mark'
created_date: '2026-06-01 09:25'
updated_date: '2026-06-01 10:50'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation (cycle-237): removed SchedLowerErrorKind::UnsupportedPartitionKind across all 5 sites. (1) ir.rs variant DEFINITION + its 33-line doc block. (2) ir.rs Display match arm (the keyword-mapping inner match + write!). (3) lower.rs:160 reachability-TABLE row. (4) lower.rs ~1095 // comment block — rewrote to credit the wildcard-free match as the exhaustiveness mechanism + note TASK-0410 removal (plain code-span mention, NOT intra-doc-link). (5) partition_rows.rs ~100 //! module-doc INTRA-DOC-LINK [crate::sched::SchedLowerErrorKind::UnsupportedPartitionKind] — rewrote to point at lower_loop_option exhaustive match. VERIFIED-NOT-ASSUMED: (a) serde — SchedLowerErrorKind derives only Debug,Clone,PartialEq,Eq (ir.rs:420); NO Serialize/Deserialize, so NOT a wire-contract concern, safe to remove. (b) no-wildcard exhaustiveness — lower_loop_option LoopOption::Partition(k) matches all 3 PartitionKind {Workers,Rows,Blocks2d} with NO wildcard arm; a 4th variant fails to compile, which IS the real exhaustiveness guard. (c) PartitionKind import at ir.rs:64 does NOT become unused (still used by ResolvedLoopOption::Partition(PartitionKind) at line 263) — no import cleanup needed, clippy -D warnings confirms. (d) zero test references (tests/ + inline cfg(test) both grepped). cargo doc --workspace --no-deps: 10 generated warnings BEFORE and AFTER, zero unresolved links. Gate: build clean, clippy -D warnings exit 0 (forced fresh), test 1237/0, test-release 1236/0, e2e 385/328/0/57/0 — all unchanged.

ORCHESTRATOR REVIEW GATE (cycle-239, batched with TASK-0411): qa GO + architect GO on eafb114, ZERO blocking findings. Variant SchedLowerErrorKind::UnsupportedPartitionKind removed across all 5 sites (def + Display arm + lower.rs table row + 2 intra-doc-links rewritten). architect confirmed: serde-SAFE (enum derives only Debug/Clone/PartialEq/Eq, NO Serialize/Deserialize -- not a wire contract); no-wildcard exhaustiveness in lower_loop_option (Workers/Rows/Blocks2d, no _ arm) is the REAL guard so the removed never-constructed variant added zero safety; PartitionKind import still live via ResolvedLoopOption::Partition; zero test refs; both rewritten doc-links + the 2 surviving plain code-span mentions resolve (cargo doc 10/10 no unresolved). Behaviour-preserving (removed Display arm was unreachable -- variant never constructed). qa: clippy fresh exit 0, test 1237/1236, e2e 385/328/0/57/0 x2.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Removed the never-constructed SchedLowerErrorKind::UnsupportedPartitionKind variant + all 5 reference sites. Serde-safe (enum is diagnostics-only, no Serialize derive); the wildcard-free PartitionKind match in lower_loop_option is the real exhaustiveness guard. Gate green; cargo-doc 10->10 (no growth).
<!-- SECTION:FINAL_SUMMARY:END -->
