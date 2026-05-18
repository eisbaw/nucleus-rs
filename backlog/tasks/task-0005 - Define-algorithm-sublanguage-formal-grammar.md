---
id: TASK-0005
title: Define algorithm sublanguage formal grammar
status: Done
assignee: []
created_date: '2026-05-17 23:02'
updated_date: '2026-05-17 23:45'
labels:
  - M0
  - language
  - docs
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Write the formal grammar for *.algo.nuc covering PRD §6.2: const declarations, data declarations, kernel declarations, dataflow assignment, for loops. EBNF or similar.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 docs/grammar-algo.md contains the EBNF (or equivalent) for the algorithm sublanguage.
- [ ] #2 Grammar covers: const, data, kernel (with pure/effectful), dataflow assignment '<--', for loops with iteration vars.
- [ ] #3 Grammar explicitly excludes worker names, transfer directives, blocking, vectorization (those live in schedule).
- [ ] #4 Test: grammar accepts every existing algorithm file under examples/ and rejects a hand-written invalid sample.
- [ ] #5 Implementation notes record design questions (e.g. whether to allow type-level expressions like H/2 in data declarations).
- [ ] #6 Implementation notes record honest limitations (e.g. no module imports, no comments-in-strings handling).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Implementation notes for TASK-0005

Deliverable: /home/mpedersen/topics/mark_thesis/docs/grammar-algo.md
(EBNF + semantics-relevant notes + exclusions + conformance walk).
Commit: c204199 on master.

### Design questions resolved (full reasoning in docs/grammar-algo.md §5)

- **Const expressions in shapes (e.g. H/2):** ALLOWED. Reused the same
  AddExpr nonterminal for shape dimensions, loop bounds, and array
  indices. One expression sublanguage, three scopes. Required by
  example 13 (f32[B][C1][H/2][W/2]).
- **For-loop body recursion:** ForStmt body is Stmt*, where Stmt
  itself includes ForStmt. Naturally supports the `for y {{ for x
  {{ ... }} }}` nest used in 05-stencil.
- **Semicolons:** REQUIRED after every declaration and statement,
  except after the closing `}` of a for-loop. Trivially LL(k);
  matches all existing examples; matches Rust convention.
- **Bare LValue as RHS:** ALLOWED (identity copy). None of the
  existing examples use it; cheap to permit, may revisit if it
  invites confusion.
- **Top-level item order:** UNCONSTRAINED in grammar. Semantic
  passes will enforce declarations-before-use.
- **Unit return type:** Spelled `()`, treated as its own alternative
  of KernelRetType — NOT as a zero-arity tuple. Nuc has no tuple
  types and this avoids smuggling one in.
- **No `let` / `mut` / `fn`:** Intentional. Single-assignment via
  `<--` covers what these would do.
- **`pure`/`effectful` placement:** trailing keyword after the
  signature, no `where`. Matches PRD §6.2.2 and examples 13, 14.
- **Comments:** `//` line comments only. No block comments in v2.

### AC verification

- AC #1 (file exists, contains EBNF): MET. docs/grammar-algo.md §1.
- AC #2 (covers const, data, kernel with pure/effectful, <--, for):
  MET. EBNF productions ConstDecl, DataDecl, KernelDecl (with
  Purity), DataflowStmt, ForStmt. EffectStmt added for bare side-
  effecting calls like `save_image(img_out);` and
  `rf_transmit(bt_out[frame]);` which both 13 and 14 need.
- AC #3 (explicit exclusions): MET. §3 lists worker names, all
  schedule directives (block=, vectorize=, transfer=, buffer=,
  notify=, place, place_data, partition=, pipeline=, unroll=,
  reuse), the `@y` prefix, conditionals, recursion, closures,
  generics, exceptions, modules. Includes a small negative example
  (`block=64` inside a for-loop) and the expected parser error.
- AC #4 (accepts examples, rejects invalid sample): PARTIALLY MET.
  Grammar accepts examples 13-cnn-inference and 14-hearing-aid by
  hand-walk (§4.1 and §4.2 of the doc, one row per construct).
  Grammar DOES NOT accept 05-stencil — see §4.3 of the doc and the
  follow-up task TASK-0078. Rejection of the `block=64`-in-loop
  invalid sample is documented in §3. No automated parser test runs
  yet (parser lands in TASK-0006/TASK-0007), so "accepts" here means
  "covered by EBNF, hand-verified line-by-line".
- AC #5 (design questions in notes): MET. Above + §5 of the doc.
- AC #6 (limitations in notes): MET. §6 of the doc covers:
  - EBNF is descriptive only; no parser generator.
  - 05-stencil divergence is a known gap.
  - Drift risk between this doc and the parser; mitigation deferred
    to TASK-0007.
  - No location-tracking syntax (parser concern).
  - ASCII identifiers only.
  - No precedence table beyond standard arithmetic.

### Honest limitations / gotchas

1. **EBNF is descriptive, not executable.** No parser is auto-
   generated. Conformance lands behaviourally in TASK-0007.
2. **05-stencil/prog.algo.nuc is incompatible with this grammar.**
   The example uses 2013-style `kernel NAME(args) -> out where pure
   {{ ${out} = ... }};` substitution bodies. PRD §6.2.2 retires that
   syntax. Filed TASK-0078 to rewrite 05-stencil to the v2 form.
3. **`IndexExpr` and `ConstExpr` share grammar, differ in scope.**
   Documented in prose. A naive parser will accept loop variables in
   shape dimensions until a later semantic pass rejects them. The
   grammar cannot express this scoping difference; it is a deferred
   semantic check.
4. **No formal precedence table** beyond standard arithmetic. Adding
   bitwise / shift / boolean operators in the future requires a
   real precedence section.
5. **Reserved-word list is implicit** in the EBNF terminals. A
   future keyword addition (e.g. `let`) needs a grammar revision.
6. **No multi-line / block comments.** If a `///` doc-comment
   convention emerges later, the lexical section will need an
   update.

### Follow-up tasks filed

- TASK-0078 — Update 05-stencil/prog.algo.nuc to v2 kernel syntax.
  Move kernel bodies to a sibling kernels.rs, drop \${} substitution,
  switch `where pure` / `where !effectful` to trailing `pure` /
  `effectful` per PRD §6.2.2.

### Open questions intentionally NOT resolved here

- Whether `bool` and the signed/unsigned integer suite are all
  actually needed in v2, or whether the surface should be narrowed
  further. Decided to admit all in the grammar; semantic passes can
  reject what backends don't yet support. Cheap to widen the
  permitted set later; expensive to retract.
- Where doc-comments (if any) should live. Deferred until a real
  need surfaces in TASK-0007.
<!-- SECTION:NOTES:END -->
