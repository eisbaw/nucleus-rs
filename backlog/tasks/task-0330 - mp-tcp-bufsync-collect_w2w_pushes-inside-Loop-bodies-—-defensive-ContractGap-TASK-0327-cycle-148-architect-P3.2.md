---
id: TASK-0330
title: >-
  mp-tcp-bufsync collect_w2w_pushes inside Loop bodies — defensive ContractGap
  (TASK-0327 cycle-148 architect P3.2)
status: To Do
assignee: []
created_date: '2026-05-25 17:40'
updated_date: '2026-05-25 19:20'
labels:
  - M6
  - backend
  - mp-tcp-bufsync
  - panic-not-diagnostic
  - forward-carried-from-TASK-0327
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0327 cycle 148's collect_w2w_pushes helper at nucleus/backends/mp-tcp-bufsync/src/lib.rs (~line 1586-1598) recurses into Event::Loop bodies to find w2w Push events for the synthetic relay schedule. The host-relay phase emit is FLAT (one block of read+write hops outside any loop), so any w2w Push nested inside a Loop would either:

- Over-count: the relay phase emits one read for the SeqTag, but the loop body emits N pushes for it → 1 read can't drain N pushes → seq mismatch on subsequent reads → fail loud (good).
- Mis-order: the relay phase reads in a flat order, but the loop body pushes seqs in nested iteration order → mismatch fires at the first nested iteration.

No in-tree schedule today nests w2w pushes inside an Event::Loop. Verified by inspection:
- 06/distributed2 (the cycle-148 reproducer): all 12 cross-tmp pushes are at top level (between pass-1 barrier and pass-2 barrier).
- 09-producer-consumer / 11-game-of-life pipelined: not host-relay candidates (different shape).
- 03-reduction/distributed: blocked on TASK-0329 (host-excluding barrier) before the relay phase would matter.

## Cycle-148 architect P3.2 disclosure

The collect_w2w_pushes doc comment honestly discloses this limitation (cycle-148 architect P3.2 finding). The defect class is the cycle-128/138/140/141/142/142b/143/144/146/147 silent-sibling meta-rule's WEAKER form — a future schedule shape would trip a contract gap that we know about but don't actively guard.

Per feedback-panic-not-diagnostic-recurring: failing LOUD at codegen (when collect_w2w_pushes detects a nested Push) is strictly better than silently producing wrong relay code.

## Acceptance criteria

### AC#1: defensive ContractGap

When collect_w2w_pushes descends into an Event::Loop body and finds a Push with non-host dst (the w2w shape), surface an EmitError::ContractGap forward-linking TASK-0327 and naming the schedule + loop iv. The error message should be precise enough that a user reading it knows EXACTLY what schedule shape is unsupported and how to file a follow-up.

### AC#2: positive + negative tests

- Negative fixture (today's 06/distributed2 shape, all w2w pushes at top level): no ContractGap fires. Already covered by host_relay_emit.rs.
- Positive fixture (synthetic ACFG with a w2w Push inside an Event::Loop): EmitError::ContractGap fires with the expected forward-link.

### AC#3: documentation update

Update the collect_w2w_pushes doc comment to reflect the AC#1 active guard (replacing the current passive 'cycle-148 limitation' disclosure).

## Dependencies

- TASK-0327 cycle 148 (the collect_w2w_pushes helper).
- TASK-0327 cycle 149+ (mp-tcp-event sibling) may want the same guard.

## Cross-reference

- nucleus/backends/mp-tcp-bufsync/src/lib.rs:collect_w2w_pushes (the helper).
- TASK-0327 cycle-148 architect P3.2 finding.
- feedback-panic-not-diagnostic-recurring (the meta-pattern AC#1 follows).

## Honest scope

LOW priority. Dormant defect. Filed for fail-loud hygiene before a future schedule shape arrives.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 149 sibling extension (TASK-0327 cycle-149 architect P2.1 + P3.1 fold-back)

The cycle-149 mp-tcp-event host-relay implementation inherits the SAME flat-emit limitation as mp-tcp-bufsync's cycle-148 slice — `collect_w2w_pushes` in `nucleus/backends/mp-tcp-event/src/multi_worker.rs` (`grep -n 'fn collect_w2w_pushes' nucleus/backends/mp-tcp-event/src/multi_worker.rs`) recurses into Event::Loop bodies but the relay phase is emitted FLAT outside any loop. Same hazard, same defensive ContractGap need.

**Action:** when this task is worked, the defensive ContractGap must land in BOTH backends in the same cycle (or a sibling task — the precedent should match the cycle-148/149 paired-lift design). Test coverage must include both backends' collect_w2w_pushes paths.

**Cross-reference:** cycle-149 mp-tcp-event host-relay test pin lives at `nucleus/backends/mp-tcp-event/tests/host_relay_emit.rs::host_emit_includes_task_0327_relay_phase_with_12_hops`. The same fixture (06/distributed2) is the negative-case "no Loop body" check for the mp-tcp-event sibling.

**Scope clarification:** the task title still says "mp-tcp-bufsync" but the AC now covers BOTH backends (mp-tcp-bufsync from cycle 148, mp-tcp-event from cycle 149). When working: either rename the task (single-arm-scope title) or treat as a paired fix; the cycle-148/149 history establishes the precedent for the paired fix.

## Cycle 150 (TASK-0331 AC#2 empirical promotion test): in-tree trigger FOUND

Cycle 150 empirically tested the cycle-149 prose claim that `05-stencil/distributed-2d × mp-tcp-event`'s remaining blocker was TASK-0294 (host 2D slice-paste). The promotion attempt FAILED at runtime: workers w0..w3 deadlock at 32.4s with run.sh reporting failure (exit code 0 from workers but timeout-shape, not bit-identical mismatch).

Root cause: the 2x2-grid partition emits halo-strip Push/Wait events INSIDE an outer time-step / iteration loop. mp-tcp-event's cycle-149 host-relay code emits a FLAT relay block. The src worker pushes N times per outer iteration; the host's flat relay runs ONCE; subsequent iterations deadlock waiting for relayed frames that never arrive.

This IS the in-tree trigger that TASK-0330 was filed pending. The same defect would hit mp-tcp-bufsync if `05-stencil/distributed-2d × mp-tcp-bufsync` were ever promoted (currently [[skip]] on TASK-0042 capability mismatch).

Priority bump: LOW -> MEDIUM. The cycle-150 finding makes this an active correctness gap rather than a defensive-only consideration.

Cycle-150 e2e-matrix.toml record: the `05-stencil/distributed-2d × mp-tcp-event` skip reason now precisely cites TASK-0330 as the remaining blocker (cycle-150 edit, lines ~776-810).

Forward-carry to TASK-0330 implementation cycle: the empirical test fixture is in-tree as `nuc-nucleus/examples/05-stencil/schedules/distributed-2d.sched.nuc`. Promote that cell as the regression-pin once the fix lands.

## Cycle 150 priority-bump RETRACTED (orchestrator self-correction)

The cycle-150 entry above attributed the `05-stencil/distributed-2d × mp-tcp-event` deadlock to "Loop-body w2w Push" — TASK-0330's scope. EMPIRICAL RE-EXAMINATION of the emitted w0.rs (cycle-150 e2e scratch dir) shows the w2w pushes are at TOP LEVEL inside `fn main`, NOT inside any `for` loop. The actual root cause is a DIFFERENT defect class: w0/w1/w2/w3 all begin with `chan_X.wait()` calls for cross-worker halo strips BEFORE any push. Cycle-149's host-relay splices the relay AFTER `bar_0.wait()`. Workers blocked at initial waits never reach bar_0; host blocks at bar_0; relay never runs; deadlock.

Root-cause classification: **WAIT-BEFORE-PUSH schedule shape vs cycle-149's scatter-compute-gather assumption**, NOT Loop-body. The in-tree trigger for TASK-0330 has NOT been found; this task remains dormant pending an actual Loop-body w2w Push schedule. Priority returned to Low.

The newly-identified defect class is filed as a SEPARATE follow-up task (see cycle-150 commit / tracker for the exact ID); TASK-0330 is unaffected.

Honesty note: this is exactly the cycle-149 architect P3.2 lesson firing in real time — a prose claim ("Loop-body limitation") was made without empirical verification of the emitted code, and the empirical-verification step (running `just e2e` + reading the generated host.rs and w0.rs) caught the mis-attribution within the same cycle. The retraction here is the honest record.

## Cycle 151 (TASK-0332 AC#2 architect P2): defensive-check divergence found

Cycle 151's `detect_wait_before_push_hazard` (added to both backends) has a precondition `has_w2w_push` that scans TOP-LEVEL events only via `events.iter().any(...)`. The cycle-148/149 `collect_w2w_pushes` helper (which decides who is a src in `Plan::relay_schedule`) ALSO recurses into Loop bodies — so a worker with `[Wait{w2w}, Loop{Push{w2w}}]`:

- `has_w2w_push` (top-only) = false → cycle-151 detector skips → no hazard reported.
- `collect_w2w_pushes` includes this worker's Loop-body Push → relay_schedule includes it as a src → host blocks on `relay_one(seq=its push)` → DEADLOCK.

This is a theoretical false-negative for cycle-151's defensive check — TASK-0330's parent Loop-body limitation. When THIS task is implemented:

1. The defensive ContractGap should fire LOUD on `collect_w2w_pushes` encountering a Push inside `Event::Loop` (per the original AC#1 of THIS task).
2. Cycle-151's `has_w2w_push` precondition in BOTH backends should be ALIGNED with `collect_w2w_pushes`'s recursion (call `collect_w2w_pushes` and check for non-empty, OR write a recursive walker). The current `events.iter().any(...)` shape is a known false-negative for the Loop-body shape THIS task scopes.

Both alignments should land in the same cycle as THIS task's implementation (paired-lift discipline — feedback-silent-sibling-defect 11th firing).

Cycle-151 left a forward-carry comment in BOTH backends' `detect_wait_before_push_hazard` documenting this divergence and pointing at TASK-0330 for the eventual closure.
<!-- SECTION:NOTES:END -->
