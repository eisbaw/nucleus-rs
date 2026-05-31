---
id: TASK-0045
title: M7 — Tier 2 MPI blocking
status: Done
assignee: []
created_date: '2026-05-17 23:08'
updated_date: '2026-05-31 08:39'
labels:
  - M7
  - backend
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
First tier-2 backend: mpi-blocking via rsmpi. SPMD codegen (one binary dispatching on MPI_Comm_rank). Localhost MPI in CI. Examples 1-6 compile. PRD §11. Placeholder.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 backends/mpi-blocking/ crate lands with capabilities.toml.
- [x] #2 SPMD codegen: one binary, rank-dispatched main; MPI_Init/MPI_Finalize wrap execution.
- [x] #3 Localhost MPI (OpenMPI in CI container) runs examples 1-6 with bit-identical output.
- [x] #4 Test: M7 acceptance includes a localhost mpiexec -n N run on each example.
- [x] #5 Implementation notes record design questions (e.g. collective recognition deferred; point-to-point emitted everywhere).
- [x] #6 Implementation notes record honest limitations (no real-cluster CI; CI is localhost-only at M7).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
FORWARD-CARRIED from TASK-0063 (M7 foundation, commit af832ec) — read before implementing:

ENV: build/run ONLY under `nix develop .#mpi` (default shell has no MPI). rsmpi = `mpi = "0.8"` (0.8.1 pinned in tests/mpi/rsmpi-smoke/Cargo.lock). The shell provides LIBCLANG_PATH + BINDGEN_EXTRA_CLANG_ARGS; the generated project inherits them — do NOT emit build.rs/.cargo/config env. `just check-mpi-smoke` proves the toolchain end-to-end.

ARCHITECTURE (confirmed by review gate):

1. MPI is SPMD: emit ONE rank-dispatched binary (`mpiexec -n N <same-elf>`, body branches on `world.rank()`). Does NOT fit the tcp_plan/WirePrimitives multi-binary substrate. Reuse multi_worker_walker/event-walk + the shared render layer (render_fire_*, rust_type_of), but emit a single SPMD main. tests/mpi/rsmpi-smoke/src/main.rs is the single-binary template.

2. Tier-2 ship bar = COMPILE (PRD §7.4 L720-722; runtime best-effort §6 L61-62). Mirror the embedded-pattern precedent: add a `just check-mpi` gate (compile examples 1-6 under .#mpi + best-effort `mpiexec -n N` run), EXCLUDED from e2e-matrix.toml `backends` (that 7-backend bit-identical RUNTIME matrix runs in the DEFAULT shell which has NO MPI). Do NOT add mpi-blocking to e2e-matrix.toml.

3. Lowering map: Push -> blocking world.process_at_rank(dst).send(&enc); Wait -> world.any_process().receive::<T>() (or process_at_rank(src).receive — DECIDE + document; seq-paired). BOTH proven by the smoke. Event::Sync/Barrier -> MPI_Barrier (world.barrier()), or Comm_split+barrier for non-full-world participants:BTreeSet<WorkerId> — this collective slice is UNPROVEN by the foundation; budget a dedicated arm + add a barrier smoke when it lands. PRD §7.2: point-to-point only is CORRECT for v2 (collective recognition deliberately deferred — AC#5).

4. capabilities.toml: tier=2, transport=mpi, notify=[blocking], supports_async=false, supports_buffer (per PRD §7.2 'by MPI impl' — pick conservative), worker_classes=[default], memory_regions=[heap]. Mirror nucleus/backends/embedded-pattern/capabilities.toml shape.

5. Driver: add `"mpi-blocking" => {...}` arm in nucleus/driver/src/main.rs (~L848 match) + add to the `unknown backend` registered list (~L1102) + the --backend help text (~L132).

6. Single-worker (0/1 used worker) emit should reuse pthreads-sync's render_single_worker_main_with_kernels_attr for byte-identical arithmetic (same pattern as mp-tcp-bufsync), wrapped in MPI_Init/Finalize + an `if rank==0` guard.

SCOPE for first backend cycle: crate + capabilities.toml + driver dispatch + SPMD codegen for the single-worker + simplest multi-worker (Push/Wait) examples; `just check-mpi` compile gate. File the barrier/collective slice + any deferred examples as follow-ups (do NOT silently skip).

## Single-worker SPMD arm LANDED (cycle M7-entry, 2026-05-31, commit c5862c6)

Verified by the parallel read-only review gate (qa-test-runner + mped-architect, both GO).

AC status:

- AC#1 (crate + capabilities.toml): DONE. nucleus/backends/mpi-blocking/ + capabilities.toml (tier=2, transport=mpi, notify=[barrier,blocking], supports_buffer=false conservative). Registered in workspace + driver dispatch + help + registered-backend list.

- AC#2 (SPMD codegen: one binary, rank-dispatched main, MPI_Init/Finalize): DONE for the single-worker arm. Emits src/compute.rs (shared single-worker renderer as `pub fn nuc_compute`, byte-identical arithmetic) + a thin src/main.rs SPMD wrapper: mpi::initialize() (MPI_Init) + Universe-drop (MPI_Finalize) + `if world.rank()==0 { compute::nuc_compute() }`. The MULTI-rank fan-out (different work per rank) is TASK-0045.01.

- AC#3 (localhost MPI runs examples 1-6 bit-identical): MET FOR THE NAIVE (single-worker) SCHEDULES of all of examples 1-6 — `just check-mpi` compiles each + runs under `mpiexec -n 1` + cmps BYTE-EXACT vs reference.bin (value-correct, stronger than the PRD §7.4 COMPILE bar). LEFT UNTICKED because the examples' MULTI-worker schedules (e.g. 02-split-add/split) are TASK-0045.01 — not overclaiming full closure.

- AC#4 (localhost mpiexec -n N run per example): DONE. check-mpi runs mpiexec on each of examples 1-6.

- AC#5 (design questions: collective deferred, point-to-point everywhere): DONE — documented in lib.rs module docstring + capabilities.toml (collective recognition MPI_Allreduce/Scatter deliberately deferred per PRD §7.2; v2 emits point-to-point + MPI_Barrier only).

- AC#6 (honest limits: no real-cluster CI, localhost-only): DONE — documented in lib.rs (no real-cluster CI; point-to-point+barrier only; single-worker arm only this cycle, multi-worker rejected loud).

REMAINING for parent closure: the multi-worker SPMD arm = TASK-0045.01 (filed, depends on this). emit() returns a loud EmitError::ContractGap for used_workers>1 until it lands. Parent stays In Progress.

## Gotchas / lessons (forward-carried to TASK-0045.01)

- SPMD process model is the trap, not the arithmetic: ONE elf, body branches on world.rank() — cannot route through tcp_plan/WirePrimitives (N separate binaries). The single-worker arm proved compute.rs == shared renderer cleanly; multi-worker needs a `match rank` / per-rank fn dispatch inside the single main.

- Renderer reuse required an ADDITIVE pthreads-sync change: render_single_worker_main_with_signature (render_main_rs gained fn_main_signature, default "fn main()" byte-identical — proven by e2e 350/293 unchanged x2). The rendered `fn main` is module-private; emitting it as `pub fn nuc_compute` in src/compute.rs makes it callable from the MPI wrapper without textual surgery.

- Send/Recv typing (sidecar element type -> rsmpi Buffer/Equivalence bounds) + barrier participant sets (whole-world vs Comm_split for host-excluding) are the two sharp edges for .01; mirror the backend election rule exactly (feedback-driver-must-mirror-backend-election-exactly); test under mpiexec -n N (all ranks live), NOT -n 1 (16-jacobi: deadlock-free != value-correct).

- The shared check-loop reporter (NucCheckCountReporter) now emits into nuc_compute()'s body; unexercised under MPI (examples 1-6 naive have no check loop). .01 AC#5 spot-verifies it.

M7 milestone CLOSED (multi-worker arm landed). AC#3 (examples 1-6 bit-identical under localhost MPI) now MET: single-worker arm covers examples 1-6 naive (mpiexec -n 1 byte-exact); multi-worker arm (TASK-0045.01) covers the SYNC multi-worker schedules 02-split/split, 03-reduction/distributed, 06-separable/distributed + distributed2 (mpiexec -n N, all ranks live, byte-exact). 05-stencil/distributed{,-2d} are async => correctly capability-rejected (mpi-nonblocking M8/TASK-0046 targets, not blocking-backend schedules). All proven by `just check-mpi`.

Deferred robustness follow-ups (no shipped consumer, loud-rejected, filed): TASK-0045.02 (Comm_split for host-excluding/non-uniform barriers), TASK-0045.03 (multi-worker check-loop per-rank-vs-aggregate reporter). Neither blocks any example. M8 = TASK-0046 (forward-carried lessons + dep edge added).
<!-- SECTION:NOTES:END -->
