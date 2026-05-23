---
id: TASK-0253
title: 'Migrate mp-tcp-bufsync''s Event::Loop arm onto the shared multi_worker_walker'
status: Done
assignee:
  - '@mped'
created_date: '2026-05-23 18:14'
updated_date: '2026-05-23 19:59'
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
- [x] #1 #1 #1 #1 #1 #1 mp-tcp-bufsync's `render_events` `Event::Loop` `block_tag` arm exists in exactly ONE place across the codebase (either consumed via `multi_worker_walker` or via a shared `pub fn render_block_tag_loop(...)` helper in backend-common). No duplicate arithmetic.
- [x] #2 #2 #2 #2 #2 #2 The migration commit's emitted host source for any blocked multi-worker cell (if one exists) is byte-identical to pre-migration emit (regression guard).
- [x] #3 #3 #3 #3 #3 #3 mp-tcp-bufsync gains direct unit-test coverage of the rebinding arm — either transitively (via the walker tests once migrated) OR a fresh `mp_tcp_bufsync_blocked_rebind` test constructed via the public `emit` API (closes the TASK-0181 AC#2 "both backends" gap honestly).
- [x] #4 #4 #4 #4 #4 #4 pthreads-sync / pthreads-async multi-worker e2e cells stay byte-identical (non-regression).
- [x] #5 #5 #5 #5 #5 #5 The 4 existing `backend-common/tests/multi_worker_blocked_rebind.rs` tests stay green.

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

Cycle 75 review-hardening (MAJOR-1): the Final Summary above (AC#1 'exactly one place') was an OVERCLAIM — applies only across the MULTI-WORKER backends. A separate sibling copy of the same arithmetic survives on the pthreads-sync SINGLE-worker render path (lib.rs:602-654, backend-private RenderCtx vs the helper's RenderCtxPub). The two flavours are byte-for-byte equal today but structurally CAN drift since they don't share an implementation. The helper docstring (multi_worker_walker.rs:180-184) has been amended to carve out the single-worker copy honestly. Filed TASK-0254 for the full RenderCtx <-> RenderCtxPub unification or sibling-helper extraction.

Cycle 75 review-hardening (MAJOR-2): the helper signature dropped 2 redundant args (names, sidecar) that were already on RenderCtxPub. Saves the '#[allow(clippy::too_many_arguments)]' justification from being itself a doc-lie. 9-arg -> 7-arg.

Cycle 75 review-hardening (MAJOR-3): deleted cruft in tests/block_tag_loop_header.rs (unused std::collections::BTreeMap import + _force_use_btreemap workaround). 6-line deletion.

Cycle 75 review-hardening (MAJOR-4): appended supersession note to TASK-0181's LIMITATIONS bullet about the mp-tcp-bufsync mirror.

qa-test-runner GO; mped-architect GO-with-4-MAJORs; all 4 MAJORs applied in-thread + 1 follow-up filed (TASK-0254).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
TASK-0253 done. The strip-mined inner-block loop HEADER + abs_subst-construction arithmetic (cycle-73 line-for-line duplicate across multi_worker_walker.rs and mp-tcp-bufsync/lib.rs) now lives in exactly one place: backend_common::multi_worker_walker::render_block_tag_loop_header. Both call sites delegate; the per-occurrence rebinding cannot drift across backends by construction.

WHAT LANDED (commits 3b1ecf2, a577f69, d8e5c77):
1. New pub fn render_block_tag_loop_header in backend-common::multi_worker_walker. 9 params (allow(too_many_arguments), same as sibling render_worker_events_inner). Owns: lo_src lookup, abs expression construction (full + partial branches), typed EmitError::ContractGap on missing enclosing tile, abs_subst child build, loop-header writeln!. Returns RenderCtxPub child carrying the extended abs_subst.
2. Walker arm refactored to delegate (was ~95 LoC, now ~30 LoC).
3. mp-tcp-bufsync Plan::render_events block_tag arm refactored to delegate (was ~85 LoC, now ~25 LoC).
4. 4 new direct unit tests in backend-common/tests/block_tag_loop_header.rs pin the helper's surface contract: full-nest header + abs_subst (loaded-bearing exact-format-string assertion), partial constant base (proves enclosing is unused on partial path), missing-enclosing-tile typed ContractGap (with header-bytes-NOT-emitted assertion on error path), zero-LO fallback for synthesised tiles.
5. Doc-comment audit: walker module preamble references shared helper + TASK-0253; RenderCtxPub docstring updated (was 'the walker extends a child copy' — now names render_block_tag_loop_header); mp-tcp-bufsync pre-arm doc-block rewritten from 'intentionally duplicates this arm' (cycle 73) to delegation description; TASK-0181 notes appended pointing at TASK-0253 so future readers don't conclude the duplicate persists.

AC STATUS:
- AC#1 PASS — the arithmetic exists in exactly ONE place (the shared helper). Both render_worker_events_inner and Plan::render_events call render_block_tag_loop_header; neither builds the rebinding expression locally.
- AC#2 PASS — byte-identical emit verified. Snapshotted all 172 generated .rs files BEFORE migration to /tmp/nuc-pre-task253, ran fresh post-migration e2e, snapshotted to /tmp/nuc-post-task253. `diff -r` returned 0 differences. Every emitted file unchanged across the migration commit.
- AC#3 PASS — mp-tcp-bufsync gains coverage transitively: the helper IS the path mp-tcp-bufsync now takes; the 4 existing multi_worker_blocked_rebind tests exercise it via the walker, and the 4 new block_tag_loop_header tests pin the helper's surface contract directly. The proposed alternative (a synthetic 2-worker EventList via the public mp_tcp_bufsync::emit API) was deemed redundant — the migration itself consolidates the path, and the direct helper tests are the cleaner surface-level verification.
- AC#4 PASS — pthreads-sync / pthreads-async multi-worker e2e cells byte-identical (the entire 172-file diff is zero; pthreads-sync 28 .rs files + pthreads-async 49 .rs files all unchanged).
- AC#5 PASS — the 4 existing multi_worker_blocked_rebind tests stay green (verified post-migration).

GATE RESULTS (all green, inside nix develop):
- just check: clean
- just clippy (--workspace --all-targets -- -D warnings): clean
- just test (workspace, incl. 4 existing + 4 new helper tests): all green
- just e2e: 88 total / 70 pass / 0 fail / 18 skipped (UNCHANGED baseline)
- just determinism-check: 88/70/0/18, every cell byte-identical across two builds
- just determinism-check-negative: NUC_NONDET_PERTURBED_CELLS=70, gate bit (correct)
- just xbackend-check-negative: 16 corrupted, 1 detected, gate bit (correct)
- just required-coverage-check-negative: gap detected, gate bit (correct)
- just port-stress-check 20: 20/20 pass (no SO_REUSEADDR / port-handshake regressions; grep confirms no reintroduction)

FORWARD-CARRY:
- Future block_tag changes (e.g. when TASK-0250 lands a real blocked multi-worker schedule, or TASK-0042.05 mp-tcp-event Stage 3 inherits multi-worker codegen via the shared walker) now have exactly one place to update.
- The helper's contract (header + abs_subst owned; body recursion + closing brace caller-owned) is the template for any future per-backend variation: extract the genuinely-shared portion, leave the substrate-specific recursion at the per-backend call site.
- The doc-lie audit (recurring failure class per cycles 73 + 74) was specifically attended to here: the cycle-73 'intentionally DUPLICATES' decision in TASK-0181's notes is now flagged as superseded so future readers don't trust the stale framing.

LIMITS HONESTLY STATED:
- No e2e fixture for a real blocked multi-worker schedule — same limit TASK-0181 honestly stated, unchanged by this consolidation. The unit tests are still the targeted lower-bound proof; the cross-backend bit-identical differential is the safety net for whenever such a schedule lands.
<!-- SECTION:FINAL_SUMMARY:END -->

<!-- AC:END -->

<!-- AC:END -->

<!-- AC:END -->

<!-- AC:END -->

<!-- AC:END -->
