---
id: TASK-0332
title: >-
  mp-tcp-event host-relay deadlocks on wait-before-push schedule shapes
  (TASK-0327 cycle-149 limitation, TASK-0331 cycle-150 empirical finding)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 18:52'
updated_date: '2026-05-26 07:09'
labels:
  - M6
  - backend
  - mp-tcp-event
  - host-relay
  - forward-carried-from-TASK-0327
  - deadlock
dependencies:
  - TASK-0327
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0327 cycle 149 landed mp-tcp-event's worker-to-worker `Push`/`Wait` via synchronous host-relay. The relay splice heuristic (`relay_phase_insertion_point`) places the relay block just BEFORE the LAST top-level `Event::Sync` — between pass-1 barrier and pass-2 barrier on the 06/distributed2 reproducer. This works for SCATTER-COMPUTE-GATHER schedule shapes where each worker pushes BEFORE waiting on w2w pairs.

## Cycle 150 (TASK-0331 AC#2) empirical finding

Promoting `05-stencil/distributed-2d × mp-tcp-event` to `[[required]]` and running `just e2e` produced a deadlock at 32.4s (workers exit code 0 but run.sh reports failure — timeout-shape).

Root cause (verified by inspecting cycle-150 e2e scratch dir's emitted code):

- Each worker's events begin with `chan_X.wait()` calls for cross-worker halo strips BEFORE any push:
  ```
  // w0.rs (emitted by mp-tcp-event for 05/distributed-2d):
  chan_4.wait();  // recv `img_in` from w2
  chan_5.wait();  // recv `img_in` from w1
  chan_7.push(img_in.clone()); // send `img_in` to w1
  chan_8.push(img_in.clone()); // send `img_in` to w2
  bar_0.wait();  // <-- worker reaches this only after the waits
  ```
- Cycle-149's host-relay block splices AFTER `bar_0.wait()` (host.rs line ~226-241).
- DEADLOCK CHAIN: w0 blocked at chan_4.wait() (waiting for w2's push via host relay). w2 symmetrically blocked. w0..w3 never reach bar_0. Host blocks at bar_0 forever (waiting for the 4 workers). Host never runs the relay phase. The w2w pushes never get forwarded.

## What works today

`05-stencil/distributed-2d × pthreads-async` PASSES bit-identical because pthreads-async has direct worker-to-worker ring buffers (no host mediation needed for the w2w pushes). The schedule's wait-before-push order is correct under the assumption of direct w2w channels.

## Acceptance criteria

### AC#1: relax the host-relay scheduling model

The cycle-149 design's scatter-compute-gather timing model needs to be replaced (or supplemented) by one of:

- **(A) Threaded host-relay**: spawn a thread in host's main that polls the reactor for inbound w2w frames and forwards them concurrently with host's own work. The CTRL barrier still works because it's on a separate stream.
- **(B) Interleaved host-relay**: instead of a single relay block, emit per-Push site interleaving — for each w2w Push event, host immediately runs `relay_one(seq, dst_peer, cap)` when its inbound[seq] arrives. Requires re-modeling host's event list to include relay events.
- **(C) Pre-bar_0 relay**: detect schedule shape ahead of time. If any worker's first w2w event is a Wait, splice the relay BEFORE bar_0 instead of after — or splice multiple times. Requires schedule-shape analysis.

(A) is the most general but adds a thread to the runtime; (B) is the cleanest but invasive in the emit walker; (C) is narrowly fix-but-only-for-known-shapes.

### AC#2: defensive ContractGap or auto-detect at Plan::build

Until AC#1 is landed, the cycle-149 host-relay should DETECT a wait-before-push hazard at `Plan::build` and fail-loud with EmitError::ContractGap forward-linking THIS task. The shape to detect: any non-host worker whose first top-level w2w `Event::Wait` precedes any top-level w2w `Event::Push` on the same worker's event list. Fail-loud > silent deadlock.

### AC#3: regression pin

Once AC#1 lands, promote `05-stencil/distributed-2d × mp-tcp-event` to `[[required]]` in `nuc-nucleus/e2e-matrix.toml`. Verify bit-identical against `05-stencil/reference.bin` (same oracle as the pthreads-async sibling that already passes).

## Honest scope

- **Distinct from TASK-0330**: TASK-0330 covers w2w Push events INSIDE Loop bodies. This task covers wait-before-push event order at TOP LEVEL. Both share the cycle-148/149 host-relay assumption set but are different defect classes.
- **Distinct from TASK-0329**: TASK-0329 covers CTRL barrier mediation for host-excluding barriers. This task covers DATA-arm wait-before-push under host-INCLUDING barriers.
- **Priority MEDIUM**: in-tree trigger exists (05/distributed-2d × mp-tcp-event). The cycle-150 empirical finding turned this from theoretical-defensive into active-correctness-gap. mp-tcp-bufsync has the same architecture but is currently capability-gated on this specific cell (TASK-0042 async + buffer=2 + notify=event), so the defect surface is mp-tcp-event-only today.
- **AC#2 defensive ContractGap is the minimum cycle-N closure**; AC#1 full fix is a substantial future cycle.

## Cross-reference

- TASK-0327 (cycle 148/149): the host-relay design this task identifies a hole in.
- TASK-0330: Loop-body sibling-defect (cycle-148 architect P3.2).
- TASK-0329: CTRL-arm barrier mediation.
- TASK-0331 (cycle 150): the audit task that empirically surfaced this finding.
- `nuc-nucleus/examples/05-stencil/schedules/distributed-2d.sched.nuc`: the in-tree trigger schedule.
- `nucleus/target/e2e-matrix/run-*/05-stencil__distributed-2d__mp-tcp-event/src/bin/{host,w0}.rs`: the emitted code showing the deadlock-shape event order.

## Memory cross-reference

- `feedback-orchestrator-narrative-also-wrong` — the cycle-149 narrative "remaining blocker is TASK-0294" was algebra-imprecise; cycle-150 empirical test corrected it.
- `feedback-silent-sibling-defect` — TASK-0330 was filed cycle 148 for a SIBLING defect class; this task is the in-tree-trigger defect class that cycle-150 originally MIS-attributed to TASK-0330.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Forward-carried from TASK-0330 (cycle 153)

When AC#1 lands one of the (A) threaded / (B) interleaved / (C) pre-bar_0 host-relay scheduling alternatives, the cycle-153 "RESOLVED by TASK-0330" comment in `detect_wait_before_push_hazard` (both backends, ~line 1740-1750 mp-tcp-bufsync and ~1146-1157 mp-tcp-event) needs re-verifying. The composition assumption is: a Loop-body w2w Push CANNOT silently reach codegen because `collect_w2w_pushes` runs from `render_relay_phase` which is on the only host emit path. If AC#1's new design diverges from that call graph (e.g. host emits relays piecewise inside the per-event loop), the TASK-0330 guard's downstream-fire timing may need to be moved up to `Plan::build` as a recursive walker — or the composition claim must be re-validated against the new emit path.

Audit step for the AC#1 implementer: `grep -rn render_relay_phase` and `grep -rn collect_w2w_pushes` in both backends; verify the TASK-0330 guard still fires on every code-emitting path before claiming this task closes.

## Cycle 166 — closing audit + tracker close

### Audit conclusion

All three ACs met:

- **AC#1 (relax host-relay scheduling model)**: MET via TASK-0329.01.01 cycle 162 (slice 1, Option D push-before-wait reorder). Option D was an UNLISTED variant beyond the original A/B/C menu, but it satisfies the AC wording "relax the host-relay scheduling model" — the relaxation is at the event-list layer (`apply_safe_push_reorder`) coupled with `relay_phase_insertion_point`'s scan-for-earliest-safe-Sync update. mp-tcp-event-only by intentional asymmetry (bufsync's per-pair FIFO constraint 3 makes the splice-point lift unsafe — see memory `project-mp-tcp-event-vs-bufsync-safety-profile`). Cycle 166 confirmed 05/distributed-2d × mp-tcp-event remains [[required]] bit-identical (1× e2e sample).
- **AC#2 (defensive ContractGap at Plan::build)**: MET at cycle 151 (landed in both backends per paired-lift discipline). Cycle-166 audit asked: is the guard now STRUCTURALLY DEAD after slice-1? Conclusion: **NO** — slice-1 only hoists HOISTABLE w2w Pushes. Non-hoistable Pushes (those tainted by a preceding Fire / w2w Wait / Loop-body write of the same data with overlapping tile on a shared axis) remain in their original position. The guard's wait-before-push hazard shape can still survive for non-hoistable cases on mp-tcp-event, and is fully active on mp-tcp-bufsync (the reorder pass is not applied there). The guard is therefore not `feedback-opacity-gate-rot`; it stays in place as a residual safety net with cycle-166 docstring reframe to name the post-cycle-162 dormancy explicitly.
- **AC#3 (regression pin)**: MET cycle 162 (05/distributed-2d × mp-tcp-event promoted [[required]] in nuc-nucleus/e2e-matrix.toml). Cycle-166 e2e re-verified the cell still PASS at 3.81s.

### Docstring reframe landed cycle 166

Two production-code edits per paired-lift discipline:

1. `nucleus/backends/mp-tcp-event/src/multi_worker.rs` — the cycle-151 design narrative comment in `Plan::build` (above the `detect_wait_before_push_hazard` call site) plus the corresponding error-message text in the `Event::Wait` arm of the guard itself. The text "AC#1 (threaded or interleaved host-relay) is the architectural fix" was outdated and removed; the replacement names the cycle-162 Option D landing and the per-residual-class diagnostic hint.
2. `nucleus/backends/mp-tcp-bufsync/src/lib.rs` — same edit shape on the sibling backend, but the narrative branches at "reorder pass NOT applied here" to preserve the cross-backend asymmetry truthfully. The error message acknowledges that no in-tree bufsync schedule is currently capability-compatible with this hazard, but the guard stays for fail-loud hygiene under a future capability lift.

Three other docstring sites pre-existed in correct cycle-153 / cycle-163 framing and were left unchanged:
- `mp-tcp-event/src/multi_worker.rs:1150-1160` ("RESOLVED by TASK-0330" cycle-151 P2 note about Loop-body delegation) — still accurate.
- `mp-tcp-bufsync/src/lib.rs:1796-1806` (paired-lift sibling of the above) — still accurate.
- `mp-tcp-event/src/multi_worker.rs::collect_w2w_pushes` docstring — cycle 166 SHARPENED to name (R-bare) and (R-singleton) residual classes explicitly (was previously only naming the (R-singleton) class implicitly via "Xfer pair the pass couldn't pair within the same Sequence").

### Forward-carried lessons

- **L1 (audit outcome)**: cycle-161b's hypothesis "TASK-0332's AC#2 guard becomes contingent on AC#1 landing" was partially correct — AC#2 becomes DORMANT on the in-tree matrix but is NOT structurally dead. The non-hoistable case + bufsync's pass-not-applied case keep it reachable in principle.
- **L2**: cycle-151 narrative text in BOTH backends carried "AC#1 (threaded or interleaved host-relay) is the architectural fix" wording that became wrong when cycle 162 landed Option D instead. This is `feedback-comment-doc-lie-recurring` firing on cycle-151 narrative when its predictive claim was overtaken by cycle-162 implementation reality. The same pattern can fire on cycle-166 narrative when a future cycle revisits these guards — every "X is the fix" claim is a hostage to fortune.
- **L3**: paired-lift discipline asymmetry — when a fix is applied to ONE backend by design (`apply_safe_push_reorder` mp-tcp-event-only), the SIBLING backend's docstring must explicitly enumerate why the lift does not apply, otherwise a future maintainer will treat "absent from bufsync" as a sibling-defect bug and try to "fix" it.

### Verification

- `just build`, `just clippy`, `just test`, `just test-release` all green; baseline preserved at 962/0/3 dev, 961/0/3 release.
- `just e2e` baseline preserved at 112/102/0/10/0; all 3 trigger cells PASS.
- `just check-textual-replace-on-codegen` + `just check-include-str-coverage` green.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
TASK-0332 closed cycle 166 after structural audit + paired-backend docstring reframe.

All three ACs met:
- AC#1 lifted via TASK-0329.01.01 cycle 162 (slice 1, Option D push-before-wait reorder — an unlisted variant beyond the original A/B/C menu but satisfying the AC's "relax the host-relay scheduling model" intent).
- AC#2 defensive ContractGap landed cycle 151; cycle-166 audit confirmed the guard is NOT structurally subsumed by slice-1 (non-hoistable Push cases and the entire mp-tcp-bufsync backend keep it reachable). Guard remains in place as residual safety net with docstring reframe.
- AC#3 (05-stencil/distributed-2d × mp-tcp-event promoted [[required]]) landed cycle 162; cycle-166 e2e re-verified PASS.

Cycle-166 docstring reframe in BOTH backends sharpens the stale "threaded or interleaved host-relay is the architectural fix" wording into a precise cycle-162 Option D landing narrative plus per-residual-class diagnostic hints.

Verification: all gates green; e2e 112/102/0/10/0 baseline preserved (dev tests 962/0/3, release tests 961/0/3, e2e 3 trigger cells PASS).
<!-- SECTION:FINAL_SUMMARY:END -->
