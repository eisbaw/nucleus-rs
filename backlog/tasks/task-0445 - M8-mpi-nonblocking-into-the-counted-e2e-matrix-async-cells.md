---
id: TASK-0445
title: M8 mpi-nonblocking into the counted e2e matrix (async cells)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-06-04 10:52'
updated_date: '2026-06-04 23:22'
labels:
  - M8
  - backend
  - validation
  - e2e
  - mpi
dependencies:
  - TASK-0444
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Extend TASK-0444's mpi tier to the M8 mpi-nonblocking backend. Today mpi-nonblocking value-correctness lives only in 'just check-mpi-nonblocking' (out-of-band, with the dual eager/forced-rendezvous arms). Add mpi-nonblocking to e2e-matrix.toml's mpi_backends tier and declare [[required]] cells for the async schedules it uniquely admits (05-stencil/distributed{,-2d}, 09-producer-consumer/pipelined, 11-game-of-life/pipelined) so they become COUNTED via 'just e2e-mpi'. The harness machinery already exists (--with-mpi tier gate, is_mpi_backend predicate, tight-tier scoping, mpiexec hard-fail probe); this is mostly a toml + capabilities decision. NOTE: the e2e run.sh path bakes default ranks = used-worker count; confirm mpi-nonblocking emits a harness-compatible run.sh (mpi-blocking does). Brings the matrix from 8 of 9 to 9 of 9 backends counted (only embedded-pattern then stays out, by design via renode-multimcu-gate).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 mpi-nonblocking added to mpi_backends; just e2e-mpi runs both mpi backends' declared cells byte-exact vs reference.bin
- [ ] #2 async-only schedules (05/distributed-2d w<->w halo, 09/pipelined host-excluding barrier, 11/pipelined) counted; dual eager/rendezvous concern noted (e2e uses default eager — forced-rendezvous stays in check-mpi-nonblocking)
- [ ] #3 default just e2e unchanged; --with-mpi still hard-fails without mpiexec
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
READINESS (grep-witness 2026-06-05, hard-rule pre-check — VERIFIED REAL, not a duplicate): mpi-nonblocking is NOT yet in e2e-matrix.toml mpi_backends (only "mpi-blocking" listed, line 157). mpi-nonblocking emits a harness-compatible run.sh (lib.rs:187/200, render_run_sh — same is_single_binary=false / run.sh path as mpi-blocking, so TASK-0444 harness machinery covers it with ZERO harness changes). capabilities: transport=mpi, supports_async=true, notify=[barrier,blocking,event] — admits the async cells mpi-blocking rejects. WORK: (1) add "mpi-nonblocking" to mpi_backends; (2) declare [[required]] M8 cells for its async targets (check-mpi-nonblocking covers: 05-stencil/distributed, 05-stencil/distributed-2d, 09-producer-consumer/pipelined, 11-game-of-life/pipelined); (3) just e2e-mpi now runs BOTH mpi backends declared cells -> verify byte-exact. NOTE the dual eager/forced-rendezvous concern is check-mpi-nonblocking-only; e2e uses default eager. COST: expensive .#mpi cross-builds (~27s+/cell, multi-worker n=5) + review gate with another .#mpi e2e-mpi run. Ready for a fresh-context cycle.

PLAN (cycle, orchestrator-direct per TASK-0444 precedent + feedback-spawned-agents-refuse-code-edits — harness/toml infra edits): (1) e2e-matrix.toml: add "mpi-nonblocking" to mpi_backends; (2) declare 4 [[required]] M8 cells backend=mpi-nonblocking: 05-stencil/distributed n=5, 05-stencil/distributed-2d n=5, 11-game-of-life/pipelined n=2, 09-producer-consumer/pipelined n=3 (exactly the check-mpi-nonblocking targets, byte-exact vs reference.bin proven there); (3) extend the tier-2 manifest pin test in e2e/src/tests.rs with an M8 mpi-nonblocking sibling assertion. Confirmed ZERO harness change: harness invokes `bash run.sh INPUT OUTPUT` relying on baked NUC_MPI_RANKS default=used-worker count; mpi-nonblocking render_run_sh (lib.rs:289) is structurally identical to mpi-blocking (lib.rs:260) incl default_ranks path. VERIFY: just e2e-mpi runs 7 cells (3 blocking + 4 nonblocking) byte-exact; default just e2e stays 427.

PROOF 1 OBSERVED (orchestrator-run just e2e-mpi via nix develop, full log /tmp/e2e-mpi-0445.log): 7 cells, total:7 pass:7 fail:0 skipped:0 required-fail:0 EXIT=0. The 4 NEW mpi-nonblocking async cells all byte-exact vs reference.bin: 05-stencil/distributed (n=5, 17.9s), 05-stencil/distributed-2d (n=5 w<->w halo, 19.1s), 09-producer-consumer/pipelined (n=3 host-excluding sub-comm barrier, 17.8s), 11-game-of-life/pipelined (n=2, 17.9s); plus the 3 pre-existing mpi-blocking cells still PASS. Harness diffs each run.sh output against reference.bin so PASS == byte-exact. PROOF 2 (default just e2e == 427 baseline) IN PROGRESS — confirmed planner reports "running 427 cell(s)" (mpi cells correctly OUT of default scope), awaiting totals.

PROOF 2 OBSERVED (orchestrator-run default just e2e via nix develop, log /tmp/e2e-default-0445.log): planner "running 427 cell(s)" then total:427 pass:364 fail:0 skipped:63 required-fail:0 EXIT=0 — IDENTICAL to the recorded 427/364/0/63/0 baseline. The 7 mpi cells stayed OUT of default scope (orthogonal --with-mpi axis via is_mpi_backend/active_backends, TASK-0444 machinery). cargo test -p e2e 100+1+1 pass (extended tier-2 manifest pin with M8 mpi-nonblocking sibling assertion); cargo clippy -p e2e --all-targets clean. AC#1/#2/#3 all met by observation. Proceeding to commit + mandatory parallel review gate (qa-test-runner + mped-architect).
<!-- SECTION:NOTES:END -->
