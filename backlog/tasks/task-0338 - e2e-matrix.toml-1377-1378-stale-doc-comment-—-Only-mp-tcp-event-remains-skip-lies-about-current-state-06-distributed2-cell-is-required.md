---
id: TASK-0338
title: >-
  e2e-matrix.toml:1377-1378 stale doc-comment — '\''Only mp-tcp-event remains
  [[skip]]'\'' lies about current state (06/distributed2 cell is [[required]])
status: To Do
assignee: []
created_date: '2026-05-26 07:51'
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
