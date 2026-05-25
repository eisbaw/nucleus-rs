---
id: TASK-0330
title: >-
  mp-tcp-bufsync collect_w2w_pushes inside Loop bodies — defensive ContractGap
  (TASK-0327 cycle-148 architect P3.2)
status: To Do
assignee: []
created_date: '2026-05-25 17:40'
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
