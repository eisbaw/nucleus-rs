---
id: TASK-0176
title: Port-picker TOCTOU hardening under concurrent/loaded e2e
status: To Do
assignee: []
created_date: '2026-05-19 01:02'
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
- [ ] #1 No close-then-rebind TOCTOU window in the port handshake (worker binds directly, or fd/listener passed)
- [ ] #2 A concurrency stress test runs mp-tcp e2e cells in parallel >=20x with zero flaky failures
- [ ] #3 Failure mode remains loud (no silent mis-connect)
<!-- AC:END -->
