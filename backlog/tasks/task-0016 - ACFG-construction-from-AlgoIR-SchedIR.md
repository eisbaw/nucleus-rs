---
id: TASK-0016
title: 'ACFG construction from (AlgoIR, SchedIR)'
status: Done
assignee: []
created_date: '2026-05-17 23:04'
updated_date: '2026-05-18 01:24'
labels:
  - M1
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Build the application control-flow graph that drives subsequent passes. Nodes: operation, repeat, sync (placeholder), xfer (placeholder). Tree shape per the 2013 thesis simplification.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 compiler crate exposes build_acfg(LinkedIR) -> ACFG.
- [ ] #2 Top-level statements become a chain of acfg nodes; for-loops become acfg::repeat with body subtree.
- [ ] #3 Each operation node carries its placement (worker id) and the DAG of operations within the basic block.
- [ ] #4 Test: snapshot tests for ACFG output on each example after linking.
- [ ] #5 Implementation notes record design questions (graph vs tree representation, when to switch to graph for back-edges if needed).
- [ ] #6 Implementation notes record honest limitations (no support for if-statements yet; only for-loops and dataflow).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Design questions (recorded)

1. **Tree vs graph.** Tree, per PRD §5 (back-edges implicit in Repeat
   bodies). Trade-off documented in src/acfg.rs module docs: forces
   structural recursion everywhere; would have to redesign if
   irreducible control flow ever lands, but v2 has none (PRD §6.2.4).

2. **Distributed placements: replicate Operation per worker, or carry
   a worker set?** Carry a BTreeSet<WorkerId> on Operation. Rationale:
   keeps one logical ACFG node per algorithm statement (1:1 with
   source), keeps the tree shape; the per-worker EventList projection
   is a later pass (PRD §8.1) and that pass owns the replication.
   Replicating in the ACFG would explode the tree (a 4-way distributed
   kernel becomes 4 sibling Operation nodes that downstream
   sync-injection has to recognise as one logical thing). Decision
   stated in src/acfg.rs module docs.

3. **DataflowDag shape at M1.** Flat Vec<DataflowEdge> with
   (data_in[], kernel, data_out). One entry per firing — i.e. per
   statement. Rationale: enough for sync- and transfer-injection to
   read producer/consumer. Richer DAG (hash-based equivalence à la
   2013 thesis §4.3.6.1, multi-firing fused blocks) is filed as
   TASK-0109.

4. **Name -> ID assignment.** Local to this pass, derived from sorted
   BTreeMap iteration over LinkedIR. Deterministic across runs. Will be
   superseded by a global ID-assignment pass; filed as TASK-0112.

5. **Loop bounds.** Eval'd to i64 here because Repeat::range is
   Range<i64>. The const evaluator handles IntLit/Ident(const)/
   Neg/BinOp with overflow- and divzero-checking. Iter-var-dependent
   bounds would panic; if a real example ever wants them, the algo
   lowering pass tightens first. Not exercised by 01/05/13/14.

## Honest limitations

- Identity-copy dataflow ('d <-- e' with bare DataRef RHS) is skipped
  (no Operation produced). Mirrors the link step's parallel limitation
  (TASK-0097). Coordinated follow-up: TASK-0111.
- No conditionals. ACFGNode has no If variant. Algorithm sublanguage
  has none, so this is correct today but post-v2-blocking; filed as
  TASK-0110.
- DataflowEdge.data_in keeps duplicates and ignores index expressions.
  That information is recovered later from the source IR when the
  access-pattern analysis runs.
- Build pass panics rather than returning a Result on
  link-pass-invariant violations (unplaced kernel, non-const loop
  bound). Documented in build_acfg's docstring. These are not user-
  facing errors; link rejects programs before reaching here.
- ACFG carries name<->ID maps locally; not shared with future passes.
  TASK-0112 will migrate to a global ID assignment.
- Sync and Xfer variants ship as empty placeholder structs at M1; they
  will be populated by TASK-0017/TASK-0018 respectively.

## AC verification

- #1 build_acfg(linked: &LinkedIR) -> ACFG exposed in compiler::acfg
  and re-exported at crate root.
- #2 Top-level stmts -> Sequence; for-loops -> Repeat with body
  subtree. Verified by acfg_example_1_naive, acfg_example_13_naive
  (5 ops in a Sequence with one Repeat depth=1).
- #3 Operation carries workers: BTreeSet<WorkerId> (placement) plus
  the per-block DataflowDag. Distributed placement verified by
  acfg_example_13_batch_parallel (conv_block_1 has 4 workers);
  singleton placement verified by acfg_example_13_pipeline_parallel.
- #4 Snapshots replaced with structural property assertions per the
  task instructions ('Assert structural properties ... don't snapshot
  the full tree'). Coverage: example-1 × naive, example-13 ×
  {naive, batch_parallel, pipeline_parallel}, example-14 × naive.
- #5 Design questions recorded above + in src/acfg.rs module docs.
- #6 Limitations recorded above + in src/acfg.rs module docs.

## Verification

- just check: green
- just clippy: green (no warnings under -D warnings)
- just test: green; 9 new tests pass in tests/acfg.rs; all
  pre-existing tests still pass.

## Follow-ups filed

- TASK-0109 — Richer dataflow DAG with hash-based equivalence
  (2013 thesis §4.3.6.1).
- TASK-0110 — Conditional / If node support (post-v2).
- TASK-0111 — Identity-copy dataflow (coordinate with TASK-0097).
- TASK-0112 — Migrate to global name->ID assignment pass.
<!-- SECTION:NOTES:END -->
