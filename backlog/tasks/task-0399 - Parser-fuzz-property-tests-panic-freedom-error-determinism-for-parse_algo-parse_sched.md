---
id: TASK-0399
title: >-
  Parser fuzz/property tests: panic-freedom + error-determinism for
  parse_algo/parse_sched
status: In Progress
assignee:
  - '@mark'
created_date: '2026-06-01 03:30'
updated_date: '2026-06-01 03:46'
labels:
  - tests
  - parser
  - proptest
  - fuzz
  - hardening
  - panic-not-diagnostic
  - determinism
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
ENDGAME HARDENING (phase3 backlog-maturity wave: fuzz the input/parsing boundary). The two .nuc front-end parsers parse_algo(src:&str)->Result<AlgoAst,ParseErrors> (nucleus/nucleus-compiler/src/algo/parser.rs:133) and parse_sched(src:&str)->Result<SchedAst,ParseErrors> (src/sched/parser.rs:78) have EXTENSIVE example-based tests (tests/algo_parser.rs 1251 LoC, tests/sched_parser.rs 1325 LoC) but ZERO property/fuzz coverage. Three durable project concerns are directly testable at this boundary and currently unpinned by any property test. (1) PANIC-NOT-DIAGNOSTIC (recurring defect class #3, CLAUDE.md): the parser is the untrusted-input boundary; it MUST return a typed Err on malformed input, never panic, never unwind, never hang. No property asserts this over arbitrary input. (2) CHUMSKY ERROR-DETERMINISM (memory project-chumsky-error-determinism; TASK-0080/0081): chumsky 0.9 Simple Display is HashSet-backed (non-deterministic); the fix routes all parser errors through a sorting error::chumsky_message path. No property pins that the SAME malformed input yields byte-identical error output across repeated parses, so a regression of the sort path is currently invisible. (3) ParseErrors INVARIANT (error.rs:53 docstring: a ParseErrors is only constructed non-empty; ParseErrors::first at error.rs:75 .expect()s it). No test asserts it as a property. Template: tests/proptest_petri.rs (proptest 1.9.0 already a dev-dep, nucleus-compiler/Cargo.toml:89). New file tests/proptest_parser.rs. SCOPE LIMIT: this is panic/determinism/invariant FUZZING, not a valid-source GENERATOR (the AST has no unparser, so parse->render->parse round-trip is out of scope; note that explicitly). If the panic-freedom property DISCOVERS a real panic on some input, that is a genuine defect (panic-not-diagnostic) -> file a precise child task with the minimized input and report; do NOT suppress or prop_assume it away.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 tests/proptest_parser.rs drives parse_algo AND parse_sched with arbitrary inputs (>=2 strategies: random UTF-8/ASCII strings, and a grammar-token-soup strategy assembling real keywords/idents/punct/numbers) and asserts neither parser panics/unwinds on any generated input (always returns a Result)
- [ ] #2 A property asserts the ParseErrors non-empty invariant: whenever parse_algo/parse_sched returns Err(e), e.0 is non-empty (error.rs:53 documented invariant; ParseErrors::first relies on it)
- [ ] #3 An error-determinism property asserts parsing the SAME malformed input twice yields byte-identical ParseErrors Display output (pins the chumsky HashSet-nondeterminism sort-path fix TASK-0080/0081 against regression)
- [ ] #4 Gate green inside nix develop: build && clippy (-D warnings, no doc_lazy_continuation) && test && test-release && e2e; e2e baseline 385/328/0/57/0 UNCHANGED (test-only, e2e-inert); new proptest cases pass. If panic-freedom finds a real panic, file a child task and report rather than mark Done
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED in-thread (orchestrator; per feedback-spawned-agents-refuse-code-edits + clean additive test-only change). New file nucleus/nucleus-compiler/tests/proptest_parser.rs: 8 proptest properties, ProptestConfig::with_cases(256), mirroring tests/proptest_petri.rs style. Two strategies: ARBITRARY_UTF8 ((?s).{0,200} regex, newlines included) + token_soup (0..=40 real grammar tokens harvested from just()/keyword() literals, concatenated). Properties: (AC#1+#2) algo+sched x {arbitrary, token-soup} never panic and any Err is non-empty (4 props); (AC#3) algo+sched x {arbitrary, token-soup} parse-twice determinism via prop_assert_eq! on the full Result (4 props; AlgoAst/SchedAst+ParseErrors all derive PartialEq). FINDINGS: all 8 PASS (parser already robust) -> this PINS existing robustness against regression, found NO live defect. Pre-verified the two highest-panic-risk paths are already defensive: int_lit (algo:474 try_map) + int_lit/size_lit (sched:235/302) map overflow to a chumsky Simple::custom error, not unwrap; the parse-entry out.expect (algo:142 / sched:87) is the chumsky empty-error-list invariant and did not fire on any fuzz input. HONEST LIMIT recorded in module docstring: panic/invariant/determinism fuzzing only, NOT a valid-source round-trip (AST has no unparser); input bounded (<=200 chars / <=40 tokens) so a stack-overflow-on-deep-nesting is unlikely and would NOT be proptest-catchable (no per-case timeout) -> distinct recursion-depth concern to file separately if ever seen. Determinism prop is a refactor-regression guard (like proptest_petri b.2/d.1), not independently proven-to-bite (would require injecting HashSet nondeterminism into the parser, declined). GATE (orchestrator-run): build OK; clippy -D warnings clean (no doc_lazy_continuation); test dev 1224/0 (was 1216, +8); test-release 1223/0 (-1 = known TASK-0291 debug_assert should_panic divergence); e2e 385/328/0/57/0 UNCHANGED (test-only, e2e-inert). Holding for parallel read-only review gate (qa-test-runner + mped-architect) before Done.
<!-- SECTION:NOTES:END -->
