---
id: TASK-0376
title: >-
  08-histogram native data-dependent WRITE (scatter) histogram[bin]
  (TASK-0044.04 companion to gather)
status: Done
assignee:
  - mark
created_date: '2026-05-30 22:46'
updated_date: '2026-05-31 05:05'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Bounded slice: native data-dependent WRITE (scatter) `histogram[input[i]] <-- inc(histogram[input[i]])` for 08-histogram, single-worker, mirroring the 17-spmv/gather READ template (TASK-0341.03.01).

DEPTH-VERIFIED before any edit (read-only):
1. LHS gather index lowers: lower.rs:1051 `lower_indices` -> lower_index_expr(allow_gather=true) -> lower_data_ref produces IrExpr::DataRef for `input[i]` in LHS subscript. Single-assignment (lower.rs:1036) keys on SYMBOL `histogram` (one statement, fired once at lower time) -> no DoubleAssignment.
2. LHS render: render_fire_output_assign -> classify_data_slice (1 index, dims=[BINS], len 1) -> SliceForm::Scalar -> index via render_int_expr(DataRef) -> render_gather_index_load -> `input[(i) as usize]`. Emits `histogram[(input[(i) as usize]) as usize] = kernels::inc(...);`. RHS arg same path.
3. KEY RISK (same-symbol RMW discriminator): sidecar.rs collect_cumulative_data_names::rhs_self_read_differs compares `r.indices != lhs.indices` (IrExpr derives structural PartialEq). For scatter both sides carry IDENTICAL `DataRef(input[i])` -> indices EQUAL -> NOT cumulative -> stays wrapping_add accumulate fan-in (same class as histogram[b], NOT jacobi cumulative). This is correct: single-pass disjoint-ish accumulate, additive identity pre-init.
4. Pre-init: collect_pre_init_data/walk_fire_outputs keys on output.indices.is_empty(); scatter output indices=[DataRef] (non-empty) -> indexed-write -> histogram pre-init `vec![0; BINS]`. Recognized regardless of data-dependent vs iter-var index.

BUILD:
- prog.scatter.algo.nuc: data input:i32[N]; data histogram:i32[BINS]; kernel inc:(i32)->i32 pure; load_input/save_output effectful. Body: for i:0..N { histogram[input[i]] <-- inc(histogram[input[i]]); }. Honest header (no verbatim gather-claim copy).
- kernels.scatter.rs: self-contained (dup load_input/save_output/consts from kernels.rs); inc(acc)=acc.wrapping_add(1) (mirror bin_inc overflow contract).
- schedules/scatter.sched.nuc: schedule for "../prog.scatter.algo.nuc" { workers={host}; place load_input/inc/save_output on host; }.
- e2e-matrix.toml: 7 [[required]] 08-histogram/scatter cells (one per tier-1 backend), mirroring 17-spmv/gather. reference.bin is shared per-example (e2e/src/main.rs:1474) -> existing reference.bin is the correct oracle, do NOT regenerate.
- README.md: fix the now-FALSE "Why not histogram[input[i]]" claim (lines 121-125) -> point to the scatter variant (doc-honesty).

GATE: nix develop --command bash -c "just build && just clippy && just test && just e2e". scatter cell must be bit-identical across the 7 tier-1 backends vs reference.bin (expect totals 329/272/0/57/0 -> 336/279/0/57/0 if all 7 pass). DROP on pre-existing cells = hard fail.

FOLLOW-UPS to file: (a) distributed scatter (partitioned data-dependent WRITE + cross-worker bin fan-in); (b) grammar-extension computed-local-bin form (PRD §6.2.4); (c) any discriminator/pre-init gap found.
<!-- SECTION:PLAN:END -->

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

CYCLE-222 IMPLEMENTED (bounded single-worker slice). Native scatter histogram[input[i]] <-- inc(histogram[input[i]]) lands + codegens + runs BIT-IDENTICAL vs reference.bin across all 7 tier-1 backends. e2e: 329/272/0/57/0 -> 336/279/0/57/0 (+7 = the 7 08-histogram/scatter cells, ALL PASS bit-identical). No pre-existing-cell regression. just build/clippy/test green; doc-citation, narrative-doc-lie, include-str-coverage, textual-replace structural checks green.

EXACT EMITTED SCATTER (pthreads-sync, tmp/scatter-out/src/main.rs, standalone build):
  let mut histogram = vec![0; 16];
  let mut input = kernels::load_input();
  for i in (0_i64)..(256_i64) {
      histogram[(input[(i) as usize]) as usize] = kernels::inc(histogram[(input[(i) as usize]) as usize]);
  }
  kernels::save_output(histogram);
Data-dependent LHS index (scatter store) AND same-symbol RHS gather read, both via render_int_expr(IrExpr::DataRef)->render_gather_index_load. No `for b`, no mask. Standalone output bit-identical to reference.bin [25,25,24,24,24,23,23,9,9,10,10,10,10,10,10,10] sum=256=N.

KEY-RISK RESOLVED — same-symbol-RMW discriminator. collect_cumulative_data_names::rhs_self_read_differs (sidecar.rs:771) tests `r.name == lhs.name && r.indices != lhs.indices`. For scatter BOTH sides carry the STRUCTURALLY-IDENTICAL index DataRef(input[i]); IrExpr derives PartialEq (ir.rs:170, structural). So r.indices == lhs.indices -> the `indices differ` branch does NOT fire -> histogram is NOT cumulative -> stays the wrapping_add ACCUMULATE fan-in (same class as the masked histogram[b], NOT the SHIFTED-self-read jacobi/game-of-life cumulative class). Empirically confirmed value-correct: scatter output == reference.bin byte-for-byte. The discriminator's defensive index-descent (rhs_self_read_differs recursing into r.indices) correctly returns false for the inner input[i] (name `input` != `histogram`). NO discriminator change needed; NO (c) follow-up.

PRE-INIT verified: collect_pre_init_data/walk_fire_outputs (pthreads-sync lib.rs:435) keys on bindings.output.indices.is_empty(). Scatter output indices=[DataRef(input[i])] is NON-empty -> classified indexed-write; histogram never whole-array written (save_output is an Effect read) -> pre-init `vec![0; 16]`. The data-dependent index is recognized as indexed-Fire identically to an iter-var index. Confirmed in emitted code.

FILES: nuc-nucleus/examples/08-histogram/{prog.scatter.algo.nuc, kernels.scatter.rs, schedules/scatter.sched.nuc} (new); nuc-nucleus/e2e-matrix.toml (+7 [[required]] scatter cells, mirroring 17-spmv/gather); README.md (fixed the now-FALSE 'algorithm language only allows loop-variable indices on LHS' claim — true at cycle 186, false after TASK-0341.03.01 lifted the lowering gate; added Native scatter section + schedule-table row). LHS-GATHER-PATH NOW PROVEN: 17-spmv/gather only exercised a data-dependent RHS index reading a DIFFERENT symbol with an iter-var LHS; this is the FIRST proof of (a) a data-dependent LHS scatter index and (b) same-symbol data-dependent RMW. Forward-carry to TASK-0384/0385.

FOLLOW-UPS FILED: TASK-0384 (distributed scatter: partitioned data-dependent WRITE + cross-worker bin fan-in, WRITE analog of deferred distributed gather); TASK-0385 (grammar-extension computed-local-bin / scalar-producing loop-body statement for the textbook bucketing scatter — same bottleneck as project-grammar-deferred-cluster; kernel-call-in-index-position also rejected at lower_index_expr Expr::Call + render_int_expr IrExpr::Call). HONEST LIMITS: single-worker ONLY; works only because input.bin is pre-clipped to [0,BINS) so input[i] IS a valid bin index (no bucketing). Pre-existing README staleness (lines 15/35-39/198-200: distributed 'STRETCH/[[skip]]' narrative) left untouched — predates this task, describes TASK-0343's distributed work which IS now [[required]]; out of TASK-0376 scope, NOT silently rewritten to avoid a self-introduced doc-lie about an unverified-this-cycle feature.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE (bounded single-worker slice) — cycle 222, commit e89aa1e.

Native data-dependent WRITE (scatter) `histogram[input[i]] <-- inc(histogram[input[i]])` for 08-histogram now lowers + codegens + runs BIT-IDENTICAL vs the existing reference.bin across all 7 tier-1 backends (pthreads-sync, mp-tcp-bufsync, pthreads-async, mp-tcp-event, openmp-rs, mp-tcp-poll, mp-uds-event), single-worker. This is the WRITE/LHS analog of 17-spmv/gather (TASK-0341.03.01, the data-dependent READ) and the FIRST proof of (a) a data-dependent LHS scatter index and (b) a same-symbol data-dependent read-modify-write (gather only exercised a RHS index on a DIFFERENT symbol with an iter-var LHS).

GATE: just build/clippy/test all green; e2e 329/272/0/57/0 -> 336/279/0/57/0 (+7 = the 7 08-histogram/scatter cells, all PASS bit-identical, no pre-existing-cell regression). doc-citation-staleness / narrative-doc-lie / include-str-coverage / textual-replace structural checks green. (Full `just ci` determinism/xbackend negative arms left for the read-only review gate per the brief.)

EMITTED (pthreads-sync): `histogram[(input[(i) as usize]) as usize] = kernels::inc(histogram[(input[(i) as usize]) as usize]);` with `let mut histogram = vec![0; 16];` pre-init. No `for b`, no mask — O(N) vs the masked variant's O(N*BINS).

KEY-RISK RESOLVED (same-symbol-RMW discriminator): rhs_self_read_differs compares r.indices != lhs.indices structurally (IrExpr derives PartialEq); the scatter's two identical DataRef(input[i]) indices are equal, so histogram is NOT classified cumulative -> stays the wrapping_add accumulate fan-in -> value-correct (== reference.bin byte-for-byte). NO discriminator/pre-init change was needed.

FILES: prog.scatter.algo.nuc + self-contained kernels.scatter.rs (inc=wrapping_add(1)) + schedules/scatter.sched.nuc (new); e2e-matrix.toml (+7 [[required]] cells); README.md (fixed the now-false "loop-variable indices only on LHS" claim + added a Native scatter section/row).

FOLLOW-UPS: TASK-0384 (DISTRIBUTED scatter — partitioned data-dependent WRITE + cross-worker bin fan-in) and TASK-0385 (grammar-extension computed-local-bin / scalar-producing loop-body statement for the textbook bucketing scatter, same bottleneck as project-grammar-deferred-cluster).

HONEST LIMITS: single-worker ONLY; works because input.bin is pre-clipped to [0,BINS) so input[i] IS a valid bin index (no value->bin bucketing — that needs TASK-0385's grammar work or a kernel). Pre-existing README distributed-"STRETCH/[[skip]]" narrative (lines ~15/35-39/198-200) left untouched — it predates this task and belongs to TASK-0343's distributed work (now [[required]]); not rewritten to avoid a self-introduced doc-lie about a feature not re-verified this cycle.
<!-- SECTION:FINAL_SUMMARY:END -->
