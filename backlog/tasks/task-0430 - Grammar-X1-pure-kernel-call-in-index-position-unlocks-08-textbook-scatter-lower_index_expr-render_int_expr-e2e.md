---
id: TASK-0430
title: >-
  Grammar X1: pure-kernel-call in index position unlocks 08 textbook scatter
  (lower_index_expr + render_int_expr + e2e)
status: Done
assignee:
  - '@me'
created_date: '2026-06-02 23:12'
updated_date: '2026-06-03 00:49'
labels:
  - compiler
  - grammar
  - scatter
  - grammar-extension-epic
dependencies:
  - TASK-0385
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-246 design-slice outcome (read-only Plan-agent investigation, orchestrator-verified against code). The cleanest resolution of TASK-0385 textbook-scatter need is NOT a new IrStmt::Let local binding but X1: admit a PURE kernel call in SUBSCRIPT index position (histogram[bucket(input[i])] <-- inc(...)), reusing the existing gather machinery (TASK-0341.03.01). Far smaller blast radius than a Let (no new AST/IR node; one lowering arm + one shared render arm; single-assignment/scope untouched). A Let, if ever wanted, can be later defined as sugar that desugars to this. VERIFIED code sites: lower_index_expr Expr::Call rejection at nucleus/nucleus-compiler/src/algo/lower.rs:1179 (NonIntegerShapeExpr kernels-not-allowed-here); render_int_expr IrExpr::Call rejection at nucleus/backend-common/src/render/expr.rs:72 (EmitError::UnsupportedFeature). The adjacent gather paths (lower.rs:1191-1202 allow_gather DataRef + render_gather_index_load render/expr.rs:71) are the machinery to mirror. Risk: LOW (additive, gated on pure-kernel callee, subscript-only; loop-bound position keeps rejecting -> const-bound rule (c) intact).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 lower_index_expr admits Expr::Call iff allow_gather AND callee is a declared PURE kernel; lowers to IrExpr::Call; effectful kernel rejected; loop-bound position (allow_gather=false) still rejects. Unit tests: positive + 2 negatives.
- [x] #2 render_int_expr IrExpr::Call arm emits callee(<rendered args>) as the integer index (recurse render_int_expr for scalar args, render_gather_index_load for data-ref args); silent-sibling sweep confirms no per-backend independent IrExpr::Call index rejection beyond the shared backend-common arm.
- [x] #3 08-histogram textbook variant (prog.textbook.algo.nuc with a pure bucket kernel over UNCONSTRAINED input + a single-worker schedule + reference oracle) emits bit-identical across the 7 tier-1 backends; new e2e cell added; e2e total baseline bumped + recorded in commit msg (cumulative/accumulate classification re-verified for the bucket(input[i]) self-read form).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Implementation Plan (cycle-247, orchestrator-in-thread)

ONBOARDING-VERIFIED CODE SITES (all confirmed against tree):
- lower.rs:1179 Expr::Call blanket reject (NonIntegerShapeExpr); adjacent gather path lower.rs:1186-1212 (allow_gather && ir.data.contains_key -> lower_data_ref). lower_rvalue Expr::Call arm (lower.rs:1235) is the kernel-call lowering template (UnknownIdent check + recurse args).
- render/expr.rs:72 IrExpr::Call reject (UnsupportedFeature); adjacent render_gather_index_load (expr.rs:88). Call site spelling kernels::{callee}({args}) per event_walker.rs:120.
- Purity: ResolvedKernel.purity (ir.rs:134); effectful-RHS reject template is the Effect arm lower.rs:1090 (purity==Pure -> EffectCalleeNotEffectful). For index position we need the INVERSE: reject NON-pure callee.

AC#1 LOWERING: replace lower.rs:1179 Expr::Call arm. If !allow_gather -> keep rejecting (loop-bound rule c intact). If allow_gather: resolve callee in ir.kernels; UnknownIdent if absent; if purity!=Pure -> typed reject (new LowerErrorKind or reuse NonIntegerShapeExpr w/ effectful reason); else lower each arg via lower_index_expr (recurses: scalar args affine, data-ref args take the gather DataRef path already in this fn) -> IrExpr::Call{callee,args}. Tests: positive subscript pure-call; neg1 effectful-in-index; neg2 pure-call-in-loop-bound still rejected.

AC#2 RENDER: render/expr.rs:72 IrExpr::Call arm -> emit kernels::{callee}({rendered_args}); recurse render_int_expr per arg (DataRef args hit the existing gather arm at expr.rs:71 -> render_gather_index_load). Surrounding subscript already applies as usize. SILENT-SIBLING SWEEP DONE: grep kernel-call-inside-integer-index + IrExpr::Call across backends/ + backend-common/ -> ONLY production reject is expr.rs:72 (group.rs:189 is a passthrough clone not a reject; render_const_expr:201 is loop-bound and stays rejected correctly). Report verbatim in final notes.

AC#3 EXAMPLE+CELL: prog.textbook.algo.nuc with pure kernel bucket:(i32)->i32 = ((v%BINS)+BINS)%BINS, body for i:0..N { histogram[bucket(input[i])] <-- inc(histogram[bucket(input[i])]); }. kernels.textbook.rs (self-contained, +bucket). schedules/textbook.sched.nuc single-worker (place bucket on host). 7 e2e cells (schedule=textbook) all tier-1 backends. SHARED-ORACLE NOTE: input.bin is pre-clipped [0,16); modulo bucket is a runtime no-op for THIS fixture so reference.bin is bit-identical (VERIFIED py: bucket%16 hist == reference.bin). Feature tested = compile+codegen of pure-call-in-index + value-correctness vs oracle; the UNCONSTRAINED-input strength is weakened by the shared fixture -> file follow-up for a dedicated unconstrained-input example dir.

ACCUMULATE RE-VERIFICATION (design honest-limit): sidecar.rs:771 rhs_self_read_differs compares r.indices != lhs.indices (Vec<IrExpr> structural eq). For textbook both LHS+self-read indices are [Call{bucket,[DataRef(input[i])]}] -> structurally EQUAL -> NOT cumulative -> stays wrapping_add accumulate fan-in, SAME class as bounded scatter. bucket() wrapper does NOT change classification (identical on both sides). Will CONFIRM empirically via e2e oracle byte-compare (deadlock-free != value-correct).

GATE: nix develop -c just build && clippy && test && test-release && e2e; then just ci. Report OLD 385/328/0/57/0 -> NEW totals; confirm fail=0 required-fail=0 no prev-pass regressed.

## OUTCOME (cycle-247, orchestrator-in-thread) — all 3 ACs met, gate green, value-correct vs oracle

LANDED (4 commits):
- c9632f2 compiler-core: lower_index_expr Expr::Call arm admits PURE kernel call in subscript position (allow_gather=true), rejects loop-bound call + effectful callee (typed, not panic). +3 algo_lower tests.
- 998c527 backend-common: render_int_expr IrExpr::Call arm emits kernels::callee(args); args recurse (data-ref->gather). Replaced the old fail-loud pin with a positive render pin + arg-recursion pin; module-header reachability prose corrected.
- fdcda58 example+e2e: prog.textbook.algo.nuc + kernels.textbook.rs (bucket=((v%BINS)+BINS)%BINS) + schedules/textbook.sched.nuc (single-worker) + 7 e2e cells.
- e8e33eb tracker-xref: TASK-0431/0432 referenced in code comments.

AC#2 SILENT-SIBLING SWEEP (verbatim grep -rn "kernel call inside an integer index|IrExpr::Call" nucleus/backends/ nucleus/backend-common/):
- backend-common/src/render/expr.rs: the ONE production index-position Call arm (now emitting). render_const_expr arm (loop-bound) STILL rejects Call - correct, consistent with lowering loop-bound rejection.
- backend-common/src/render/group.rs:189 IrExpr::DataRef|Call => e.clone() — a PASSTHROUGH (abs_subst strip-mine walker leaves Call inert), NOT a reject.
- ZERO per-backend (backends/) independent Call-in-index handling. All 7 tier-1 backends consume the single shared render_int_expr arm. Confirmed empirically: identical emitted scatter line across all 7 backends + 7 PASS cells.

ACCUMULATE-CLASSIFICATION RE-VERIFICATION (design honest-limit): the bucket() wrapper does NOT change the classification. sidecar::collect_cumulative_data_names::rhs_self_read_differs compares r.indices != lhs.indices (structural IrExpr eq). For textbook BOTH LHS and self-read carry the IDENTICAL [Call{bucket,[DataRef(input[i])]}] -> indices compare EQUAL -> NOT cumulative -> stays wrapping_add accumulate fan-in (SAME class as the bounded scatter, NOT jacobi/game-of-life cumulative). VERIFIED EMPIRICALLY by output value-correctness, not just deadlock-freedom: directly ran the generated pthreads-sync binary against input.bin -> output.bin BIT-IDENTICAL to reference.bin (cmp clean). And the e2e cells PASS = output.bin byte-match reference + cross-backend determinism.

GATE NUMBERS (all inside nix dev shell):
- just build: clean.
- just clippy (-D warnings): clean (one doc_lazy_continuation self-caught + fixed on the new lower_index_expr bullet list; re-ran clean).
- just test (dev): all green, 0 failed (incl 3 new algo_lower + 2 new render_guard_siblings; replaced 1 obsolete fail-loud pin).
- just test-release: all green, 0 failed (121 test-result-ok suites).
- just e2e: 385/328/0/57/0 -> 392/335/0/57/0 (+7 textbook cells). fail=0, required-fail=0, skipped unchanged (57), no previously-passing cell regressed. Reproduced NON-FLAKY across 2 independent runs (summary + verbose).

just ci PRE-EXISTING RED (NOT introduced by this task, NOT in scope): check-doc-test-name-staleness fails on a stale task0371_* test-name citation at nucleus/driver/src/main.rs:304 — reproduced on CLEAN master via git stash (failed identically with no TASK-0430 changes). I did not touch driver/src/main.rs. All OTHER just-ci structural fences pass (check, clippy, test, test-release, include-str, textual-replace, narrative-doc-lie, citation-staleness x2, cell-path, mega-files, doc-links). The cheap pre-commit subset (build+clippy+test+test-release+e2e) is fully green.

SUBTLETIES / REJECTED APPROACHES / LIMITS:
- Order-of-checks: in loop-bound position the !allow_gather guard fires FIRST (kernel calls are not allowed here), before purity/existence checks - the position itself is the error, the most specific diagnostic.
- REJECTED IrStmt::Let (per design: huge blast radius). Used the inline-pure-call route.
- FIXTURE LIMIT: shared input.bin pre-clipped to [0,BINS) -> bucket(v)==v at RUNTIME for this fixture (modulo is a runtime no-op); the UNCONSTRAINED-input strength is demonstrated at algorithm-surface/codegen, not at runtime. Dedicated unconstrained fixture + distributed textbook scatter = TASK-0432.
- ARG-CAST LIMIT: render_int_expr Call arm emits args without a sig-driven as-type cast; a bare iter-var (i64) arg to an i32-param index kernel would hit a loud E0308 at generated-crate build (not a silent miscompile). No current example needs it. = TASK-0431.
- Downstream passes (transfer_inject record_access_per_dim, halo_inference expr_contains_dataref_or_call, acfg collect_dataref_access) ALREADY handle a Call-in-index conservatively (OPAQUE / data-dependent / index-first recursion, built by TASK-0373); single-worker textbook needs none of it but it is in place for the distributed follow-up. The TASK-0373 UNREACHABLE-on-production comments in partition.rs are now slightly less true (a pure Call-in-index can reach production single-worker) but their behaviour (record nothing -> whole-array) stays correct - noted for TASK-0432 to revisit if it widens distributed scatter.

REVIEW GATE (cycle 246, orchestrator-independent — the implementer self-reviewed inline; this is the real parallel gate): qa-test-runner GO + mped-architect GO.

qa (re-run): build OK; clippy clean x3 incl forced recompile of nucleus-compiler+backend-common; test 1273 dev / 1271 release (0 failed); e2e POSITIVE 392/335/0/57/0 (pass 328->335 = the 7 new 08-histogram/textbook cells; no previously-passing cell regressed); FULL just ci exit 0 (all 9 structural fences OK + all 3 negative/falsifier arms correctly bit). VALUE-CORRECTNESS confirmed (not deadlock-only): qa independently recomputed the histogram from input.bin and it byte-matches reference.bin; all 7 textbook cells are [[required]] PASS via the harness oracle byte-diff, not skipped.

architect: GO, no P1/P2. All 6 risks verified against code: (1) lowering gate correct — call admitted ONLY when allow_gather (subscript), loop-bound position still rejects (const-bound rule intact), purity genuinely checked, effectful->typed error, guard ORDERING pinned by test; (2) silent-sibling CLEAN — exactly one shared backend-common render arm consumed by all 7 backends, render_const_expr loop-bound path still rejects Call, no per-backend sibling; (3) accumulate-classification correct (structural IrExpr eq; bucket-wrap does not change it) AND noted the single-worker cell does not even exercise the classifier (codegen unconditionally correct); (4) no doc-lies; (5) typed errors not panic; (6) follow-ups TASK-0431/0432 honest + code-xref.

P3 DISPOSITIONS:
- P3-1 (architect): accumulate-classifier header note could be misread as a single-worker-cell dependency. FOLDED in-thread (commit 65630b2): added a SCOPE clause stating the classifier is distribution-time forward-verification (TASK-0432), not a dependency of the single-worker cell. e2e re-verified inert 392/335/0/57/0.
- P3-2 (qa+architect): unconstrained-input strength is compile+codegen-only (pre-clipped shared fixture makes bucket a runtime no-op). Honestly disclosed in 3 places + filed TASK-0432. No action (acceptable documented limit).

All 3 ACs met + gate green + review GO. Done status confirmed.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE cycle-247. X1 (pure-kernel-call in array-subscript index position) landed end-to-end: histogram[bucket(input[i])] <-- inc(histogram[bucket(input[i])]) compiles + runs bit-identical to reference.bin across all 7 tier-1 backends. Mechanism: lower_index_expr admits a PURE kernel call in subscript position (gated on purity, subscript-only; loop-bound + effectful still rejected, typed not panic) + shared render_int_expr emits kernels::callee(args) (single arm, all 7 backends, no silent sibling). No new AST/IR node (reused IrExpr::Call), no IrStmt::Let. Accumulate classification UNCHANGED by the bucket() wrapper (both sides carry identical Call -> structural-eq -> wrapping_add fan-in) - VERIFIED by output value-correctness vs the oracle, not just deadlock-freedom. e2e 385/328/0/57/0 -> 392/335/0/57/0 (+7 cells, fail=0, required-fail=0, reproduced non-flaky). Build/clippy/test-dev/test-release all green. HONEST LIMITS: shared fixture is pre-clipped so the modulo bucket is a runtime no-op (feature proven at codegen+value level; dedicated unconstrained fixture + distributed textbook scatter = TASK-0432); index-kernel scalar-arg sig-cast gap = TASK-0431. PRE-EXISTING (not-introduced, not-in-scope) just-ci red: stale task0371_* test-name citation at driver/src/main.rs:304 (reproduced on clean master).
<!-- SECTION:FINAL_SUMMARY:END -->
