---
id: TASK-0389
title: >-
  Distributed gather/scatter: general worker-Wait ordering must match host-Push
  ordering (multi-gather / reordered-declaration FIFO robustness)
status: Done
assignee:
  - '@me'
created_date: '2026-05-31 14:34'
updated_date: '2026-05-31 20:14'
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

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A gather fixture+distributed schedule exists where the GATHERED (outer) array is produced BEFORE its index array (reversed vs prog.gather.algo.nuc), exercising the FIFO host-Push vs worker-Wait mismatch shape on a row-band-partitioned schedule.
- [x] #2 Empirically confirmed WITHOUT the root fix this new cell fails-LOUD on a strict-FIFO backend (mp-tcp-bufsync): a read_msg_expect seq/tag-mismatch panic (the TASK-0373 architect 'receiver expected X, wire delivered Y' repro reproduced). Recorded in notes.
- [x] #3 Root fix at the compiler-pass layer (transfer_inject/acfg build, NOT by re-ordering the example): the worker per-channel Wait sequence is derived to match the host per-channel Push order (producer-statement position), so ANY declaration order is FIFO-correct.
- [x] #4 The new reversed-declaration distributed-gather cell is byte-identical across all 7 tier-1 backends AND matches the reference oracle (no FIFO-backend panic).
- [x] #5 e2e byte-identity preserved on ALL pre-existing cells (the worker-Wait reorder is a no-op for current declaration orders): full just e2e totals show NO regression vs carried baseline 364/307/0/57/0; new cells are purely additive.
- [x] #6 Doc reconciliation: the 'LIMITATION ... TASK-0389' comment in collect_dataref_access_expr (build.rs) and the index-first traversal rationale are updated to the resolved state (no doc-lie claiming the mismatch is still unfixed); any downstream per-channel seq-monotonicity assumption is verified compatible with the reorder (or seq allocation adjusted), recorded in notes.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. FIXTURE (AC#1): prog.gather_revdecl.algo.nuc (x declared+loaded BEFORE col_idx, gather stmt identical) + kernels.gather_revdecl.rs (verbatim copy of kernels.gather.rs; loaders offset-keyed=order-independent so same input.bin/reference.bin oracle) + schedules/distributed_gather_revdecl.sched.nuc (mirror of distributed_gather). DONE.
2. PRE-FIX REPRO (AC#2): emitted mp-tcp-bufsync; host sends val(0),x(8),col_idx(4) on data_w0 (producer order); w0 waits val(0),col_idx(4),x(8) (data_in index-first traversal). DIVERGE. Runtime: 'wire: seq tag mismatch: receiver expected 4, wire delivered 8'. CONFIRMED.
3. ROOT FIX (AC#3): in build_waits_for_op (transfer_inject.rs), after collecting the per-op Wait placeholders, reorder them per (src->dst) channel to match the host Push order = producer-statement position of each data. Compute per-DataId host-Push rank from producer position (reuse producer_repeat_path machinery / an ACFG producing-Operation walk), key on PRODUCER position not DataId order. seqs travel with their pairs => once both endpoints traverse the channel in producer order, tags line up by construction (no seq reallocation) PROVIDED no downstream pass needs per-channel seq monotonicity (VERIFY: grep the ~4566 monotonic comment + read_msg_expect; if monotonicity IS required, allocate seqs in producer order instead).
4. VERIFY (AC#4): reversed-decl cell byte-identical across 7 tier-1 backends + matches reference.bin; no bufsync panic.
5. NO-REGRESSION (AC#5): worker-Wait reorder is a NO-OP for current decl orders; full just e2e totals = baseline 364/307/0/57/0, new cells additive (+7).
6. DOC (AC#6): rewrite build.rs LIMITATION/TASK-0389 comment block to resolved state; add invariant comment at fix site; record seq-monotonicity verification + scatter-sibling analysis in notes.
GATE before commit: nix develop just build && clippy && test && test-release && e2e. Scatter sibling (TASK-0384) shares build_waits_for_op => confirm its distributed.scatter cell stays byte-identical; if scatter needs separate reorder, file follow-up.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED + GATE GREEN (cycle TASK-0389).

ROOT-CAUSE TRACE (verified empirically, not narrative):
- Channel = per (host, worker) TCP connection (data_conn_var). All host->worker data shares ONE FIFO. read_msg_expect(cv, seq) reads the NEXT message and asserts seq==expected (EXACT match in FIFO order, NOT monotonicity — wire_runtime.rs:184/423 never compares two seqs for <). So worker Wait order on a channel MUST equal host Push order.
- Host Push order = producer-statement position (splice_pushes_global pins each Push at its producer). Worker Wait order WAS data_in TRAVERSAL order (build_waits_for_op iterates edge.data_in; gather index-first per collect_dataref_access_expr). Coincided ONLY because col_idx declared before x.

AC#2 PRE-FIX REPRO (exact panic captured): emitted prog.gather_revdecl (x produced before col_idx) on mp-tcp-bufsync. Host sends val(seq0),x(seq8),col_idx(seq4) on data_w0; worker waited val(0),col_idx(4),x(8). Ran it:
  "wire: seq tag mismatch: receiver expected 4, wire delivered 8 — Push/Wait pairing diverged between the two generated endpoints (protocol v0 contract violation)"
(all 4 workers; expected 4/5/6/7, delivered 8/9/10/11). This IS the TASK-0373 architect repro.

ROOT FIX (AC#3): producer_rank_by_data(root) ranks each data by producing-Operation walk position; build_waits_for_op STABLE-sorts its Wait list by (dst, src, producer_rank, data, seq). seqs travel with their (src,dst,data) pairs => tags line up by construction, no reseq. POST-FIX emit: worker reads val(0),x(8),col_idx(4) matching host send order. No panic; output==reference.bin.

SEQ-MONOTONICITY VERIFICATION (AC#6): NOT required. read_msg_expect/read_msg_expect_poll do exact-match-in-FIFO-order only. The ~4566 "SeqTags monotonically increase" comment is a LOCAL halo-strip visit-order determinism contract, not a wire requirement. So the no-reseq approach is sound.

AC#4: prog.gather_revdecl × all 7 tier-1 backends: each output.bin == reference.bin (32 bytes), no FIFO panic. e2e all 7 cells PASS.

AC#5 NO-REGRESSION: worker-Wait sort is a NO-OP for current decl orders. Verified by git-stash pre/post emit byte-identity on: existing 17-spmv/distributed_gather (col_idx-first) IDENTICAL; 08-histogram/distributed.scatter IDENTICAL; AND the loop-output multi-data-per-channel cells 06-separable-filter/distributed2 + 03-reduction/distributed IDENTICAL across pthreads-sync/mp-tcp-bufsync/mp-tcp-poll/mp-uds-event. e2e 364/307/0/57/0 -> 371/314/0/57/0 (+7 additive, fail 0, skipped 57 unchanged).

SCATTER-SIBLING ANALYSIS (silent-sibling): scatter (TASK-0384) shares build_waits_for_op so the fix COVERS it. But scatter (histogram[input[i]]) has only ONE host->worker input array (input); histogram is a private worker partial. So at most one Wait per host-channel => sort is a trivial no-op there; scatter has NO ordering coincidence to exploit and needs NO separate fix. Confirmed byte-identical pre/post + e2e PASS. NO follow-up needed for scatter.

RESIDUAL (honest, filed TASK-0389.01): producer_rank keys on raw producer-Op walk position. splice_pushes_global CAN hoist a loop-output Push past its enclosing Repeat (cut branch); if two same-channel data have producers at different nesting hoisting to different positions vs rank, the sort could still diverge. NO in-tree schedule exercises it (06-sep-filter/distributed2 + 03-reduction proven byte-identical). The precise key would be the post-splice Push position. TASK-0389.01 either builds the bite-test+refines or proves the cut-hoist preserves per-channel rank order. Code anchor: "SCOPE of the producer-rank key" comment in build_waits_for_op.

GATE: just build OK; just clippy OK (0 warn); just test 0 failed; just test-release 0 failed; just e2e 371/314/0/57/0 (reproduced twice, non-flake); check-mega-files/textual-replace/include-str all OK. +3 unit tests (task0389_*).

DOC RECONCILIATION (AC#6): build.rs collect_dataref_access doc + collect_dataref_access_expr body LIMITATION/TASK-0389 block rewritten to resolved state (index-first is now purely the data_in dependency contract; FIFO ordering decoupled via the build_waits_for_op sort). No doc-lie.

FIXTURE NOTE: prog.gather_revdecl.algo.nuc reuses the SAME input.bin/reference.bin oracle because loaders are OFFSET-keyed (order-independent). kernels.gather_revdecl.rs is a verbatim functional copy of kernels.gather.rs (variant rule derives the filename from the program stem); header documents the keep-in-sync requirement.

Forward-carried from TASK-0389.01 (DONE): the RESIDUAL filed at TASK-0389 close was REAL, not vacuous. The TASK-0389 producer-rank Wait sort matched the host only for the COMMON case (Push lands right after producer). For >=2 loop-OUTPUT data on ONE channel co-hoisting past the SAME Repeat, splice_after_repeat REVERSED the co-hoisted Pushes (inserted each immediately after the Repeat => most-recent landed closest), so the host sent reverse-rank while the worker waited rank-order — captured as a real mp-tcp-bufsync read_msg_expect panic ("receiver expected 2, wire delivered 3") on the NEW example 18-multigather/distributed. FIX (TASK-0389.01): splice_pushes_global feeds Pushes in producer-rank order + splice helpers APPEND each new Push after already-spliced Pushes => host textual Push order == producer_rank order == worker Wait order. e2e 371/314/0/57/0 -> 385/328/0/57/0 (+14). The TASK-0389 SCOPE-of-the-producer-rank-key comment is now rewritten to the RESOLVED state.
<!-- SECTION:NOTES:END -->
