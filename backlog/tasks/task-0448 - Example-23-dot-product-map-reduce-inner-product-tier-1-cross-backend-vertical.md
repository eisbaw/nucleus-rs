---
id: TASK-0448
title: >-
  Example 23-dot-product: map-reduce (inner product) tier-1 cross-backend
  vertical
status: In Progress
assignee:
  - '@claude'
created_date: '2026-06-05 02:32'
updated_date: '2026-06-05 02:44'
labels: []
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Standalone dot-product (elementwise multiply two vectors then tree sum-reduce to scalar) — the canonical map-reduce, uncovered by the existing 22 examples. Composition of ex01 map + ex03 reduction shapes; tier-1, i32-deterministic, bit-identical cross-backend.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation plan (cycle 2026-06-05):
- New tier-1 example 23-dot-product = map-reduce / inner product. NO new compiler work; compose ex01 two-input map + ex03 two-phase tree reduction.
- prog.algo.nuc: const N=256, NUM_WORKERS=4, PARTITION_SIZE=64. data a,b,prod : i32[NUM_WORKERS][PARTITION_SIZE]; partials : i32[NUM_WORKERS]; half1,half2,result : i32. Phase 0 MAP prod[w][i]<--mul(a[w][i],b[w][i]); Phase 1 accumulate(partials,prod); Phase 2 tree combine. Explicit prod edge because v2 disallows nested kernel calls in arg expr.
- kernels.rs: mul=wrapping_mul, accumulate/combine=wrapping_add; load_input reads input[0..N], load_input_b reads input[N..2N], save_output writes 4 LE bytes.
- schedules/naive.sched.nuc: single host worker.
- reference/: standalone std-only, --gen-input (a[k]=(k%7)-3, b[k]=(k%5)-2) + --in/--out. result = sum over 256 of a[k]*b[k] wrapping.
- input.bin (2048B) + reference.bin (4B) committed.
- README.md cloned from 03.
- Wiring: e2e-matrix runnable_examples += 23-dot-product; 7 [[required]] naive x tier-1 M6 (mirror 17-spmv). Root README 22->23 prose + sentinel.
- Gate: just build/clippy/test/test-release/check-readme-counts/e2e. Baseline 427/364/0/63/0 -> expect +7 total+pass.
- KEY RISK to verify: 2D double-input map prod[w][i]<--mul(a[w][i],b[w][i]) lowering across all 7 backends.

GATE RESULT (cycle 2026-06-05, all green):
- just build: clean. just clippy (workspace + standalone reference crate): clean. just test: ok 0fail. just test-release: ok 0fail. just check-readme-counts: OK prose=23 sentinel=23 dirs=23.
- just e2e: BEFORE 427/364/0/63/0 -> AFTER 434/371/0/63/0 (total +7, pass +7, fail 0, skipped 63 unchanged, required-fail 0). All 7 tier-1 backends 23-dot-product/naive PASS (bit-identical vs reference.bin), each a required (*) cell.

GOTCHAS / SUBTLETIES:
- 2D double-input map LOWERED AS DESIGNED (no fallback). Generated pthreads-sync main.rs: prod[((w)*64+(i)) as usize] = kernels::mul(a[((w)*64+(i)) as usize], b[((w)*64+(i)) as usize]); then identical 03-reduction accumulate/combine shape. The two 2D inputs in one kernel call composed cleanly across all 7 backends; no EmitError.
- Fixture: a[k]=(k%7)-3 in [-3,3], b[k]=(k%5)-2 in [-2,2]; product in [-6,6]; |dot| bounded 1536 << i32 max so wrapping_* never wraps on this fixture. result = -1 (0xffffffff), independently cross-checked in Python and matched by pthreads-sync emit. moduli 7/5 keep a,b out of phase (non-trivial dot, not accidental zero).
- input.bin 2048B (a in [0..N), b in [N..2N)); reference.bin 4B. Reference uses a FLAT left-to-right fold (independent control structure) vs the Nucleus partition+tree reduction; equal because wrapping_add is assoc+commut over i32.
- Example dir count was already 22 (22-dma-pio-demo is the embedded/Renode example, NOT in e2e runnable_examples); 23-dot-product makes 23. README 22->23 prose+sentinel.
<!-- SECTION:NOTES:END -->
