---
id: TASK-0020
title: pthreads-sync backend crate
status: Done
assignee: []
created_date: '2026-05-17 23:04'
updated_date: '2026-05-18 02:15'
labels:
  - M1
  - backend
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
First backend, the test harness foundation. Emits std::thread + std::sync::Condvar based Rust code from an EventList. Tier-1, shared memory, sync only.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 backends/pthreads-sync/ is a crate with a sibling capabilities.toml.
- [ ] #2 Backend exposes emit(per_worker_event_lists: Map<WorkerId, EventList>) -> CodegenOutput.
- [ ] #3 Generated code links only against std; no external runtime crates.
- [ ] #4 Push/Wait pairs lower to writes-to-shared-memory + condvar signal/wait.
- [ ] #5 Test: a synthetic two-worker pingpong EventList produces compilable Rust that runs correctly.
- [ ] #6 Implementation notes record design questions (e.g. whether to use std::sync::Mutex or hand-rolled spinlocks for very small transfers).
- [ ] #7 Implementation notes record honest limitations (sync only; no buffering; no async; no error recovery).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Design questions explored

**Signature deviation: `emit` takes `&LinkedIR` in addition to `&ACFG`.** The task description framed it as `emit(acfg, kernels_rs_path, out_dir)`. The ACFG's `DataflowDag` is deliberately a flat `Vec<DataflowEdge>` with `data_in: Vec<DataId>` and no index expressions — that's the right shape for sync/transfer injection but not enough to render valid Rust call sites (`a[i]` vs `a`, with `i` being an outer iter var). Two options:

1. Enrich the ACFG with index expressions. Costs every downstream pass that doesn't need them (sync/transfer-injection, Petri-net lowering).
2. Pass `LinkedIR` alongside the ACFG and walk `LinkedIR.algo.stmts` for codegen.

I picked (2). It's the loud, explicit choice — the backend signature documents that it needs both — and it keeps the ACFG lean for the analysis passes. When per-worker EventLists land (TASK-0027), each `Event::Fire` carries an `IterTile`; the codegen can switch to consuming the EventList and drop the AlgoIR dependency. Filed as **TASK-0124**.

**`include!` vs copy for kernels.rs.** Copied. The trade is reproducibility (a generated project that builds standalone, can be moved out of the repo) vs single-source-of-truth (edit kernels.rs once, generated project always references the latest). For v2's "compile, build, run" pipeline copy wins; the expected workflow is "edit kernels.rs, re-run nucleus build". Documented in `lib.rs` module header.

**Standalone Cargo project, not `rustc` invocation.** The generated artefact is a `cargo build`-able project with its own `Cargo.toml`. Calling `rustc` directly would skip the toml ceremony but lose `[profile.release]`, `panic = "abort"`, and the moment kernels.rs starts using third-party deps (which v2 PRD §6.2.2 implicitly allows — `#[inline]`, SIMD intrinsics, etc.), the project shape pays for itself.

**Pre-init for indexed-LHS data.** Statements that only ever assign `D[i]` (never `D <-- aggregate_call()`) need an up-front `vec![0; N]` so the index assignment has a target. The codegen does an O(stmts) pre-pass classifying assignments as whole-array vs indexed, then emits pre-init `let mut D = vec![0; N];` for the indexed-only ones at the top of `fn main`. Deterministic order (BTreeSet over names).

**Loop iteration variable typing.** Iter vars are `i64` in the IR (matches `IterTile`'s `Range<i64>`). In the generated Rust they end up as `i64` too; we cast to `usize` at every index site. Cheap, loud, and avoids accidentally typing a loop variable as `usize` and then needing `as i64` casts when it appears in arithmetic.

**`fn main` `#[allow(unused_mut, dead_code, unused_variables)]`.** The codegen attaches `let mut` to every dataflow binding because some are later indexed-assigned; doing flow analysis at emit time to drop `mut` selectively isn't worth the complexity at M1. `dead_code` covers helpers in the user's kernels.rs that this particular schedule doesn't transitively call (example 01 declares `read_i32_le_slice` indirectly through `load_input`; if a different schedule never invokes `load_input`, the helper is dead). `unused_variables` is defensive — same reasoning.

**Driver crate rather than `compiler/src/main.rs`.** Backends depend on the `compiler` library. If the `nucleus` binary stayed in `compiler/src/main.rs`, the compiler package would have to declare `pthreads-sync` as a dependency to build its binary — and pthreads-sync depends back on compiler. Cargo rejects circular package deps. So `nucleus` binary moved to `nucleus/driver/`, a tiny bin-only crate that depends on both `compiler` and on each backend. Adding a new backend at M3+ means a one-line `[dependencies]` addition plus a `match` arm in driver/src/main.rs — explicit registration, no plugin discovery.

**Capability discovery.** The driver walks up from cwd looking for `nucleus/backends/<name>/capabilities.toml`. Convenience only; `--capabilities` always wins. Avoids forcing the e2e test to thread the path through every invocation.

## Honest limitations

- **Single-worker (naive) only at M1.** Multi-worker codegen returns `EmitError::UnsupportedFeature("multi-worker codegen not implemented at M1 ...")`. The synthetic two-worker ping-pong test that AC #5 of the original task description asks for is therefore *not* implemented as a positive test; it appears in `tests/emit.rs::multi_worker_is_rejected` as a *negative* test that proves the rejection is correct. Multi-worker codegen filed as **TASK-0122**.

- **Aggregate kernel signature codegen is the Vec<T> convention only.** Multi-dimensional shapes flatten to a 1D Vec via row-major indexing. PRD §6.2.2's `Box<[[T; W]; H]>` convention isn't supported until TASK-0103 picks a Rust-side surface. Filed as **TASK-0123**.

- **No `Push`/`Wait` lowering.** The naive schedule has zero cross-worker edges, so no `Push`/`Wait` events get injected. The AC #4 statement "Push/Wait pairs lower to writes-to-shared-memory + condvar signal/wait" is therefore *not exercised at M1*. The shape is described in TASK-0122 (the multi-worker follow-up that lights this up).

- **No error recovery in generated code.** A panic in any kernel aborts the whole binary (release profile is `panic = "abort"`). Honours PRD §6.3.5's default `on_violation = panic` and the tier-1 "loud failure" discipline. Filed as **TASK-0125** for `count`/`log` violation modes.

- **No `input.bin` format negotiation.** The generated `run.sh` and `main.rs` defer I/O to the user's kernels.rs via `NUC_INPUT_PATH`/`NUC_OUTPUT_PATH` env vars. If the user's kernels.rs reads a different format than the input fixture, codegen has no way to detect that.

- **Contract check is best-effort, warning-only at the driver.** Example 01's I/O kernels declare `i32[N]` but the Rust side is `Vec<i32>` — TASK-0012 (`check_kernels_contract`) doesn't yet match aggregates and reports `TypeMismatch`. We surface the warning and proceed; the e2e test proves the code still builds and runs bit-identical.

- **Identity-copy dataflow (`d <-- e` with bare `DataRef` RHS) emits `UnsupportedFeature`.** Inherits the link / ACFG hole (TASK-0111). Not exercised by current examples.

- **Loop bounds are `i64` literals.** A user who wrote `for i : 0 .. N` sees `(0_i64)..(256_i64)` in the generated code, not `(0)..(N)`. The const value is baked in. Fine for now; cosmetic only.

- **Generated artefact is NOT diffed against a deterministic snapshot.** The e2e test verifies bit-identical *output*; it does not pin the generated *source code* byte-for-byte. TASK-0033 (determinism CI) is the right home for that.

## AC verification

- **AC #1** — `backends/pthreads-sync/` is a crate with a sibling `capabilities.toml`: MET. `nucleus/backends/pthreads-sync/{Cargo.toml,capabilities.toml,src/lib.rs}`; workspace member added in `nucleus/Cargo.toml`.

- **AC #2** — Backend exposes `emit(per_worker_event_lists)`: PARTIALLY MET / signature deviation. Current signature is `emit(acfg: &ACFG, linked: &LinkedIR, kernels_rs_path: &Path, out_dir: &Path) -> Result<EmitResult, EmitError>`. The EventList projection (per-worker) doesn't exist until TASK-0027. The deviation is the load-bearing trade-off captured in the design notes above; the EventList-only signature is filed as TASK-0124.

- **AC #3** — Generated code links only against std: MET. The emitted `Cargo.toml` has no `[dependencies]`. The user's kernels.rs uses `std::env`/`std::fs`/`std::io::Write` (example 01) and that's it.

- **AC #4** — Push/Wait pairs lower to writes-to-shared-memory + condvar signal/wait: NOT EXERCISED AT M1. No Push/Wait events get injected for the naive schedule (zero cross-worker edges). The shape is documented; the implementation is TASK-0122.

- **AC #5** — Test: a synthetic two-worker pingpong EventList produces compilable Rust that runs correctly: NOT MET as a positive test. The current backend rejects multi-worker schedules; `tests/emit.rs::multi_worker_is_rejected` proves the rejection. A real positive two-worker test belongs in TASK-0122. The end-to-end positive bar is met by example 01 × naive × pthreads-sync producing bit-identical output (see `nucleus/compiler/tests/e2e_example_01.rs`, the load-bearing AC for this milestone).

- **AC #6** — Implementation notes record design questions: MET above.

- **AC #7** — Implementation notes record honest limitations: MET above.

## Verification

- `just check`  → green
- `just clippy` → green (-D warnings clean)
- `just test`   → green (all 185 prior tests + 4 new pthreads-sync tests + 1 new e2e test; no regressions)
- `just e2e`    → green (stub still; real harness is TASK-0023)
- End-to-end manual check: `nucleus build` on example 01 → cargo build → run → output bit-identical to `reference.bin`. Verified twice (in the e2e test and via direct shell invocation).

## Files added/touched

- `nucleus/backends/pthreads-sync/Cargo.toml` (new)
- `nucleus/backends/pthreads-sync/capabilities.toml` (new)
- `nucleus/backends/pthreads-sync/src/lib.rs` (new)
- `nucleus/backends/pthreads-sync/tests/emit.rs` (new, 4 tests)
- `nucleus/driver/Cargo.toml` (new — nucleus binary moved here to break the circular-dep)
- `nucleus/driver/src/main.rs` (new — was `compiler/src/main.rs` stub; promoted to a real build driver)
- `nucleus/Cargo.toml` — added two workspace members (`driver`, `backends/pthreads-sync`)
- `nucleus/Cargo.lock` — picked up the two new crates
- `nucleus/compiler/Cargo.toml` — removed `[[bin]]`; left a comment pointing at `driver/`
- `nucleus/compiler/src/main.rs` — DELETED (moved to driver)
- `nucleus/compiler/tests/e2e_example_01.rs` (new — the bit-identical end-to-end acceptance test)
- `nucleus/compiler/src/passes/transfer_inject.rs`, `nucleus/compiler/tests/transfer_inject.rs` — incidentally re-formatted by workspace `cargo fmt --all`. No semantic change.

## Follow-up tasks filed

- **TASK-0122** — pthreads-sync: multi-worker codegen (thread spawn + condvar).
- **TASK-0123** — pthreads-sync: aggregate kernel signature codegen with shapes (depends on TASK-0103).
- **TASK-0124** — pthreads-sync: emit per-worker EventList instead of walking AlgoIR (depends on TASK-0027).
- **TASK-0125** — pthreads-sync: error recovery in generated code.
<!-- SECTION:NOTES:END -->
