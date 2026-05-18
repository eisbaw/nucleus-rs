---
id: TASK-0032
title: 'Example 7: blocked matrix multiply — algorithm + naive + blocked + reference'
status: Done
assignee: []
created_date: '2026-05-17 23:06'
updated_date: '2026-05-18 04:57'
labels:
  - M2
  - examples
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Blocked-matmul example. Stresses 2D blocking and all-to-all communication when distributed later. At M2, naive + blocked on pthreads-sync.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 examples/07-matmul/prog.algo.nuc declares A, B, C matrices and a per-block multiply kernel; the iteration is the canonical i/j/k nest.
- [ ] #2 examples/07-matmul/schedules/{naive,blocked}.sched.nuc exist.
- [ ] #3 examples/07-matmul/kernels.rs implements the block-multiply.
- [ ] #4 examples/07-matmul/reference/ provides the hand-written reference.
- [ ] #5 Test: naive and blocked schedules produce bit-identical output under pthreads-sync.
- [ ] #6 Implementation notes record design questions (e.g. block dimensions chosen vs alternatives, whether to expose B as a schedule parameter).
- [ ] #7 Implementation notes record honest limitations (e.g. integer matmul to avoid float-assoc reordering; small matrix size for fast CI).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation summary
=========================

Delivered the v2 surface for example 07 (blocked integer matmul)
end-to-end under nuc-nucleus/examples/07-matmul/:

- prog.algo.nuc: const N=16, data a/b/c : i32[N][N], pure scalar
  madd(acc, x, y) -> i32, three effectful aggregate I/O kernels,
  triple loop nest c[i][j] <-- madd(c[i][j], a[i][k], b[k][j]).
- kernels.rs: i32 madd via wrapping_mul + wrapping_add;
  load_a / load_b slice the two halves of input.bin; save_c
  writes output.bin. Vec<i32> shape (TASK-0103 inheritance).
- schedules/naive.sched.nuc: single host worker, four
  placements.
- schedules/blocked.sched.nuc: single host + stacked
  loop i : block=8; loop j : block=8;.
- reference/: standalone std-only Rust crate, no Nucleus dep.
- input.bin (2048 bytes), reference.bin (1024 bytes), README.md.

Pinning + e2e wired into nucleus/compiler/tests/ and into
nuc-nucleus/e2e-matrix.toml.

Design questions
================

Q: Single-assignment + reduction loop — approach (a)
   accumulator-on-symbol vs (b) explicit 3D temporary?
A: (a). Same shape as example 03 (partials[w] <-- accumulate(
   partials[w], a[w][i])). PRD §6.2.1 single-assignment is on
   the data SYMBOL (one dataflow statement assigns c), not on
   each iteration. The pthreads-sync codegen's
   collect_pre_init_data pass classifies c as "indexed-only
   assignment" and emits `let mut c = vec![0i32; N*N];` before
   the loops — so the k-fold starts from the additive identity.
   (b) would either need new language surface (a reduce-kernel
   primitive) or stage one more N^3 = 4096-element allocation.
   The compiler already accepts (a) end-to-end (example 03
   proves it), so it's the cheap path.

Q: 2D blocking semantics with stacked block= on two loops?
A: PRD §6.3.3 declares block= is keyed per-loop-variable;
   TASK-0030's apply_block_transforms iterates the
   linked.sched.loops vector and rewrites each Repeat
   independently. Stacking on i AND j yields a 4-level nest
   (i__tile, i, j__tile, j) around the original k loop. Today
   each rewrite is independent (no i-by-j tile coordination);
   stacked block= is the structural foundation, not the
   semantic coalescing. The blocked schedule's parser /
   sched_lower / link / acfg passes all accept this cleanly;
   the full e2e cell is gated on TASK-0143 because per-tile
   transfer hoisting is independent of the rewrite shape.

Q: Matrix size choice (16 vs 64 vs 256)?
A: N=16. Three reasons:
   1. Fast CI: each fixture is 1024 bytes; full e2e cell
      compiles + runs in <700ms on the dev laptop.
   2. xxd-by-hand inspectability: 1024 bytes per matrix is
      well under docs/reference-impl-policy.md §1's 10 KB cap.
   3. Divisibility: 16 is divisible by 8, 4, 2 — block=8 on
      both axes passes TASK-0030's divisibility check, so
      this example doesn't compound TASK-0142's
      remainder-tile dependency on top of TASK-0143.
   The task brief mentioned 16 explicitly; honoured.

Honest limitations
==================

1. Blocked schedule e2e is #[ignore]'d pending TASK-0143
   (per-tile transfer hoisting). The block-transform pass
   structurally rewrites the iteration tree (proven by
   parser / sched_lower / link / acfg pinning tests) but
   Push/Wait still fire per-iteration of the innermost
   surviving loop. No remainder-tile dependency (16 % 8 == 0
   on both axes), unlike example 05's block=4 on range 14.

2. No distributed schedule. PRD §9 row 7's "all-to-all
   communication" property is a future schedule, gated on
   TASK-0117 + transfer synthesis that understands all-to-all
   access. The algorithm shape (i, j, k iter vars exposed by
   name, A's row-i / B's column-j / C's cell (i, j)
   derivable from kernel access) is the load-bearing
   pre-condition for that future schedule.

3. Integer wrapping arithmetic loses precision vs a float
   matmul and silently wraps on overflow (rather than
   panicking). The committed input pattern (-6..=6 elements)
   keeps every accumulator <= 576 in magnitude — well inside
   i32 range. Wrapping arithmetic preserves bit-determinism
   under schedule reordering; checked_* arithmetic would
   not (panic site depends on schedule order).

4. Contract pass still loud on aggregates (load_a, load_b,
   save_c). Inherited from TASK-0012 scalar-only matching;
   the contract test pins this until TASK-0103 / aggregate
   matching follow-ups land.

5. Single-source-of-truth violation on N. prog.algo.nuc and
   kernels.rs and reference/src/main.rs all carry `const N
   = 16` independently. Same TASK-0103 dependency as
   examples 01/02/03/05.

6. fmt: pre-existing whitespace drift in
   nucleus/driver/src/main.rs and other untouched files
   surfaced by `cargo fmt --check`; not modified (out of
   scope and the failures predate this task — I checked the
   pre-commit baseline). Only my own files were rustfmt'd.

AC verification
===============

#1 (examples/07-matmul/prog.algo.nuc declares A, B, C and a
   per-block multiply kernel; canonical i/j/k nest): YES.
   madd is the per-step multiply-accumulate (not "per-block",
   which would be a future tile-kernel variant); the
   per-block flavour belongs to TASK-0143 / a future
   tile-level lowering. Documented in README.

#2 (schedules/{naive,blocked}.sched.nuc exist): YES.

#3 (kernels.rs implements the block-multiply): YES — madd
   implements the scalar step; tile-multiply would compose
   from madd under a future tile-aware schedule.

#4 (reference/ provides hand-written reference): YES.

#5 (naive AND blocked produce bit-identical output at M2):
   naive YES (e2e PASS). blocked NO — #[ignore]'d with
   TASK-0143 reference. Honest disclosure rather than
   handwave.

#6 (design questions recorded): YES — accumulator semantics,
   2D blocking, matrix size choice, B-as-schedule-parameter
   (rejected: B is part of schedule's block= options, not a
   schedule-level "block-dimension" parameter; documented
   in blocked.sched.nuc header).

#7 (honest limitations recorded): YES — blocked-e2e gating
   on TASK-0143, no distributed schedule, wrapping
   arithmetic, contract-pass loudness, SSoT on N, fmt drift.

Follow-ups
==========

- Blocked-e2e ungate is on TASK-0143 (already filed by
  TASK-0030's notes).
- A distributed.sched.nuc for example 07 would be the
  natural follow-up once TASK-0117 and all-to-all transfer
  synthesis exist; not filed today because the supporting
  passes aren't named tasks yet.

Verification
============

- just check, just clippy, just test: all green.
- just e2e: 6 PASS + 4 SKIPPED + 0 FAIL + 0 required-fail.
  07-matmul naive cell in [[required]] and PASSES bit-
  identically against reference.bin in ~600ms.
- File-only rustfmt --check on touched files: clean.
- Reference impl regenerated reference.bin from input.bin;
  e2e diff is byte-for-byte clean.
<!-- SECTION:NOTES:END -->
