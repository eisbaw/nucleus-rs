---
id: TASK-0368
title: >-
  Reconcile PRD section 8 Petri-net framing with production reality
  (check_bounded / check_deadlock_free are production-dead)
status: To Do
assignee: []
created_date: '2026-05-30 11:08'
labels:
  - docs
  - PRD
  - petri-net
  - honesty
  - cycle-213-followup
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-213 strategic-analysis finding (F3, honesty). VERIFIED: check_bounded (passes/boundedness.rs) and check_deadlock_free (passes/deadlock.rs) have ZERO production call sites — every non-test reference is a doc-comment. acfg_to_net runs ONLY under the --emit-pn inspection branch (driver/main.rs). Net soundness in the shipping compiler is enforced STRUCTURALLY (TtoP-arc elision + ad-hoc ACFG guards), NOT by the Petri analyses. PRD section 8 bills the Petri net as the central technical contribution with "analyses fall out as standard properties; failures are compile errors" — true of the TEST suite, not the shipping path. This is a PRD-vs-code framing gap. Cross-ref TASK-0219 (dead-code status accepted; no wire-in task filed). DECISION task: pick one and execute.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Decision recorded (in this task) between: (A) wire check_bounded + check_deadlock_free into the production compile pipeline as a real gate (acfg_to_net + analyses run on every build, failures => compile error), OR (B) downgrade PRD section 8 framing to "inspection/spec artifact; soundness enforced structurally" and update any other doc claiming the analyses are a shipping gate
- [ ] #2 The chosen option is executed: either the wire-in lands with a test proving an unbounded/deadlocking net is REJECTED at build time, OR the PRD + related docstrings are corrected and a grep shows no remaining "compile error / shipping gate" claim about the Petri analyses
<!-- AC:END -->
