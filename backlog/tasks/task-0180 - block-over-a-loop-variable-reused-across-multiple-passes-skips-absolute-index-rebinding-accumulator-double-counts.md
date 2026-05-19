---
id: TASK-0180
title: >-
  block= over a loop variable reused across multiple passes skips absolute-index
  rebinding (accumulator double-counts)
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-19 01:18'
updated_date: '2026-05-19 02:06'
labels:
  - M3
  - backend
  - findings
dependencies:
  - TASK-0039
  - TASK-0173
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Surfaced by TASK-0039 (example 04-prefix-sum, blocked schedule). divisible_inner_block_vars (nucleus/backends/pthreads-sync/src/lib.rs ~455) only grants absolute-index rebinding to an inner-block IterVar whose loop appears EXACTLY ONCE in the EventList (counts==1). That count==1 guard exists to avoid the non-divisible full+partial two-nest ambiguity (TASK-0173). But it ALSO excludes a loop variable NAME legitimately reused across several independent passes: example 04 has three passes each  (NB=4) with  (EVENLY divisible, no remainder). block_transform reuses b's IterVar for all three inner loops, so counts[b]==3, b is dropped from divisible_inner, NO rebinding is applied, and the inner loop runs the FULL source range while wrapped by the tile loop => each accumulator body executes tiles*range times instead of range times. 04-prefix-sum/blocked output is exactly 2x the correct prefix sums on BOTH backends. 05-stencil/07-matmul don't hit this because each uses its tiled var in exactly one loop. Root issue: the count==1 heuristic conflates 'this IS the divisible single-nest' with 'this name is reused across passes'. Fix: distinguish divisible single-nest inner vars structurally (e.g. block_transform tags each inner loop with its (lo, num_full, partial?) so the backend can rebind per-occurrence) rather than by a global occurrence count. Until fixed, a blocked schedule over any algorithm that reuses a loop var name across passes (esp. accumulators) is WRONG. Mitigation in 04: blocked schedule shipped but [[skip]]'d with this reason; only naive is a required differential cell.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 block_transform tags each strip-mined inner loop so the backend rebinds per-occurrence (not by global EventList count)
- [ ] #2 A blocked schedule over a loop var reused across >=2 passes (accumulator) is bit-identical to its naive schedule on both backends
- [ ] #3 04-prefix-sum/blocked moves from [[skip]] to [[required]] for both backends; existing blocked cells (05,07) stay green
- [ ] #4 block_transform tags each strip-mined inner loop so the backend rebinds per-occurrence (not by global EventList occurrence count)
- [ ] #5 A blocked schedule over a loop var reused across two or more passes (accumulator) is bit-identical to its naive schedule on both backends
- [ ] #6 04-prefix-sum blocked moves from skip to required for both backends; existing blocked cells (05, 07) stay green; determinism stays green
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
ROOT CAUSE: divisible_inner_block_vars uses a program-GLOBAL Event::Loop occurrence count (counts==1) to decide absolute-index rebinding. Conflates (i) divisible single-nest, (ii) non-divisible full+partial sibling pair, (iii) a loop-var NAME reused across N evenly-divisible passes (04-prefix-sum, counts==3 -> wrongly excluded -> accumulator runs tiles*range -> 2x/Nx wrong).

DESIGN DECISION (tag carrier): per-OCCURRENCE BlockTag. The NameSidecar is keyed by IterVar (per-program) and inner_block_iter_vars is a BTreeSet<IterVar> (per-program) — BOTH structurally cannot represent per-occurrence facts (same collision as counts==1). Per the FireBinding(TASK-0156)/sidecar(TASK-0160) precedent: per-event facts go ON the event, per-program facts in the sidecar. Strip-mine rebinding is a per-loop-OCCURRENCE fact -> additive optional field on ACFGNode::Repeat (origin: block_transform, the only site that knows lo/N/num_full/partial) threaded onto Event::Loop. Accept the mechanical Repeat-destructuring churn (use ..); the inner_block_iter_vars per-IterVar-set was an M2 shortcut this task's root cause proves insufficient. Keep inner_block_iter_vars for the transfer-hoist consumer (unchanged).\n\nPLAN:\n1. compiler/event.rs + acfg.rs: add BlockTag { lo_src tracked via sidecar by iter_var (unchanged), block_n: i64, num_full: i64, is_partial: bool }. Add Option<BlockTag> additive field to ACFGNode::Repeat (serde default) and Event::Loop (serde default, manual Hash arm).\n2. block_transform.rs tile_nest: emit the inner Repeat with Some(BlockTag): full nest {block_n:N,num_full,is_partial:false}; partial nest {block_n:N,num_full,is_partial:true}.\n3. petri_to_events.rs: thread Repeat.block_tag verbatim onto Event::Loop.block_tag.\n4. pthreads-sync lib.rs: DELETE divisible_inner_block_vars + the divisible_inner set + RenderCtx.divisible_inner. Rebind per-occurrence purely from Event::Loop.block_tag: full -> lo + tile*N + inner; partial -> lo + num_full*N + inner. lo_src still from sidecar.loop_bounds (unchanged, keyed by reused IterVar, same lo for all passes). Typed EmitError (no panic) if a partial tag lacks an enclosing tile loop.\n5. mp-tcp / multi_worker mirror: apply identical rebinding via the pub shims.\n6. 04-prefix-sum: un-#[ignore] e2e_example_04 blocked test; e2e-matrix.toml 04/blocked [[skip]]->[[required]] x2 backends.\n7. GATE x3 e2e, test, determinism (+negative), clippy. 05/06/07-blocked + all 27 existing cells must stay byte-identical. This co-resolves TASK-0173 IF non-divisible falls out cleanly (the tag literally gives 0173 AC#1: per-tile-nest base offset + partial marker + N + num_full); else scope 0173 separately, keep 05 idempotence-safe path UNCHANGED, file honestly.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED (root-cause). Tag carrier decision: per-OCCURRENCE BlockTag{block_n,num_full,is_partial} as an additive serde-default field on Event::Loop (carried through ACFGNode::Repeat from block_transform). NOT the NameSidecar and NOT a BTreeSet<IterVar> like inner_block_iter_vars: both are keyed per-IterVar/per-program and structurally CANNOT distinguish the 3 conflated cases (they share one reused IterVar) — exactly the collision that caused the bug. Mirrors FireBinding(TASK-0156): per-event facts on the event, per-program in the sidecar. LO not duplicated into the tag (stays single-source in sidecar.loop_bounds keyed by the reused IterVar; same lo for all reused passes). Accepted the mechanical ACFGNode::Repeat destructuring churn (the inner_block_iter_vars sidecar-set rationale "keep payload stable" was an M2 shortcut this task's root cause proves insufficient for per-occurrence facts); inner_block_iter_vars kept UNCHANGED for the transfer-hoist consumer.

Backend: DELETED divisible_inner_block_vars + RenderCtx.divisible_inner + RenderCtxPub.empty_inner (net state reduction). render_event Event::Loop arm rebinds purely from block_tag: full -> LO+tile*N+inner (typed EmitError if no enclosing tile loop, not panic); partial -> LO+num_full*N+inner. Multi-worker path (pthreads multi_worker.rs + mp-tcp lib.rs) does NOT thread the tag yet -> fail-loud typed ContractGap if a tagged loop reaches it (no tier-1 blocked multi-worker schedule exists); scoped as TASK-0181 (dep on 0180, code comments reference it).

VERIFIED codegen: 04-prefix-sum/blocked now emits `(0_i64 + (b__tile * 2_i64) + b)` in ALL THREE reused-`b` passes (was raw `b`, double-counting). 05-stencil/blocked (non-divisible) now emits full `1 + y__tile*4 + y` AND partial `1 + 3*4 + y` — STRUCTURALLY correct, no longer idempotence-dependent (was emitting full source range 1..15 inside both tile loops pre-fix). This means TASK-0173 AC#1+AC#2 are now actually IMPLEMENTED + exercised by 05/blocked (forward-carried, not self-checked; 0173 AC#3 synthetic-accumulator test still open).

GATE (inside nix develop, all green): just test 365 pass / 0 fail / 1 ignored (the +1/-1 vs prior 364/2 = the now-active e2e_example_04 blocked test; remaining ignored = unrelated e2e_03 TASK-0117/0126 distributed). just e2e 28 total / 24 pass / 0 fail / 4 skip / 0 required-fail, 3x non-flaky, 04/blocked PASS both pthreads-sync AND mp-tcp-bufsync, 05/06/07-blocked unchanged-PASS (was 22 pass pre-fix; +2 = the 2 newly-required 04/blocked cells). determinism-check byte-identical 28/24/0; determinism-check-negative correctly bites. clippy --workspace -D warnings clean.

HONEST LIMITATION: qa-test-runner / mped-architect sub-agents could not be spawned in this environment (no agent Task tool); performed the equivalent verification + architect self-review manually (full gate x3 + codegen inspection proving the fix is real, not accidental).
<!-- SECTION:NOTES:END -->
