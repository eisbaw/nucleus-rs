---
id: TASK-0469
title: 'Adoption depth: from usable tool to a preferred language within its class'
status: To Do
assignee: []
created_date: '2026-06-13 10:39'
labels:
  - adoption
  - epic
  - production
  - ux
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Strategic epic. TASK-0455 took Nuc from falsification rig to usable tool; TASK-0453 closed the rigour gaps. Becoming the language an engineer reaches for by default within its class (regular, deterministic algorithms that must move across parallel and embedded targets) is a third, distinct bar, won mostly on diagnostics, editor tooling, and a flagship audience win, NOT on more backends or more examples.

Two preconditions for preferred: (1) the class boundary must be discoverable in seconds via teaching diagnostics, not after days of investment; (2) zero friction inside the boundary via IDE tooling and explainability. The system is correct and falsifiable but not yet pleasant, and pleasantness is what preferred is made of.

Sequencing bet (recorded, not a separate task): the embedded firmware engineer is the beachhead, not HPC. Nothing else offers compile-time-deadlock-free multi-MCU codegen with co-simulated byte-exactness; the competition (hand-written C, Embassy) is weakest there; static scheduling is already the norm in that world so the class restriction costs least; and a buffer overrun becoming a compile error rather than a bricked field device is the strongest value story. HPC has entrenched alternatives and demands the perf study first.

Explicitly OUT (a different system wearing this one's clothes): GPU/NPU tier, full sparse solvers, dynamic scheduling/work-stealing, multi-objective prescriptive scheduling.

Children group the depth axes: editor tooling + explainability (0469.01-03), production hardening of generated code (0469.04), HPC perf credibility + tier-2 codegen depth (0469.05, 0469.07), numerics breadth (0469.06), and the embedded-beachhead capstone (0469.08). Actionable fix-hint diagnostics live under the existing diagnostics home TASK-0455.06.04.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 All child tasks created and dependency-ordered
- [ ] #2 Embedded-beachhead sequencing reflected in child priorities (embedded + diagnostics + tooling ahead of HPC perf)
- [ ] #3 No child duplicates an existing open task; each references the prior art it extends
<!-- AC:END -->
