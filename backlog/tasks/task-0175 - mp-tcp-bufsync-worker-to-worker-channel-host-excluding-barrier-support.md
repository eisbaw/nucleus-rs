---
id: TASK-0175
title: 'mp-tcp-bufsync: worker-to-worker channel / host-excluding barrier support'
status: To Do
assignee: []
created_date: '2026-05-19 00:52'
labels: []
dependencies:
  - TASK-0036
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
mp-tcp-bufsync (TASK-0036) uses a one-(data,ctrl)-connection-pair-per-(host,worker) STAR topology: every cross-worker transfer and every barrier is mediated through the host. This is sufficient for the tier-1 example set (02-split: {host,w0}) and is the deliberate M3 scope. A schedule with a worker-to-worker Push/Wait (peer != host) OR a barrier whose participant set excludes host currently fails LOUD with EmitError::ContractGap (data_conn_var / Plan::build) — it is NOT silently mis-routed. To support distributed placements (TASK-0117) with direct worker-worker edges, mp-tcp-bufsync needs a full connection mesh (or host-relayed forwarding) and a barrier protocol that does not require host as the hub. Referenced by code comments in nucleus/backends/mp-tcp-bufsync/src/lib.rs. Depends on TASK-0036; related to TASK-0117 (distributed placement) and TASK-0172 (stable Event::Sync identity).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 mp-tcp-bufsync establishes the connections needed for worker-to-worker Push/Wait (peer != host)
- [ ] #2 Barriers whose participants exclude host lower correctly (mesh or relayed), not ContractGap
- [ ] #3 A distributed-placement example (e.g. 03-reduction/distributed) is differentially green under mp-tcp-bufsync once TASK-0117 lands
<!-- AC:END -->
