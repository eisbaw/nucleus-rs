---
id: TASK-0118
title: Capability-matrix check on TransferPolicy at codegen time
status: To Do
assignee: []
created_date: '2026-05-18 01:44'
labels:
  - M1
  - compiler
  - codegen
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Per TASK-0018 spec, transfer_inject deliberately does NOT validate that the chosen backend can satisfy a TransferPolicy (async, buffer>1, notify=event). The backend isn't picked at the pass. Once a backend with a capabilities.toml is wired through to the codegen pass, walk every XferPlaceholder and reject combinations the backend lacks. PRD §6.3.4 says this must be a hard error, not a silent fallback. Errors must name the offending data symbol, the requested option, and the backend.
<!-- SECTION:DESCRIPTION:END -->
