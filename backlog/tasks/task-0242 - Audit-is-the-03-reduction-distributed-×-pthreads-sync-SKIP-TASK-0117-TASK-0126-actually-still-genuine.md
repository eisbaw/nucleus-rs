---
id: TASK-0242
title: >-
  Audit: is the 03-reduction/distributed × pthreads-sync SKIP (TASK-0117 +
  TASK-0126) actually still genuine?
status: To Do
assignee: []
created_date: '2026-05-22 08:54'
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
