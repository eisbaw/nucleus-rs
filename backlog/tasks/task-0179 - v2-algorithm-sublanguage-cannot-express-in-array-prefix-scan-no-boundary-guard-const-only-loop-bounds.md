---
id: TASK-0179
title: >-
  v2 algorithm sublanguage cannot express in-array prefix scan (no boundary
  guard, const-only loop bounds)
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-19 01:13'
updated_date: '2026-05-19 03:37'
labels:
  - M3
  - language
  - findings
dependencies:
  - TASK-0039
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Surfaced by TASK-0039 (example 04 prefix-sum). Three concrete v2 limitations make a textbook in-array carried prefix scan inexpressible: (1) carried shifted index out[i-1] underflows usize at i=0 and there is no conditional (PRD 6.2.4) to guard it; single-assignment (keyed by symbol name) forbids a base-case + loop split on the same array. (2) Loop bounds must be compile-time const (acfg.rs:697 eval_const) and PANICS rather than returning a clean LowerError on a non-const bound — triangular loops impossible AND the failure mode is an ugly panic not a diagnostic. (3) Single-assignment ignores differing constant indices so block unrolling as separate statements is rejected. TASK-0039 worked around this in-language by pushing the carry/boundary logic into hand-written Rust kernels (legal) over a rectangular reduction-accumulator; this task tracks the underlying language gaps. Options: add a clamp/saturating index intrinsic, an exclusive-scan/segmented-scan algorithm builtin, or a guarded-first-iteration form; at minimum convert the acfg.rs:697 panic into a LowerError.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 acfg.rs non-const loop bound returns a LowerError (not panic!)
- [ ] #2 Decision recorded (decision doc or PRD note) on whether in-array prefix scan gets language support or stays a kernel-level idiom
- [ ] #3 If supported: an example expresses prefix scan WITHOUT pushing the boundary into a kernel
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. AC#1 code fix: add pub enum BuildAcfgError (Debug/Clone/PartialEq/Eq + Display + impl std::error::Error) modelled on BlockTransformError/SidecarError, variant NonConstLoopBound { var: String, bound: LoopBoundKind (Lower/Upper), expr: IrExpr }.
2. Change build_acfg -> Result<ACFG, BuildAcfgError>; replace BOTH eval_const panics (lo/hi ~718-721) with Err(NonConstLoopBound). build_seq/build_stmt thread Result.
3. Export BuildAcfgError from compiler/src/lib.rs (acfg pub use block).
4. Update ALL callers: driver main.rs:224 -> .map_err(|e| format!("acfg build error: {e}"))?; every test/bench caller -> .expect("build_acfg") (mechanical, like apply_block_transforms).
5. AC#1 test: characterisation test in tests/acfg.rs — triangular loop (for j:0..i referencing enclosing iter var i) lowers+links but build_acfg returns Err(BuildAcfgError::NonConstLoopBound) NOT panic; assert variant + var name + Display.
6. AC#2 decision: no decisions/ dir / no ADR convention exists; record as PRD note. Add §6.2.5 "Recorded decision: in-array prefix scan is a kernel-level idiom for v2" (keep §6.2.4 intact) with rationale (PRD §6.2.2 kernels=Rust split, §6.2.4 no-conditionals, single-assignment-by-symbol; kernel boundary = designed escape hatch; clamp/scan/segmented-scan/guarded-first-iter = FUTURE language work, not v2 M3). Reference 04-prefix-sum as canonical pattern.
7. AC#3: N/A under AC#2 decision (kernel-level idiom => no boundary-free example is a v2 deliverable; it is future language work). Document explicitly in PRD note + task notes. Do NOT fake an example.
8. Gate before each commit (nix develop): just test / just e2e (expect UNCHANGED 30/26/0/4/0) / determinism-check + -negative + xbackend-check-negative / cargo clippy -D warnings / just ci. Commit per logical unit, no AI credit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
AC#1 DONE: build_acfg now returns Result<ACFG, BuildAcfgError>. New pub enum BuildAcfgError (Debug/Clone/PartialEq/Eq + Display + impl std::error::Error) + pub enum LoopBoundEnd{Lower,Upper}, modelled on BlockTransformError/SidecarError. Variant NonConstLoopBound{var:String, end:LoopBoundEnd, expr:IrExpr}. Both eval_const panics (lo/hi) replaced with ok_or_else(NonConstLoopBound). build_seq/build_stmt thread Result. Exported from compiler/src/lib.rs. Driver main.rs:224 -> .map_err(|e| format!("acfg build error: {e}"))?. 38 test/bench call sites mechanically -> .expect("build_acfg"). Reachability CONFIRMED: algo::lower::lower_index_expr->resolve_ident resolves in-scope iter vars, so triangular `for j:0..i` parses/lowers/links but eval_const (consts-only) returns None -> typed error not panic.
AC#1 test: tests/acfg.rs build_acfg_non_const_loop_bound_is_typed_error_not_panic — inline triangular program, asserts expect_err + variant + var="j" + end=Upper + expr=Ident("i") + Display contents.
GATE (nix develop, repo root): just test 40/40 binaries 0 failed; clippy -D warnings clean; just e2e UNCHANGED 30/26/0/4/0; determinism-check byte-identical 30/26/0; determinism-check-negative + xbackend-check-negative both correctly bite; just ci green.
Kept-as-panic (genuine link-valid-IR invariants, NOT converted, verified link rejects first): bind_arg undeclared symbol, resolve_worker_set no-placement, worker-not-in-name-table.
<!-- SECTION:NOTES:END -->
