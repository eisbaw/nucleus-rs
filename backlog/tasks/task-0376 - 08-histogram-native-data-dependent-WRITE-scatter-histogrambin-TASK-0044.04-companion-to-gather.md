---
id: TASK-0376
title: >-
  08-histogram native data-dependent WRITE (scatter) histogram[bin]
  (TASK-0044.04 companion to gather)
status: To Do
assignee: []
created_date: '2026-05-30 22:46'
updated_date: '2026-05-31 04:39'
labels:
  - compiler
  - scatter
  - histogram
  - broaden
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
BROADEN: the data-dependent WRITE / scatter sibling of the gather (TASK-0341.03.01 landed the READ). 08-histogram fakes histogram[bin] <-- bin_inc(...) with a rectangular masked accumulator over (i,b) gated on value==bin. The native form histogram[bin] <-- inc(histogram[bin]) where bin is a loaded value needs a data-dependent index in WRITE (LHS) position. Much harder than the gather READ: single-assignment is keyed on the base data name (not the index), so a scatter has write-conflict / fan-in semantics (multiple iterations writing the same bin) that the gather read does not. Scope: (1) admit a data-dependent LHS index in lowering (lower_indices already lowers lhs.indices via lower_index_expr with allow_gather true after TASK-0341.03.01 — verify the LHS path); (2) codegen the scatter write histogram[(bin) as usize] = ...; (3) the accumulation semantics (read-modify-write to the same bin across iterations) must be sound single-worker; distributed scatter is a further step. Companion to TASK-0044.04.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle-221 orchestrator depth-triage (read-only Explore investigation + 2 claims personally re-verified). VERDICT: backend/lowering half is ALREADY DONE; the blocker is GRAMMAR/expression, not codegen — re-scopes this task.

Backend-ready evidence:
- lower.rs:1130-1135 `lower_indices` passes allow_gather=true UNCONDITIONALLY to lower_index_expr (PERSONALLY VERIFIED). So a data-dependent index in WRITE (LHS) position already lowers (TASK-0341.03.01 wired the LHS path, not just the RHS gather).
- render/fire.rs:66 render_fire_output_assign renders the LHS index via classify_data_slice -> render_int_expr, which handles IrExpr::DataRef gather loads (expr.rs:71) (render_fire_output_assign PERSONALLY VERIFIED earlier this session). So histogram[bin[i]] = rhs would render today.
- single-assignment check is PER-SYMBOL not per-index (lower.rs:1033-1047, per Explore — not re-verified this turn): histogram[bin[i]] <-- inc(...) in a loop does NOT trip DoubleAssignment. Fan-in/accumulate is already modeled (cumulative-discriminator; histogram[b]<--bin_inc classified NOT cumulative, stays wrapping_add).

THE REAL BLOCKER (Explore #4): the algorithm DSL has no syntax for a computed local bin (`bin = compute_bin(input[i])`) — no local variables / scalar-producing statements inside a loop. So the textbook native scatter (compute bin, then histogram[bin]++) is GRAMMAR-BLOCKED — belongs with the grammar-extension cluster [[project-grammar-deferred-cluster]] (TASK-0179 / 0044.05.01 / 0044.06.01).

RECOMMENDED SCOPING for whoever picks this up (NOT done this cycle — fresh context):
- TRACTABLE bounded slice: the TOP-LEVEL-BIN-ARRAY form `histogram[bin[i]] <-- inc(histogram[bin[i]])` where `bin` is a top-level input data symbol (the direct LHS analog of spmv gather `x[col[k]]`). This MAY be expressible TODAY without grammar extension. OPEN QUESTION that needs care: histogram appears on BOTH sides with a data-dependent index (same-symbol RMW) — verify the cumulative-vs-accumulate discriminator classifies a DATA-DEPENDENT-index RMW correctly (the existing classification was validated for ITER-VAR index histogram[b], not data-dependent histogram[bin[i]]). Verify single-worker bit-identity across the 7 tier-1 backends via the e2e differential; distributed scatter is a further step.
- DEEPER alt: extend the DSL with computed locals (the textbook form) — grammar-extension epic.
This is NOT a clean bounded codegen cycle; it needs either a grammar decision or careful same-symbol-RMW-classification verification under the full e2e gate.
<!-- SECTION:NOTES:END -->
