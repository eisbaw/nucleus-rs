---
id: TASK-0331
title: >-
  e2e-matrix.toml skip-reason audit post-cycle-149: opacity-rot of TASK-0175
  citations after DATA-arm lift (TASK-0327 cycle-149 architect P3.2)
status: To Do
assignee:
  - '@mark'
created_date: '2026-05-25 18:35'
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
