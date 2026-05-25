---
id: TASK-0331
title: >-
  e2e-matrix.toml skip-reason audit post-cycle-149: opacity-rot of TASK-0175
  citations after DATA-arm lift (TASK-0327 cycle-149 architect P3.2)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 18:35'
updated_date: '2026-05-25 18:58'
labels:
  - e2e-matrix
  - documentation
  - opacity-rot
  - forward-carried-from-TASK-0327
dependencies:
  - TASK-0327
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0327 cycle 149 lifted the DATA-side worker-to-worker `Push`/`Wait` gap for mp-tcp-event via host-relay (mirror of cycle-148's mp-tcp-bufsync slice). The original TASK-0175 filing combined the DATA arm + the CTRL arm (host-excluding barriers); cycle 148/149 split the lift into TASK-0327 (DATA, now Done) and TASK-0329 (CTRL, still gated).

Multiple `nuc-nucleus/e2e-matrix.toml` `[[skip]]` entries cite TASK-0175 as the blocker. Some of these citations are now opacity-rotted — they name a blocker the cycle-149 lift removed, in part or in full.

## Cycle-149 architect P3.2 disclosure (READ-ONLY review)

Per `feedback-opacity-gate-rot`: deferral facilities (skip reasons here) filed in cycle N that predate later precise-tracking machinery (cycle 148/149) become redundant or wrong. Audit needed.

## Already folded back in cycle 149 (P2.2 in-thread)

- Line 786-791 (05-stencil/distributed-2d × mp-tcp-event): updated to name TASK-0294 as the remaining blocker (host 2D slice-paste gather defect); cited TASK-0327 cycle 149 as the DATA-arm lift.
- Line 1082-1086 (09-producer-consumer/pipelined × mp-tcp-event): updated to name TASK-0329 as the remaining CTRL-arm blocker; cited cycle 149 lifting the DATA arm.

## Remaining audit scope (this task)

1. Line 985 (03-reduction/distributed × mp-tcp-event): skip reason cites TASK-0175 for a host-excluding barrier (CTRL arm). Accurate but should ALSO cite TASK-0329 for precision after the cycle-148/149 split.
2. Line 1143 (13-cnn-inference/pipeline_parallel × mp-tcp-event): same shape as line 985.
3. Any OTHER citation of TASK-0175 in e2e-matrix.toml — grep -n 'TASK-0175' nuc-nucleus/e2e-matrix.toml. Each citation should be reviewed:
   - Is the blocker DATA-arm? -> it's lifted by cycle 149 (TASK-0327). Update or promote the cell.
   - Is the blocker CTRL-arm (host-excluding barrier)? -> still gated. Add a TASK-0329 forward-link.
   - Is the blocker BOTH? -> only the CTRL arm remains.

## Acceptance criteria

### AC#1: full audit of TASK-0175 citations in e2e-matrix.toml

For every `[[skip]]` entry's `reason =` text or comment block citing TASK-0175, classify (DATA / CTRL / both / unclear) and update to reflect the post-cycle-149 truth. Where the blocker has been fully lifted, EMPIRICALLY VERIFY by promoting the cell to [[required]] and running `just e2e` (cycle 119 precedent for milestone-close empirical-verification).

### AC#2: do NOT promote a cell on prose alone

The architect P3.2 carries the cycle-148/149 cross-cycle lesson: a prose claim that a blocker is lifted is not sufficient. The empirical verification is the bit-identical e2e cell PASS. For each candidate-promotion cell, run `just e2e` post-promotion and verify.

### AC#3: file new tasks for the residual blockers found

If the audit discovers any blocker that was NOT TASK-0175 but was masked behind the TASK-0175 narrative (the 2D slice-paste defect that line 786-791 was masking is a precedent), file each as its own tracker entry.

## Honest scope

- LOW priority: skip-reason narratives are documentation, not behavioural. The cycle-149 cell flips that are CLEARLY lifted (06/distributed2 × mp-tcp-event) have already been promoted in-thread; this task is the cleanup.
- The audit is mechanical (grep + per-cell classification) but the empirical verification step makes the EFFORT bounded-but-non-trivial.

## Cross-reference

- TASK-0327 (cycle 148/149): the lift that triggered this opacity-rot.
- TASK-0329 (cycle 148): the sibling for the CTRL arm of the host-mediated star.
- `feedback-opacity-gate-rot` memory note.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 150 final state — Done after parallel review gate (with architect P1 + P2 fold-back)

### What landed

**AC#1 — full grep audit of TASK-0175 citations in e2e-matrix.toml**: 5 in-scope citations classified + updated. Of these, 4 had the cycle-148/149 CTRL/DATA split forward-linked to TASK-0329 (lines ~398-440 for 03/distributed × mp-tcp-bufsync; ~471-479 for 13/batch_parallel × mp-tcp-bufsync; ~575-584 narrative; ~989 for 03/distributed × mp-tcp-event; ~1126-1146 for 13/pipeline_parallel × mp-tcp-event). 1 (786-810 for 05/distributed-2d × mp-tcp-event) was attempted-promoted empirically — see AC#2.

**AC#2 — empirical promotion of fully-DATA-blocked cells**: ONE candidate, 05/distributed-2d × mp-tcp-event (cycle-149 prose claimed TASK-0294 was the sole blocker). Promoted [[skip]] → [[required]] and ran `just e2e`: cell FAIL/run at 32.4s (workers exit code 0 + run.sh reports failure). Inspecting the emitted code (`nucleus/target/e2e-matrix/run-96488-.../05-stencil__distributed-2d__mp-tcp-event/src/bin/{host,w0}.rs`):
- Workers begin with `chan_X.wait()` calls for cross-worker halo strips BEFORE any push.
- Host's relay starts with `relay_one(11)` = wait(seq=11) from w0; w0 hasn't pushed seq=11 because w0 is blocked at wait(seq=8) which is host's 5th relay hop.
- Even if `bar_0` weren't an issue, the sequential-ordered relay has a CIRCULAR seq dependency that deadlocks.
This is a new architectural limitation NOT previously filed: cycle-148's TASK-0327 implementation plan disclosed it vaguely ("complex interleaved schedules would need either threaded relay or scheduled relay events") but no follow-up task existed.

**Empirical-then-correct**: REVERTED the [[required]] back to [[skip]]; filed **TASK-0332** with the precise mechanism (architect P1 fold-back captured both the surface bar_0 framing + the underlying sequential-ordering mechanism). Updated 05/distributed-2d skip reason to cite TASK-0332. Two false attributions (cycle-149: TASK-0294; cycle-150 first attempt: TASK-0330) were retracted in-thread before close. TASK-0330's priority bump Low→Medium was REVERTED — the in-tree trigger does NOT match TASK-0330's "Loop-body" scope.

**AC#3 — sibling-file sweep (cycle-150 architect P2 fold-back)**: 4 sibling files outside e2e-matrix.toml updated:
- `nucleus/backends/mp-tcp-event/Cargo.toml:27` — TASK-0175 → TASK-0329 in module header.
- `nuc-nucleus/examples/13-cnn-inference/README.md:47` — same.
- `nucleus/driver/src/main.rs:44-46, :597` — both occurrences.
- `nuc-nucleus/examples/05-stencil/schedules/distributed-2d.sched.nuc:62` — TASK-0175 → TASK-0332 (the empirically-verified blocker, not TASK-0329).
- `nucleus/backends/mp-tcp-bufsync/src/lib.rs:376-380` — prose comment forward-linked to TASK-0329 + an explicit "do not update the ContractGap string below — test-pinned" note added (architect P2 mitigation discipline).
- `nucleus/backends/mp-tcp-event/src/multi_worker.rs:227-230` — same shape.

The 2 ContractGap string literals (bufsync `lib.rs:390`, mp-tcp-event `multi_worker.rs:239`) are intentionally LEFT as "TASK-0175" — they are test-pinned by `multi_worker_emit.rs` and `host_relay_emit.rs` for cross-backend differential stability. The architectural lineage forward-link is now in the surrounding comments.

### Parallel review gate

- **mped-architect** (read-only): initial NO-GO with P1 + P2 findings. ALL findings folded back in-thread before commit:
  - **P1 root-cause misattribution** (TASK-0330 vs TASK-0332): retracted TASK-0330 priority bump; filed TASK-0332 with architect's more precise mechanism (sequential-ordering circular seq dependency); updated 05/distributed-2d prose.
  - **P2 silent-sibling sweep too narrow**: 6 sibling files updated per architect's enumeration; ContractGap test-pinned strings explicitly carved out with clarifying comments.
  - **P3 minor**: doc rot at e2e-matrix.toml line 585 ("Three-way differential covers {pthreads-sync, pthreads-async}" — that's TWO, not three) acknowledged but deferred — out of cycle-150 scope.
  - **P3 minor**: 1-sample non-flake post-revert flagged; cycle 150 ran a 2nd sample after the architect's review (also 112/96/0/16/0) — non-flake confirmed.

### Honesty gaps disclosed

- **Three wrong-attributions before the right one** (cycle-149: TASK-0294 → cycle-150 first: TASK-0330 → cycle-150 second: TASK-0332). The empirical-verification step (read the actual emitted code) is the safety net; without it, narrative-rot would have iterated further.
- **2-sample non-flake on baseline** (acceptable per cycle-119 precedent for doc-only cycles, lower bar than the 3-sample milestone-close standard).
- **TASK-0330 is now demonstrated to have NO in-tree trigger today** (cycle-150 confirmed 05/distributed-2d's w2w pushes are top-level, not Loop-nested). Priority correctly returned to Low.

### Closure

**TASK-0331 status: DONE.** All 3 ACs landed. The empirical-promotion AC#2 + sibling-sweep AC#3 are completed; the audit pass discovered TASK-0332 as a NEW architectural limitation that cycle-149's host-relay design has on wait-before-push schedule shapes. TASK-0332 is now the explicit gate on 05/distributed-2d × mp-tcp-event (replacing the cycle-149 prose-only TASK-0294 claim).

### Forward-carry to future cycles

- **TASK-0332** (Medium, M6): the architectural follow-up — threaded or interleaved host-relay for wait-before-push schedules. AC#1 is the substantive fix; AC#2 (defensive ContractGap at Plan::build) is the minimum cycle-N+1 closure.
- **Memory note candidate**: `feedback-orchestrator-narrative-also-wrong` fired THREE times in 2 cycles for this same defect class (TASK-0294 → TASK-0330 → TASK-0332). The hygiene rule should evolve: when speculating about a remaining blocker post-cycle, NEVER write the prose without first inspecting the emitted code. The empirical-verification step costs a few minutes; the cumulative cost of three wrong attributions is much higher.
<!-- SECTION:NOTES:END -->
