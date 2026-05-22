---
id: TASK-0245
title: >-
  Audit pthreads-async + mp-tcp-bufsync IndexExpr const-resolution parity with
  pthreads-sync's render_int_expr
status: Done
assignee:
  - mped-architect-impl
created_date: '2026-05-22 10:52'
updated_date: '2026-05-22 11:07'
labels:
  - compiler
  - audit
  - tech-debt
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle 35 (TASK-0042.04) discovered + fixed a bug in pthreads_sync::render_int_expr: bare const identifiers (e.g. ITERS, N) were rendered as Rust identifiers when used inside an IndexExpr. The fix routes render_int_expr through RenderCtx and consults sidecar.consts (matching render_const_expr's precedence: abs_subst > consts > bare-ident).

Examples 01..09 + 13 never had consts in IndexExpr, so the bug was inert. Example 11's `grid[(t + ITERS) % (ITERS + 1)][i]` triggered it.

Architect review-gate (cycle 35) flagged that the fix lives in pthreads_sync::render_int_expr (private fn), called via:
- render_const_expr (loop bounds, pub via render_const_expr_pub) — both other backends use this.
- render_flat_index (IndexExpr, pub via render_flat_index_pub) — both other backends use this.

So mp-tcp-bufsync + pthreads-async inherit the fix free THROUGH the pub shims — IF and only if they consume IndexExpr via render_flat_index_pub (not a private/copy renderer of their own).

Audit steps:
1. Confirm by grep that mp-tcp-bufsync's only IndexExpr code path is render_flat_index_pub. If it has its own renderer, port the consts fix in lockstep.
2. Confirm by grep that pthreads-async's only IndexExpr code path is render_flat_index_pub (via the shared multi_worker_walker landed cycle 31 TASK-0239). If it has its own renderer (it shouldn't, given the cycle-31 dedup), port the consts fix.
3. Run example 11/pipelined and example 09/pipelined on pthreads-async — both should still PASS bit-identical (cycle 35 verified 11/pipelined PASSes; that's evidence the centralized fix already reaches the backend that runs the example).
4. Add a synthetic test: an algo+sched that uses a const inside an IndexExpr on a multi-worker schedule, exercised on pthreads-sync + mp-tcp-bufsync + pthreads-async, all bit-identical to reference.

Acceptance:
- Either: confirm structurally (via grep) that all three backends consume render_int_expr via the centralized pub shims AND the IR-test exists, OR
- File HIGH follow-up if a divergent IndexExpr renderer is found in mp-tcp or pthreads-async.

Defer-OK signal: cycle 35 e2e tally 66/55/0/11 with example 11 pipelined × pthreads-async PASS bit-identical — evidence that the centralized fix reaches all consuming backends via the shared shims. This audit is hygiene / belt-and-braces, not a known-broken path.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Onboard: read cycle-35 fix at commit 894f63f.
2. Audit grep mp-tcp-bufsync + pthreads-async for any private render_int_expr / render_flat_index / render_const_expr (parallel renderer that would have its own consts gap).
3. Audit shared walker (multi_worker_walker.rs) uses pub shims.
4. If clean: add 3 sister tests (one per backend) consuming a shared CONST_IN_INDEXEXPR_* fixture in test_common, each asserting (a) the resolved const literal appears in emitted main.rs at the IndexExpr site, (b) the bare const ident does NOT appear as a bare identifier in main.rs.
5. Gate: just test / clippy / e2e / determinism-check-negative / xbackend-check-negative.
6. Append notes with grep counts, sites, test file:line, whether fix was ported.
7. Do NOT mark Done, do NOT commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
---

**Cycle 36 audit + test (2026-05-22) — READY FOR REVIEW + COMMIT**

## Audit results (step 1 — STRUCTURAL: CLEAN)

`grep -rnE "fn render_int_expr|fn render_flat_index|fn render_const_expr" nucleus/backends/mp-tcp-bufsync/`
→ 0 matches.

`grep -rnE "fn render_int_expr|fn render_flat_index|fn render_const_expr" nucleus/backends/pthreads-async/`
→ 0 matches.

`grep -n "render_int_expr\|render_flat_index\|render_const_expr" nucleus/backends/pthreads-sync/src/multi_worker_walker.rs`
→ shared walker uses `render_const_expr_pub` (lines 287-288) ONLY; no private renderer.

`grep -rnE "render_flat_index_pub|render_const_expr_pub|render_int_expr_pub" nucleus/backends/`
→ all consumers (`mp-tcp-bufsync/src/lib.rs:724-725`, `pthreads-sync/src/multi_worker_walker.rs:287-288`, `pthreads-sync/src/multi_worker.rs:287-288`) go through the pub shims. `RenderCtxPub::inner()` (pthreads-sync/src/lib.rs:1614-1620) ALWAYS threads `sidecar` through, so `render_const_expr_pub` + `render_flat_index_pub` both consult `sidecar.consts` — cycle-35 fix reaches all backends by construction.

Auxiliary confirmation: mp-tcp-bufsync deps `pthreads_sync` (single source of truth for renderers per `mp-tcp-bufsync/Cargo.toml`); pthreads-async deps `pthreads_sync` AND consumes `pthreads_sync::multi_worker_walker` (cycle-31 TASK-0239 dedup — `pthreads-async/src/multi_worker.rs:48`).

**Conclusion: NO private parallel renderer exists in either backend. No fix port needed.**

## Test added (step 2 — option (b), 3 sister tests, shared fixture)

Single-source-of-truth fixture added in `nucleus/test-common/src/lib.rs`:
- `CONST_IN_INDEXEXPR_ALGO_SRC` (lines ~234-246): 2-worker algo, `const ITERS : usize = 8` referenced inside `y[ITERS][i]` (LHS, w0) and `y[ITERS][0]` (RHS, host).
- `CONST_IN_INDEXEXPR_SCHED_SRC` (lines ~256-266): 2-worker `{host, w0}` schedule with `transfer x : sync`, `transfer y : sync` (mp-tcp-compatible).
- `CONST_IN_INDEXEXPR_ITERS_VALUE = 8` and `CONST_IN_INDEXEXPR_ITERS_IDENT = "ITERS"` for assertion driving.

3 sister tests, each pinning:
- (1) emitted main.rs / per-worker bin contains the resolved-literal IndexExpr fingerprint `"(8) * 4"` (the row-stride product, where 4 = N and 8 = resolved ITERS).
- (2) emitted source does NOT contain the bare ident `"ITERS"`.

Test sites:
- `nucleus/backends/pthreads-sync/tests/multi_worker.rs:962-1041` — `const_in_indexexpr_pthreads_sync_resolves_to_literal_value`.
- `nucleus/backends/mp-tcp-bufsync/tests/pingpong.rs:256-350` — `const_in_indexexpr_mp_tcp_bufsync_resolves_to_literal_value` (iterates all `worker_bins`, asserts bare-ident absence on EVERY per-worker file).
- `nucleus/backends/pthreads-async/tests/skeleton.rs:330-401` — `const_in_indexexpr_pthreads_async_resolves_to_literal_value`.

All 3 use `test_common::lower_for_test` with default opts (no partition/block/check). Emit-string only (no cargo-build) — fast drift-detection focused, per task brief.

## Fix ported: NO (audit clean — no private renderer to patch).

## Gate

- `nix develop -c just test`: 0 FAILED across all suites. 3 new tests pass.
- `nix develop -c just clippy`: clean (`-D warnings`).
- `nix develop -c just e2e`: **66 / 55 / 0 / 11** — UNCHANGED from cycle-35 baseline.
- `nix develop -c just determinism-check-negative`: OK, `PERTURBED_CELLS=55`.
- `nix develop -c just xbackend-check-negative`: OK, `CORRUPTED_DETECTED=1`, `APPLIED=16`.

## Honest limits

1. The IndexExpr fingerprint asserted (`"(8) * 4"`) is the SPECIFIC row-stride spelling produced by `render_flat_index` for the 2D shape `y : i32[ITERS+1][N]` with N=4. A future refactor that changes flat-index parenthesisation (e.g. coalescing strides differently) breaks the assertion even though the const resolution is still correct. Acceptable trade-off: it's the simplest LOAD-BEARING fingerprint and the substring's specificity catches the bug case (`(ITERS) * 4`).
2. The fixture uses `N=4` (small), so flat-stride `* 4` could collide with an unrelated `4` somewhere in the emitted source. Inspection confirms it does not (the only `* 4` in main.rs is the IndexExpr row-stride site), but a future renderer refactor that introduces a stray `* 4` elsewhere could theoretically false-positive. The bare-ident negative assertion is the load-bearing regression-pin; the positive assertion is supporting evidence.
3. mp-tcp test reads `worker_bins` (per-worker files) rather than a single main.rs — asserts the bare-ident absence on EVERY file. Asymmetric vs pthreads-sync/async (which emit one main.rs), but matches mp-tcp's multi-process emit shape.
4. The cycle-31 multi_worker_walker is the load-bearing audit element for pthreads-async — if a future cycle un-dedups the walker (re-introduces a copy in pthreads-async), the audit assumption breaks. The structural test catches the consequence (bare `ITERS` leaks) but the audit grep itself would need to be re-run.
5. No fix was ported (no work to do). The test exists purely as belt-and-braces; the cycle-35 e2e (`11-game-of-life pipelined × pthreads-async PASS bit-identical`) was already evidence that the centralized fix reaches the consuming backends. The test adds STRUCTURAL drift-detection on top of that BEHAVIOURAL evidence.

## Files touched

- `nucleus/test-common/src/lib.rs` — added `CONST_IN_INDEXEXPR_*` pub consts (+ ~100 lines incl. docs).
- `nucleus/backends/pthreads-sync/tests/multi_worker.rs` — appended test (+ ~80 lines incl. docs).
- `nucleus/backends/mp-tcp-bufsync/tests/pingpong.rs` — appended test (+ ~95 lines incl. docs).
- `nucleus/backends/pthreads-async/tests/skeleton.rs` — appended test (+ ~70 lines incl. docs).

No source code in `backends/*/src/` or `compiler/src/` modified — this is a pure test-coverage / audit commit.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 36 (2026-05-22) — closed. Audit verdict CLEAN: no parallel IndexExpr renderer exists in mp-tcp-bufsync or pthreads-async. Both consume pthreads_sync's render_flat_index_pub / render_const_expr_pub via the cycle-31 shared multi_worker_walker + the RenderCtxPub::inner() shim that always threads sidecar through. The cycle-35 render_int_expr const-resolution fix is therefore structurally centralised — all 3 backends inherit it free.

Audit grep result-verifiable: 'fn render_int_expr|fn render_flat_index|fn render_const_expr' returns ZERO matches in nucleus/backends/{mp-tcp-bufsync,pthreads-async}/. Verified twice (implementer + architect).

Added 3 sister tests across all three backends pinning the invariant 'a const used inside an IndexExpr emits as a literal value, not a bare Rust identifier':
- nucleus/test-common/src/lib.rs:183-274 — shared fixture (CONST_IN_INDEXEXPR_ALGO_SRC + _SCHED_SRC) so all 3 tests consume the SAME input.
- nucleus/backends/pthreads-sync/tests/multi_worker.rs:962-1041 — const_in_indexexpr_pthreads_sync_resolves_to_literal_value
- nucleus/backends/mp-tcp-bufsync/tests/pingpong.rs:256-350 — const_in_indexexpr_mp_tcp_bufsync_resolves_to_literal_value (iterates worker_bins)
- nucleus/backends/pthreads-async/tests/skeleton.rs:330-401 — const_in_indexexpr_pthreads_async_resolves_to_literal_value

Each pins both: (1) the resolved literal '(8) * 4' appears (row-stride positive fingerprint), AND (2) bare 'ITERS' identifier does NOT appear in emitted source (load-bearing negative — would catch a future cycle that copy-pastes a private renderer back in without updating it).

Gate (cycle 36): 3/3 new tests pass; just test 0 FAILED; just clippy clean; just e2e 66/55/0/11 unchanged; NUC_NONDET_PERTURBED_CELLS=55; NUC_XBACKEND_CORRUPTED_DETECTED=1.

Honest limits documented by implementer: positive '(8) * 4' fingerprint is renderer-parenthesisation-coupled (would break under a paren refactor even with const resolution still correct); negative !contains('ITERS') is the durable invariant. N=4 small enough that '* 4' could theoretically collide; inspection confirms it doesn't in the current renderer.

Review-gate (parallel read-only): both qa-test-runner + mped-architect GO. Pure-test-coverage cycle — no production source modified. Belt-and-braces on top of cycle-35's behavioural evidence (11-game-of-life pipelined × pthreads-async PASSed bit-identical).
<!-- SECTION:FINAL_SUMMARY:END -->
