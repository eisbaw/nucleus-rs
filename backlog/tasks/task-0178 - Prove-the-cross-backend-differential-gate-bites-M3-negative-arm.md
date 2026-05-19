---
id: TASK-0178
title: Prove the cross-backend differential gate bites (M3 negative arm)
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-19 01:05'
updated_date: '2026-05-19 02:55'
labels:
  - M3
  - validation
  - quality
dependencies:
  - TASK-0036
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0041 AC#5: the cross-backend e2e differential must be PROVEN to bite, analogous to determinism-check-negative (TASK-0145) and the required-coverage guard (TASK-0163). Deliberately perturb one mp-tcp-bufsync cell (e.g. flip a sign / off-by-one in the shared renderer reachable only via the mp-tcp path, or a transport encode bug) and assert just e2e / CI FAILS that cell with required-fail>0 — then revert; the durable guard is a test/recipe, not a committed broken backend (mirror the determinism-check-negative pattern: recipe SUCCEEDS iff the harness correctly FAILS). Without this, "differential green" is only the positive arm — a false-negative in the cross-backend falsifier would go unnoticed.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A standing negative check perturbs an mp-tcp cell and asserts the e2e/CI differential FAILS it (required-fail>0), then is non-destructive (env/flag or transient, like determinism-check-negative)
- [x] #2 Wired into just ci
- [x] #3 Reverting the perturbation returns the gate to green (proven)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add NUC_XBACKEND_NEGATIVE env gate (value-gated =="1", off by default) in mp-tcp-bufsync backend.
2. Injection point: the wire.rs emission at lib.rs:139 (mp-tcp-EXCLUSIVE — copied verbatim into generated multi-process projects; pthreads-sync never touches wire.rs). Deterministically corrupt the emitted enc_vec encoder (last byte +1 wrapping) so a multi-process mp-tcp cell (02-split-add/split) decodes wrong array data while pthreads-sync stays byte-identical to reference.bin. NOT the shared single-worker renderer (lib.rs:141-163 reuses pthreads-sync render_single_worker_main; perturbing it would break BOTH backends and the recipe could not distinguish a cross-backend diff from a global break).
3. Loud eprintln! banner when armed, mirroring TASK-0145 NUC_NONDET_TEST discipline.
4. New justfile recipe xbackend-check-negative mirroring determinism-check-negative: run e2e harness with NUC_XBACKEND_NEGATIVE=1; recipe exit 0 + OK iff harness exits non-zero (required-fail>0 on the mp-tcp cell); exit 1 + FAIL otherwise.
5. Wire into ci recipe alongside determinism-check-negative.
6. Verify gate: just test, bare just e2e (28/24/0/4/0 unchanged), xbackend-check-negative 3x non-flaky, determinism-check + -negative still bite, clippy clean, just ci green. Prove pthreads-sync 02-split/split passes while mp-tcp 02-split/split fails under the gate.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented as a deterministic source-rewrite env gate NUC_XBACKEND_NEGATIVE (value-gated =="1", off by default, loud eprintln! banner, anchor-guarded panic on wire_runtime drift) — mirrors the TASK-0145 NUC_NONDET_TEST discipline.

Injection point (mp-tcp-EXCLUSIVE, deliberate): maybe_corrupt_wire() rewrites the emitted wire.rs enc_vec body (last payload byte wrapping_add(1)) at lib.rs:139, the multi-process wire.rs emission. wire.rs is copied verbatim into generated multi-process mp-tcp projects ONLY; pthreads-sync emits no wire, and the single-process mp-tcp path (lib.rs:141-163) reuses pthreads-sync render_single_worker_main, so neither is touched. Did NOT perturb the shared single-worker renderer: that would break BOTH backends and the recipe could not distinguish a cross-backend diff biting from a global break.

Proof the cross-backend differential genuinely bites (gate on): 02-split-add/split/mp-tcp-bufsync = FAIL/diff (byte differs at offset 1023); 02-split-add/split/pthreads-sync = PASS; 02-split-add/naive/mp-tcp-bufsync = PASS (single-process, shared renderer). harness exit non-zero, required-fail=1, total 28/pass23/fail1/skip4.

Gate numbers: just test 0 failed; bare just e2e UNCHANGED 28/24/0/skip4/required-fail0 (mp-tcp split PASS gate-off); xbackend-check-negative OK 3/3 non-flaky (deterministic, no RNG/PID/clock); determinism-check byte-identical 24/0; determinism-check-negative still bites; clippy -D warnings clean; just ci exit 0 green end-to-end incl. the new arm.

Seam debt filed as TASK-0183 (parallel to TASK-0157) with code-comment pointer.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Prove the cross-backend e2e differential actually BITES (TASK-0178 / TASK-0041 AC#5 negative arm).

Adds NUC_XBACKEND_NEGATIVE, a runtime env gate (value-gated =="1", OFF by default, loud stderr banner, anchor-guarded) that DETERMINISTICALLY corrupts the mp-tcp-EXCLUSIVE wire encode. maybe_corrupt_wire() rewrites the emitted wire.rs enc_vec body (last array-payload byte +1 wrapping) only on the multi-process wire.rs emission path. A multi-process mp-tcp cell (02-split-add/split) then decodes wrong values and its output.bin diverges from the committed hand-written reference.bin oracle; pthreads-sync emits no wire and stays byte-identical. The asymmetry mp-tcp != reference while pthreads-sync == reference is precisely the cross-backend differential biting, NOT a global break.

Why not the shared renderer: emit() routes single-process cells through pthreads-sync render_single_worker_main (the one arithmetic renderer); wire.rs is reached only on the multi-process path. Perturbing the shared renderer would break BOTH backends and the recipe could not tell "differential caught a backend-specific bug" from "everything broken" — a weaker, wrong test.

New justfile recipe xbackend-check-negative (mirrors determinism-check-negative): runs the e2e matrix with the gate on; SUCCEEDS iff the harness FAILS (non-zero, required-fail>0). Wired into just ci alongside determinism-check-negative.

Deterministic by construction (fixed string rewrite, no RNG/PID/clock) so non-flaky AND it does not perturb --check-determinism.

Verification (all inside nix develop): just test 0 failed; bare just e2e UNCHANGED 28/pass24/fail0/skip4/required-fail0 (mp-tcp split PASS gate-off); xbackend-check-negative OK 3/3 non-flaky; under the gate 02-split/split/mp-tcp FAIL/diff @ offset 1023 while 02-split/split/pthreads-sync PASS (required-fail=1); determinism-check byte-identical 24/0; determinism-check-negative still bites; cargo clippy --workspace -D warnings clean; just ci exit 0 green end-to-end incl. the new arm.

Follow-up: TASK-0183 filed (parallel to TASK-0157) to relocate the inline test-scaffolding seam out of production codegen; code-comment pointer added. TASK-0041 AC#5 is now satisfiable (forward-carried; not self-checked here).
<!-- SECTION:FINAL_SUMMARY:END -->
