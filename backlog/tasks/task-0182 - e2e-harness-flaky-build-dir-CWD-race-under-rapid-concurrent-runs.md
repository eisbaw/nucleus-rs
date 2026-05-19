---
id: TASK-0182
title: e2e harness flaky build-dir/CWD race under rapid/concurrent runs
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 02:44'
updated_date: '2026-05-19 14:16'
labels:
  - e2e
  - reliability
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
qa-test-runner advisory during TASK-0167 review (pre-existing, NOT a TASK-0167 regression — reproduced once in 3 back-to-back just e2e runs): 07-matmul/naive/pthreads-sync FAILed with `shell-init: getcwd: cannot access parent directories` + `ld.bfd: cannot open output file .../target/e2e-matrix/.../nuc_generated: No such file or directory`. Root: the harness builds all cells under one shared nucleus/target/e2e-matrix tree; rapid/concurrent invocations race on cwd/build-dir (a prior cells dir removed/recreated under another). Does NOT reproduce serially; the CI per-job matrix already serialises so CI is not currently exposed, but it undermines local reproducibility and any future parallel-cell execution (TASK-0023.01). Harden: per-cell isolated build dir (unique tmp per (example,schedule,backend) run), never chdir into a shared mutable tree, or an explicit lock. Add a concurrency stress test.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Each e2e cell builds/runs in an isolated dir; no shared-tree cwd race
- [x] #2 A stress test runs e2e cells concurrently >=20x with zero infra-race failures
- [x] #3 Serial just e2e remains byte-deterministic
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. ROOT CAUSE: scratch_dir/determinism_dir derive a DETERMINISTIC path per cell-key under nucleus/target/e2e-matrix (resp. e2e-determinism). Two concurrent/rapid harness procs (or a still-cwd-d Command in proc A while proc B remove_dir_all's) race -> getcwd/ld output-file errors.
2. FIX (run-id, not flock): compute a process-wide run id ONCE in Paths::discover (std::process::id() + a nanos nonce), store on Paths. Insert <run-id> segment into both roots: target/e2e-matrix/<run-id>/<cell> and target/e2e-determinism/<run-id>/<cell>__<label>. Within a run, cell paths stay STABLE+UNIQUE so determinism a/b trees for the same cell remain comparable; two concurrent runs never share a mutable path. Lock-free; also unblocks TASK-0023.01 parallel-cell exec (flock would serialise it).
3. DETERMINISM LEAK PROOF: verified enumerate_files returns root-RELATIVE paths and the diff compares file BYTES only (never hashes abs paths); backend emitters use out_dir only to LOCATE writes (out_dir.join), never embed it in content; run.sh uses here=$(cd dirname) relativised; input/output via NUC_INPUT/OUTPUT_PATH env at runtime. So per-run-unique abs path cannot leak into emitted bytes -> AC#3 preserved. Re-verify empirically by running determinism-check TWICE byte-identical.
4. CLEANUP: on a fully-successful run remove the per-run root at the very end of run(); on ANY failure keep it and print its abs path (debuggability). Stays under nucleus/target/ so cargo clean still sweeps. Bounded cruft.
5. STRESS TEST (AC#2): new tests/concurrency_stress.rs — N>=20 threads each allocating a fresh per-(thread)-run scratch tree via the real Paths::scratch_dir path under distinct run-ids AND doing a minimal real build/run that reproduces the OLD race (interleaved remove_dir_all+create+cwd-Command). Assert ZERO infra-race failures (no getcwd / ld output-file). Deterministic, not flaky, representative small cell. Verify it FAILS against a forced-shared-path variant to prove it bites.
6. GATE before every commit (inside nix develop): just e2e 30/26/0/4/0; just determinism-check x2 byte-identical; determinism-check-negative >=5; xbackend-check-negative >=5; stress test x3 non-flaky; just test; just clippy --all-targets; just ci exit 0; confirm per-run dir cleaned on success, retained+printed on failure.
7. Commit per logical unit (git only, no push). backlog notes with ACTUAL numbers + proofs. Forward-carry note to TASK-0023.01.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED (commit 5735b19) — per-run isolated scratch dirs.

ROOT CAUSE: scratch_dir/determinism_dir derived a DETERMINISTIC path per cell-key under nucleus/target/e2e-matrix (resp. e2e-determinism). Two concurrent/rapid harness procs raced: proc A still Command::current_dir-d into a cell dir while proc B remove_dir_all-d + recreated that exact path -> getcwd/ld-output-file infra errors. Harness never set_current_dir-s the process (confirmed); the bug was the shared deterministic PATH, not a process chdir.

FIX: process-wide run_id = "run-{pid}-{nanos}" computed ONCE in Paths::discover, stored on Paths. Inserted as a path segment: e2e-matrix/<run-id>/<cell> and e2e-determinism/<run-id>/<cell>__<label>. Within a run cell paths are stable+unique (determinism a/b stay comparable); disjoint runs never share a remove_dir_all-able tree. RUN-ID vs FLOCK: chose run-id — lock-free, no lock-file cleanup, and a lock would SERIALISE concurrent runs AND block the future parallel-cell executor (TASK-0023.01) whose whole point is concurrency. Run-id makes concurrency safe by construction.

DETERMINISM-NO-LEAK PROOF (AC#3, the critical risk): (1) empirical — just determinism-check run TWICE byte-identical 30/26/0/4 (different run-ids each run, both all-PASS). (2) root proof — emitted the SAME cell (07-matmul/blocked/mp-tcp) to two DIFFERENT absolute out dirs (/tmp/leak-A vs /tmp/leak-B) via the nucleus driver: diff -r = BYTE-IDENTICAL, and grep found NO absolute out-dir/run-id/scratch path embedded anywhere in the emitted tree. Codegen is out-dir-independent: out_dir is only used to LOCATE writes (out_dir.join), run.sh uses here=$(cd dirname) relativised, input/output via NUC_INPUT/OUTPUT_PATH env at runtime. enumerate_files returns root-RELATIVE paths; the diff compares file BYTES only, never abs paths. So a run-unique abs path CANNOT leak.

CLEANUP / CRUFT BOUND: finalize_run_scratch — clean exit 0 removes this run's per-run roots (verified: e2e-matrix empty after a successful just e2e); any non-zero/Err retains them + prints abs path (debuggability). Stays under nucleus/target/ so cargo clean sweeps. KNOWN trade-off (documented in code): the negative-gate recipes (determinism/xbackend-check-negative) exit non-zero BY DESIGN so their per-run dirs are retained — a developer repeatedly running negative recipes accumulates run-* dirs until cargo clean. Accepted: negative recipes are infrequent + intentionally failing, and debuggability of a failing falsifier run is valuable. Normal e2e path is bounded.

STRESS TEST (AC#2): tests/concurrency_stress.rs — 24 concurrent nucleus-e2e invocations on one representative cell (01-elementwise-add/naive/pthreads-sync). POSITIVE arm (hard assert): ZERO infra-race, all exit 0. NEGATIVE control: gate-only NUC_E2E_FORCE_SHARED_RUN_ID pins all 24 onto one shared tree (exact pre-fix condition) — DOES IT BITE: YES, 23/24 hit the infra race / fail. So the test genuinely reproduces the OLD race and proves the fix removes it. Control is flaky-SKIP-not-FAIL (race is probabilistic) so the bite-proof is never itself flaky; positive arm is the hard assertion. Non-flaky: 6/6 runs (3 pre + 3 post clippy-fix) positive-clean + control-bites-23/24.

GATE (all inside nix develop, ACTUAL numbers): just e2e 30/26/0/4/0. determinism-check x2 byte-identical 30/26/0/4 both. determinism-check-negative x5: NUC_NONDET_PERTURBED_CELLS=26 + "OK: determinism check correctly bit" all 5. xbackend-check-negative x5: APPLIED=13 DETECTED=1 + "OK: cross-backend differential correctly bit" all 5. just test 389 passed / 0 failed (incl new concurrency_stress + existing determinism + TASK-0187/0188/0183 sibling tests). just clippy --workspace --all-targets -- -D warnings clean (fixed 3 needless_borrow at run_inner call sites after refactor). just ci EXIT 0.

GOTCHA fixed mid-impl: refactor of run()->run_inner(&Paths) made paths a &Paths, tripping clippy needless_borrow at plan_cells/check_cell_determinism/run_cell call sites — fixed (passed `paths` not `&paths`).

NO subagent review: qa-test-runner/mped-architect subagents are not available as tools in this environment (no agent-spawn tool surfaced; MEMORY notes spawned agents refuse code edits in this repo). Performed rigorous self-review instead: no unsafe/unwrap/panic in prod paths (one documented unwrap_or(0) clock fallback), single-source-of-truth run_id, fail-fast dir errors, cleanup errors logged-not-swallowed, test-only seam mirrors NUC_NONDET_TEST discipline & verified inert when unset.

ORCHESTRATOR review-gate close (phase3-ralph): both reviewers GO, no blocking findings, no follow-up needed. qa-test-runner: e2e 30/26/0/4/0; determinism-check byte-identical across TWO different-run-id processes (no path leak); determinism-negative 5/5 + xbackend-negative 5/5 (both sibling seams unaffected); stress positive arm 0 infra-race x3 non-flaky while control bites 23/24 (real test, not no-op); seam inert when unset; workspace 389/0; clippy --all-targets clean; ci exit 0; successful-run cruft self-cleaned. mped-architect: no-path-leak STRUCTURALLY guaranteed (no-arg render_cargo_toml/render_run_sh signatures actively resist a leak; run.sh relocatable; I/O via env; enumerate_files root-relative); run_id computed once & a/b-consistent; NUC_E2E_FORCE_SHARED_RUN_ID inert-when-unset (correct runtime-env precedent); finalize_run_scratch removes ONLY own run-id subtree (cannot delete a concurrent run); run->run_inner behaviour-preserving; previously-lying "removed/recreated each invocation" doc corrected; forward-carry to TASK-0023.01 accurate incl. non-obvious cleanup hazard. Non-blocking cosmetic only (has_infra_race parenthesization readability; vestigial per-cell remove_dir_all comment) — both reviewers: no follow-up; not worth a full gate cycle for zero behavioural value. TASK-0182 Done stands.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Eliminated the e2e harness shared-build-dir/CWD race so concurrent/rapid runs cannot stomp each other, with determinism preserved.

WHAT CHANGED
- nucleus/e2e/src/main.rs: Paths now carries a process-wide run_id ("run-{pid}-{nanos}") computed ONCE in discover(). scratch_dir/determinism_dir build under e2e-matrix/<run-id>/ resp. e2e-determinism/<run-id>/ via new run_scratch_root/run_determinism_root helpers. New finalize_run_scratch: clean exit 0 removes the per-run roots (bounded cruft), any failure retains + prints the path (debuggability). run() refactored into a thin wrapper that owns the run-id lifecycle and delegates to run_inner, so every exit path gets one consistent cleanup decision. Gate-only NUC_E2E_FORCE_SHARED_RUN_ID test seam (mirrors NUC_NONDET_TEST discipline, strictly inert when unset).
- nucleus/e2e/tests/concurrency_stress.rs (new): 24 concurrent harness invocations; positive arm hard-asserts ZERO infra-race; shared-tree control proves the test bites the pre-fix condition (23/24 race). Non-flaky over 6 runs.

WHY run-id NOT a lock: lock-free, no lock-file cleanup, and an flock would serialise concurrent runs AND block the future parallel-cell executor (TASK-0023.01, whose run-id isolation this is a prerequisite for). Run-id makes concurrency safe by construction.

DETERMINISM (AC#3) — the run-unique absolute path does NOT leak into emitted bytes: proven empirically (determinism-check byte-identical across two separate runs with different run-ids) AND at the root (same cell emitted to two different absolute out dirs is byte-identical; no absolute path embedded anywhere; codegen is out-dir-independent; the diff is root-relative + byte-only).

USER IMPACT: local `just e2e` / `determinism-check` are now safe under concurrent/rapid invocation; CI unchanged (serialised); unblocks TASK-0023.01 parallel-cell execution.

TESTS (actual): e2e 30/26/0/4/0; determinism-check byte-identical x2; determinism-check-negative x5 (PERTURBED=26, bit); xbackend-negative x5 (APPLIED=13 DETECTED=1, bit); just test 389/0; clippy --workspace --all-targets clean; just ci exit 0; stress test non-flaky 6/6 (positive clean, control bites 23/24).

RISKS/FOLLOW-UPS: negative-gate recipes exit non-zero by design so their per-run dirs are retained — repeated negative-recipe runs accumulate run-* dirs until cargo clean (documented, accepted: infrequent + debuggability-valuable; normal e2e path is bounded). qa-test-runner/mped-architect subagents unavailable in this env — substituted a rigorous self-review.
<!-- SECTION:FINAL_SUMMARY:END -->
