---
id: TASK-0409
title: >-
  Hardening: complete comment-doc-lie sweep of the 5 unswept large passes (+
  grep test-body/match-arm comments, not only ///)
status: To Do
assignee: []
created_date: '2026-06-01 08:57'
labels:
  - hardening
  - doc-lie
  - review-pass
  - cycle-237-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0408 (cycle-237) was a bounded 10-claim spot-check; ~66 grep hits across the high-traffic passes remain unverified. UNSWEPT modules: halo_inference.rs (132KB, largest), reuse_inference.rs (65KB), host_data_relay_inject.rs (54KB), acfg_to_petri.rs/petri_to_events.rs bodies, event_plan/plan.rs claims beyond the grep listing. METHOD REFINEMENT (load-bearing, from TASK-0408 cb5fc51 fold-back): the comment-doc-lie sweep MUST grep test-body comments AND match-arm comments, NOT only /// docstrings -- the architect found a 4th eval_const-conflation sibling hiding in a #[test] body comment (common.rs:650) that the /// -only grep could not see. Extend the keyword recipe to // comments too. HONEST EXPECTED YIELD: LOW -- TASK-0408 found 9/10 claims TRUE and characterised the swept modules as in genuinely good shape (defect density is narrative-WHY accuracy, lower than expected; staleness fences saturated). File-and-defer so the avenue is durably recorded, not lost; pick up in a fresh context when higher-leverage work is exhausted. Sibling of TASK-0407 (dead-code audit) -- both are the remaining review-pass endgame dimensions.
<!-- SECTION:DESCRIPTION:END -->
