---
id: TASK-0386
title: >-
  Emitted check-count static NUC_CHECK_COUNT_&lt;ident&gt; trips
  non_upper_case_globals when loop_var is lowercase
status: Done
assignee:
  - '@claude'
created_date: '2026-05-31 06:03'
updated_date: '2026-05-31 12:06'
labels:
  - codegen
  - check-loop
  - cosmetic
  - backend-common
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Pre-existing wart surfaced (newly visible on tier-1) by TASK-0369's check_count cells. `backend-common/src/check_frame.rs::emit_count_static` emits `static NUC_CHECK_COUNT_{ident}` where `ident = sanitize_loop_var(loop_var)`. For a lowercase loop var like `i` this yields `NUC_CHECK_COUNT_i`, which trips rustc's `non_upper_case_globals` warning in the GENERATED crate (warning-only — does NOT fail the e2e `cargo build`, which has no `-D warnings`; so it is cosmetic, not a gate failure). TASK-0369 made it newly visible because it added the first TIER-1 on_violation=count cells (the embedded fixtures already had it via AtomicU32). \n\nFix options: (a) uppercase-mangle the ident in the static name (`NUC_CHECK_COUNT_I`) — CAUTION: changes emitted bytes, so the embedded_check_count golden/determinism fixtures + the new tier-1 check_count cells must be re-verified bit-identical after; or (b) emit `#[allow(non_upper_case_globals)]` on the static. (b) is lower-blast-radius. Either way: a shared check_frame.rs change touches pthreads-sync/pthreads-async/openmp-rs/mp-tcp-bufsync single-worker emit + the embedded backend; run `just e2e` + `just determinism-check` after. Found by mped-architect review of TASK-0369 (cycle-222).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 emit_count_static emits #[allow(non_upper_case_globals)] on the count static; tier-1 generated crate no longer warns on a lowercase-ident count static.
- [x] #2 All existing count-static snapshot tests stay green (attribute on separate line; static-decl line byte-identical).
- [x] #3 emit_count_static docstring updated to reflect two-line emission (no doc-lie).
- [x] #4 Gate green: build/clippy/test/test-release; e2e 350/293/0/57/0 unchanged + determinism-check pass (source-only change, runtime output identical).
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Root cause: emit_count_static (backend-common/src/check_frame.rs) emits a file-scope static whose name embeds the (case-faithful) source loop-var ident for greppability; a lowercase ident like `i`/`n` trips rustc non_upper_case_globals in the GENERATED tier-1/multi-process crate, which (unlike the embedded skeletons) has NO crate-level allow. Verified: all 3 embedded skeletons (skeleton/mod.rs:201, bin.rs:370, multimcu.rs:309) ALREADY carry #![allow(... non_upper_case_globals)] (TASK-0048.08), so the embedded path is clean; the gap is ONLY tier-1, surfaced by TASK-0369 adding the first tier-1 count cells. Fix (task option b, lowest blast radius + root cause): emit a targeted #[allow(non_upper_case_globals)] on a SEPARATE line directly above the static in the SHARED emit_count_static helper — covers all consumers (pthreads-sync/async, openmp-rs via shared helper + tcp_plan/event_plan multi-process substrates) in one edit. Separate line keeps the static-declaration line byte-identical, so all existing .contains()/.matches().count() snapshot tests stay green (verified: mp-tcp-bufsync/pthreads-sync/pthreads-async check_frame tests + pthreads-sync multi_worker all match the static line, not the attribute). Update the emit_count_static docstring (one-line->two-line) to avoid a doc-lie. Gate: build/clippy/test/test-release/e2e + determinism-check; e2e runtime output UNCHANGED (attribute is source-only, not runtime).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
CYCLE OUTCOME (orchestrator-verified, GOx2, no fold-back). Implemented in-thread (repo policy: spawned implementers refuse edits). Commit 5ac3408.

Root cause: emit_count_static (the SOLE shared tier-1/multi-process emitter; 6 consumers — pthreads-sync lib.rs:351 + multi_worker.rs:364, pthreads-async multi_worker.rs:393, openmp-rs multi_worker.rs:271, tcp_plan worker_program.rs:97, event_plan worker_program.rs:92) names the counter NUC_CHECK_COUNT_<ident> embedding the case-preserved source loop-var (sanitize_loop_var is deliberately case-preserving). A lowercase ident trips rustc non_upper_case_globals in the GENERATED tier-1 crate, which lacks the crate-wide allow the 3 embedded skeletons carry (mod.rs:201/bin.rs:370/multimcu.rs:309, TASK-0048.08). Surfaced on tier-1 by TASK-0369.

Fix: per-static #[allow(non_upper_case_globals)] on a SEPARATE line (one shared-helper edit covers all 6 consumers; static-decl line byte-identical so snapshot tests stay green). Architect confirmed option (a) uppercase-mangle would be WORSE: collides i/I + n/N to one static AND loses directive-greppability. Per-static (not crate-level) keeps other globals linted.

VERIFICATION (re-run, not narrated): build/clippy(0/0)/test(1192/0/3)/test-release(1191/0/3); generated tier-1 crate (01-elementwise-add/check_count, loop var `i`) carries the attr on line 21 above the static on line 22 and cargo build = WARN_COUNT=0 (was warning before); 28 count-static snapshot tests pass; e2e 350/293/0/57/0 unchanged; determinism-check 350/293/0/57 byte-identical. QA re-confirmed clippy/tests/WARN_COUNT=0 independently; both reviews GO.

P3 observations (architect, NO action this cycle): (P3.1) item-scoped allow is inherent + still better than crate-level. (P3.2) the docstring tail keeps PRE-EXISTING fragile line-number citations "lib.rs:551-559 / lib.rs:463-471" — not introduced by this commit, left as-is; spot-check/de-number if that doc is touched again (fragile-citation recurring class).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE (GOx2). emit_count_static now emits #[allow(non_upper_case_globals)] on the generated count static, so the tier-1/multi-process generated crate no longer warns on a lowercase-ident counter (e.g. NUC_CHECK_COUNT_i) — the case-faithful, greppable name is preserved (uppercasing would collide i/I and lose traceability). One shared-helper edit covers all 6 tier-1/multi-process consumers; the embedded path was already covered crate-wide. Empirically verified: generated tier-1 count crate builds with 0 warnings. No regression: 28 snapshot tests pass, e2e 350/293/0/57/0 + determinism-check 350/293/0/57 byte-identical (source-only change). Commit 5ac3408.
<!-- SECTION:FINAL_SUMMARY:END -->
