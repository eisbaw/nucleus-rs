---
id: TASK-0242
title: >-
  Audit: is the 03-reduction/distributed × pthreads-sync SKIP (TASK-0117 +
  TASK-0126) actually still genuine?
status: Done
assignee:
  - mped-architect-impl
created_date: '2026-05-22 08:54'
updated_date: '2026-05-22 09:19'
labels:
  - e2e
  - tech-debt
  - M3
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle 27 (TASK-0229) discovered a candidate stale SKIP: 03-reduction/distributed × pthreads-async PASSES and is bit-identical to reference.bin. pthreads-async's multi-worker emit is a near-verbatim copy of pthreads-sync's multi_worker.rs (TASK-0228 Wave B-2, cycle 26, commit 299e1b0) — same Plan::emit, same render_worker_events match arms, same Wait gather, same partition_worker_ranges override. So if the COPY passes, the original SHOULD pass too.\n\nThe pthreads-sync × 03-reduction/distributed SKIP at e2e-matrix.toml line 387-390 cites:\n    reason = 'TASK-0117 + TASK-0126: distributed placement + per-tile transfer codegen not yet implemented'\n\nBut TASK-0117 (replicate Push/Wait pairs across distributed worker entities) and TASK-0126 (ACFG-driven xfer placement) have both seen substantial work since the SKIP was filed. The cycle-26 multi-worker emit handles the same Push/Wait + tile gather codegen this schedule needs. So the SKIP reason is plausibly stale.\n\nThe sibling mp-tcp-bufsync × 03-reduction/distributed SKIP at line 416-419 cites TASK-0117 + TASK-0172 (non-uniform-barrier identity). TASK-0172 closed earlier; if TASK-0117 is also now non-blocking, that SKIP is also stale.\n\nAudit steps:\n1. Read pthreads-sync's distributed_placement_is_rejected test (the upstream rejection check the original schedule comment mentions). Is that still bites? If yes, the SKIP is still real and pthreads-async only passes because its multi-worker arm bypasses the check.\n2. Try removing the pthreads-sync SKIP for 03-reduction/distributed and running 'just e2e --example 03-reduction --schedule distributed --backend pthreads-sync'. Does it PASS or surface a real ContractGap?\n3. If it passes, REMOVE both stale SKIPs and PROMOTE both cells to [[required]]. The three-way differential becomes stronger.\n4. If it surfaces a real gap, document precisely what remains for TASK-0117/0126/0172 to genuinely close, and file follow-ups if any sub-scope has already landed but the SKIP wording is too broad.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle-30 audit findings (TASK-0242)

**Verdict: outcome (b)** — one SKIP was stale, the other had a stale citation but the cell genuinely fails (different root cause).

### pthreads-sync × 03-reduction/distributed: SKIP WAS STALE
- Removed `[[skip]]` (e2e-matrix.toml line 386-391), promoted to `[[required]]`.
- Result: **PASS** (323ms, bit-identical to `03-reduction/reference.bin` via e2e harness Phase::Diff).
- Hypothesis confirmed: pthreads-async's multi-worker emit being a near-verbatim copy of pthreads-sync's (TASK-0228 Wave B-2, commit 299e1b0) meant that once pthreads-async passed this cell in cycle 27, pthreads-sync should pass too. The SKIP citing TASK-0117 + TASK-0126 was filed before the cycle-26 refactor closed the upstream gaps and never re-checked.

### mp-tcp-bufsync × 03-reduction/distributed: CITATION STALE, CELL GENUINELY FAILS
- Removed `[[skip]]` (e2e-matrix.toml line 415-420), promoted to `[[required]]` temporarily for the audit.
- Result: **FAIL/compile** (76ms).
- Verbatim failure (from `EmitError::ContractGap` at `nucleus/backends/mp-tcp-bufsync/src/lib.rs:377-383`):
  ```
  pthreads-sync: EventList/sidecar contract gap: barrier #1 participants
  {WorkerId(1), WorkerId(2), WorkerId(3), WorkerId(4)} exclude the host worker;
  mp-tcp-bufsync's one-connection-per-(host,worker) topology requires host
  as the barrier hub. A host-excluding barrier needs a worker-to-worker
  mesh (filed as TASK-0175).
  ```
- The OLD SKIP reason cited TASK-0117 (distributed placement — now CLOSED, sibling pthreads-sync proves it) + TASK-0172 (non-uniform-barrier identity — also CLOSED). The ACTUAL current failure is a **transport limitation** (TASK-0175: host-excluding barrier requires worker-to-worker mesh). Identical root cause to `13-cnn-inference batch_parallel × mp-tcp-bufsync`.
- Restored `[[skip]]` with corrected reason naming TASK-0175 (the real blocker) and explicitly noting TASK-0117/0172 are NO LONGER the cause.

### e2e-matrix.toml edits

- Lines 378-390 (pthreads-sync block): swapped `[[skip]]` → `[[required]]`, added 11-line block comment describing the cycle-30 promotion and the reference.bin bit-identity.
- Lines 399-432 (mp-tcp-bufsync block): rewrote 16-line block comment to route the failure to TASK-0175 (transport, not front-pass); rewrote skip reason to cite TASK-0175 verbatim and explicitly note TASK-0117/0172 are NOT the cause; the sibling pthreads-sync PASS is named as the falsification of the prior citation.
- Lines 553-562 (pthreads-async block): updated cycle-27 comment to mark the audit closed and name the mp-tcp-bufsync transport-not-front-pass route.

### e2e tally (after promotion)

```
total: 54   pass: 47   fail: 0   skipped: 7   required-fail: 0
```
(Was 46 pass / 8 skipped pre-promotion. Net: +1 required cell PASSING bit-identical to reference.bin.)

### Falsifier-seam re-verification

- `just determinism-check-negative`: `NUC_NONDET_PERTURBED_CELLS=47`, exit OK ("determinism check correctly bit on injected nondeterminism").
- `just xbackend-check-negative`: `NUC_XBACKEND_CORRUPTED_APPLIED=14`, `NUC_XBACKEND_CORRUPTED_DETECTED=1`, exit OK ("cross-backend differential correctly bit on injected mp-tcp corruption").
- `just test`: all unit/integration suites pass.
- `just clippy`: clean (workspace `-D warnings`).

### Follow-up tasks
- None filed. TASK-0175 already exists ("mp-tcp-bufsync: worker-to-worker channel / host-excluding barrier support") and is the correct blocker; the SKIP now cites it precisely.

READY FOR REVIEW + COMMIT.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 30 (2026-05-22) — closed. Audit verdict: outcome (b) — one stale SKIP, one with stale citation but real underlying gap.

- pthreads-sync × 03-reduction/distributed: PASS bit-identical (323ms). Promoted from SKIP to [[required]] at M3. The prior SKIP citing TASK-0117 + TASK-0126 was stale; cycle 26's TASK-0228 Wave B-2 (commit 299e1b0) closed the underlying multi-worker codegen gap.
- mp-tcp-bufsync × 03-reduction/distributed: FAIL with verbatim ContractGap 'barrier participants exclude the host worker; mp-tcp-bufsync's one-connection-per-(host,worker) topology requires host as the barrier hub. A host-excluding barrier needs a worker-to-worker mesh (filed as TASK-0175).' SKIP retained; reason rewritten to cite TASK-0175 (real cause, transport-layer) and explicitly negate the stale TASK-0117 / TASK-0172 citations.

Tally: 54 / 47 / 0 / 7 (was 54 / 46 / 0 / 8); three-way differential coverage of 03-reduction/distributed now extends to two backends.

Falsifier-seam re-verification: NUC_NONDET_PERTURBED_CELLS=47, NUC_XBACKEND_CORRUPTED_DETECTED=1 (14 applied). All green.

Review-gate (parallel read-only): qa-test-runner GO (all 4 gate numbers re-derived bit-for-bit). mped-architect GO with one LOW (stale 'line ~660' hint at line 432). LOW fixed in-thread before commit: replaced numeric line ref with 'skip block below' (resilient to future line shifts).
<!-- SECTION:FINAL_SUMMARY:END -->
