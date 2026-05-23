---
id: TASK-0253
title: 'Migrate mp-tcp-bufsync''s Event::Loop arm onto the shared multi_worker_walker'
status: To Do
assignee: []
created_date: '2026-05-23 18:14'
labels:
  - backend
  - M3
  - refactor
dependencies:
  - TASK-0181
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0181 landed per-occurrence strip-mine rebinding on the SHARED multi_worker_walker AND mirrored the same logic onto mp-tcp-bufsync's parallel Event::Loop arm in lib.rs::render_events. The mirror is line-for-line equivalent to the walker but lives in a separate file — a known drift hazard the cross-backend bit-identical differential gate (PRD §10.1) catches LATE (only when a blocked multi-worker schedule exists and the two diverge).

The clean fix is to migrate mp-tcp-bufsync's render_events onto backend_common::multi_worker_walker::render_worker_events, the same way pthreads-sync multi_worker.rs and pthreads-async multi_worker.rs already do. The blocker is the substrate: mp-tcp-bufsync uses TCP sockets + ctrl_<peer> / sock_<peer> barriers, not Slot/Ring rendezvous. The walker is parameterised by ONE knob (`rendezvous_prefix: &str`); mp-tcp-bufsync would need a second axis of variation (the ctrl_/sock_ barrier scheme + host-vs-worker dispatch) the walker today does NOT model.

Sub-tasks:
1. Audit which parts of mp-tcp-bufsync::render_events differ structurally from multi_worker_walker::render_worker_events_inner: Event::Sync (host-mediated star barrier), Event::Push (sock_<peer>.write_all + length prefix), Event::Wait (sock_<peer>.read_exact + decode). These are ~60 LoC each and differ from the walker's bar_/slot_/ring_ idioms.
2. Decide between (a) extending WalkerCtx with a sync/push/wait dispatch trait (heavier; introduces the second axis the walker explicitly rejected in its design doc), (b) lifting just the Event::Loop arm into a shared helper called from both renderers, (c) generating a fourth `rendezvous_prefix`-equivalent abstraction (probably impossible — substrates are too different).
3. Migrate per the chosen option; delete mp-tcp-bufsync's duplicate Loop arm.

Until then: the cross-backend bit-identical differential is the safety net. The 4 backend-common unit tests in tests/multi_worker_blocked_rebind.rs pin the rebinding shape (LO+tile*N+inner etc.) — any drift in mp-tcp-bufsync's mirror would surface only when a blocked multi-worker schedule lands AND that schedule reaches both backends.
<!-- SECTION:DESCRIPTION:END -->
