---
id: TASK-0389
title: >-
  Distributed gather/scatter: general worker-Wait ordering must match host-Push
  ordering (multi-gather / reordered-declaration FIFO robustness)
status: To Do
assignee: []
created_date: '2026-05-31 14:34'
labels:
  - compiler
  - gather
  - transfer_inject
  - distributed
  - fifo
  - tech-debt
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Review P2.2 on TASK-0373. The distributed-gather col_idx-into-data_in recursion (acfg/build.rs collect_dataref_access_expr) currently relies on INDEX-FIRST traversal order to make the worker Wait sequence coincide with the host Push sequence on strict-FIFO backends (mp-tcp-bufsync, mp-tcp-poll, via read_msg_expect). That coincidence holds ONLY because in prog.gather.algo.nuc the index array col_idx is declared BEFORE its outer array x. Two independent orderings are at play: host Push order follows producer/DECLARATION position (splice_pushes_global), worker Wait order follows data_in TRAVERSAL order. A program that (a) declares the gathered array before its index array, or (b) interleaves multiple gathers with ordinary args, would re-introduce the mismatch — fail-LOUD on bufsync/poll (read_msg_expect tag-mismatch panic), masked on per-seq-demux event backends. ROOT FIX: derive/sort the worker Wait sequence from the host Push sequence (per-channel) rather than relying on traversal order, so any declaration order is FIFO-correct. Add a negative e2e/unit cell for a gather whose index array is declared AFTER the outer array. Carries the empirical repro from the TASK-0373 architect review (reverting to outer-first produced: receiver expected 4, wire delivered 8 on mp-tcp-bufsync).
<!-- SECTION:DESCRIPTION:END -->
