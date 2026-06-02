---
id: TASK-0426
title: >-
  mp-tcp-bufsync check_frame_emit scratch_dir parallel-test filesystem race
  (flaky NotFound on kernels_stub write)
status: In Progress
assignee:
  - '@me'
created_date: '2026-06-02 03:04'
updated_date: '2026-06-02 16:43'
labels:
  - compiler
  - test-flake
  - mp-tcp-bufsync
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Flaky test failure surfaced during TASK-0421 gate. Under parallel cargo test (full just test), tests in nucleus/backends/mp-tcp-bufsync/tests/check_frame_emit.rs intermittently fail with: write kernels stub: Os { code: 2, kind: NotFound } at check_frame_emit.rs:412 (write_kernels_stub). DIFFERENT test fails each run (mp_tcp_bufsync_multi_worker_{log,count,panic}_emit_*); all pass in isolation (cargo test -p mp-tcp-bufsync --test check_frame_emit green 3/3). ROOT CAUSE (hypothesis): scratch_dir() (check_frame_emit.rs:38) does create_dir_all then remove_dir_all/create_dir_all on a SHARED parent nucleus/target/mp-tcp-bufsync-check-frame-scratch/; concurrent test threads race on the shared parent so a subdir is mid-deletion when write_kernels_stub fs::write runs -> ENOENT. UNRELATED to TASK-0421 (net_soundness, different crate, no fs code). FIX: give each test a unique scratch dir (e.g. include a per-test nonce / process id, or use tempfile::tempdir), or stop touching the shared parent after first create. Pre-existing; reproduced on clean tree (stash-pop test).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 check_frame_emit tests use per-test-unique scratch dirs (no shared-parent remove/create race)
- [ ] #2 full just test is green across 10 consecutive runs (flake gone)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle-242 disposition (orchestrator in-thread fix + independent architect review):

INVESTIGATION (measure-first): the reported ENOENT flake did NOT reproduce in 16 runs (12 mp-tcp-bufsync crate-only + 4 full cargo test --workspace). The task root-cause was a HYPOTHESIS; static analysis refuted the single-process version of it (each test already uses a UNIQUE leaf name, and within ONE cargo-test process the shared parent is only ever create_dir_alld, never removed, so unique-named subdirs cannot race).

REAL MECHANISM (found by architect review): repo_root() derives from CARGO_MANIFEST_DIR (source dir, PROFILE-INDEPENDENT), so just test (dev) and just test-release (release) are two PROCESSES with different pids but the SAME pid-less leaf path target/{parent}/{name}. Run concurrently, the dev process remove_dir_all(leaf) deletes the dir the release process write_kernels_stub fs::write is mid-writing -> the exact observed ENOENT. just ci runs them sequentially (justfile:186-187) so the gate itself is safe, but a developer/IDE running both profiles overlapping hits it. This explains why it surfaced during a gate yet is unreproducible under a single-profile loop.

FIX (commits 0e70cdd + 87ba11f): per-call-unique leaf {name}-{pid}-{atomic counter}, created once, never removed, in all 3 mp-tcp-bufsync scratch_dir helpers (check_frame_emit, multi_worker_emit, reuse_codegen_emit). The pid makes dev/release leaves disjoint -> closes the identified cross-process mechanism BY CONSTRUCTION.

GATE: build OK; clippy clean; workspace test 1256 dev / 1254 release (0 failed, unchanged); e2e unaffected (test-helper only, no driver/codegen path). Architect review: GO (disposition honest, no over-claim; code correct + safe; mechanism identified).

AC STATUS: AC#1 (per-test-unique scratch dirs, no shared-parent remove/create) — MET (checked). AC#2 (full just test green 10x, flake gone) — its reproduction-based verification is INFEASIBLE: a single just test runs only the dev binary, so 10x green never exercises the concurrent dev+release overlap the flake needs, and the flake was unreproducible in 16 runs. Closure rests on by-construction elimination of the architect-identified dev/release cross-process mechanism, NOT on a reproduction. Recorded honestly rather than falsely ticking AC#2 via a vacuous 10x-green run.

FOLLOW-UP: TASK-0426.01 (repo-wide sweep — the identical profile-independent pid-less scratch_dir pattern exists in 20+ other test files across all backends + e2e_example tests; defense-in-depth, none observed flaky).
<!-- SECTION:NOTES:END -->
