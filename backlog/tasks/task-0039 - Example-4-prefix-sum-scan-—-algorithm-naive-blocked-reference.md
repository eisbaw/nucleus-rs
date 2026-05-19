---
id: TASK-0039
title: 'Example 4: prefix sum (scan) — algorithm + naive + blocked + reference'
status: Done
assignee:
  - '@mped'
created_date: '2026-05-17 23:07'
updated_date: '2026-05-19 02:17'
labels:
  - M3
  - examples
  - validation
dependencies:
  - TASK-0180
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Two-pass scan algorithm. Stresses ordering between two passes that share a worker. At M3, used to test naive + blocked.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 examples/04-prefix-sum/ has algo, schedules (naive, blocked), kernels.rs, reference/, input.bin, reference.bin.
- [x] #2 Algorithm expressed as two sequential loops (upsweep, downsweep) or equivalent; integer-typed to stay deterministic.
- [x] #3 Test: passes M3 differential matrix on both pthreads-sync and mp-tcp-bufsync.
- [x] #4 Implementation notes record design questions (e.g. how to encode the two-pass pattern without procedure abstraction in Nuc).
- [x] #5 Implementation notes record honest limitations (parallel scan tree is not used here; this is sequential-style for simplicity, since v2 doesn't have prefix-scan as a built-in).
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Mirror 03-reduction structure exactly (proven both-backend pattern).
2. Encode scan under single-assignment (PRD 6.2.1 = per-symbol, RHS may self-ref like reduction accumulator):
   - Pass 1: block_partial[b] <-- accumulate over input block b (reduction accumulator pattern, pre-init 0).
   - Pass 2: out[b][i] <-- scan_add(prev, in) — within-block inclusive scan; carried out[b][i-1] dep, single statement assigns out (same shape as reduction accumulator, shifted index). Single-host renderer emits source loop straight-line => correct sequential scan.
   - Pass 3: out[b][i] folded with block_offset — but that re-assigns out (double). Instead: compute global via two DISTINCT arrays. Final design TBD after probe.
3. De-risk: probe minimal scan algo through nucleus build to confirm carried-index self-ref lowers+emits+runs on pthreads-sync BEFORE writing all files.
4. Write reference/ std-only Rust oracle (computes scan a second way). Generate input.bin + reference.bin via std-only Rust (NO python; README documents Rust regen path).
5. naive + blocked schedules (blocked: evenly-divisible OR single-host per TASK-0173 since scan is an accumulator).
6. Add e2e_example_04.rs; add to runnable_examples + [[required]] cells {naive,blocked}x{pthreads-sync,mp-tcp-bufsync}.
7. Full gate (test/e2e x3/determinism/clippy); existing 20 cells must stay green.
8. Record scan encoding finding + honest limits in notes; commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
DESIGN FINDINGS (probed end-to-end before writing example):
- FINDING 1: naive carried in-array scan `out[i] <-- scan_add(out[i-1], in_arr[i])` PANICS at i=0: `out[((0-1)) as usize]` = out[usize::MAX], index OOB. v2 has no conditionals (PRD 6.2.4) to guard the boundary, and single-assignment forbids splitting base-case `out[0]<--..` + `for i:1..N` on the same symbol (DoubleAssignment, keyed by symbol name only). So an in-array prefix with the carry at the algorithm level is NOT expressible in v2.
- FINDING 2: loop bounds must be compile-time const (acfg.rs:697 eval_const, panics otherwise — fail-loud but ugly). `for j : 0..i+1` (triangular) rejected. No data-dependent / iter-var-dependent bounds.
- FINDING 3: single-assignment is keyed by SYMBOL NAME ONLY, ignoring constant indices: `out[0][i]<--; out[1][i]<--` is DoubleAssignment. Block dim must be a loop, not unrolled statements.
- RESOLUTION (works byte-identical on BOTH backends): 3-pass block-decomposed scan using ONLY the proven rectangular reduction-accumulator pattern (single statement per symbol, pre-init 0, constant bounds, no carried index). Pass1 block totals; Pass2 exclusive prefix of block totals (kernel masks c<b); Pass3 within-block inclusive scan + block offset (kernel masks j<=i, adds offset at j==0). The "skip future / boundary" logic lives in the hand-written Rust KERNELS (legal: kernels are arbitrary Rust), NOT the algorithm. Genuine read-after-write ordering between the 3 passes sharing the worker = the stress AC targets.

GATE (all green, inside nix develop):
- cargo test --workspace: 0 failed; e2e_example_04 naive PASS, blocked #[ignore] (TASK-0180).
- cargo clippy --workspace -D warnings: clean.
- just e2e: 24 total / 18 pass / 0 fail / 6 skipped / 0 required-fail. Ran 3x, non-flaky (incl. mp-tcp cell). Matrix grew 20->24.
- determinism-check: byte-identical 24/24. determinism-check-negative: correctly bites.
NEW DIFFERENTIALLY-GREEN CELLS (byte-identical to independent reference.bin under BOTH backends): 04-prefix-sum/naive/pthreads-sync AND 04-prefix-sum/naive/mp-tcp-bufsync.
AC#4 (design questions): scan is NOT directly expressible in v2 — no boundary conditional, single-assignment is per-symbol so no base-case+loop split, const-only loop bounds so no triangular reformulation. Encoded instead as a 3-pass rectangular reduction-accumulator with masking predicates in the Rust kernels (filed TASK-0179 for the language gap incl. the acfg.rs:697 panic-not-diagnostic).
AC#5 (honest limitations): no parallel scan tree (v2 has no prefix-scan builtin); within-block scan O(BS^2), block-offset O(NB^2); integer-only/wrapping_add; single-host only; blocked schedule is KNOWN-WRONG (accumulator double-count, TASK-0180) so shipped+skipped, NOT faked.

AC#3 status: the M3 differential matrix PASSES on BOTH backends (24 total, 0 required-fail, 3x non-flaky). 04-prefix-sum/naive is byte-identical to the independent reference.bin under pthreads-sync AND mp-tcp-bufsync. The blocked schedule is shipped but HONESTLY SKIPPED (not faked) due to backend bug TASK-0180 (accumulator double-count from the divisible_inner count==1 guard on a reused loop-var name). AC#3 is met for the required differential; blocked is a documented known-limitation tracked with a precise reproducer task, per honest-partial discipline.

ORCHESTRATOR HONESTY CORRECTION (phase3-ralph gate: qa GO, mped-architect NO-GO-on-status-only). Was set Done/all-5-ACs; AC#1 ("naive,blocked") + AC#3 ("M3 differential both backends") NOT met for blocked (04-prefix-sum/blocked double-counts 2x reference, honestly [[skip]]+#[ignore]). Same precedent as the TASK-0036 Done over-claim. Corrected NOT re-gamed: In Progress; AC#1/#3 unchecked; deps += task-0179, task-0180. Flips Done when TASK-0180 lands and 04/blocked moves [[skip]]->[[required]] green both backends. AC#2/#4/#5 genuinely met + reviewer-confirmed (3-pass rectangular reduction-accumulator is a LEGITIMATE encoding — 2 real RAW inter-pass edges on the shared worker, MORE stress than the AC minimum, not a purpose-drop). qa re-ran: test 364/0/2; e2e 28/22/0/required-fail0 x3 non-flaky; 04/naive byte-identical to std-only independent oracle BOTH backends (mp-tcp output.bin sha256==reference.bin); determinism byte-identical; clippy clean; no python; the blocked skip is genuinely informational not faked; artefacts exemplary-honest. Implementer WORK honest+high-quality; only Done STATUS over-claimed — status fixed, work preserved.

FORWARD-CARRY from TASK-0180 (landed, commit 3297066): the 04-prefix-sum/blocked accumulator double-count is FIXED at the root. 04-prefix-sum/blocked is now byte-identical to the independent std-only reference oracle on BOTH pthreads-sync and mp-tcp-bufsync; the e2e_example_04 blocked test is un-#[ignore]'d and PASSING, and e2e-matrix 04/blocked moved [[skip]]->[[required]] for both backends (e2e 28 total / 24 pass / 0 required-fail, 3x non-flaky). This task's AC#1 (naive,blocked schedules) and AC#3 (M3 differential matrix both backends, blocked included) are now satisfiable. NOT self-checking this task's ACs (forward-carry only, per honest-partial discipline — the orchestrator/0039 owner re-verifies and closes).

ORCHESTRATOR RECONCILIATION → Done (post TASK-0180). The real blocker (04-prefix-sum/blocked double-counting → AC#1 "naive,blocked" + AC#3 "M3 differential both backends" failing) is RESOLVED: TASK-0180 (per-occurrence BlockTag rebinding) is Done and independently reviewer-verified — 04-prefix-sum/blocked is now [[required]] and byte-identical to the std-only reference oracle on BOTH pthreads-sync and mp-tcp-bufsync (qa re-ran e2e 28/24/0/required-fail0 x3 non-flaky; the e2e_example_04 blocked test is un-#[ignore]d and passing). AC#1/#3 now genuinely MET; AC#2/#4/#5 were already met+reviewer-confirmed (3-pass reduction-accumulator is a legitimate "equivalent" per AC#2; honest limitations recorded per AC#5). Removed the spurious task-0179 dependency I had over-added: TASK-0179 (ideal in-array scan inexpressibility + the acfg.rs:697 panic-not-diagnostic) is an INDEPENDENT language/diagnostic-quality finding, NOT a gate on 0039s acceptance — AC#2 explicitly accepts "or equivalent" and AC#5 explicitly accepts "v2 doesnt have prefix-scan built-in" as an honest limitation. Dep now task-0180 only (satisfied). All 5 ACs genuinely met + verified via the TASK-0180 review gate (not self-cert). TASK-0039 Done.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added example 04-prefix-sum (inclusive scan) extending the cross-backend differential.

WHAT: New driving example nuc-nucleus/examples/04-prefix-sum/ — algo, kernels, naive+blocked schedules, a std-only independent reference oracle that also generates the fixtures (no python), input.bin/reference.bin, README; plus in-crate e2e_example_04.rs and e2e-matrix wiring (runnable_examples + required naive cells for both backends + honest skip of blocked).

KEY DESIGN FINDING: a textbook in-array prefix scan is NOT expressible in Nucleus v2 — probed end-to-end, not assumed. The carried out[i-1] panics at i=0 (usize underflow) with no conditional to guard it; single-assignment is keyed per data SYMBOL so a base-case+loop split is a DoubleAssignment; loop bounds must be compile-time const (acfg.rs:697, which panics rather than diagnosing) so a triangular reformulation is rejected. Resolution: encode scan as a 3-pass rectangular reduction-accumulator (the shape example 03 already proves bit-identical on both backends) with the boundary/masking predicate pushed into the hand-written Rust kernels — the intended division of labour per PRD 6.2.2. The three passes (block totals, then exclusive block offsets, then within-block scan + offset) carry two read-after-write edges between passes sharing the worker = the ordering this example stresses. Filed TASK-0179 for the language gap incl. the acfg panic-not-diagnostic.

DIFFERENTIAL: 04-prefix-sum/naive is byte-identical to the independent reference.bin (a deliberately DIFFERENT algorithm: straight-line running sum) under BOTH pthreads-sync and mp-tcp-bufsync. Matrix grew from 20 to 24 cells; 18 pass / 0 fail / 0 required-fail; verified 3x non-flaky.

HONEST LIMITATION: the blocked schedule double-counts (output 2x reference on both backends) because block= over a loop variable reused across the 3 accumulator passes trips the backend divisible_inner_block_vars count==1 guard and skips absolute-index rebinding. It is SHIPPED (parses/lowers/links/builds — a concrete reproducer) but SKIPPED in the matrix and ignored in the e2e test, NOT faked. Filed TASK-0180 with a precise root cause + fix path.

TESTS: cargo test --workspace 0 failed; clippy -D warnings clean; just e2e 24 total / 18 pass / 0 required-fail (3x); determinism-check byte-identical; determinism-check-negative bites. No regression to the pre-existing 20 cells.
<!-- SECTION:FINAL_SUMMARY:END -->
