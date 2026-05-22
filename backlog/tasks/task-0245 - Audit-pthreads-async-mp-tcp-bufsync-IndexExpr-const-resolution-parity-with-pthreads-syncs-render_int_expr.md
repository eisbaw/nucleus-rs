---
id: TASK-0245
title: >-
  Audit pthreads-async + mp-tcp-bufsync IndexExpr const-resolution parity with
  pthreads-sync's render_int_expr
status: To Do
assignee: []
created_date: '2026-05-22 10:52'
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
