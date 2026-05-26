---
id: TASK-0338
title: >-
  e2e-matrix.toml:1377-1378 stale doc-comment — '\''Only mp-tcp-event remains
  [[skip]]'\'' lies about current state (06/distributed2 cell is [[required]])
status: In Progress
assignee:
  - '@mark'
created_date: '2026-05-26 07:51'
updated_date: '2026-05-26 08:23'
labels:
  - feedback-comment-doc-lie-recurring
  - e2e-matrix
  - docs
  - cleanup
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

The doc-comment at nuc-nucleus/e2e-matrix.toml:1370-1378 (preamble of the 06-separable-filter / distributed2 block) reads:

> POST-CYCLE-148 (TASK-0327 first slice): mp-tcp-bufsync lifted via host-relay (data_conn_var routes non-host peer through data_host; HOST runs a synchronous 12-hop relay phase spliced between pass-1 barrier and pass-2 barrier). 3 of 4 tier-1 backends bit-identical on 06/distributed2 (pthreads-sync, pthreads-async, mp-tcp-bufsync). Only mp-tcp-event remains [[skip]], pending cycle-149 replication of the host-relay shape into the mio reactor.

But the very next data-cell block (lines 1411-1415) marks `06-separable-filter / distributed2 × mp-tcp-event` as `[[required]]`. The cycle-149 replication DID land (the lines 1397-1409 block describes it correctly: "cycle 149 mp-tcp-event applied the same DATA-arm host-relay shape"). The 1370-1378 preamble was not updated when the conclusion line stopped being true.

## Pattern

`feedback-comment-doc-lie-recurring` (per memory). A multi-claim narrative paragraph that was true at filing time is partially or wholly invalidated by a later cycle, but the narrative was not re-touched.

## Fix scope

Replace the "Only mp-tcp-event remains [[skip]]" sentence in lines 1377-1378 with a one-line update reflecting that cycle-149 replicated the host-relay shape and 4/4 tier-1 backends are now bit-identical on 06/distributed2. Mirror the cycle-149 narrative the lines 1397-1409 block already provides — the 1370-1378 preamble just needs the conclusion line corrected.

## Acceptance criteria

1. Lines 1377-1378 of nuc-nucleus/e2e-matrix.toml are updated to reflect current state (mp-tcp-event is [[required]] not [[skip]]).
2. The fix narrative cites cycle-149 (or a later more-precise cycle if known).
3. No other comments anywhere in e2e-matrix.toml make the same overclaim (sweep for sibling instances).

## Cross-reference

- Cycle 167 closure-cycle discovery (TASK-0042.05 closure audit).
- Memory: feedback-comment-doc-lie-recurring.
- Memory: feedback-orchestrator-narrative-also-wrong (orchestrator narratives in e2e-matrix.toml carry this risk).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 169 (orchestrator-direct hygiene)

### Edit applied

nuc-nucleus/e2e-matrix.toml lines 1372-1378: the POST-CYCLE-148 block now carries a follow-up POST-CYCLE-149 block (TASK-0327 second slice) stating the mp-tcp-event host-relay lift landed and 4/4 tier-1 backends are bit-identical on 06/distributed2. Refers the reader to the per-cell cycle-149 narrative at lines 1402-1409 (which was already accurate).

### Sibling sweep (AC#3)

Greped nuc-nucleus/e2e-matrix.toml for analogous overclaim phrasings:
- 'Only .* remains' — single hit (the line being fixed).
- 'remains \[\[skip\]\]' — single hit (the line being fixed).
- '3 of 4 tier-1' — single hit (same block, fixed in same edit).
- 'only one backend', 'the last [a-z]+ backend', 'pending cycle' — no hits.
- Line 1203 ('only remaining blocker is the CTRL-arm') is in a different block narrating 13-cnn-inference/pipeline_parallel × mp-tcp-event, and that block has cycle-160 and cycle-165 follow-up updates — NOT a doc-lie.

No sibling overclaims remain in e2e-matrix.toml.

### Verification

- toml parse: OK (python3 tomllib).
- just e2e: total 112, pass 102, fail 0, skipped 10, required-fail 0. Matches post-cycle-165 expected baseline (cycle 165 promoted 13-cnn-inference/pipeline_parallel × mp-tcp-event from skip → pass, taking the baseline from 112/101/0/11/0 → 112/102/0/10/0). Memory's stored baseline (cycle 163's 112/101/0/11/0) was stale by two cycles; updated implicitly.

### Forward-carried lesson

The 1372-1378 doc-lie was the conclusion line of a multi-claim narrative paragraph that was true at filing time and was invalidated by a follow-up cycle (cycle 149 replicated the host-relay shape into the mio reactor exactly as the line predicted, but the line was not re-touched to reflect that it had happened). The cycle-149 narrative was correctly added INSIDE the per-cell block (lines 1402-1409) but the preamble's predictive conclusion was not back-edited. **Forward-carried hygiene rule**: when adding a cycle-N narrative inside a per-cell block, ALSO scan the surrounding-block preamble for a predictive conclusion that cycle-N has now answered, and either delete the prediction or replace it with the outcome. The check is local (same TOML block) and cheap.
<!-- SECTION:NOTES:END -->
