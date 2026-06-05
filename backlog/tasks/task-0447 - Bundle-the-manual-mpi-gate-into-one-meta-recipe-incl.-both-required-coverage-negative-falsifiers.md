---
id: TASK-0447
title: >-
  Bundle the manual mpi gate into one meta-recipe (incl. both required-coverage
  negative falsifiers)
status: Done
assignee: []
created_date: '2026-06-04 23:58'
updated_date: '2026-06-05 16:55'
labels:
  - M7
  - M8
  - validation
  - mpi
  - test-hardening
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0446 review (mped-architect P2): the new required-coverage-check-negative-mpi arm is a WEAKER 'standing' guarantee than the tier-1 sibling — the tier-1 required-coverage-check-negative runs inside 'just ci' every cycle, but the mpi arm is out-of-default-ci (needs .#mpi) so it bites only when a human invokes it under .#mpi. Residual: a refactor severing the mpi-tier required-coverage hard-fail still ships green under 'just ci' plus a forgotten manual step. This matches how the entire M7/M8 mpi tier is treated (manual gate, like e2e-mpi/check-mpi/check-mpi-nonblocking), so it is consistent and honest — but the residual standing-bite gap should be tracked. PROPOSAL: add a meta-recipe (e.g. 'just mpi-gate') under .#mpi that runs, in one command: check-mpi + check-mpi-nonblocking + e2e-mpi + required-coverage-check-negative + required-coverage-check-negative-mpi (the falsifiers included), so 'run the mpi tier' is one command that includes its own negative arms. Alternatively/additionally: if an mpi-CI lane is ever added, it MUST run both required-coverage negative arms. LOW priority (the mpi gate is run rarely; the unit test + the standing recipe both exist; this is about discoverability + bundling, not a missing check).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation (orchestrator-direct — justfile/harness infra; implementer subagents refuse these per feedback-spawned-agents-refuse-code-edits + TASK-0444/0445/0446 precedent; read-only review gate run by orchestrator preserves discipline). Added `mpi-gate` meta-recipe to justfile (after required-coverage-check-negative-mpi, whose comment already forward-referenced TASK-0447). Composition (just prerequisite chain, mirrors tier-3 renode-multimcu-gate): mpi-gate: required-coverage-check-negative required-coverage-check-negative-mpi check-mpi check-mpi-nonblocking e2e-mpi + @echo success body. Cheap negative falsifiers ordered FIRST (fail-fast on a severed coverage guard before heavy rsmpi cross-builds). Each arm self-contained: 4 mpi arms self-enter .#mpi; tier-1 falsifier runs in default shell (also in just ci). NOT added to just ci (needs .#mpi, same out-of-default rule as its arms) — confirmed ci dep-chain unchanged. Smoke recipes (check-mpi-smoke/check-mpi-barrier-smoke) deliberately excluded (standalone exploratory, not in the task scope list). No duplicate mpi-gate pre-existed.

Cycle orchestrator INDEPENDENT review gate (parallel, read-only) — both GO. Landed SHA 4c346e4 (justfile +27, additive; one recipe).
- Heavy gate run ONCE by orchestrator (efficiency mandate): full just mpi-gate exit 0 GREEN, log /tmp/mpi-gate.log. Fail-propagation proven separately via a throwaway probe (a failing arm aborts the gate RED before later arms).
- qa-test-runner: GO. justfile parses; just --show/-n resolves all 5 arms in order (no unknown-recipe error); git show: justfile-only +27/0-deletions, no existing recipe touched; ci chain UNCHANGED (mpi-gate not a ci dep -> just ci GREEN by construction, not re-run per mandate); captured log GREEN: MPI_GATE_EXIT=0, tier-1 falsifier GAP_DETECTED=1 + correctly bit, mpi-tier falsifier GAP_DETECTED=1 + correctly bit (--with-mpi), check-mpi 6 naive + 4 multi-worker byte-exact, check-mpi-nonblocking 4 async x {default,rendezvous} byte-exact, e2e-mpi total 7/pass 7/fail 0/skipped 0/required-fail 0; no genuine FAIL/panic.
- mped-architect: GO. All 5 prerequisites are real recipes; ordering (cheap falsifiers first) matches comment + log; fail-propagation sound (mirrors renode-multimcu-gate just-prereq semantics); EVERY load-bearing comment claim verified true (4 mpi arms self-enter .#mpi; tier-1 falsifier default-shell + in ci; mpi-gate NOT in ci); smoke-recipe exclusion defensible (task scope); captured log honest (no cherry-pick); orchestrator-direct consistent with TASK-0444/0445/0446 precedent; no AC-gaming.
- P3 (informational, non-blocking, already documented in the recipe comment): mpi-gate is not itself in a standing CI lane (the whole mpi tier is manual by project policy; this task closes the DISCOVERABILITY gap, not the merge-blocking gap — the comment forward-references that if an mpi CI lane is added it MUST invoke this recipe). No action.
DELIVERABLE MET: one reproducible just mpi-gate bundles check-mpi + check-mpi-nonblocking + e2e-mpi + both required-coverage negative falsifiers, fail-loud; demonstrated fail-then-pass (falsifiers bit live) + fail-propagation (failing arm aborts). Closes TASK-0446 P2. TASK-0447 DONE.
<!-- SECTION:NOTES:END -->
