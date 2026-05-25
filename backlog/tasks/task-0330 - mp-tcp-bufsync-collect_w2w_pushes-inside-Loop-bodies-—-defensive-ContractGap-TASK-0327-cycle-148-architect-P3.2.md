---
id: TASK-0330
title: >-
  mp-tcp-bufsync collect_w2w_pushes inside Loop bodies — defensive ContractGap
  (TASK-0327 cycle-148 architect P3.2)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 17:40'
updated_date: '2026-05-25 19:50'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
## Cycle-NNN implementation plan

**Goal**: AC#1 (defensive ContractGap) + AC#2 (positive/negative tests) + AC#3 (doc-comment update) for BOTH backends in the same cycle, per the cycle-148/149 paired-lift precedent recorded in this task's Implementation Notes.

**Code changes**:

1. `mp-tcp-bufsync/src/lib.rs::collect_w2w_pushes`:
   - Add an inner helper with an `inside_loop: bool` parameter; entry point passes `false`.
   - When recursing into `Event::Loop`, set `inside_loop = true`.
   - When `inside_loop && Event::Push { dst != host, .. }` matches, return `EmitError::ContractGap(...)` with: backend prefix `mp-tcp-bufsync`, TASK-0330 + TASK-0327 forward-link, the data/dst/seq fields, and the mechanism (flat relay block can't drain N pushes / order-mismatched).
   - Change signature from `fn collect_w2w_pushes(...)` to `Result<(), EmitError>`.
   - Update single call site at `relay_schedule()` to bubble `?`.

2. `mp-tcp-event/src/multi_worker.rs::collect_w2w_pushes`:
   - Same shape; helper already returns Result, just add the inside_loop flag.
   - Backend prefix `mp-tcp-event`.

3. Doc comments at both sites: replace the dormant-limitation framing ("Filed as part of TASK-0327 cycle-149 follow-up if a future schedule nests w2w pushes inside Loops") with an active-guard statement naming TASK-0330 + the in-cycle test pin.

**Tests** (new files):

- `mp-tcp-bufsync/tests/loop_body_w2w_push.rs`:
  - Positive: 3-worker synthetic fixture with a w2w Push (w1->w2) inside an `Event::Loop` body; assert ContractGap mentioning `Loop`, `mp-tcp-bufsync`, `TASK-0330`.
  - Negative: same shape but with a host-bound Push inside the Loop (dst == host); assert Ok / no fire (the relay only cares about non-host destinations).

- `mp-tcp-event/tests/loop_body_w2w_push.rs`:
  - Mirror, with `mp-tcp-event` backend prefix.

**Verification gate**:

```
nix develop --command bash -c "just build && just clippy && just test && just test-release && just e2e"
```

Expected pre/post e2e baseline unchanged (no schedules in the matrix trip the new guard; in-tree workloads have top-level w2w pushes only).

**Honest scope / out-of-scope**:

- AC#1 (the host-relay LIFT for nested Pushes) is **not** in scope — this task is the fail-loud DEFENSIVE GUARD only, per its filing scope (LOW, dormant, fail-loud hygiene). The day a real in-tree schedule needs nested w2w Pushes, file a separate task.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 153 — landed

### What landed

- `mp-tcp-bufsync` (`src/lib.rs::collect_w2w_pushes`): signature changed from `fn ... -> ()` to `Result<(), EmitError>`; private inner helper `collect_w2w_pushes_inner` with an `inside_loop: bool` accumulator fires `EmitError::ContractGap` forward-linking TASK-0330 + TASK-0327 when a w2w `Push` (dst != host) is found INSIDE an `Event::Loop` body. Single call site (`relay_schedule`) updated to propagate the Result via `?`; `render_relay_phase` propagates via `?` to the existing host-emit code path.
- `mp-tcp-event` (`src/multi_worker.rs::collect_w2w_pushes`): same shape; the helper already returned Result so only the `inside_loop` accumulator + new ContractGap arm were added. Backend prefix `mp-tcp-event` for the message.
- Cycle-151 architect P2 note in `detect_wait_before_push_hazard` (both backends) updated from "TASK-0330 tracks the parent Loop-body-w2w-Push limitation; when worked, align this precondition with collect_w2w_pushes's recursion" to "RESOLVED by TASK-0330" — the composition is now sound: a Loop-body w2w Push CANNOT silently reach codegen because `collect_w2w_pushes` runs from `render_relay_phase` (called in host's render_worker_program) and fail-louds before any code is written. The earlier top-level-only precondition in `detect_wait_before_push_hazard` is intentionally kept narrow because the Loop-body case is covered by TASK-0330's downstream guard.

### Test coverage (3 tests × 2 backends = 6 total, all PASS)

- `loop_body_w2w_push_is_typed_contract_gap` (positive, range 0..1) — pins the structural trigger + asserts message contains backend prefix, TASK-0330, TASK-0327, the "Loop" trigger, and the "FLAT outside any loop" mechanism narrative.
- `host_bound_push_inside_loop_does_not_trigger_guard` (negative) — host-bound Push inside a Loop body must NOT trigger the new guard (predicate is dst != host).
- `multi_iter_loop_body_w2w_push_is_typed_contract_gap` (positive, range 0..3) — cycle-153 architect P3.1 fold-back: quantitatively exercises the N>1 over-count narrative the error message claims.

### Cycle-153 review fold-back (P3 nice-to-have)

The parallel read-only review gate (qa-test-runner + mped-architect) both returned GO with three P3 findings; all three folded back before commit:

- **Architect P3.1**: multi-iteration positive test added (`range: 0..3`) — exercises the "worker pushes N times around the loop" narrative at N=3 rather than N=1.
- **Architect P3.2**: sibling-walker comments added at `mp-tcp-event/src/multi_worker.rs::collect_push_pairs` and `mp-tcp-bufsync/src/lib.rs::collect_xfer_data`. Both ALSO recurse into Loop bodies but are incidentally robust (or_insert / set-union) — the comment now documents the TASK-0330 guard as the upstream rejection point so a future maintainer doesn't add a redundant guard.
- **QA optional tightening**: the `flat` assertion in both backends' positive tests tightened to `FLAT outside any loop` (exact substring pin).

Architect P3.3 was a no-fix observation about call-graph soundness; left as-is per the architect's recommendation.

### Gotchas / subtleties for future maintainers

1. `Event::Loop` worker emit requires `names.iter_var` entries for all iter_vars referenced by Loop bodies. The negative test initially failed with `iter var IterVar(0) in Event::Loop has no name in NameTables` (an UNRELATED rejection from the worker emit path) and had to be fixed by adding the iter_var name to the fixture. Future synthetic Loop fixtures must do the same.

2. The TASK-0330 guard fires in `render_relay_phase` (the rendering pass), NOT in `Plan::build` (the validation pass). `detect_wait_before_push_hazard` (cycle 151) scans top-level events only and so cannot catch the Loop-body case; the composition relies on `render_relay_phase` being on the only emit path (it is — verified by grep on `render_relay_phase` callers in both backends: only `render_host_program` / `emit_host_main`). A future refactor that lets a host emit skip `render_relay_phase` for non-relay schedules would silently re-introduce the gap unless this guard is also moved up to Plan::build. Filed forward as a soundness-pin in [[feedback-orchestrator-narrative-also-wrong]] (the "composition of cycle-151 + TASK-0330 is sound" claim is a call-graph property, not an asserted invariant).

3. Test counts: 893/0/3 → 895/0/3 (+2 multi-iter tests, one per backend). e2e baseline 112/96/0/16/0 preserved across pre-fold-back and post-fold-back samples.

### Forward-carried lessons

- **For TASK-0329 (host-mediated barrier mediation, the sibling CTRL-arm task)**: when that task lands, the same paired-backend discipline applies. Its lift will likely follow the cycle-148/149 splice pattern; the cycle-153 P3.2 sibling-walker audit shape (grep for ALL recursive Event::Loop walkers, document their robustness) is a reusable hygiene step for that task too.

- **For TASK-0332 AC#1 (relax the host-relay scheduling model, the wait-before-push lift)**: if AC#1 lands one of the (A) threaded / (B) interleaved / (C) pre-bar_0 alternatives, the cycle-153 "RESOLVED by TASK-0330" comment in `detect_wait_before_push_hazard` needs revisiting — the composition assumption may need re-verification depending on whether the new design still goes through `render_relay_phase`.
<!-- SECTION:NOTES:END -->
