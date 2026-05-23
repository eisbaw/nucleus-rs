---
id: TASK-0253
title: 'Migrate mp-tcp-bufsync''s Event::Loop arm onto the shared multi_worker_walker'
status: To Do
assignee: []
created_date: '2026-05-23 18:14'
updated_date: '2026-05-23 18:23'
labels:
  - backend
  - M3
  - refactor
dependencies:
  - TASK-0181
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0181 (cycle 73) landed per-occurrence strip-mine rebinding on the SHARED `multi_worker_walker` AND mirrored the same arithmetic onto mp-tcp-bufsync's parallel `Event::Loop` arm in `lib.rs::render_events`. The mirror is line-for-line equivalent to the walker but lives in a separate file — a known drift hazard the cross-backend bit-identical differential gate (PRD §10.1) catches LATE (only once a blocked multi-worker schedule lands AND reaches both backends).

The clean fix is to migrate mp-tcp-bufsync's `render_events` onto `backend_common::multi_worker_walker::render_worker_events`, the same way pthreads-sync `multi_worker.rs` and pthreads-async `multi_worker.rs` already do. The blocker is the substrate: mp-tcp-bufsync uses TCP sockets + `ctrl_<peer>` / `sock_<peer>` barriers, not Slot/Ring rendezvous. The walker is parameterised by ONE knob (`rendezvous_prefix: &str`); mp-tcp-bufsync needs a second axis the walker today does NOT model.

## Folded-in scope (from cycle-73 mped-architect review MAJOR-3)

TASK-0181 AC#2 reads "both backends" but the cycle-73 landing added 4 unit tests against the WALKER only — mp-tcp-bufsync's mirror arm has no direct test (only the line-for-line arithmetic equivalence + the reactive cross-backend differential). The honest closure of AC#2's mp-tcp-bufsync half belongs here: when the migration lands, the walker's existing tests transitively cover mp-tcp-bufsync too. If the migration is deferred indefinitely, a small mp-tcp-bufsync-specific unit test (constructing a synthetic 2-worker EventList via the public `emit` API, asserting the rebinding shape in the emitted host source) closes the gap independently.

## Sub-tasks

1. Audit which parts of `mp-tcp-bufsync::render_events` differ structurally from `multi_worker_walker::render_worker_events_inner`: `Event::Sync` (host-mediated star barrier), `Event::Push` (sock_<peer>.write_all + length prefix), `Event::Wait` (sock_<peer>.read_exact + decode). These are ~60 LoC each and differ from the walker's bar_/slot_/ring_ idioms.
2. Decide between (a) extending `WalkerCtx` with a sync/push/wait dispatch trait (heavier; introduces the second axis the walker explicitly rejected in its design doc), (b) lifting just the `Event::Loop` arm into a shared `pub fn render_block_tag_loop(...)` helper called from both renderers, (c) generating a fourth `rendezvous_prefix`-equivalent abstraction (probably impossible — substrates are too different).
3. Migrate per the chosen option; delete mp-tcp-bufsync's duplicate Loop arm.

Until then: the cross-backend bit-identical differential is the safety net. The 4 backend-common unit tests in `tests/multi_worker_blocked_rebind.rs` pin the rebinding shape (`LO+tile*N+inner` etc.) — any drift in mp-tcp-bufsync's mirror would surface only when a blocked multi-worker schedule lands AND that schedule reaches both backends.

## Acceptance criteria

- [ ] #1 mp-tcp-bufsync's `render_events` `Event::Loop` `block_tag` arm exists in exactly ONE place across the codebase (either consumed via `multi_worker_walker` or via a shared `pub fn render_block_tag_loop(...)` helper in backend-common). No duplicate arithmetic.
- [ ] #2 The migration commit's emitted host source for any blocked multi-worker cell (if one exists) is byte-identical to pre-migration emit (regression guard).
- [ ] #3 mp-tcp-bufsync gains direct unit-test coverage of the rebinding arm — either transitively (via the walker tests once migrated) OR a fresh `mp_tcp_bufsync_blocked_rebind` test constructed via the public `emit` API (closes the TASK-0181 AC#2 "both backends" gap honestly).
- [ ] #4 pthreads-sync / pthreads-async multi-worker e2e cells stay byte-identical (non-regression).
- [ ] #5 The 4 existing `backend-common/tests/multi_worker_blocked_rebind.rs` tests stay green.

## Dependencies

- Hard-blocks-on TASK-0181 (Done cycle 73 — the duplicate arm to be deleted).
<!-- SECTION:DESCRIPTION:END -->
