---
id: TASK-0119
title: 'Transfer-injection: support conflicting sync/async options on one directive'
status: To Do
assignee: []
created_date: '2026-05-18 01:44'
labels:
  - M1
  - compiler
  - sched
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Currently policy_from_directive() lets the LAST option win when a schedule writes 'transfer D : sync, async;'. The schedule lowering pass already flags this as a linker concern (grammar §2 note 7). Either reject in schedule lowering or in link, before transfer_inject runs. Filed so the silent last-wins behaviour doesn't become a maintenance pothole.
<!-- SECTION:DESCRIPTION:END -->
