---
id: TASK-0329
title: >-
  host-mediated barrier mediation for host-excluding barrier shapes (TASK-0327
  cycle-148 architect P3.1)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-05-25 17:40'
updated_date: '2026-05-26 01:33'
labels:
  - M6
  - backend
  - mp-tcp-bufsync
  - host-mediation
  - forward-carried-from-TASK-0327
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0327 cycle 148 lifted mp-tcp-bufsync's worker-to-worker Push/Wait rejection via host-mediated relay (host reads from data_<src>, forwards to data_<dst>). The sibling rejection at nucleus/backends/mp-tcp-bufsync/src/lib.rs:382-393 — host-excluding barriers (e.g. barrier participants = {w0..w3} with no host) — STILL fails LOUD with EmitError::ContractGap citing TASK-0175.

Cycle-148 architect P3.1 noted: this rejection is now STRUCTURALLY ANALOGOUS to the lifted data_conn_var rejection. Both are 'star topology forbids X' rejections; both have a host-mediation lift available (host injects itself as a mediating hub: host crosses with each non-host participant on ctrl_<peer>; each non-host worker crosses with host on ctrl_host).

## Honest exposure

LOW (dormant). No in-tree schedule produces a host-excluding barrier on mp-tcp-bufsync:
- 03-reduction/distributed × mp-tcp-bufsync would (workers do a reduction barrier excluding host), but it's currently SKIPPED on TASK-0175 — the same blocker this task would lift.
- 13-cnn-inference/batch_parallel × mp-tcp-bufsync also SKIPPED with mixed TASK-0175 + TASK-0117 reasons.

## Acceptance criteria

### AC#1: lift mp-tcp-bufsync host-excluding barrier rejection

In Plan::build, replace the rejection at lib.rs:382-393 with host-mediation injection:
1. For each barrier with participants not including host: ADD host to that barrier's participant set in the sidecar barrier_participants map.
2. Synthesize a corresponding Event::Sync on host's per-worker event list at the right ordering point (analogous to the cycle-148 relay-phase splice — likely between adjacent worker barriers).
3. The host's render_events Sync emit (existing barrier_cross loop) handles the injected participation transparently.

### AC#2: e2e cell promotion

Once AC#1 lands, promote 03-reduction/distributed × mp-tcp-bufsync from [[skip]] to [[required]] in nuc-nucleus/e2e-matrix.toml. Bit-identical against reference.bin.

### AC#3: defensive test fixture

Add a fixture exercising a host-excluding barrier shape; assert the barrier_cross emit is generated correctly on both host and non-host workers.

## Dependencies

- Builds on TASK-0327 cycle 148 (the cycle-148 splice/scheduling machinery is precedent).
- mp-tcp-event sibling lift is part of TASK-0327 cycle 149.

## Cross-reference

- nucleus/backends/mp-tcp-bufsync/src/lib.rs:382-393 (the rejection site).
- nucleus/backends/mp-tcp-bufsync/src/lib.rs:render_relay_phase / relay_phase_insertion_point (the analogous mediation precedent).
- TASK-0327 cycle-148 architect parallel-review P3.1 finding.

## Honest scope

LOW priority. Dormant defect (no in-tree schedule trips it on mp-tcp-bufsync today). Filed so the asymmetry surfaced by cycle-148's lift has a tracker anchor; promote when an actual schedule needs it.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Forward-carried from TASK-0330 (cycle 153)

The TASK-0330 cycle established a useful sibling-walker audit step that should be reused when this task's host-mediated barrier mediation lands:

After implementing the lift, grep ALL recursive event-walkers in BOTH backends for `Event::Loop` recursion (mp-tcp-bufsync: `collect_xfer_data`, `collect_w2w_pushes`; mp-tcp-event: `collect_push_pairs`, `collect_w2w_pushes`, others). For each one, audit how it handles Loop-body events of the kind your lift touches — does it use set-union (idempotent), or_insert (first-visit-wins), or list-append (over-count)? Document the answer at each walker site, even if it's "incidentally robust".

This audit defends against [[feedback-silent-sibling-defect]]: the structural pattern "recursive event-walker over Loop bodies" repeats; a new fail-loud or new accumulator pattern may have silent siblings that need the same treatment.

## Cycle 157 (TASK-0334) empirical-verification update

The TASK-0329 description (filed cycle 148) included the line: '13-cnn-inference/batch_parallel × mp-tcp-bufsync also SKIPPED with mixed TASK-0175 + TASK-0117 reasons.'

Cycle 157 empirically verified this cell — it does NOT trigger TASK-0329 OR TASK-0117. The schedule has no host-excluding barrier; transfer_inject emits cleanly (one Push per (data, producer)). The cell PASSES bit-identical against reference.bin and has been PROMOTED [[skip]] → [[required]] cycle 157.

The TASK-0329 description's enumeration of impacted cells is therefore stale on that one entry. The remaining in-tree trigger for TASK-0329 (empirically confirmed cycle 157) is **13-cnn-inference/pipeline_parallel × mp-tcp-event** — that cell's barriers {w1,w2,w3} genuinely exclude host. 03-reduction/distributed cells on both bufsync and mp-tcp-event were also forward-linked to TASK-0329 by cycle 150, but cycle 157 refuted all 3 of those misattributions (the bufsync arm now cites TASK-0335; the mp-tcp-event arm now PASSES with a masking disclosure).

Net: TASK-0329's in-tree trigger today is exactly ONE cell, not the original four cycle-150's prose enumerated.

## Cycle 160 — AC#1 + AC#3 landed via compiler-level host_mediation_inject pass

### Approach: compiler-level pass, not in-backend modification

Instead of modifying Plan::build at each TCP backend (the original AC#1 framing), the lift was implemented as a new compiler pass apply_host_mediation_inject (nucleus/nucleus-compiler/src/passes/host_mediation_inject.rs). The pass walks the ACFG, finds every SyncPlaceholder excluding host, and inserts host into the participants set. The driver applies the pass conditionally for mp-tcp-bufsync and mp-tcp-event only; pthreads-sync and pthreads-async do NOT apply it (their std::sync::Barrier::new(N) primitive handles host-excluding barriers natively).

### Rationale for the pass-level lift

1. **Single source of truth.** Adding host to the participants set in the ACFG before projection means acfg_to_events naturally places host's Sync at the structurally correct position (preserving any enclosing Repeat/Sequence nesting). Mutating per_worker after projection would require the backend to re-derive that structural position from event-list inspection — error-prone, and the cycle-148/149 relay-phase splice heuristic (last top-level Sync vs first Wait) is known to be fragile (cycle 150 TASK-0332 finding).

2. **Pthreads symmetry preserved.** Pthreads-sync/pthreads-async observe ZERO ACFG change (driver skips the pass for them), so their per_worker projections and emitted code are byte-identical to pre-cycle-160.

3. **Defensive ContractGap stays.** The existing host-excluding-barrier rejection at mp-tcp-bufsync/src/lib.rs:391-402 and mp-tcp-event/src/multi_worker.rs:241-252 remains as defense-in-depth: if the driver fails to apply the pass for any reason (e.g., a downstream caller bypasses the driver), the backend still fails loud. Both rejection comments updated to cite cycle 160 TASK-0329 as the lifted-by site.

### AC status

- **AC#1 (lift mp-tcp-bufsync / mp-tcp-event host-excluding barrier rejection)**: GREEN via the compiler-level pass + driver dispatch (apply_host_mediation_inject is applied for backend in {mp-tcp-bufsync, mp-tcp-event}). The original AC#1 wording cited mp-tcp-bufsync only, but cycle-148/149 already had a sibling rejection in mp-tcp-event; the pass-level lift handles BOTH symmetrically per paired-lift discipline (feedback-silent-sibling-defect).
- **AC#2 (e2e cell promotion)**: BLOCKED-DIFFERENTLY. The two in-tree TASK-0329 trigger cells (09-producer-consumer/pipelined × mp-tcp-event AND 13-cnn-inference/pipeline_parallel × mp-tcp-event) now advance past the cycle-148 ContractGap but immediately hit TASK-0330's defensive guard against in-loop worker-to-worker Push events. Both schedules push w↔w data per-iteration inside Loop bodies; cycle-149's flat host-relay cannot drain that shape. Filed TASK-0329.01 (host-relay redesign for in-loop w2w Push) as the cycle-160 fold-back blocker. The cells stay [[skip]] in e2e-matrix.toml; the skip reasons were UPDATED to cite TASK-0330 as the front blocker (not TASK-0329).
- **AC#3 (defensive test fixture)**: GREEN. Seven new tests in nucleus-compiler/src/passes/host_mediation_inject.rs cover: (a) host-excluding Sync at top level mediated, (b) host-including Sync unchanged, (c) Sync inside Sequence mediated, (d) Sync inside Repeat body mediated, (e) idempotence, (f) no-Sync ACFG no-op, (g) composed-with-acfg_to_events projects Sync to host. The existing host_excluding_barrier_is_typed_contract_gap test (multi_worker_emit.rs:222) still passes — it tests the defensive guard, which is intentionally preserved.

### Gate (post-cycle-160)

- cargo test --workspace (dev): 913 / 0 / 3 (was 906/0/3 baseline; +7 new pass tests).
- cargo test --release --workspace: 913 / 0 / 3.
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- just check-textual-replace-on-codegen: OK.
- just check-include-str-coverage: OK.
- just e2e: 112/99/0/13/0 — UNCHANGED from cycle-159 baseline (no cell-level effect because both trigger cells stay [[skip]] on TASK-0330 instead).

### Empirical verification (cycle 160)

Ran nucleus driver directly on both TASK-0329 trigger cells:
- 09-producer-consumer/pipelined × mp-tcp-event: emitted TASK-0330 ContractGap forward-link (was TASK-0329 / TASK-0175 ContractGap pre-cycle-160). Front blocker migrated as expected.
- 13-cnn-inference/pipeline_parallel × mp-tcp-event: same migration confirmed.

### Gotchas & subtleties (forward-carried to TASK-0329.01)

- **The pass IS idempotent.** A second application is a no-op (BTreeSet::insert returns false when the key already exists). Verified by test idempotence_one_pass_equals_two_passes.
- **Host election in the driver mirrors the per-backend Plan::build logic**: prefer worker literally named 'host', fall back to smallest WorkerId in name_workers. In practice every multi-worker schedule in this codebase declares 'host'; the fallback is defensive.
- **Pass runs AFTER inject_syncs / inject_transfers but BEFORE acfg_to_events**: this ordering is structural. The barriers must exist in the ACFG (created by inject_syncs) for the pass to find them; the per_worker projection must run AFTER mediation so host's Sync lands at the right position.
- **paired-lift discipline (feedback-silent-sibling-defect) is structurally satisfied**: the pass operates on the ACFG above the backend split, so both mp-tcp-bufsync and mp-tcp-event get the lift in lockstep. Even though mp-tcp-bufsync has no in-tree trigger today (the original 03-reduction/distributed cell was cycle-157 misattributed and is now TASK-0335-fixed; 13-cnn-inference/batch_parallel passes cleanly), the paired lift prevents future cross-backend asymmetry.
- **No interaction with the cycle-149 host-relay or cycle-151 wait-before-push hazard.** The host_mediation_inject pass only touches Sync nodes; it leaves all Push/Wait events unchanged. The downstream relay_phase_insertion_point heuristic and detect_wait_before_push_hazard guard are unaffected.
- **TASK-0330 substantive lift is the in-tree-trigger unblocker, NOT this task.** Filed as TASK-0329.01 (cycle-160 fold-back). The two trigger cells now block on TASK-0329.01's host-relay redesign (likely shared design surface with TASK-0332 AC#1).

## Cycle 160 in-thread fold-back (review-gate findings)

After landing the initial cycle-160 pass + driver wire-up, the parallel read-only review gate (qa-test-runner + mped-architect) returned GO/NO-GO:

- qa-test-runner: GO with 2 P2 findings (idempotence-with-projection test gap, doc forward-link to TASK-0330) + 2 P3 findings.
- mped-architect: NO-GO on **P1.1 — host-election divergence**: the driver's election rule (acfg.name_workers.get("host").or_else(values().min())) did NOT mirror Plan::build's exact rule (which filters by used_workers and falls back to used_workers.first()). A schedule whose "host" worker is declared but has zero projected events would mediate against an ID the backend wouldn't elect, leaving the backend's defensive rejection to re-fire against the *backend-elected* host.

### Folded back in-thread (before commit)

- **P1.1**: driver now derives used_workers via a one-time acfg_to_events preview projection, then elects host using the same rule the backend uses (prefer "host" name AND in used_workers; else used.iter().next()). The mediated ACFG is re-projected at the existing acfg_to_events call. Cost: one extra O(nodes) projection per build; cheap compared to the cross-backend skew this would otherwise leak.

- **P2.1**: added test  pinning . Guards future projection changes that could re-introduce host-excluding Syncs post-mediation.

- **P2.2**: added exhaustive-match comment on 's match block (). The compiler catches additions structurally; the comment is the manual reminder when a new variant lands.

### Deferred to follow-up (not in this cycle)

- **architect P2.3** (forward-link to TASK-0330 in driver block): docstring already cites cycle-160 + the cross-backend lift rationale; adding the precise TASK-0330 cross-link in the driver-side comment is cosmetic. P3-level; can fold in a future tightening pass.
- **architect P2.4** (pthreads-sync / pthreads-async preservation test): a driver-level test asserting the conditional only fires for {mp-tcp-bufsync, mp-tcp-event} is high-leverage defense against future refactor. P2 surface — file as a forward-carried lesson for next cycle's hardening pass or fold into a future TASK-0329.01 closure.
- **architect P2.5** (test-pinned-string cross-citation in docstring): noted; cosmetic.
- **qa P2** (driver-level integration test for host-election fallback paths): same surface as P2.4; defer to a driver-tests hardening cycle.

### Final gate (post-fold-back)

- cargo test --workspace (dev): 914 / 0 / 3 (was 913 with the cycle-160 initial pass; +1 = the new idempotence-with-projection test).
- cargo test --release --workspace: 914 / 0 / 3 (parity; no TASK-0291 release-only divergence).
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- just check-textual-replace-on-codegen + just check-include-str-coverage: OK.
- just e2e: 112/99/0/13/0 — 2 runs, non-flake confirmed.

### Status

Stays In Progress. AC#1 + AC#3 GREEN; AC#2 BLOCKED-DIFFERENTLY on TASK-0329.01 (substantive host-relay redesign for in-loop w2w Push events; the new front blocker after the cycle-160 lift). When TASK-0329.01 lands, the two trigger cells promote to [[required]] and AC#2 closes.

## Cycle 160 in-thread fold-back — correction of P2.1 / P2.2 specifics

The previous append used double-quotes inside backticks; bash command substitution consumed the parenthesised text. Re-recording the specifics here:

- **P2.1 specifics**: new test name is task0329_idempotence_with_projection_acfg_to_events_is_stable (file: nucleus/nucleus-compiler/src/passes/host_mediation_inject.rs). It pins the equality: projecting after a single pass-application equals projecting after a double pass-application — formally acfg_to_events of apply_host_mediation_inject equals acfg_to_events of apply_host_mediation_inject applied twice.

- **P2.2 specifics**: the exhaustive-match comment was added on the inject_at private function's match block. The comment reads (paraphrased): EXHAUSTIVE — every new ACFGNode variant that can transitively contain a Sync MUST be added to this match. The compiler catches additions structurally; this comment is the manual reminder when a future variant lands. See nucleus/nucleus-compiler/src/passes/host_mediation_inject.rs lines around the match.

## Correction: test name prefix

The previous note said test name 'task0329_idempotence_with_projection_acfg_to_events_is_stable' with the task-id prefix. The actual fn name in nucleus/nucleus-compiler/src/passes/host_mediation_inject.rs (around line 246) is idempotence_with_projection_acfg_to_events_is_stable (NO task0329 prefix). Grep target: 'idempotence_with_projection'. Per feedback-comment-doc-lie-recurring: honest correction recorded.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 160 lands the TASK-0329 lift via a new compiler-level pass apply_host_mediation_inject (nucleus/nucleus-compiler/src/passes/host_mediation_inject.rs) instead of the original AC#1 framing's per-backend Plan::build modification. The pass walks the ACFG, inserts host into every SyncPlaceholder's participants set, and the driver applies it conditionally for mp-tcp-bufsync / mp-tcp-event only (pthreads-sync and pthreads-async unchanged; their std::sync::Barrier::new(N) handles host-excluding barriers natively). Rationale: a compiler-level lift means acfg_to_events naturally places host's Sync at the structurally correct position (preserving Repeat/Sequence nesting), avoiding the cycle-148/149 splice-heuristic fragility surfaced by cycle-150 TASK-0332. The cycle-148/149 defensive rejection at both backends stays as defense-in-depth. AC status: AC#1 GREEN (pass + driver dispatch + 7 unit tests including a composed-with-acfg_to_events end-to-end pin); AC#2 BLOCKED-DIFFERENTLY (both in-tree TASK-0329 trigger cells now advance past the cycle-148 ContractGap and hit TASK-0330's defensive guard against in-loop w2w Push events; filed TASK-0329.01 as the cycle-160 fold-back blocker; cells stay [[skip]] with updated reasons citing TASK-0330 as the front blocker); AC#3 GREEN. Gates: dev/release 913/0/3 each (+7 new pass tests; baseline was 906/0/3), clippy clean, structural checks pass, e2e 112/99/0/13/0 UNCHANGED from cycle-159 baseline. Empirical verification: both trigger cells now emit the TASK-0330 ContractGap forward-link instead of the cycle-148 TASK-0175 one, confirming the front blocker has migrated. Forward-carried lessons: pass-level lift cleanly preserves paired-lift discipline (operates above the backend split); idempotence guaranteed by BTreeSet::insert; pass ordering matters (after inject_syncs / inject_transfers, before acfg_to_events). Commits: this cycle.
<!-- SECTION:FINAL_SUMMARY:END -->
