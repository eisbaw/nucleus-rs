---
id: TASK-0421
title: >-
  PRD §8.4(a) enforcement gap: no free-choice/conflict-free structural gate on
  the emitted net (the one tractability-restriction with no per-build check)
status: To Do
assignee: []
created_date: '2026-06-02 02:25'
labels:
  - M6
  - compiler
  - petri
  - fail-loud
  - prd-invariant-audit
  - cycle-241
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD-invariant audit (cycle-241) finding GAP-1, VERIFIED. PRD §8.4 names FOUR restrictions that keep the net tractable: (a) statically-determined firing order / NO free-choice, NO confusion, NO conflicts; (b) bounded-by-construction; (c) acyclic event DAG; (d) no coloured/stochastic/hierarchical. (b) and (c) have real per-build gates (check_bounded / check_deadlock_free, run every build at driver/src/main.rs:626, both bite-tested). (d) is a TYPE-LEVEL guarantee (Marking=BTreeMap<PlaceId,u32>; no colour/time/probability fields in petri.rs Place/Transition/Arc). But (a) — the precondition the WHOLE single-order-replay soundness argument explicitly rests on (cited verbatim at passes/net_soundness.rs:30-31 and petri.rs:14) — has ZERO asserting code. VERIFIED: grep of nucleus-compiler/src for free.?choice|conflict.?free|conflicting|confusion returns only docstring prose + the UNRELATED SchedLowerErrorKind::ConflictingTransferMode. So the soundness PRECONDITION is assumed-by-construction with no tripwire: a future inject-pass change producing a net where a place feeds two distinct enabled transitions (a free-choice conflict) would silently violate the precondition, and the boundedness/deadlock gates would still pass on their one arbitrary order while the real net is nondeterministic.

SCOPE: add a structural soundness check (e.g. check_statically_ordered / check_conflict_free) that verifies the emitted net actually has the §8.4(a) shape; fail-loud as a new typed error (e.g. PetriAnalysisError::ConflictingChoice { place, transitions }); wire into check_net_sound so it runs EVERY build (mirroring check_bounded/check_deadlock_free). Bite-test with a hand-built free-choice net (mirror the existing tests/net_soundness.rs unsound-net reject pattern).

CRITICAL DESIGN (panic-on-valid-input risk, TASK-0419 precedent): the conflict-free PREDICATE must be defined correctly for v2 statically-ordered nets and MUST NOT false-reject any shipping net. A place feeding >1 transition is NOT automatically a violation if the static firing order serialises them. MANDATORY PRECONDITION: empirically verify NO net in the e2e corpus (385 cells) trips the new check before wiring it into the gate (instrument + run, like TASK-0419 AC#1). If a legitimate multi-out-arc shipping net exists, the predicate must account for the static-order serialisation, not naively reject. 

EXPECTED DISPOSITION: like check_bounded/check_deadlock_free and TASK-0281/0419, this is likely a PROVABLY-DEAD-TODAY tripwire (net_soundness.rs:80-85 argues the inject passes make conflicts structurally impossible today) — that is the accepted project pattern (a conservative soundness tripwire), and the asymmetry that (a) is the ONE §8.4 restriction with no gate is the argument for filing it. Honest value: MEDIUM (closes the §8.4 enforcement set; makes the soundness precondition CHECKED not assumed).

Pointers: nucleus/nucleus-compiler/src/passes/net_soundness.rs (the check_net_sound aggregator + the soundness-precondition prose at :30 and :80-85); src/passes/boundedness.rs + deadlock.rs (the sibling gate pattern to mirror); src/petri.rs (Net/Place/Transition/Arc structure); tests/net_soundness.rs (bite-test template).
<!-- SECTION:DESCRIPTION:END -->
