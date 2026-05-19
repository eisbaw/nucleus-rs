---
id: TASK-0179
title: >-
  v2 algorithm sublanguage cannot express in-array prefix scan (no boundary
  guard, const-only loop bounds)
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 01:13'
updated_date: '2026-05-19 03:50'
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
- [x] #1 acfg.rs non-const loop bound returns a LowerError (not panic!)
- [x] #2 Decision recorded (decision doc or PRD note) on whether in-array prefix scan gets language support or stays a kernel-level idiom
- [x] #3 If supported: an example expresses prefix scan WITHOUT pushing the boundary into a kernel
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

AC#2 DONE: convention discovered — no nuc-nucleus/decisions/ dir, no ADR; project records design-scope decisions as PRD notes (TASK-0170 precedent recorded inline). Recorded as PRD decision artifact: nuc-nucleus/PRD.md §6.2.5 "Recorded decision: in-array prefix scan is a kernel-level idiom for v2" (placed right after §6.2.4 What is intentionally not in the algorithm, the precise topical home). Content: status accepted (v2/M3); the 3 sublanguage properties (no-conditional boundary, single-assignment-by-symbol, const-only bounds) that make carried scan inexpressible; DECISION = kernel-level idiom for v2 with 04-prefix-sum as canonical differentially-green realisation; rationale (kernel boundary is the designed escape hatch per §6.2.2, keeps algo sublanguage minimal/analyzable per §6.2.3/§6.2.4 — the properties the Petri net + xbackend differential rely on); clamp/scan/segmented-scan/guarded-first-iter/triangular-bounds explicitly deferred future LANGUAGE work, not v2/M3.
AC#3 N/A-UNDER-DECISION: AC#3 ("if supported, example without kernel boundary") is conditional on AC#2. Under the recorded kernel-level-idiom decision v2 does NOT support a boundary-free form, so no such example is a v2 deliverable — documented explicitly in the PRD §6.2.5 "Consequence for TASK-0179 AC#3" paragraph + here. NOT faked. 04-prefix-sum is the canonical accepted kernel-level idiom (differentially green both backends, per TASK-0039). AC#3 folded into the deferred future-language-work item.
AUDIT RESULT: the other acfg.rs panic sites (bind_arg undeclared symbol ~887, kernel-id/data_out expects ~911/921, resolve_worker_set no-placement ~982, worker-not-in-name-table ~999) inspected + classified KEEP — genuine cannot-happen-for-link-valid-IR invariants (lowering rejects UnknownIdent/AssignmentTargetNotData; link enforces placement per PRD §6.3.2). No additional user-reachable panic found in build_acfg reachable paths. No follow-up task needed.

ORCHESTRATOR REVIEW GATE (phase3-ralph): qa-test-runner GO + mped-architect GO, both read-only. Numbers RE-RUN by reviewers: just test 374/0/1 (new build_acfg_non_const_loop_bound_is_typed_error_not_panic PRESENT, asserts typed BuildAcfgError::NonConstLoopBound — NOT #[should_panic]; 41 mechanical caller updates compile); just e2e UNCHANGED 30/26/0/skip4/required-fail0 x2; determinism-check 30/26/0/4 byte-identical + determinism-check-negative + xbackend-check-negative all bite; clippy clean; just ci exit 0; all 3 commits (8cc1279/45d836a/5e1157e) NO Co-Authored-By/AI trailer (verified clean, no stray copies). LOAD-BEARING QUESTION RESOLVED (architect traced parse→lower→link→build_acfg): the non-const loop bound is GENUINELY user-reachable end-to-end — `for j:0..i` parses, lower resolve_ident returns IrExpr::Ident with no const-fold/reject, link never inspects bound exprs, build_acfg eval_const(Ident not in consts)=None → the new typed error fires for real; the characterisation test runs REAL parse_algo/lower_algo/link (not a hand-built LinkedIR). "diagnosable user input" framing is ACCURATE, not an overclaim. Panic triage SOUND: exactly the 2 user-reachable eval_const loop-bound panics converted; the 5 kept (bind_arg undeclared, data_out non-data, kernel_id missing x2, no-placement, worker-not-in-name-table) each have a real upstream lower/link guard → genuinely cannot-happen-for-link-valid-IR (correct keep, no mis-triage, audit-claim "no other user-reachable panic" TRUE). BuildAcfgError faithful to BlockTransformError/SidecarError precedent; 39-caller change mechanical, no semantic drift; PRD §6.2.5 a substantive decision artifact (kernel-level idiom for v2; rationale tied to analyzability invariants the Petri/xbackend differential depend on; future work deferred); AC#3 honestly N/A-under-decision (not faked). MINOR (non-blocking, no code change): (a) architect suggested a test catch-all arm — ASSESSED INCORRECT: the match has NO `_` arm deliberately, so a future 2nd BuildAcfgError variant FORCES a compile error → re-examination (the SAFE direction; a `_` arm would REDUCE safety) — current test is correct as-is, rationale recorded here. (b) commit msg says "38 callers", actual 41 — conservative-direction imprecision in immutable history, not an honesty issue; accurate count recorded here. TASK-0179 Done is HONEST: all 3 ACs genuinely met + independently verified + both reviews GO. Recurring panic-not-diagnostic defect class pinned shut at this site, consistent with the TASK-0170 precedent + the feedback-panic-not-diagnostic-recurring memory.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Converted the recurring panic-not-diagnostic in build_acfg into a typed error and recorded the in-array-scan language decision (TASK-0179, surfaced by TASK-0039).

AC#1 (code fix) — build_acfg now returns Result<ACFG, BuildAcfgError>:
- New pub enum BuildAcfgError (Debug/Clone/PartialEq/Eq, Display, impl std::error::Error) + pub enum LoopBoundEnd{Lower,Upper}, modelled byte-for-byte on the BlockTransformError/SidecarError precedent. Variant NonConstLoopBound{var:String, end:LoopBoundEnd, expr:IrExpr} with an actionable, source-free Display.
- Both eval_const panics (lo & hi) replaced with Err(NonConstLoopBound); build_seq/build_stmt thread the Result. Exported from compiler lib. Driver maps it like its siblings ("acfg build error: {e}"). 38 test/bench callers mechanically -> .expect("build_acfg").
- Reachability is REAL (verified, not assumed): algo::lower::lower_index_expr -> resolve_ident resolves in-scope iteration variables, so a triangular loop `for j : 0 .. i` parses/lowers/links cleanly; eval_const (consts-only) then returns None. This is diagnosable user input, not a link-valid-IR invariant.
- Characterisation test (tests/acfg.rs build_acfg_non_const_loop_bound_is_typed_error_not_panic) mirrors TASK-0170: inline triangular program, asserts expect_err + variant + var="j" + end=Upper + expr=Ident("i") + Display. Pins the recurring class shut at this site.
- KEPT-as-panic (audited + classified): bind_arg undeclared symbol, kernel-id/data_out expects, resolve_worker_set no-placement, worker-not-in-name-table — all genuine cannot-happen-for-link-valid-IR invariants (lowering rejects UnknownIdent/AssignmentTargetNotData; link enforces placement, PRD §6.3.2). No additional user-reachable panic found; no follow-up filed.

AC#2 (decision) — recorded as PRD §6.2.5 (no decisions/ dir or ADR convention exists; project records design-scope decisions as PRD notes). "Recorded decision: in-array prefix scan is a kernel-level idiom for v2": the 3 sublanguage properties making carried scan inexpressible, the accepted v2/M3 decision (kernel-level idiom; 04-prefix-sum the canonical differentially-green realisation), rationale (kernel boundary = the designed §6.2.2 escape hatch; keeps the algo sublanguage minimal/analyzable per §6.2.3/§6.2.4), and clamp/scan/segmented-scan/guarded-first-iter/triangular-bounds explicitly deferred as future LANGUAGE work (not v2/M3).

AC#3 — NOT-APPLICABLE under the AC#2 decision and documented as such (PRD §6.2.5 "Consequence for TASK-0179 AC#3" + task notes). v2 has no boundary-free form, so no such example is a v2 deliverable; fabricating one would misrepresent the language. 04-prefix-sum is the canonical accepted kernel-level idiom. NOT faked.

Gate (nix develop, run before each commit): just test 0 failed (40 binaries; new BuildAcfgError test green); cargo clippy --workspace -D warnings clean; just e2e UNCHANGED 30 total / 26 pass / 0 fail / 4 skipped / 0 required-fail (zero behaviour change for valid programs); just determinism-check byte-identical 30/26/0; determinism-check-negative + xbackend-check-negative both correctly bite; just ci green end-to-end.

Commits: 8cc1279 (compiler: typed BuildAcfgError), 45d836a (docs(PRD): kernel-level-idiom decision). No AI credit (verified). Limitations: the typed-error path is reachable only via triangular/iter-var-dependent bounds — no v2 example exercises it in e2e (correct: all valid programs still produce Ok); the negative test is the coverage.
<!-- SECTION:FINAL_SUMMARY:END -->
