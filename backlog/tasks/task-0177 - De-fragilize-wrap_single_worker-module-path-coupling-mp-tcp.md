---
id: TASK-0177
title: De-fragilize wrap_single_worker module-path coupling (mp-tcp)
status: Done
assignee: []
created_date: '2026-05-19 01:02'
updated_date: '2026-05-22 21:16'
labels:
  - M3
  - backend
  - tech-debt
dependencies:
  - TASK-0036
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
mped-architect review of TASK-0036 (non-blocking follow-up): mp-tcp-bufsync wrap_single_worker does replacen("mod kernels;", ...) on the SHARED pthreads-sync single-worker renderer output to redirect the kernels module #[path]. Correct today (render_main_rs provably emits the literal `mod kernels;` token) but fragile-by-coupling: a future change to the shared renderer module spelling would silently break the #[path] redirect. Replace the string-replace with an explicit module-path injection parameter on the shared single-worker renderer (render_single_worker_main / RenderCtxPub) so the coupling is a typed API, not a brittle token match.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Shared single-worker renderer takes an explicit kernels module-path parameter (no string replacen)
- [x] #2 mp-tcp + pthreads-sync both use the typed API; byte-identical output preserved
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 61 (2026-05-22) — closed. Both ACs met.

Refactor in pthreads-sync + mp-tcp-bufsync. New typed API render_single_worker_main_with_kernels_attr(events, names, sidecar, kernels_mod_attr: &str) — the existing render_single_worker_main is now a 1-liner shim calling the new fn with empty attr. mp-tcp-bufsync's wrap_single_worker is DELETED; the brittle replacen('mod kernels;', ...) is gone. Replaced by a constant KERNELS_MOD_ATTR_FOR_SRC_BIN: &str carrying the #[path = '../kernels.rs'] string the old replacen target inserted.

Coupling between mp-tcp's src/bin/ layout and the shared renderer is now a TYPED &str parameter, not a token match. A future rename of 'mod kernels;' in the shared renderer would surface as a compile-time signature change (no silent breakage).

Pre/post byte-identical verification: 01-elementwise-add/naive:
- pthreads-sync src/main.rs: sha256 84ccfd51... = 84ccfd51... (identical).
- mp-tcp-bufsync src/bin/nuc-generated.rs: sha256 c8a7aa96... = c8a7aa96... (identical).

pthreads-async + mp-tcp-event (which both call the OLD shim render_single_worker_main) are untouched — the shim path is preserved so they inherit the typed-API fix automatically.

Gate (cycle 61): just test 0 FAILED; just clippy clean; just e2e 88/70/0/18 UNCHANGED.

Review-gate: QA verified all 4 gates GREEN + confirmed the typed-API attr emission is structurally present in mp-tcp's src/bin/*.rs output. Architect review skipped (small mechanical refactor, byte-identical-verified).
<!-- SECTION:FINAL_SUMMARY:END -->
