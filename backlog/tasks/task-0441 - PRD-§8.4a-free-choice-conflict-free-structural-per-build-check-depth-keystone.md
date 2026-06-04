---
id: TASK-0441
title: >-
  PRD §8.4(a) free-choice / conflict-free structural per-build check (depth
  keystone)
status: To Do
assignee: []
created_date: '2026-06-04 08:16'
labels: []
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Per WIP review (2026-06-04). PRD §8.4(a) names the free-choice / conflict-free restriction as the KEYSTONE tractability assumption of the Petri-net soundness model. TASK-0421 exists as a stub but there is NO per-build check enforcing this. This is the only PRD-named tractability restriction without a per-build check.

Scope: implement 'check_free_choice(&PetriNet) -> Result<(), NetSoundnessError>' in nucleus-compiler/src/passes/net_soundness.rs. Wire into the existing per-build check_net_sound aggregator. Add ~3 bite tests including a synthetic conflict net (two transitions sharing a single input place, both enabled). ~200-400 LoC.

Why: Cleanest available DEPTH win - closes a named PRD obligation with concrete code. A reviewer asking 'how do you enforce §8.4(a)?' currently gets prose, not code.

Dependencies: Reuse the existing PetriNet types + CONFLICT_BFS_TRANSITION_LIMIT precedent. Reference TASK-0421 + TASK-0427.01.

Estimated effort: MEDIUM priority, 1 cycle if shallow OR 2 cycles if structurally non-trivial.
<!-- SECTION:DESCRIPTION:END -->
