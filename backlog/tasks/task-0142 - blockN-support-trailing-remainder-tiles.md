---
id: TASK-0142
title: 'block=N: support trailing remainder tiles'
status: Done
assignee:
  - '@mped'
created_date: '2026-05-18 04:24'
updated_date: '2026-05-18 22:07'
labels:
  - M3
  - compiler
  - language
  - followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0030 (block=N transformation) currently rejects any (HI-LO) not evenly divisible by N. The PRD §6.3.3 example shows 'for y_inner : y_outer..min(y_outer+N, H)' which clamps the trailing tile.

To support this, ACFGNode::Repeat needs to express a dynamic upper bound (function of an outer iter var) or we need a new variant (e.g. TileTail) that carries the remainder size. Codegen for the inner loop also needs to know which tile is partial.

For now, schedules with non-divisible block= are a hard compile error with BlockTransformError::NotDivisible. Unblock once a driving example needs it.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 ACFG can represent a trailing partial tile
- [x] #2 block=64 on 0..100 produces an outer loop of 2 tiles, an inner of 64 for tile 0, and an inner of 36 for tile 1
- [x] #3 all required (algo, sched) cells stay green
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. CARRIED FINDING: single-worker backend render_main_rs walks LinkedIR::algo source directly (NOT the block-transformed ACFG); block= only shapes the ACFG used for petri/boundedness/deadlock/invariants. For 05-stencil/blocked (single host, no Xfer) the emitted code == naive => bit-identical by construction. ONLY blocker = apply_block_transforms hard NotDivisible reject.
2. block_transform.rs: in rewrite_node, when rem = len % N != 0, emit Sequence[ full-tile nest (outer 0..num_full x inner 0..N, only if num_full>0), tail nest (outer tile_id 0..1 x inner iter_var 0..rem) ]. All static-range Repeat/Sequence => no ACFGNode::Repeat type change; petri/boundedness/deadlock unchanged by construction (total firings = num_full*N + rem = len).
3. Keep rem==0 path byte-identical (existing 07-matmul/blocked, hoist tests, determinism stay green).
4. apply_block_transforms: remove the NotDivisible validation rejection (keep the variant + Display for now; mark dead-ish). Keep UnknownLoopVar + empty-range passthrough.
5. Update unit tests in block_transform.rs (module) + integration tests/block_transform.rs: replace block_rejects_non_divisible_range with a remainder-structure assertion (AC#2: block=64 on 0..100 => 2 tile-loop iters, inner 64 then 36). Add stencil-shaped remainder test (len 14, N=4 => 3 full + tail 2).
6. Un-skip 05-stencil/blocked in e2e-matrix.toml; promote to [[required]].
7. GATE: nix develop -c just test / e2e (target 8 pass/0 fail/2 skip) / determinism-check (8/0) / determinism-check-negative (bites) / cargo clippy -D warnings. Commit per logical unit, no AI credit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
TASK-0142 implemented via LOW-RISK STATIC DECOMPOSITION (the steered path; no dynamic ACFGNode::Repeat bound needed).

Key carried-context finding CONFIRMED: the single-worker backend render_main_rs (pthreads-sync/src/lib.rs ~25-33,78-81) walks LinkedIR::algo SOURCE statements directly, NOT the block-transformed ACFG. So for 05-stencil/blocked (single host, no Xfer) the emitted Rust == naive and is byte-identical to the schedule-independent reference BY CONSTRUCTION. The ONLY blocker was apply_block_transforms hard-rejecting the non-divisible 1..15/block=4 range. The block transform here is structurally INVISIBLE in emitted code; its correctness for this cell is exercised by the petri/boundedness/deadlock passes consuming the rewritten ACFG + the unit/integration tests, not by a diff in generated Rust.

DESIGN: rewrite_node now emits, for rem = len%N != 0, Sequence[ full-tile nest (outer tile_id 0..num_full x inner iter_var 0..N, only if num_full>0), trailing partial tile (degenerate outer 0..1 x inner 0..rem) ] via a new tile_nest() helper. All nodes stay static-range Repeat/Sequence => ACFGNode::Repeat type UNCHANGED. acfg_to_petri/petri_to_events unroll by range.end-range.start; total firings = num_full*N + rem == len, identical to untiled loop => boundedness/deadlock/determinism correct by construction. rem==0 path is byte-identical to the old single-nest (verified: 07-matmul/blocked + hoist tests + determinism stay green). BlockTransformError::NotDivisible RETIRED (kept unconstructed, #[allow(dead_code)], for error-enum ABI stability + accurate message).

GOTCHAS for future work: (1) synthetic_one_loop test helper is pinned to example-01 0..256 and link() rejects loop vars absent from the algo, so the exact AC#2 64/36-on-0..100 numbers had to be a passes::block_transform UNIT test (rewrite_node is accessible there with an arbitrary range); the integration test asserts the 2-tile full+partial STRUCTURE via block=200 on 0..256. (2) The rewritten Sequence replaces the original Repeat IN PLACE inside the surrounding program tree, so tests must collect tile nests recursively (collect_repeats_with_var), not at acfg.root directly. (3) tile_nest deep-clones the body for the partial tile - source-bounded, not iteration-bounded; documented.

Gate (actual, run inside nix develop): just test = 0 failed (all suites); just e2e = 8 pass / 0 fail / 2 skip (05-stencil/blocked PASS, promoted to [[required]]); just determinism-check = 8/0 byte-identical (4 files/cell incl 05-stencil/blocked); just determinism-check-negative = correctly bites; just clippy = clean. Commit 8fd0ffc. NOTE: cargo clippy --tests flags pre-existing len_zero lints in tests/acfg_to_petri.rs (untouched, NOT in the just-clippy gate) - out of scope.

ORCHESTRATOR REVIEW GATE (phase3-ralph, on commit 8fd0ffc + hardening). qa-test-runner GO, mped-architect GO — both read-only, run by orchestrator (NOT implementer self-cert). Numbers RE-RUN by reviewers/orchestrator this cycle (not transcribed from implementer claims): just test 333 passed/0 failed/2 ignored; just e2e 8 pass/0 fail/2 skip (05-stencil/blocked PASS + required); just determinism-check 8/0 byte-identical; determinism-check-negative bites on 2 consecutive runs; clippy (workspace -D warnings) clean.

Architect findings hardened IN-THREAD by orchestrator: (1) stale #[ignore] on e2e_example_05::blocked_pthreads_sync_bit_identical removed — test now ACTIVE and PASSES (re-run: e2e_example_05 2 passed/0 ignored); (2) stale module doc + stale blocked.sched.nuc comment block (which still claimed the schedule would be REJECTED with NotDivisible) rewritten to reality; comment-only .nuc change verified inert (determinism still 8/0 byte-identical). Honest-scope caveat now stated in both the test doc and the schedule comment: 05-stencil is single-host so the backend emits from AlgoIR source, not the tiled ACFG — this cell pins no-reject/no-panic/bit-identical; tiling structure is pinned by block_transform unit/integration tests. Larger findings filed as TASK-0161 (index-reconstruction landmine, dep 0142+0159) and TASK-0162 (pre-existing len_zero test-lint debt + --all-targets gate).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Support trailing-remainder tiles for block=N (PRD §6.3.3).

What changed:
- apply_block_transforms no longer rejects a non-divisible (HI-LO) range. rewrite_node decomposes a non-divisible block into Sequence[ full-tile nest, one trailing partial tile ] using a new tile_nest() helper. Only static-range Repeat/Sequence nodes are used — ACFGNode::Repeat type is unchanged and no dynamic upper bound is introduced.
- BlockTransformError::NotDivisible retired (kept unconstructed for error-enum ABI stability; message + docs updated to say so).
- e2e-matrix.toml: 05-stencil/blocked un-skipped and promoted to a required cell.
- Tests: new passes::block_transform unit tests (rewrite_node_emits_trailing_partial_tile, rewrite_node_ac2_64_then_36 = AC#2 verbatim) and integration tests (block_non_divisible_emits_trailing_partial_tile, block_non_divisible_two_tiles_full_then_partial); the old block_rejects_non_divisible_range deleted; recursive collect_repeats_with_var helper added.

Why static decomposition: acfg_to_petri/petri_to_events unroll Repeat by range length and boundedness/deadlock consume that net; a dynamic bound would ripple into all of them + backend + determinism. Static decomposition keeps total firings = num_full*N + rem == len, so downstream passes are correct by construction. The rem==0 path is byte-identical to the prior shape (07-matmul/blocked + hoist + determinism unaffected).

User impact: schedules with block=N on non-divisible ranges now compile and run instead of failing with NotDivisible. 05-stencil/blocked computes bit-identically vs the schedule-independent reference.

Tests run (inside nix develop): just test = 0 failed; just e2e = 8 pass / 0 fail / 2 skip; just determinism-check = 8/0 byte-identical; just determinism-check-negative = bites correctly; just clippy = clean. Commit 8fd0ffc.

Limitations / follow-ups: (1) single-worker backend emits from source algo not the ACFG, so for 05-stencil/blocked the tiling is structurally invisible in generated code (correctness via petri/analysis passes + tests). A multi-worker non-divisible block= cell would additionally exercise per-tile transfer hoisting through the partial tile — not covered by an example yet. (2) cargo clippy --tests surfaces pre-existing len_zero lints in the untouched tests/acfg_to_petri.rs (not part of the just-clippy gate).
<!-- SECTION:FINAL_SUMMARY:END -->
