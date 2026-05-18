---
id: TASK-0009
title: Define and lower algorithm to AlgoIR
status: Done
assignee: []
created_date: '2026-05-17 23:02'
updated_date: '2026-05-18 00:25'
labels:
  - M0
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Define the algorithm IR data types and the AST → AlgoIR lowering pass. PRD §6.2.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 compiler crate exposes AlgoIR types: KernelDecl, DataDecl, ConstDecl, Stmt (dataflow or for), arithmetic expressions.
- [ ] #2 Lowering desugars syntax, resolves const expressions in data shape declarations.
- [ ] #3 Single-assignment is enforced: assigning twice to the same name in scope is an error.
- [ ] #4 Test: lowering produces stable IR for each example algorithm file.
- [ ] #5 Test: violations of single-assignment are rejected with a typed error.
- [ ] #6 Implementation notes record design questions (e.g. handling of partial-array assignment like img[y][x] <-- ...; is that single-assignment per cell?).
- [ ] #7 Implementation notes record honest limitations (e.g. const-expression evaluator may be restricted to integer arithmetic).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Commit: 697b15b compiler(M0): AlgoIR types and AST -> IR lowering pass (TASK-0009)

## Design questions / choices made

- AlgoIR as separate types (algo/ir.rs), not annotated AST. The
  AST/IR boundary makes invariants explicit: AST has parse-shaped
  Expr::LValue-with-empty-indices for bare idents; IR has concrete
  shapes (Vec<usize>) and distinct DataRef/Ident/Call cases.
  Annotating the AST in place would force every downstream pass to
  keep handling 'shape may or may not be resolved' cases.

- Const evaluator scope: integer arithmetic only (+ - * / % and
  parentheses + previously declared const refs). FP, calls, data
  references rejected with typed variants. i64 with checked_*
  throughout; non-positive dim values rejected on data/kernel-sig
  shape positions.

- Recursive const refs: SUPPORTED in principle (with cycle
  detection), but the declarations-before-use rule (lowering
  iterates in source order, consts are sealed before later items
  see them) means cycles cannot form in practice through valid
  programs. The cycle-detection code path is defence-in-depth for
  if/when forward references are admitted in a later relaxation.

- Single-assignment is GLOBAL per data symbol, not per lexical
  scope. PRD 6.2.1 talks about 'single assignment within a scope'
  but every existing example has at most one assignment site per
  data symbol globally; if a real example needs scope-relative
  single-assignment (e.g. assign img in two disjoint sibling for
  bodies), file a relaxation task.

- Iteration-variable scoping is strictly lexical. To produce a
  good 'you used y after its loop ended' diagnostic vs the generic
  UnknownIdent, the lowering pass keeps a seen_iter side-set
  recording every iter-var name introduced during the pass.

- Iter-var shadowing a declared const/data/kernel is rejected
  (IterVarShadowsDecl). PRD wording allows shadowing in general,
  but the existing examples have no such case and the
  conservative choice keeps diagnostics clean. Relax if a real
  example needs it.

- Forward references to consts (using N before declaring it)
  are rejected via ConstRefersToNonConst because consts are added
  to the IR only after evaluation. The lowering loop iterates
  source-order; later items see earlier ones. Same applies to
  data/kernel decls referencing each other and to shape
  expressions. If a future stage needs whole-program collection
  before evaluation, a two-pass lowering would suffice; out of
  scope here.

- IrExpr::Ident vs IrExpr::DataRef in lowering output:
  resolve_ident emits Ident for consts and iter-vars, DataRef
  (with empty indices) for a bare data symbol used as RValue
  (identity-copy). The two cases are kept distinct in the IR so
  later passes don't have to re-resolve from the symbol table.

## Honest limitations

1. Type-checking is NOT in this pass. The IR retains kernel callee
   names as strings; no signature compatibility check at call
   sites. Follow-up: TASK-0088.

2. Purity-vs-statement-form not enforced (effect stmts may call
   pure kernels, dataflow RHS may call effectful ones). The
   purity is preserved on ResolvedKernel; the check is a later
   pass. Follow-up: TASK-0089.

3. AST nodes carry no spans (TASK-0007 limitation); LowerError
   variants therefore carry only identifier strings, not
   (line, column). When spans land, variants will gain position
   fields without surface break. Follow-up: TASK-0090.

4. Declarations-before-use is enforced through the source-order
   iteration. Forward refs of consts / data / kernels are
   rejected even when they'd be semantically clean. If a real
   example needs forward refs, a two-pass lowering (declare,
   then evaluate) is straightforward. Follow-up: TASK-0091.

5. Single-error reporting: the lowering pass aborts on the first
   violation. Matches the parser's behaviour. Follow-up: TASK-0092.

6. Const evaluator is i64-only. Range narrowing to the declared
   scalar type (e.g. const N : u8 = 1000 should overflow) is
   not done. The narrow-check belongs to the type-checking pass
   in TASK-0088.

7. Iter-var shadowing of a declared name is rejected even though
   PRD 6.2.3 permits it. Conservative; revisit when a real
   example requires shadowing.

## AC verification

- AC #1 (AlgoIR types exposed): MET. algo::ir exports ResolvedConst,
  ResolvedData, ResolvedKernel, ResolvedType, IrStmt, IrExpr,
  IndexedRef, AlgoIR, LowerError. algo::lower exports lower_algo.
  Re-exported from algo::mod.

- AC #2 (lowering resolves shape consts): MET. eval_shape_expr
  resolves DimList expressions to concrete usize. feat1 in the
  CNN example becomes f32[16][8][14][14] from f32[B][C1][H/2][W/2].

- AC #3 (single-assignment enforced): MET. DoubleAssignment carries
  data name and the scope that owned the prior assignment. Test
  double_assignment_to_same_data_is_error covers it.

- AC #4 (lowering produces stable IR per example): MET via
  structural assertions in lowers_example_13_cnn_inference and
  lowers_example_14_hearing_aid (counts, concrete shape dims,
  statement-kind distribution). Snapshot-of-whole-IR rejected
  for the same reason TASK-0007 rejected it: brittle to encoding
  drift.

- AC #5 (single-assignment violations rejected with typed error):
  MET. LowerError::DoubleAssignment with data + scope fields.

- AC #6 (design questions recorded in notes): MET (above).

- AC #7 (limitations recorded in notes): MET (above).

## Verification (inside nix develop)

- just check  -> pass
- just clippy -> pass (-D warnings)
- just test   -> pass (13 lower tests + 10 algo-parser + 14
                       schedule + the existing zero-test crates)

## Follow-up tasks filed

- TASK-0088: Type-check AlgoIR (kernel-sig vs call-site shapes;
  const range narrowing).
- TASK-0089: Enforce kernel-purity vs statement-form.
- TASK-0090: Add per-node spans to AST and propagate to LowerError.
- TASK-0091: Relax declarations-before-use (two-pass lowering).
- TASK-0092: Multi-error reporting in AlgoIR lowering.
<!-- SECTION:NOTES:END -->
