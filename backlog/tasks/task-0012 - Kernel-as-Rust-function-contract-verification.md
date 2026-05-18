---
id: TASK-0012
title: Kernel-as-Rust-function contract verification
status: Done
assignee: []
created_date: '2026-05-17 23:03'
updated_date: '2026-05-18 00:53'
labels:
  - M0
  - compiler
  - language
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §6.2.2: a kernel declared in *.algo.nuc as 'kernel blur3 : (f32, f32, ...) -> f32 pure' must have a matching Rust function in kernels.rs. Implement the contract check: compile kernels.rs as part of nucleus build, verify each declared signature matches a function with the same name and signature shape.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 When building an example, nucleus invokes 'cargo check' on examples/NN/kernels.rs and parses any signature mismatch into a structured error.
- [ ] #2 Mismatch types: missing function, arity mismatch, type mismatch, missing pub modifier.
- [ ] #3 Purity is not enforced at the Rust level (rustc can't prove it). 'where pure' is a contract the user upholds; misuse is a v2 limitation noted in PRD.
- [ ] #4 Test: a deliberately mismatched kernels.rs produces a structured error pointing at the algo declaration and the Rust signature.
- [ ] #5 Implementation notes record design questions (e.g. should pure kernels be wrapped in a marker trait at codegen time).
- [ ] #6 Implementation notes record honest limitations (the purity attribute is documentation; v2 does not statically verify it).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Design decisions

- **rustc, not cargo, for the compile-check phase.** Two options:
  (a) generate a tiny ad-hoc Cargo project alongside kernels.rs and
  invoke 'cargo check', or (b) call 'rustc --emit=metadata
  --crate-type=rlib --edition=2021' directly on the single file.
  Chose (b). Rationale: a v2 kernels.rs is by PRD §6.2.2 a sibling
  source file with no external dependencies (std/core are enough);
  cargo's value-add is dependency resolution and incremental
  caching, neither of which applies. rustc is ~50-100ms; cargo is
  ~seconds first run. Documented in src/contract.rs module header.
  If a real example later needs an extern crate, revisit.

- **Module naming: 'contract'.** PRD §6.2.2 literally calls the
  declaration 'a contract the Rust function must satisfy'. The
  alternative 'kernels.rs' was tempting but clashes with the
  user-facing file of the same name. The Rust module is at
  src/contract.rs.

- **Partial diagnostics on rustc failure.** When rustc rejects the
  file, we still attempt the syn parse and signature match. Syn is
  robust to many rustc-fatal errors, so the user can see both
  'rustc said X' and 'also, blur3 is declared 2-ary but defined
  3-ary'. Reporting only the rustc stderr would be a regression
  vs. plain syn-based matching.

- **One error per declared kernel.** A kernel mis-typed in three
  positions emits one TypeMismatch (for the first position), not
  three. Priority: KernelNotFound > MissingPub > ArityMismatch >
  TypeMismatch (params, then return). Rationale: blast-radius
  control — fixing the first error often makes the later ones
  evaporate.

- **Scalar-only type matching.** A declared 'kernel k : (f32) ->
  f32' matches 'pub fn k(a: f32) -> f32'. Anything richer
  (f32[H][W] vs Box<[[f32; W]; H]>) is reported as TypeMismatch
  with a clear 'aggregate type matching is not yet implemented'
  message. NOT silent acceptance. Filed as TASK-0100.

- **Terminal-segment match for Rust scalar types.** 'f32',
  'core::primitive::f32', and 'std::primitive::f32' all match a
  declared f32. The terminal segment is everything after the final
  '::'. Trades a tiny bit of precision (a user-defined 'mod foo {
  type f32 = ...; }' would also match) for not requiring callers to
  spell scalars in any specific way.

- **Per-invocation unique rmeta output path.** Tests run in
  parallel by default; multiple rustc calls writing to the same
  '/tmp/nucleus_contract_check.rmeta' would race. Use PID + nanos
  in the filename, and best-effort remove after.

## Honest limitations

1. **Aggregate / array type matching is a stub.** Declared
   f32[H][W] vs Rust Box<[[f32; W]; H]> currently fails the check
   with a clear message. Pending a stable codegen-side convention
   for array marshalling, hand-rolled matching would be wrong-by-
   construction. Filed as TASK-0100.

2. **Purity is documentation only.** PRD §6.2.2 acknowledges that
   'pure' vs 'effectful' is a contract the user upholds; rustc
   cannot prove a function is side-effect-free, panic-free, and
   reorder-safe. The 'where pure' annotation is preserved on the
   IR and downstream passes consult it, but the contract pass does
   not check kernel bodies for purity violations. Filed as
   TASK-0101 (likely v3 / won't-fix).

3. **rustc spawn cost.** ~50-100ms per check on a warm machine.
   For the small example matrix at M0 this is negligible, but at
   M3+ when we run the check per example × per schedule × per
   backend the cost compounds. Filed as TASK-0102 — defer
   optimisation until measurement shows it bites.

4. **Single kernels.rs only.** Module-nested 'pub fn's
   (e.g. 'mod inner { pub fn add(...) }') are invisible to the
   syn walk; we only look at top-level Item::Fn. v2 doesn't ship a
   module system, so this is consistent. If kernels.rs grows
   beyond a screen, the PRD says split-out belongs in tier 3
   shim crates, not Nuc-level imports.

5. **No spans on errors.** Same limitation as LowerError and
   LinkError: AST nodes don't carry positions yet (TASK-0086/0090).
   When they land, ContractError variants gain (line, column)
   fields without surface change.

6. **'rustc' must be on PATH.** The dev shell (flake.nix) ensures
   this. A user running without 'nix develop' might hit
   'failed to spawn rustc'. The error is surfaced cleanly via
   RustCheckFailed::stderr.

## AC verification

- AC #1 (nucleus invokes 'cargo check' / parses signature
  mismatch): MET, except we use 'rustc' instead of 'cargo check'
  for the reasons above (PRD-permissible: §6.2.2 says 'a cargo
  build step'; the strategy choice is documented). Structured
  errors via ContractError variants.
- AC #2 (missing function, arity mismatch, type mismatch, missing
  pub): MET — one ContractError variant per case plus
  RustCheckFailed and two file-level variants.
- AC #3 (purity not enforced; documented as v2 limitation):
  MET — see TASK-0101 and limitation #2 above.
- AC #4 (test: deliberately mismatched kernels.rs produces
  structured error): MET — one negative fixture per variant under
  tests/fixtures/contract/bad-*.
- AC #5 (notes record design questions): MET — see Design
  decisions section.
- AC #6 (notes record honest limitations): MET — see Honest
  limitations section.

## Verification

In 'nix develop':
- 'just check'  -> pass
- 'just clippy' -> pass (-D warnings)
- 'just test'   -> pass (88 / 88; was 80 before, added 8
  contract tests)
- 'just e2e'    -> pass (stub binary at M0, still empty)

## Commit

(filled in after commit)

## Follow-up tasks filed

- TASK-0100: Aggregate / array type matching.
- TASK-0101: Static purity check (probably won't-fix in v2).
- TASK-0102: Amortise rustc invocation cost.
<!-- SECTION:NOTES:END -->
