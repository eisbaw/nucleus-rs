---
id: TASK-0405
title: >-
  Optional: per-backend bite tests for EmitError I/O variants (cross-backend
  silent-sibling sweep)
status: Done
assignee:
  - '@mark'
created_date: '2026-06-01 06:21'
updated_date: '2026-06-05 13:55'
labels:
  - hardening
  - testing
  - silent-sibling
  - prove-the-check-bites
  - cycle-236-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-236 TASK-0404 follow-up. TASK-0404 added bite tests for KernelsReadFailed/OutputCreateFailed/WriteFailed on the CANONICAL backend (pthreads-sync), proving the variants at VARIANT granularity. The same 3 fs::*().map_err(EmitError::X) sites recur verbatim in all 10 backends (pthreads-async, mp-tcp-{bufsync,event,poll}, mp-uds-event, openmp-rs, mpi-{blocking,nonblocking}, embedded-pattern). A cycle-236 grep audit DISCHARGED the silent-sibling risk: every backend uses .map_err(EmitError::...) for these ops -- NONE uses unwrap/expect/?-on-raw-io -- so no backend panics on I/O failure. This task would add per-backend bite tests for completeness (SITE granularity per the TASK-0397 variant-vs-site lesson).

LOW / OPTIONAL: the sites are mechanical identical copies and the grep audit already proves consistency; per-backend tests would be duplication with low marginal yield. Only pursue if a backend diverges its I/O handling (e.g. a future backend that builds paths differently) OR to harden against a future refactor that could re-introduce an unwrap. Each backend embeds its emit entry differently (some lib-only, some bin), so the test scaffolding is not uniform -- mirror each backends existing emit/tests harness.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0410/0411 (cycle-237): the just-ci gate does NOT build docs, so any change touching a doc-linked symbol (removing/narrowing a pub item, removing an error variant referenced by [`...`]) must run cargo doc --workspace --no-deps before/after and diff the generated-N-warning sum (baseline 10). For bite/sibling-sweep tasks that ADD tests this is usually moot, but if the work removes or renames a symbol carrying an intra-doc-link, add the cargo-doc diff to the gate.

Implementation plan (cycle): TEST-ONLY. Add 3 bite tests x 9 sibling backends (27 total) for EmitError::{KernelsReadFailed,OutputCreateFailed,WriteFailed}, mirroring canonical pthreads-sync/tests/emit.rs TASK-0404 pattern. Fixture: test_common::lower_for_test on 01-elementwise-add/naive (single-worker reaches all 3 fs sites before any ContractGap). Scratch via test_common::unique_scratch_dir. Per-backend first-write (WriteFailed target) discovered by reading each emit(): pthreads-async/openmp-rs/embedded-pattern = out/Cargo.toml; mp-tcp-{bufsync,event,poll}/mp-uds-event/mpi-{blocking,nonblocking} = out/src/kernels.rs. New tests/ dirs for mpi-blocking, mpi-nonblocking, embedded-pattern. embedded-pattern returns MultiEmitResult and emits no_std lib (single-worker project_dir=out_dir). Will teeth-check by breaking one .map_err then reverting, run full just ci once, hold e2e baseline 483/420/0/63/0.

DONE (commit de699b6). Deliverable checklist:
[x] 27 bite tests across 9 sibling backends (3 each: KernelsReadFailed / OutputCreateFailed / WriteFailed) in tests/emit_io_errors.rs. New tests/ dir created for mpi-blocking, mpi-nonblocking, embedded-pattern.
[x] Real-teeth verified. Teeth-check: flipped embedded-pattern write_file variant WriteFailed->OutputCreateFailed; write_failed test FAILED naming out/Cargo.toml with IsADirectory (Os code 21), proving it reaches the exact first-write site; reverted (git diff on all backends src/ empty).
[x] SITE-granularity grep-audit re-run CLEAN: all 10 backends wrap the 3 fs sites via .map_err(EmitError::KernelsReadFailed) + .map_err(EmitError::OutputCreateFailed) + write_file()->WriteFailed; NONE uses unwrap/expect/?-on-raw-io. No silent-sibling defect found; no follow-up task needed.
[x] Full just ci GREEN (exit 0). e2e baseline HELD EXACTLY 483/420/0/63/0 (re-confirmed via standalone just e2e). dev test count 1432 passed/0 failed/3 ignored; release 1430 passed/0 failed/3 ignored (the dev/release -2 delta is pre-existing debug_assert-gated tests, NOT from this change; the 27 new tests are +27 in BOTH profiles, no debug_assert gating). clippy --workspace --all-targets clean (no doc_lazy_continuation / needless_borrow).
[x] TEST-ONLY: zero production-code change (git diff --stat shows only test files + this tracker md).

FORWARD-CARRIED LESSONS for the next person:
1. Per-backend FIRST-WRITE (the WriteFailed target) differs and is NOT always Cargo.toml:
   - out/Cargo.toml: pthreads-async, openmp-rs, embedded-pattern.
   - out/src/kernels.rs: mp-tcp-bufsync, mp-tcp-event, mp-tcp-poll, mp-uds-event, mpi-blocking, mpi-nonblocking (these write src/kernels.rs BEFORE the single-vs-multi-worker branch / before Cargo.toml).
2. OutputCreateFailed dir also differs: out/src (pthreads-async, openmp-rs, mpi-*, embedded-pattern) vs out/src/bin (all four mp-* backends create_dir_all bin_dir). The OutputCreateFailed test only asserts path.starts_with(out), so it is robust to this.
3. embedded-pattern is the layout outlier: emit() returns MultiEmitResult (one EmitResult per used worker) and emits a no_std LIB (Cargo.toml + src/lib.rs), not a bin/main project. For a SINGLE-worker schedule the per-worker project_dir IS out_dir, so first-write = out/Cargo.toml. The bite tests only use expect_err so the Ok return type does not affect them.
4. CONFIRMED: mpi-blocking / mpi-nonblocking / embedded-pattern are NORMAL host std crates; just ci builds+runs these tests WITHOUT any MPI/embedded toolchain (the mpi crate dep lives only in the GENERATED project). All three already had test-common + nucleus-compiler as dev-dependencies.
5. Uniform fixture that reaches all 3 fs sites with real teeth: test_common::lower_for_test on 01-elementwise-add/naive (single-worker; never trips a multi-worker ContractGap before the I/O), + test_common::unique_scratch_dir for scratch (project convention; never hand-roll target.join+remove_dir_all).
6. just ci output piped through tail truncates the real-e2e totals (only the negative NUC_XBACKEND_NEGATIVE arm survives, which shows fail:21 by DESIGN - 57 corrupted/18 detected). Do not misread that as a regression; re-run standalone just e2e for the true baseline line. Gate exit 0 is authoritative.

Cycle orchestrator INDEPENDENT review gate (parallel, read-only) — both GO:
- qa-test-runner: GO. Independently RE-RAN (not implementer-claimed): just build clean; clippy --workspace --all-targets clean (9 new test crates freshly checked, no doc_lazy_continuation/needless_borrow); just test 1432/0/3 dev; just test-release 1430/0/3; just e2e 483/420/0/63/0 x2 (non-flaky). Sampled 4 backends incl mpi-blocking/mpi-nonblocking/embedded-pattern: each emit_io_errors.rs reports exactly 3 tests, run + pass with real typed-EmitError assertions (not 0-case no-ops). Zero production src/ change; tree clean (only pre-existing cruft/*).
- mped-architect: GO, no P1/P2. Verified NON-VACUOUS per-backend (fixture = 01-elementwise-add/naive, workers={host} = single-worker, reaches fs-I/O before any branch); WriteFailed first-write targets correct vs actual emit() (Cargo.toml: pthreads-async/openmp-rs/embedded-pattern; src/kernels.rs: 4 mp-* + 2 mpi-*); SITE grep-audit independently re-confirmed clean (all 10 backends .map_err(EmitError::...)/write_file()->WriteFailed, none unwrap/expect/?-on-raw-io); teeth-check credible (commit has no src/ file; assertions pin variant+path); docs no overclaim.
- Architect P3 (informational) folded back IN-THREAD by orchestrator (commit e211713): mpi-blocking emit() fn-doc was a STALE DOC-LIE — claimed >=2-worker -> EmitError::ContractGap stub, but TASK-0045.01 landed the real SPMD arm (lib.rs:167 render_main_rs_multi) and the module header already described it correctly. Rewrote fn-doc to match reality + mpi-nonblocking sibling style. Verified mpi-nonblocking sibling fn-doc ALREADY correct (NOT a silent-sibling pair). Doc-only; clippy clean; cargo doc shows only the pre-existing benign "links to private item multi_worker" convention warning (just ci does not build docs).
DELIVERABLE: 27 bite tests (9 backends x 3 I/O EmitError variants), all real-teeth-verified, grep-audit clean (no silent-sibling defect found, no follow-up needed), full just ci GREEN, e2e baseline held 483/420/0/63/0. Commits: de699b6 (tests) + e211713 (P3 doc fix). TASK-0405 stays DONE.
<!-- SECTION:NOTES:END -->
