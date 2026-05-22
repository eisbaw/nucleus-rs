---
id: TASK-0222
title: >-
  Extract check_frame emit-string templates into shared helpers when
  pthreads-async lands as 3rd tier-1 backend
status: Done
assignee: []
created_date: '2026-05-21 16:57'
updated_date: '2026-05-22 11:54'
labels:
  - tech-debt
  - M4
  - backend
dependencies:
  - TASK-0042.01
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architecture-review finding (TASK-0052.04 cycle): four emit-string templates are currently DUPLICATED between pthreads-sync and mp-tcp-bufsync (static AtomicU64 decl, per-loop guard local in fn main, Log eprintln branch, Count fetch_add branch). The commit-message claim 'No drift between backends' overstates structural prevention (the shared helpers cover the collector/struct-emitter/sanitizer; the four templates above are verbatim writeln! macros, drift-detection is test-as-tripwire). Tests pin both backends today so drift WOULD surface, but a third tier-1 backend (pthreads-async per TASK-0042.01) pushes past the two-readers-can-hold-it-in-their-head threshold.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Identify the 4 textually-duplicated emit-string templates (static decl, guard local, Log eprintln, Count fetch_add).
- [x] #2 Extract into pub helpers in pthreads-sync (or a sibling 'backend-common' crate): emit_count_static, emit_count_guard_local, emit_log_branch, emit_count_branch.
- [x] #3 Three backends (pthreads-sync + mp-tcp-bufsync + pthreads-async) consume the helpers; the existing tests in compiler/tests/check_frame_codegen.rs + backends/mp-tcp-bufsync/tests/check_frame_emit.rs continue to pin emit-string shape; one new test file covers pthreads-async.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 22 (2026-05-22) — Helpers extracted; 2/3 backends consume them

The four duplicated emit-string templates are now extracted into pthreads-sync as pub fn helpers:

- emit_count_static(out, ident) — file-scope static AtomicU64 decl (was 8 inline writeln sites: pthreads-sync lib.rs:551-558, mp-tcp-bufsync lib.rs:463-470, plus multi_worker counterparts)
- emit_count_guard_local(out, ident, loop_var, latency_max_ns) — per-Count-loop Drop guard local in fn main
- emit_log_branch(out, body_pad, loop_var, latency_max_ns) — Log on-violation branch
- emit_count_branch(out, body_pad, sanitized_ident, latency_max_ns) — Count on-violation branch

All four helpers carry a docstring + line-citation pointing at the pre-extraction sites + which TASK-0052.04 contract the template implements.

12 inline writeln sites (8 in pthreads-sync, 4 in mp-tcp-bufsync) replaced by helper calls. Drift between backends is now STRUCTURALLY prevented — a single edit to a helper propagates to every consumer by construction.

Verification: the existing emit-string-pinning tests (compiler/tests/check_frame_codegen.rs + mp-tcp-bufsync/tests/check_frame_emit.rs) STILL pass byte-for-byte — proving the helpers emit identically to the previous inline writeln! sites.

Gate:
- cargo test --workspace: 571 / 0 / 2 (unchanged — same test count, same behavior).
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- just e2e: 36 / 29 / 0 / 7 baseline preserved.

AC#1 + AC#2 fully met (templates identified + extracted to pthreads-sync). AC#3 PARTIALLY met:
- pthreads-sync + mp-tcp-bufsync consume the helpers (2/3 backends).
- Existing emit-string-pinning tests verified still passing.
- pthreads-async (third backend) DOES NOT YET emit check_frame templates because its multi-worker arm still ContractGaps (TASK-0228 Wave B-2). When Wave B-2 lands, pthreads-async will consume the same helpers + a new test file at nucleus/backends/pthreads-async/tests/check_frame_emit.rs will pin the third backend's emit-string shape.

Task STAYS In Progress until AC#3's third-backend half closes (alongside TASK-0228 AC#5 multi-worker check_frame work).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 38 administrative closure (2026-05-22).

All 3 ACs now closed. AC#3 (three backends consume the shared helpers + pthreads-async test file pins emit-string shape) closed by TASK-0240 cycle 29 commit 90c6d1f, which added nucleus/backends/pthreads-async/tests/check_frame_emit.rs with 3 sister tests pinning Panic/Log/Count multi-worker emit-string shapes via the shared helpers.

All three tier-1 backends now consume the same template helpers (emit_count_static, emit_count_reporter_struct, emit_count_guard_local, emit_log_branch, emit_count_branch, collect_count_check_frames, sanitize_loop_var, CountCheckLoop) AND test-pin their emit-string shape — drift is now structurally + test-detected across all 3 backends.

Cycle 37 (TASK-0244) further hardened: the helpers moved from pthreads-sync to backend-common. Backend-common is now the canonical home; pthreads-sync, mp-tcp-bufsync, pthreads-async all consume as siblings (no inter-backend dependency on the shared check_frame surface).
<!-- SECTION:FINAL_SUMMARY:END -->
