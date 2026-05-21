---
id: TASK-0233
title: >-
  NameSidecar contract extension: per-seq transfer buffer size for
  pthreads-async multi-worker codegen
status: Done
assignee: []
created_date: '2026-05-21 23:08'
updated_date: '2026-05-21 23:22'
labels:
  - M4
  - backend
  - contract
  - sidecar
dependencies:
  - TASK-0042.01
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Wave B of TASK-0228 needs to emit Arc<Ring<T>> instances sized by the schedule's 'transfer DATA : buffer=N' directive. That value lives in ACFG::XferPlaceholder::policy.buffer today; the backend receives NameSidecar (not ACFG) per the EventList contract (TASK-0124, AlgoIR/LinkedIR/ACFG-free). The Event::Push/Wait variants carry seq but NOT buffer size.

Cycle-18 architect review: 'If new Event variants are needed (BufferPush vs Push etc.), file the variant work as a sibling sub-task and depend on it; do NOT add a side-channel.' That's exactly this gap.

This task adds a NEW NameSidecar field: transfer_buffer_for_seq: BTreeMap<SeqTag, u64>. Mirrors the existing partition_worker_ranges + loop_bounds precedent on NameSidecar — additive sidecar extension with serde-default backward-compat, populated by build_sidecar walking the ACFG's XferPlaceholder nodes.

Scope:
1. NameSidecar gains the new field (serde-default).
2. build_sidecar walks the ACFG (mirroring acfg.rs's existing tree walks), extracts (seq -> policy.buffer) for every XferPlaceholder.
3. Unit test asserting the map is populated correctly for example 13/pipeline_parallel (transfer feat1 : async, buffer=3 -> SeqTag mapped to 3).
4. NO backend consumer change in THIS task — the new field is available but unused. Wave B of TASK-0228 wires it.

Why a separate task: this is a contract extension affecting the shared sidecar; landing it as a precondition isolates the change (and its review gate) from the Wave B integration work that depends on it.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 NameSidecar.transfer_buffer_for_seq: BTreeMap<SeqTag, u64> exists with serde-default.
- [x] #2 build_sidecar populates it by walking the ACFG tree (Repeat/Sequence/Xfer) and reading XferPlaceholder.policy.buffer keyed by .seq.
- [x] #3 A Push and its matching Wait share one SeqTag and one buffer value (acfg-level invariant); the map is keyed by SeqTag so each pair gets exactly one entry, not two.
- [x] #4 Unit test exercises example 13/pipeline_parallel (async, buffer=3); asserts the map has the expected SeqTag -> 3 entries and the corresponding sync-only schedule has the map empty.
- [x] #5 Workspace tests pass, clippy -D warnings clean, just e2e baseline preserved (no behavior change).
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 19 (2026-05-22): all 5 ACs met. NameSidecar.transfer_buffer_for_seq landed in nucleus/compiler/src/sidecar.rs; populated by collect_transfer_buffers walking ACFGNode::{Operation/Sync(no-op), Xfer(extract), Sequence/Repeat(recurse)}. 5 unit tests in nucleus/compiler/tests/sidecar_buffer.rs pin: async pipeline_parallel populates exactly 3 entries with cap=3; sync naive produces empty map; multi-worker sync produces non-empty all-cap=1 map; the walker descends Repeat (defensive vs independent ACFG walker); literal old-wire JSON without the new field deserializes with empty default (serde-default backward-compat).

Gate: cargo test --workspace 564 / 0 / 2 (was 559 before TASK-0233; +5 new tests). Clippy clean. just e2e 36/29/0/7 preserved. Commits: 67a02f6 (initial landing) + cycle-19 review-gate lockstep fixes (this commit).

Unblocks TASK-0228 Wave B (the multi-worker codegen consumer).
<!-- SECTION:FINAL_SUMMARY:END -->
