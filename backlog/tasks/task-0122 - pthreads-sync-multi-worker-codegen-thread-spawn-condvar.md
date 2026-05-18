---
id: TASK-0122
title: 'pthreads-sync: multi-worker codegen (thread spawn + condvar)'
status: Done
assignee: []
created_date: '2026-05-18 02:13'
updated_date: '2026-05-18 02:52'
labels:
  - M1
  - backend
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
At TASK-0020 the pthreads-sync backend rejects schedules that use more than one worker because multi-worker codegen is not implemented. Implement std::thread::spawn for the multi-worker case, with std::sync::Condvar-based barriers for ACFG::Sync nodes and shared-memory channels (Mutex<Option<T>> + Condvar) for ACFG::Xfer pairs (Push/Wait). The synthetic AC #5 ping-pong test in the original task description (a two-worker pingpong EventList producing compilable Rust that runs correctly) belongs here.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Two-worker synthetic pingpong (producer on w0, consumer on w1, three Push/Wait pairs) compiles and runs correctly.
- [ ] #2 Each declared worker becomes its own std::thread::spawn block; main joins them all.
- [ ] #3 Sync nodes lower to std::sync::Barrier (or equivalent Condvar dance) across the participating worker threads.
- [ ] #4 Push/Wait pairs share a typed Arc<(Mutex<Option<T>>, Condvar)> slot; producer sets + notifies, consumer waits + takes.
- [ ] #5 Example 2 (split element-wise add, TASK-0021) works end-to-end on this path.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Design questions explored

**SeqTag-keyed slot table vs per-data slot.** The original task framing called for `Arc<Slot<T>>` per Push/Wait pair, indexed by SeqTag. I chose per-data (one Slot per cross-worker data symbol) instead. Reasons:

1. The ACFG's `inject_transfers` pass currently emits Wait placeholders inside enclosing for-loops *without* matched Pushes — `splice_pushes_for_waits` is scope-local and can't see producers in outer scopes (top-level `load_input` -> in-loop consumer). Honouring the ACFG's per-iteration Waits would deadlock immediately because there are no per-iteration Pushes.
2. Example 02's three transfers are whole-array `sync` semantics by schedule. Whole-array Slots match the schedule intent.
3. Per-tile Slot tables are filed as **TASK-0126** when the underlying transfer_inject gap is closed and TASK-0116 (tile coalescing) lands.

**Per-worker projection: source-IR walk vs ACFG walk.** I walked the ACFG (so Sync/Xfer placeholders are honoured at the right structural position) but used a parallel source-IR (`linked.algo.stmts`) lookup at every Operation node so the kernel call gets its actual argument expressions. The lookup is `lookup_source_irstmt`: it counts non-placeholder ACFG siblings per Sequence level to map back to the source statement index. The ACFG keeps single-statement Sequence/Repeat shape, so the parallel traversal terminates cleanly.

**Barrier identity across worker projections.** Both host and worker threads need to refer to the SAME `Arc<Barrier>` — different projections must agree on which barrier is which. I assign each `ACFGNode::Sync` a stable BarrierId based on its structural path (chain of Sequence indices, `usize::MAX` marking "into a Repeat body"). Both worker projections walk the same paths and look up by the same key.

**Slot indexing.** SlotIds are assigned by DataId ordering of cross-worker data — deterministic and worker-independent.

**Host selection.** I pick the worker literally named `"host"` if present (PRD §6.3 convention), else the lexicographically smallest used worker. This is just naming convenience — host is the main thread, non-host workers are `thread::spawn`'d.

**Validation deferred vs upfront.** `validate_placements` rejects distributed placements upfront (`place k on {w0, w1, ...}` — needs iteration-space partitioning, TASK-0117). Multi-consumer-entity rejection (TASK-0127) and async / buffer>1 rejection happen during Plan::build for clearer error messages.

## Honest limitations

- **AC #5 (example 02 e2e bit-identical) is MET.** `cargo test -p compiler --test e2e_example_02` is green, output bit-identical to `examples/02-split-add/reference.bin`.
- **Whole-array transfer granularity only.** The per-tile semantics in the ACFG are ignored — the codegen synthesises whole-array Slots from `linked.data_producers`/`data_consumers`. Symptom: an inner-loop barrier still fires per iteration in example 02 (256 redundant `bar_1.wait()` calls). Correct, slow. Filed as **TASK-0128**.
- **No data fan-out.** If a data symbol has more than one consumer entity, codegen rejects with `UnsupportedFeature`. Not needed for example 02; will block example 13 (CNN, batch inputs broadcast to many workers). Filed as **TASK-0127**.
- **No distributed kernel placement.** `place k on {w0, w1, ...}` rejected before plan build. Filed as **TASK-0117** (already existed).
- **ACFG Xfer nodes silently ignored.** Documented in `multi_worker.rs` module header. The `inject_transfers` pass's per-tile Waits without matching Pushes can't be honoured as-is. When TASK-0126 lands the ACFG-driven path becomes load-bearing.
- **No optimisation: per-iteration Sync barriers honoured literally.** PRD says these are over-syncs in the sync-injection rule's coarse modelling. Filed as **TASK-0128**.
- **No identity-copy support.** Inherited from the single-worker emitter and earlier passes.
- **No coalesced clone elision.** Producer pushes via `.clone()` even when it's the last use. Could be a move-with-fallback (e.g. read the binding out of scope first); cosmetic perf concern.

## AC verification

- **AC #1** — Two-worker synthetic pingpong (three Push/Wait pairs) compiles and runs correctly: **MET**. `nucleus/backends/pthreads-sync/tests/multi_worker.rs::two_worker_pingpong_compiles_and_runs`. The test parses a synthetic algo (`x <-- produce_x; y <-- produce_y; z <-- combine(x, y); sink(z)`), runs the full pipeline, builds the generated Cargo project, runs it, and verifies the i32 sum sink wrote is `120 + 1600 = 1720`. Three Pushes (host -> w0 for x and y, w0 -> host for z) and three matching Waits, all condvar-based.

- **AC #2** — Every declared worker becomes its own thread::spawn block; main joins them all: **MET**. The host worker is the main thread; every used non-host worker becomes a `thread::spawn(move || { ... })` block. Each handle is `.join().expect(...)`'d after the host body.

- **AC #3** — Sync nodes lower to std::sync::Barrier across participating worker threads: **MET**. Each `ACFGNode::Sync` becomes an `Arc<Barrier>::new(participants_count)` allocated in main, cloned into each participating worker's thread closure, and `.wait()`'d at the corresponding path position. The single-Barrier-per-Sync identity is preserved across the worker projections via path-based BarrierId.

- **AC #4** — Push/Wait pairs share a typed Arc<Slot<T>>: **MET in shape**, **deviated in addressing**. Slots are `Arc<Slot<T>>` with `Mutex<Option<T>>` + `Condvar`. T is the data symbol's Rust type (`Vec<i32>` for example 02). Producer calls `slot.push(value.clone())`; consumer calls `let value = slot.wait();`. **Deviation**: slots are keyed by data symbol (SlotId per cross-worker DataId), not by SeqTag per Push/Wait pair. Rationale in design notes; per-tile slot tables filed as **TASK-0126**.

- **AC #5** — Example 02 split.sched.nuc works end-to-end on this path: **MET** (load-bearing). `cargo test -p compiler --test e2e_example_02` green. Generated project builds, runs, output bit-identical to `examples/02-split-add/reference.bin` (verified by `assert_eq!` over the bytes).

## Verification

- `just check`  -> green.
- `just clippy` -> green (-D warnings clean).
- `just test`   -> green; new tests: `multi_worker::two_worker_pingpong_compiles_and_runs` (1 test), `e2e_example_02::split_pthreads_sync_bit_identical` un-ignored (1 test), `emit::distributed_placement_is_rejected` replacing the prior `multi_worker_is_rejected` (1 test). No regressions; all prior tests still pass.
- `just e2e`    -> green (stub harness; real e2e matrix is in compiler tests).
- Manual: ran example 02 codegen + cargo build + binary; output diff against reference.bin clean.

## Files added/touched

- `nucleus/backends/pthreads-sync/src/multi_worker.rs` (new): the multi-worker codegen module — ~600 lines.
- `nucleus/backends/pthreads-sync/src/lib.rs`: removed the multi-worker UnsupportedFeature gate; dispatches to multi_worker::render_main_rs_multi when used_workers.len() > 1.
- `nucleus/backends/pthreads-sync/tests/emit.rs`: replaced `multi_worker_is_rejected` with `distributed_placement_is_rejected` (the rejection is now narrower).
- `nucleus/backends/pthreads-sync/tests/multi_worker.rs` (new): synthetic two-worker pingpong (AC #1).
- `nucleus/compiler/tests/e2e_example_02.rs`: removed `#[ignore]`; updated header.

## Follow-up tasks filed

- **TASK-0126** — replace whole-array Slot hoist with ACFG-driven per-tile xfer placement (depends on transfer_inject fix + TASK-0116).
- **TASK-0127** — fan-out (multi-consumer-entity) data symbols.
- **TASK-0128** — avoid per-iteration barriers inside loops (Sync optimisation pass or projection-time elision).
<!-- SECTION:NOTES:END -->
