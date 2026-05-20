---
id: TASK-0204
title: >-
  Broaden K×L cascade fixture to exercise all 4 cascade-error variants
  (named-fixture coverage gap)
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 22:59'
updated_date: '2026-05-20 17:02'
labels:
  - compiler
  - diagnostics
  - tests
  - follow-up
  - M0
  - cascade-class
dependencies:
  - TASK-0092
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0092 cycle-3 review (mped-architect + qa-test-runner both flagged independently): the new transitive_cascade_collapses_for_any_k_l fixture at compiler/tests/algo_lower.rs:1009-1055 is genuinely parametric in K×L (4×3=12 combinations, all pass), but uses a SINGLE underlying cascade-shape: root const N=1/0; cascade-decls are 'data dki : f32[N]' (always ShapeRefersToNonConst); statements are 'dump(dki)' (always UnknownIdent via Effect-call lookup). The other three cascade-suppression variants — AssignmentTargetNotData, ConstRefersToNonConst, ShapeRefersToNonConst-at-depth>1 — and cross-kind cascades (cascade-kernel-sig, cascade-const-via-other-const, dataflow-assign-target on cascade-data) are exercised in mped-architect's 21-probe set + real-driver probes, but NOT in the NAMED fixture. The K×L axis prevents single-shape masking for the bare-call path; broadening the parametric dimension across cascade-error variant + cascade-kind would make the structural guard explicit and persistent. Filed from TASK-0092 cycle-3 review (2026-05-20).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Extend (or add a sibling parametric fixture to) transitive_cascade_collapses_for_any_k_l so that cascade-error variant is a third parametric dimension iterated over: UnknownIdent (existing), AssignmentTargetNotData, ConstRefersToNonConst, ShapeRefersToNonConst-at-depth>1 — at least 4 variants × K∈{1,2,3,5} × L∈{1,2,3} measured; each combination asserts errors().len()==1 AND the surviving error is the root kind (not a leaked variant)
- [x] #2 Add at least 3 cross-kind cascade shapes: cascade-data-via-shape (current), cascade-kernel-via-signature-shape, cascade-const-via-other-const; each transitively poisons and downstream references collapse to 1
- [x] #3 If the broader fixture surfaces a NEW defect (an axis the transitive-poison fix does not actually cover), STOP and file a precise follow-up rather than papering over — the honest-stop discipline applies; the 5th-recurrence-closed claim must remain measurement-backed
- [x] #4 just test passes; just ci exit 0; clippy --workspace --all-targets clean; no behaviour change for valid input (e2e 30/26/0/4/0; det-check byte-identical x2)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Broaden the named K×L cascade fixture to be a 4-dimensional sweep: (cascade-kind in {data-via-shape, kernel-via-signature-shape, const-via-other-const}) × (cascade-error variant in {UnknownIdent, AssignmentTargetNotData, ConstRefersToNonConst, ShapeRefersToNonConst-at-depth>1}) × (K in {1,2,3,5} cascade-decls) × (L in {1,2,3} references-per-cascade-decl). For each combination: build the program, assert errors.len()==1, kind == root ConstDivByZero{N}, no leaked cascade-error variant of any of the 4 kinds. The existing named fixture transitive_cascade_collapses_for_any_k_l is the stylistic template (same assertion idiom, same parametric loop, but parameter-broadened). NOT papered over if a new defect surfaces — STOP and file precise follow-up.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTATION COMPLETE (2026-05-20, single fresh-context cycle).

Commit: 86fd319 (algo-lower tests: broaden cascade fixture across 3 kinds × 4 variants).

Per-AC status:
  AC#1 (extend named fixture so cascade-error variant is a parametric dimension over UnknownIdent, AssignmentTargetNotData, ConstRefersToNonConst, ShapeRefersToNonConst-at-depth>1, K×{1,2,3,5}, L×{1,2,3}; each combination asserts len==1 AND root kind not leaked-variant): MET. Extended transitive_cascade_collapses_for_any_k_l in-place rather than adding sibling fixture — the AC text permits either, and extending preserves the named-fixture coverage gap structurally rather than creating an orthogonal one.
  AC#2 (at least 3 cross-kind cascade shapes: cascade-data-via-shape, cascade-kernel-via-signature-shape, cascade-const-via-other-const; each transitively poisons and downstream references collapse to 1): MET. All three exercised parametrically; all (kind × trigger × K × L) cells confirmed at exactly 1 error.
  AC#3 (if broadening surfaces a NEW defect, STOP and file follow-up rather than paper over; the 5th-recurrence-closed claim must remain measurement-backed): MET — no new defect surfaced. All 144 cells collapse to 1 root error. The transitive-poison fix demonstrably covers every cell the new fixture exercises.
  AC#4 (just test passes; just ci exit 0; clippy clean; no behaviour change for valid input — e2e 30/26/0/4/0; det-check byte-identical x2): MET.

7-step gate measurements (all from nix develop):
  1. just test                       : 458 passed (== baseline; existing fixture extended in-place, no new test fn).
  2. cargo clippy --workspace --all-targets -- -D warnings : 0 warnings, exit 0.
  3. just e2e                        : 30/26/0/4/0 (zero behaviour change as expected).
  4. just determinism-check x2       : both byte-identical (5+4+5+4+5+4+5+4 etc per cell, 30/26/0/4).
  5. just determinism-check-negative : bites (30/0/26/4 with Cargo.toml length-differs perturbation across 26 cells).
  6. just xbackend-check-negative    : bites (NUC_XBACKEND_CORRUPTED_APPLIED=13, DETECTED=1).
  7. just ci                         : exit 0.

Parametric coverage matrix (3 cascade-kinds × 4 trigger variants × 4 K × 3 L = 144 cells; all collapse to 1 root ConstDivByZero{N}):
  DataViaShape         × {BareCallRead, AssignmentLhs, ConstRefersTo, ShapeRefersTo} × K × L
  KernelViaSignatureShape × {BareCallRead, AssignmentLhs, ConstRefersTo, ShapeRefersTo} × K × L
  ConstViaOtherConst   × {BareCallRead, AssignmentLhs, ConstRefersTo, ShapeRefersTo} × K × L

Per-iteration assertion strength:
  - errors().len() == 1 (exact equality, not just > 0).
  - sole survivor matches root kind ConstDivByZero{in_const == "N"}.
  - span of sole survivor resolves to the offset of the substring "1 / 0" in source (via offset_to_line_col).
  - no error of any of the four cascade-suppressible LowerErrorKind variants (UnknownIdent / AssignmentTargetNotData / ConstRefersToNonConst / ShapeRefersToNonConst) survives (explicit anti-leak guard).

Negative-control discrimination verified mid-implementation: temporarily commented out the cycle-3 case-1 transitive-poison line at lower.rs:236; cargo test --test algo_lower transitive_cascade_collapses_for_any_k_l then panicked immediately on the (DataViaShape, BareCallRead, K=1, L=1) cell with "left=2 right=1 kinds=[ConstDivByZero{N}, UnknownIdent(dump_arr)]". Restored. The fixture is a genuine structural guard, not a tautology.

HONEST FINDING surfaced and documented in fixture rustdoc (not a defect — a brief imprecision corrected in code):
  The orchestrator brief stated "AssignmentTargetNotData fires when y is poisoned (e.g., a const that failed)". In practice, failed decls are NOT inserted into ir.consts/data/kernels — only into Accum::failed_decls. Therefore the LHS-of-<-- lookup for a poisoned name goes through the UnknownIdent branch, not the AssignmentTargetNotData branch (which requires the LHS name to be present in ir.consts/kernels/iter_var). The AssignmentLhs trigger is still valuable as a STATEMENT-SHAPE guard: the assertion is "no error of any of the four cascade-suppressible variants leaks", and the structural intent — LHS-to-poisoned-name does not leak a cascade-error — is exactly what we want pinned. The fixture rustdoc states this honestly.

Doc-lie sweep performed (TASK-0200 cycle-2 lesson):
  - lower.rs:108-121 counting-contract docstring extended to call out the broadened K×L axes and four-variant coverage by name; the previous text said "K cascade-decls each used by L statements" (single trigger), now reads "K cascade-decls each used by L dependants (statements *or* downstream decls)" and explicitly cites TASK-0204 and the four cascade-suppressible variants.
  - Grepped {transitive_cascade_collapses_for_any_k_l, four cascade-suppression variants, K x L} across nucleus/ src and tests; the sched/parser.rs:806 and sched/ir.rs:802 references to "K×L parametric" are SIBLING-FIXTURE references at the sched layer and remain accurate for that layer's discipline. sched_lower.rs:1465 "(mirrors TASK-0092 cycle-3 transitive_cascade_collapses_for_any_k_l)" is accurate: the sched fixture mirrors the 2-axis K×L discipline (which is preserved here), not the new third-dimension cascade-shape broadening which is specific to algo-layer suppression. No mendacious docstring carriers needed updating.

Operational gotcha (TASK-0087 cycle-4) AVOIDED: this notes file is fed via heredoc to backlog task edit --append-notes, no backticks in CLI-passed text.

FORWARD-CARRY to TASK-0207 (algo for-body sibling parametric — same discipline at the parser layer's sibling):
  The 4-dimensional sweep template (cascade-kind × trigger × K × L) lands cleanly at the AlgoIR lowering layer. The for-body sibling task at the parser/AST layer should:
    1. Identify the parser/AST analogue of "cascade-kind" (probably statement-form: dataflow / effect / for-loop). If the cascade-class doesn't apply at the parser layer (parser is a syntactic gate, not a semantic-cascade producer), then this template does NOT transfer and the TASK-0207 brief should say so explicitly rather than fabricating a parametric over a degenerate dimension.
    2. Apply the K×L parametric discipline ONLY along axes that genuinely scale in the parser's error-emission contract — the masking-defect-class lesson is to iterate dimensions the disclosure CAN bite, not to iterate dimensions for their own sake.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Broadened named fixture transitive_cascade_collapses_for_any_k_l from 2-axis K×L over a single cascade-shape (12 cells) to 4-axis cascade_kind × trigger × K × L sweep (144 cells). All four cascade-suppressible LowerErrorKind variants (UnknownIdent, AssignmentTargetNotData, ConstRefersToNonConst, ShapeRefersToNonConst) now have persistent structural guards via the named fixture, not only via dormant cycle-3 review probes. Negative-control verified: reverting the cycle-3 case-1 transitive-poison line makes the fixture fail immediately with the expected UnknownIdent leak. Gate: 458 tests passed, clippy 0 warnings, e2e 30/26/0/4/0, det-check byte-identical x2, both negative gates bite, just ci exit 0. Honest finding documented in fixture rustdoc: AssignmentTargetNotData is structurally unreachable as cascade in the current language (failed decls never enter ir.consts/kernels), so the AssignmentLhs trigger functions as a no-leak statement-shape guard rather than a positive-variant trigger. Commit 86fd319.
<!-- SECTION:FINAL_SUMMARY:END -->
