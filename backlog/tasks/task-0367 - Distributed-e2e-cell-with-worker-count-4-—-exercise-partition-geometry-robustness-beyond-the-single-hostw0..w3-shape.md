---
id: TASK-0367
title: >-
  Distributed e2e cell with worker count != 4 — exercise partition-geometry
  robustness beyond the single {host,w0..w3} shape
status: Done
assignee:
  - '@mark'
created_date: '2026-05-30 11:08'
updated_date: '2026-05-30 11:34'
labels:
  - compiler
  - e2e
  - partition
  - M6
  - coverage
  - cycle-213-followup
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-213 strategic-analysis finding (F1, highest-leverage non-grammar functionality gap). VERIFIED: every multi-worker schedule in the tree uses the identical workers={host,w0,w1,w2,w3} shape (12 cells) — partition geometry never varies in worker COUNT. Note 16-jacobi/distributed ALREADY exercises an UNEVEN 4-worker partition (interior y 1..7 = 6 rows / 4 workers => floor-with-spillover bands 2,2,1,1, TASK-0262), so the residual/last-band path IS exercised at N=4. The genuinely-untested dimension is WORKER-COUNT VARIATION: there is no 2-, 3-, or 8-worker partition anywhere, so the partition decompose + transfer_inject + halo machinery is proven for exactly one N. Add a distributed schedule with N!=4 over a size that leaves a non-divisible remainder, on an example whose distributed cell is [[required]] on all 7 tier-1 backends (candidates: 03-reduction partition=workers, 07-matmul partition=workers, 08-histogram partition=rows — pick the cheapest to add a sibling schedule to). This probes whether the floor-with-spillover residual policy and the per-tile transfer/gather codegen generalise across worker counts, currently structurally untestable.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A new distributed schedule (sibling of an existing all-7-required distributed cell) declares a worker count != 4 (e.g. 3 or 8 compute workers) over a data dimension that does NOT divide evenly by that count, so the floor-with-spillover residual path runs under a NEW geometry
- [x] #2 The new cell is bit-identical to the example reference.bin AND to its own naive cell, on every backend where it is promoted [[required]]
- [x] #3 Promote to [[required]] on backends that pass; any backend that does not pass gets an honest [[skip]] with a precise reason (no silent omission)
- [x] #4 Edge case: if the chosen N exceeds the partitioned dim (some workers get an empty band), the codegen MUST either handle it correctly (bit-identical) OR fail-loud with a typed EmitError — never silently miscompile. Document the chosen policy in the schedule header + a regression test
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Implementation plan (cycle 214, implementer):

Target = 07-matmul (orchestrator-confirmed clean; reference.bin worker-count-invariant per its distributed.sched.nuc header). N=16 rows on outer i.

Pre-flight code audit (DONE, no compiler change expected):
- compute_partition_bands (passes/common.rs) is already worker-count-agnostic: floor-with-spillover, N entries, asserts cover-exactly. 16/3 => floor 5, extras 16%3=1 => bands 6,5,5. 16/8 => 2 each.
- partition_workers.rs maps PartitionBandError -> PartitionError (typed, fail-loud) via map_band_error; driver main.rs:388 maps that to a compile-error string via ?. No panic on L<N.
- No hardcoded 4 / power-of-2 / even-division assumption in multi_worker_walker, transfer_inject, or host_election.
- Parser does NOT support w0..w3 range shorthand (parser.rs:47) => must enumerate workers explicitly.

Steps:
1. distributed3.sched.nuc: workers={host,w0,w1,w2}; place madd on {w0,w1,w2}; loop i:partition=workers; transfer a/b/c:sync. Bands 6,5,5 (uneven AND N!=4). [AC#1 primary]
2. distributed8.sched.nuc: workers={host,w0..w7}; place madd on {w0..w7}. Bands 2 each (even, N>4 — proves >4-worker codegen).
3. Fast inner loop: nucleus build distributed3 x pthreads-sync into a /tmp out dir, cargo build --release, run vs input.bin, cmp output.bin reference.bin (NEVER trust exit code). Inspect emitted main.rs for 6/5/5 bands + correct gather slice-pastes.
4. AC#4 empty-band probe: a 17+-worker set over the 16-row i dim => L<N => expect PartitionError::InsufficientWork surfaced as a fail-loud compile error (NOT panic, NOT silent). Verify empirically by attempting a build. If fail-loud: document policy in schedule header + add/confirm a negative regression test (bands_insufficient_work_rejects already exists in common.rs; add an end-to-end driver-level negative test if absent). Do NOT ship an empty-band [[required]] cell.
5. Promote distributed3 (+distributed8 if green) to [[required]] on each bit-identical backend; honest [[skip]] for any that legitimately cannot run. Mirror the existing 07-matmul/distributed TOML block.
6. Full gate before every commit: nix develop --command bash -c "just build && just clippy && just test && just test-release && just e2e". Baseline e2e 308/251/0/57/0; only allowed delta is +N pass/total for promoted cells.

Verify-by-cmp, never exit code. Report actual numbers.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle-214 result (DONE). All 4 ACs met + verified by cmp; full gate green.

WHAT WAS BUILT (commit e40331a):
- 07-matmul/distributed3.sched.nuc: workers={host,w0,w1,w2}; 16 rows / 3 => floor-with-spillover bands 6,5,5 (uneven AND N!=4). Emitted main.rs verified: compute loops i in 0..6 / 6..11 / 11..16; a recv slices [0..96]/[96..176]/[176..256] (stride 16); host gathers c into the same flat offsets; b received whole-array (TASK-0301 filter). [AC#1 primary]
- 07-matmul/distributed8.sched.nuc: workers={host,w0..w7}; 16/8 => 2 rows each (even, N>4).
- 14 cells ([[required]] M6, 2 scheds x 7 backends) — cmp BIT-IDENTICAL to reference.bin on ALL 7 tier-1 backends (pthreads-sync, mp-tcp-bufsync, pthreads-async, mp-tcp-event, openmp-rs, mp-tcp-poll, mp-uds-event). [AC#2, AC#3]

GOTCHAS / SUBTLETIES for next cold session:
1. The partition decompose IS genuinely worker-count-agnostic — compute_partition_bands (passes/common.rs) had ZERO 4/power-of-2/even assumptions; no compiler change was needed. Confirmed by reading multi_worker_walker, transfer_inject, host_election too.
2. AC#4 EMPTY-BAND: the policy is REJECT-BEFORE-EMPTY-BAND, not emit-a-0-width-worker. compute_partition_bands rejects L<N with PartitionBandError::InsufficientWork; partition_workers map_band_error -> PartitionError::InsufficientWork; driver main.rs:388 maps via ? to `partition-workers error: ...` exit 1. Empirically probed with a 17-worker sched over the 16-row i dim: fail-loud, exit 1, NO panic. So there is no empty-band codegen path to test — the geometry that would create one is rejected upfront. This is the correct no-silent-miscompile outcome. Pinned by 2 regression tests.
3. Parser does NOT support the w0..w3 range shorthand (sched/parser.rs:47) — workers must be enumerated explicitly; distributed8 lists w0..w7 by hand.
4. The matmul reference is worker-count-invariant (per distributed.sched.nuc header): same sum over k regardless of which/how-many workers compute row i => NO new reference.bin needed.
5. e2e: schedule files are auto-discovered, so adding distributed3/8 added them as informational cells on all 7 backends FIRST; promoting to [[required]] flips them to gating. Baseline 308/251/0/57/0 -> 322/265/0/57/0 (+14 total, +14 pass; the +14 total is the new informational cells now counted, all promoted).

GATE NUMBERS (cycle 214): build OK; clippy CLEAN (-D warnings, independently re-grepped); just test all green (0 failed); just test-release all green; e2e 322/265/0/57/0 (fail 0, required-fail 0). 5 new partition unit tests pass dev+release.

LIMITS / honest caveats: (a) only 07-matmul exercised — 03-reduction/08-histogram still N=4-only, but the shared compute_partition_bands is now proven for N in {3,8} + the existing {4} so the decompose generalises; (b) the new cells over-transfer b (whole-array broadcast) same as the 4-worker cell — that is bit-correct, not a regression; (c) empty-band is a REJECT not a runnable cell, so AC#4's 'bit-identical OR fail-loud' is satisfied via the fail-loud branch.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 214 — DONE, all 4 ACs met + verified by cmp; full gate green (commit e40331a).

Added worker-COUNT robustness coverage for 07-matmul (the gap: every prior multi-worker schedule used the identical {host,w0..w3} N=4 shape):
- distributed3.sched.nuc: 16 rows / 3 workers => floor-with-spillover bands 6,5,5 (uneven AND N!=4). [AC#1]
- distributed8.sched.nuc: 16 rows / 8 workers => 2 each (even, N>4 — proves >4-worker emit + N-way gather).

Both BIT-IDENTICAL to 07-matmul/reference.bin on ALL 7 tier-1 backends (verified by cmp per backend, not exit code; emitted bands inspected). [AC#2] Promoted all 2x7=14 cells to [[required]] (M6); no backend needed an honest skip. [AC#3]

NO compiler change was needed: compute_partition_bands (passes/common.rs) is worker-count-agnostic by construction (no 4 / power-of-2 / even-division assumption anywhere in the decompose / transfer_inject / multi_worker_walker / host_election path).

AC#4 EMPTY-BAND policy: reject-before-empty-band. A worker count exceeding the partitioned dim (probed: 17 workers over 16 rows, L<N) is REJECTED fail-loud — PartitionError::InsufficientWork, driver exit 1, actionable message, NO panic, NO silent miscompile. There is no runnable empty-band cell because the geometry that would create one is rejected upfront. Pinned by 5 regression tests (3 geometry pins + 1 helper-level empty-band reject + 1 pass-level fail-loud-not-panic with Display-message assertions).

Gate: build OK; clippy CLEAN; just test green; just test-release green; e2e 308/251/0/57/0 -> 322/265/0/57/0 (+14 pass, fail 0, required-fail 0).
<!-- SECTION:FINAL_SUMMARY:END -->
