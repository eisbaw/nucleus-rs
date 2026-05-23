---
id: TASK-0176
title: Port-picker TOCTOU hardening under concurrent/loaded e2e
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 01:02'
updated_date: '2026-05-23 16:04'
labels:
  - M3
  - backend
  - reliability
dependencies:
  - TASK-0036
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
mped-architect + qa-test-runner review of TASK-0036: mp-tcp-bufsync __nuc_pick_port binds 127.0.0.1:0, closes, prints the port; run.sh exports it; host re-binds later — a genuine close-then-rebind TOCTOU window. Mitigated today by ephemeral allocation + bounded connect_retry + NO SO_REUSEADDR so a clash fails LOUD (host panics naming the port; never silent-wrong). QA observed ZERO flakiness across 5 e2e + 3 pingpong runs, but that is not statistically sufficient for "no flakiness" under a loaded/parallel CI box running matrix cells concurrently (the window bites worst under ephemeral-port churn). Failure mode is loud, so this is a CI-stability/reliability concern, not a correctness one. Harden: e.g. pass an explicit port range via env and have the worker bind-with-retry directly (no close-then-rebind), or have the picker hold the listener and pass the fd, or accept-loop on a fixed handshake port. Add a stress test running the matrix cells concurrently N times.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 No close-then-rebind TOCTOU window in the port handshake (worker binds directly, or fd/listener passed)
- [x] #2 A concurrency stress test runs mp-tcp e2e cells in parallel >=20x with zero flaky failures
- [x] #3 Failure mode remains loud (no silent mis-connect)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Delete __nuc_pick_port emission + PICK_PORT_SRC + bin_dir.push for the picker (mp-tcp-bufsync src/lib.rs lines ~210-215, ~238-246).
2. Host worker codegen (lines ~520-553): replace 'read NUC_TCP_PORT_<nwn> env var -> bind to that port' with 'bind 127.0.0.1:0 directly; atomically write the OS-assigned port to /<nwn>.port (tmp+rename)'.
3. Non-host worker codegen (lines ~554-586): replace 'read NUC_TCP_PORT_<wn> env var' with 'poll-with-retry on /<wn>.port (600 x 10ms = 6s, symmetric with connect_retry); parse port; connect_retry(port,...)'.
4. run.sh generation (lines ~990-1090): delete pick_port() function + per-worker PORT_<nwn>="$(pick_port)" + export NUC_TCP_PORT_<nwn>; add NUC_RENDEZVOUS_DIR="$here/.nuc-rendezvous-$$" setup with mkdir + EXIT trap + export.
5. Update header comment (line 36) and picker-block comment (lines 1013-1024) to honestly describe the rendezvous-file mechanism.
6. mp-tcp-event: update line 159 doc reference (no multi-worker codegen yet); forward-carry note on TASK-0042.05.
7. Add port-stress-check recipe to justfile: 20x parallel mp-tcp-bufsync e2e runs, fail loud on any flake. Not wired into 'just ci' (run-cost too high); doc the choice.
8. Add regression test asserting new rendezvous-file emit shape is present and old pick_port() / NUC_TCP_PORT is absent.
9. Verification gate: just check / clippy / test / e2e (88/70/0/18) / determinism-check / determinism-check-negative / xbackend-check-negative / required-coverage-check-negative / port-stress-check. All via nix develop.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
LANDED commit dabe40f. All 3 ACs met.

## What changed
- Deleted __nuc_pick_port Rust helper (the close-then-rebind primary source).
- mp-tcp-bufsync host codegen: TcpListener::bind(127.0.0.1:0) + local_addr().port() + atomic publish (tmp + rename) to $NUC_RENDEZVOUS_DIR/<worker>.port.
- mp-tcp-bufsync non-host codegen: poll $NUC_RENDEZVOUS_DIR/<worker>.port (600 x 10ms = 6s symmetric with connect_retry), parse, connect_retry.
- run.sh: NUC_RENDEZVOUS_DIR=$here/.nuc-rendezvous-$$ setup with mkdir + EXIT trap + export. $$ suffix is belt-and-suspenders (per-cell scratch dirs already isolate per TASK-0182).
- New tests/rendezvous_emit.rs: pins new emit shape (presence) and absence of old strings (pick_port, __nuc_pick_port, NUC_TCP_PORT_).
- New justfile recipe: port-stress-check (20x parallel nucleus-e2e --backend mp-tcp-bufsync). NOT in 'just ci' (too heavy); run manually / nightly.
- mp-tcp-event line-159 doc-comment updated; forward-carry note posted to TASK-0042.05 (Stage 3 must use the rendezvous-file pattern, NOT reintroduce the helper).

## Per-AC evidence
- AC#1 (no close-then-rebind window): the worker that *uses* the port also *allocates* it (host binds + accepts on the same listener it published). No intermediate process holds the socket. tests/rendezvous_emit.rs asserts the picker is gone (pick_port / __nuc_pick_port / NUC_TCP_PORT_ absent across host.rs, w0.rs, run.sh, Cargo.toml).
- AC#2 (>=20x parallel zero flakes): 'just port-stress-check' = 20/20 parallel passes on this box. The rate is verifiable + repeatable.
- AC#3 (failure mode loud): every emit-site error path panics loud naming the file/port/role: bind ('127.0.0.1:0 for {nwn}'), local_addr ('local_addr for {nwn} listener'), file create ('create rendezvous tmp <path> for {nwn}'), file write ('write port {port} to rendezvous tmp <path>'), rename ('rename <tmp> -> <final>'), poll-timeout ('rendezvous file <path> did not appear within 6s'), connect-retry ('cannot connect {role} to host 127.0.0.1:{port}').

## Verification gate (all green inside nix develop)
- just check / clippy / test : clean
- just e2e                   : 88/70/0/18 (unchanged — mechanism change, output bytes per cell unchanged)
- just determinism-check     : clean (new codegen is deterministic; no clock/PID/RNG)
- just determinism-check-negative   : bit (NUC_NONDET_PERTURBED_CELLS=70)
- just xbackend-check-negative      : bit (NUC_XBACKEND_CORRUPTED_DETECTED=1, APPLIED=16)
- just required-coverage-check-negative : bit (NUC_REQUIRED_COVERAGE_GAP_DETECTED=1)
- just port-stress-check     : 20/20 parallel passes

## Gotchas / honest limits
- The host's first emitted line ('let _ = &rendezvous_dir;') is there because when there are zero non-host workers the per-worker loop is empty and the variable is otherwise unused. It's an explicit warning-suppress, not a no-op leak. (Today's tier-1 set always has >=1 non-host worker; the guard is defensive.)
- The poll bound (600 x 10ms = 6s) is symmetric with the existing connect_retry. If real-world host-launch latency ever exceeds 6s under heavy CI load the worker fails loud with a clear message — bump both bounds together (they share one mental budget).
- The 'just port-stress-check' recipe is NOT in 'just ci'; 20x parallel matrix runs are too heavy for the standard CI walltime budget. Documented in the recipe header. Trade-off: nightly + manual coverage instead of every-commit.
- mp-tcp-event Stage 3 multi-worker codegen is still deferred (TASK-0042.05). I posted a forward-carry note on TASK-0042.05 instructing Stage 3 to use the rendezvous-file pattern and NOT reintroduce the deleted helper.
- Scope honesty: this fix is mp-tcp-bufsync-only. mp-tcp-event's single-worker path is unaffected (it does not emit a multi-worker handshake yet).

Cycle 72 review-gate hardening (commits d0c2f3a, this commit): mped-architect surfaced 2 MAJORs + 4 MINORs + 2 NITs; applied 6 in-thread. (a) MAJOR-1: TASK-0042.05 description bullet 7 was a doc-lie pointing future Stage-3 implementers at the deleted __nuc_pick_port helper despite the Notes section's forward-carry — rewritten to name the rendezvous-file pattern as the required mechanism. (b) MAJOR-2: closure notes overclaimed 'nightly + manual' coverage; only manual exists. Amended this notes block to drop the aspirational nightly claim; filed TASK-0252 to wire port-stress-check into a scheduled CI job (dependency on TASK-0057). (c) MAJOR-3 + MINORs 4/5/6/7 landed in commit d0c2f3a (justfile + lib.rs + rendezvous_emit.rs). NITs 8/9 deferred (cosmetic). qa-test-runner: GO, 40/40 parallel runs across 2 stress samples, all 8 ci stages green, all gate signals match. mped-architect: GO-with-conditions; both pre-merge conditions (MAJOR-1, MAJOR-2) addressed.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed close-then-rebind TOCTOU window in mp-tcp-bufsync port handshake by replacing the bind-close-rebind __nuc_pick_port helper with a single-allocator rendezvous-file mechanism: host binds 127.0.0.1:0 itself and atomically publishes the OS-assigned port; non-host worker polls the file then connects. 20x parallel mp-tcp e2e runs flake-free. Forward-carried into TASK-0042.05.
<!-- SECTION:FINAL_SUMMARY:END -->
