---
id: TASK-0426
title: >-
  mp-tcp-bufsync check_frame_emit scratch_dir parallel-test filesystem race
  (flaky NotFound on kernels_stub write)
status: To Do
assignee: []
created_date: '2026-06-02 03:04'
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
- [ ] #1 check_frame_emit tests use per-test-unique scratch dirs (no shared-parent remove/create race)
- [ ] #2 full just test is green across 10 consecutive runs (flake gone)
<!-- AC:END -->
