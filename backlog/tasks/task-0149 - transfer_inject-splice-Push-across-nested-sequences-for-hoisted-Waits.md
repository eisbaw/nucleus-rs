---
id: TASK-0149
title: 'transfer_inject: splice Push across nested sequences for hoisted Waits'
status: To Do
assignee: []
created_date: '2026-05-18 05:44'
updated_date: '2026-05-18 07:22'
labels:
  - M2
  - compiler
  - bug
  - critical-path
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
After TASK-0143, transfer_inject can hoist a Wait up to a per-tile body sequence; the matching Push for a producer in a different (typically top-level) sequence is never spliced because splice_pushes_for_waits only walks the same sequence. The pthreads-sync codegen does not consume Push placeholders so this is not a correctness blocker for the current backend, but it becomes one for backends whose codegen (mp-tcp, MPI, async/buffered pthreads) reads Push events. Add a final global pass that walks the rewritten ACFG, builds a tree-wide producer index, and splices Push placeholders after each producer Op for every matching hoisted Wait elsewhere in the tree.
<!-- SECTION:DESCRIPTION:END -->
