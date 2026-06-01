---
id: TASK-0409
title: >-
  Hardening: complete comment-doc-lie sweep of the 5 unswept large passes (+
  grep test-body/match-arm comments, not only ///)
status: To Do
assignee: []
created_date: '2026-06-01 08:57'
updated_date: '2026-06-01 09:27'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## forward-carried from TASK-0407 (cycle-236 dead-code/limitation review-pass)

Two items bearing on the continued comment-doc-lie sweep:

1. CONFIRMED doc-lie found + fixed in commit e38267f: backend-common/src/lib.rs:75-78 convenience-root-re-export comment claimed backends reach "the most-frequently used surface" via the crate root. Reality (grepped all consumer crates): of 35 root re-exports only 4 are consumed via the crate ROOT (EmitError, elect_host_from_name_workers, elect_host_from_worker_names, render_fire_args_nostd); the other 31 are reached via the SUBMODULE path. The comment asserted the opposite usage pattern. This is the SAME class as the TASK-0408 eval_const-attribution lie: a comment describing a usage/causation FACT that is empirically false. LESSON for 0409: comments asserting "X is the common/frequent/typical path" are CLAIMS -- grep the actual call sites and count before trusting; usage-frequency claims are as falsifiable as causation claims.

2. Gotcha that affects any allow(dead_code)/grep-based doc sweep: all 8 #[allow(dead_code)] in backends/mpi-blocking/src/multi_worker.rs (lines 351-381) and backends/mpi-nonblocking/src/multi_worker.rs (lines 433-494) live INSIDE emitted string-literal preludes (let prelude = "\ ... "; out.push_str(prelude)), as do every backend KERNELS_MOD_ATTR const. A docstring/comment audit must not treat the // comments inside those string literals as compiler-level docs -- they are emitted-code comments. Confirm by locating the enclosing string-literal boundary before classifying.
<!-- SECTION:NOTES:END -->
