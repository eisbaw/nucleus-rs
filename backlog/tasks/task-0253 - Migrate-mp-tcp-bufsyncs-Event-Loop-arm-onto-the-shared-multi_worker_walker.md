---
id: TASK-0253
title: 'Migrate mp-tcp-bufsync''s Event::Loop arm onto the shared multi_worker_walker'
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-23 18:14'
updated_date: '2026-05-23 19:41'
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

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 #1 #1 mp-tcp-bufsync's `render_events` `Event::Loop` `block_tag` arm exists in exactly ONE place across the codebase (either consumed via `multi_worker_walker` or via a shared `pub fn render_block_tag_loop(...)` helper in backend-common). No duplicate arithmetic.
- [ ] #2 #2 #2 The migration commit's emitted host source for any blocked multi-worker cell (if one exists) is byte-identical to pre-migration emit (regression guard).
- [ ] #3 #3 #3 mp-tcp-bufsync gains direct unit-test coverage of the rebinding arm — either transitively (via the walker tests once migrated) OR a fresh `mp_tcp_bufsync_blocked_rebind` test constructed via the public `emit` API (closes the TASK-0181 AC#2 "both backends" gap honestly).
- [ ] #4 #4 #4 pthreads-sync / pthreads-async multi-worker e2e cells stay byte-identical (non-regression).
- [ ] #5 #5 #5 The 4 existing `backend-common/tests/multi_worker_blocked_rebind.rs` tests stay green.

## Dependencies

- Hard-blocks-on TASK-0181 (Done cycle 73 — the duplicate arm to be deleted).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Option (b): lift the BlockTag-specific portion of Event::Loop into a shared pub fn in backend-common; both walker + mp-tcp-bufsync delegate.

Helper signature lives in backend-common/src/multi_worker_walker.rs (sibling of render_worker_events): takes (out, indent, iter_var, range, tag, enclosing, render_ctx, names, sidecar). Returns (child_ctx_with_abs_subst). Caller owns body recursion + closing '}'.

Sub-steps:
1. Add render_block_tag_loop_header(out, indent, iter_var, range, tag, enclosing, ctx, names, sidecar) -> Result<RenderCtxPub<'a>, EmitError> in backend-common::multi_worker_walker. Emits the strip-mined inner loop header + builds the abs_subst child. Caller recurses + emits closing brace.
2. Refactor multi_worker_walker::render_worker_events_inner block_tag arm to delegate.
3. Refactor mp-tcp-bufsync::Plan::render_events Event::Loop block_tag arm to delegate (4 lines).
4. Update doc-comments: walker module preamble, mp-tcp-bufsync's pre-arm doc-block, new helper's doc-comment.
5. Gate: just check / clippy --workspace -D warnings / test / e2e 88/70/0/18 byte-identical / determinism-check / 3 negative gates / port-stress 20/20.

Optional: backend-common unit test exercising render_block_tag_loop_header directly (positive + missing-enclosing-tile error).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Helper extracted: `backend_common::multi_worker_walker::render_block_tag_loop_header` (9 params, allow(too_many_arguments) — same rationale as sibling render_worker_events_inner). Helper owns: lo_src lookup, abs expression construction (full + partial branches + typed ContractGap on missing enclosing tile), abs_subst child construction, loop-header writeln!. Caller owns: body recursion + closing brace. Both call sites (walker arm + mp-tcp-bufsync Plan::render_events arm) now delegate; the duplicated arithmetic is GONE.

Doc-comment updates: (a) walker module preamble at line 21 now references the shared helper + TASK-0253; (b) RenderCtxPub docstring no longer says 'the walker' (was misleading post-migration) — now names render_block_tag_loop_header explicitly; (c) mp-tcp-bufsync's pre-arm doc-block was rewritten to describe the delegation (was the cycle-73 'intentionally duplicates this arm' paragraph — now states the consolidation).

Tests:
- backend-common/tests/multi_worker_blocked_rebind.rs (existing 4) — STILL GREEN. Transitively cover the walker call site; the helper is exercised via render_worker_events.
- backend-common/tests/block_tag_loop_header.rs (NEW, 4 tests) — pin the helper's surface CONTRACT directly: full-nest header + abs_subst, partial-nest constant base (proves enclosing is unused), missing-enclosing-tile typed ContractGap (with header-bytes-NOT-emitted assertion on error path), zero-LO fallback when sidecar.loop_bounds has no entry.
- AC#3: mp-tcp-bufsync now transitively shares the walker tests (the helper is the same code path); plus the new direct tests pin the surface. No mp-tcp-bufsync-specific public-API test was needed — the migration itself closes the AC.

Byte-identical proof (AC#2 — the load-bearing guard): snapshotted all 172 generated .rs files BEFORE migration to /tmp/nuc-pre-task253, ran fresh post-migration e2e and snapshotted to /tmp/nuc-post-task253, `diff -r` returned 0 changes (zero diff). Every emitted file is byte-identical pre vs post.
<!-- SECTION:NOTES:END -->

<!-- AC:END -->

<!-- AC:END -->
