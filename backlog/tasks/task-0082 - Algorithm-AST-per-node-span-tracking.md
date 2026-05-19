---
id: TASK-0082
title: 'Algorithm AST: per-node span tracking'
status: Done
assignee:
  - '@mped'
created_date: '2026-05-18 00:03'
updated_date: '2026-05-19 15:48'
labels:
  - compiler
  - language
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
AST nodes (Item, Stmt, Expr, ...) currently carry no source-span info. TASK-0009 (AlgoIR lowering) and TASK-0011 (link step) want spans to point at user source in diagnostics. Either (a) wrap each node in a Spanned<T> { node: T, span: Range<usize> }, or (b) attach a parallel side-table keyed by node id. Bias: (a), small wrapper, derive PartialEq via the inner node to keep tests cheap. Update parse_algo accordingly.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Algorithm AST nodes (Item, Stmt, Expr, and the declaration/identifier nodes diagnostics must point at) carry source spans via a Spanned<T> { node: T, span: Range<usize> } wrapper (bias option a); parse_algo (chumsky) populates correct byte ranges
- [x] #2 Spanned<T> PartialEq/Eq/Hash/Debug delegate to the inner node (span EXCLUDED) so existing AST-equality tests stay valid without span boilerplate
- [x] #3 A test asserts spans are populated and point at the correct source substring for a representative parse (validated via error::offset_to_line_col)
- [x] #4 Zero behaviour change for valid input: just test green, just e2e 30/26/0/4/0, just determinism-check byte-identical, clippy --all-targets clean, ci exit 0; lowering still ignores spans (LowerError wiring is TASK-0090 — scope not bled; TASK-0086 schedule AST out of scope)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add algo/span.rs: Spanned<T>{node,span:Range<usize>}; MANUAL impl PartialEq/Eq/Hash/Debug forwarding ONLY to .node (span EXCLUDED — no derive); Deref/DerefMut to T for low-churn field access; serde-transparent under feature; new() ctor + map helper. Module doc states granularity rationale.
2. Wrap AST (ast.rs) at chosen granularity: Item, Stmt (incl for-body Vec<Spanned<Stmt>>), Expr (all recursive positions: Unary/Binary/Call args/indices/dims/for bounds), and name-bearing Strings (ConstDecl.name, DataDecl.name, KernelDecl.name, IndexedLValue.name, Call.callee, Stmt::For.var) -> Spanned<String>. NOT wrapped: ScalarType/Purity/UnaryOp/BinOp (Copy leaves), Type/KernelSig containers (bounded blast radius; their diagnostic content reachable via inner Spanned<Expr>/name spans). AlgoAst.items: Vec<Spanned<Item>>.
3. parser.rs: populate every wrapped node span via chumsky 0.9 .map_with_span(|n,s| Spanned::new(n,s)). Byte ranges = node source start..end.
4. lower.rs: mechanical .node / Deref plumbing; lowering semantics UNCHANGED, spans IGNORED (LowerError wiring is TASK-0090, scope NOT bled). Only consumer of algo::ast in src.
5. Tests: algo_parser.rs needs .node plumbing for AST field access (it pattern-matches Item/Stmt/Kernel and reads .name/.purity/.sig). algo_lower.rs asserts only IR -> unchanged (proves AST internal). NEW span test (AC#3): representative parse, assert wrapped node spans point at correct source substring, validated via error::offset_to_line_col (evidence not assertion).
6. Update ast.rs/mod.rs/parser.rs module docs to truthfully describe spans-now-tracked (comment-honesty defect class). No new panic on user-reachable path (decision-0003) — Spanned is infallible, parser stays typed-Result.
7. Full gate inside nix develop before each commit: just test (existing equality/count tests UNCHANGED green + new span test 0 fail), e2e 30/26/0/4/0, determinism byte-identical x2, both negatives bite, clippy --all-targets, ci exit 0. Commit per logical unit (git only, no push, no AI credit).
8. backlog: append-notes (granularity+rationale, actual numbers, AC#3 evidence, regression result); check-ac only gate-verified; forward-carry substrate facts to TASK-0090/0086/0080/0096.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED (commits 436c12a, 3c158d3) — full gate green.

Chosen wrap-granularity (bounded, argued): Item, Stmt (incl. every nested for-body stmt), Expr (all recursive positions: unary/binary operands, call args, indices, shape dims, for bounds), and the identifier-bearing String fields diagnostics name -> Spanned<String> (ConstDecl/DataDecl/KernelDecl.name, IndexedLValue.name, Call.callee, Stmt::For.var). NOT wrapped: ScalarType/Purity/UnaryOp/BinOp (Copy leaves, never independently diagnosed) and Type/KernelSig structural containers (bounded blast radius — their diagnosable content is reachable via inner Spanned<Expr> dims / the owning decl-name span). Rationale: smallest set that lets a future error point precisely at an undeclared/duplicate identifier, a bad sub-expression, or a malformed declaration/statement; under-wrapping loses a diagnostic site, over-wrapping bloats every node+combinator.

Spanned<T> design: struct{node,span:Range<usize>} in new algo/span.rs. MANUAL impl PartialEq/Eq/Hash/Debug forwarding to self.node ONLY (span EXCLUDED) — NOT derived; this is the load-bearing AC#2 control so existing AST-equality/IR-equality tests stay valid. Deref/DerefMut to T to bound .node churn (does not mask intent: match still needs .node, .span still explicit). serde-transparent under the default-on feature (no AST type is serde-derived today; defensive+future-proof).

GATE (actual numbers, inside nix develop):
- just test: 400 passed / 0 failed / 2 ignored. algo_parser.rs 18/18 (17 original structural/count tests UNCHANGED in semantics + 1 new span test). algo_lower.rs 15/15 with ZERO file edits (asserts IR only — proves the AST wrapper is internal & PartialEq-ignores-span keeps IR-equality valid).
- just e2e: total 30 / pass 26 / fail 0 / skipped 4 / required-fail 0.
- just determinism-check: byte-identical, 30/26/0/4, RUN TWICE — load-bearing zero-behaviour-change proof (spans additive, lowering ignores them, codegen bit-identical).
- just determinism-check-negative: 26 cells perturbed, correctly bit (>=1).
- just xbackend-check-negative: 13 corrupted, 1 detected, correctly bit (>=1).
- just clippy (--workspace --all-targets -D warnings): clean, no warnings.
- just ci: exit 0.

AC#3 SPAN EVIDENCE (test reconstructs source text from span alone). Source:
  L1 const N : usize = 4;  L2 data x : f32[N];  L3 for i : 0 .. N {  L4 x[i] <-- inc(i);  L5 }
Verified &src[node.span] == expected, (line,col) via error::offset_to_line_col:
- item0 span -> "const N : usize = 4;" @ (1,1)  (TIGHT: excludes trailing newline)
- const name -> "N" @ (1,7);  const value -> "4" @ (1,19)
- data shape-dim expr -> "N" @ (2,14)
- for-loop item span -> "for i : 0 .. N {\n    x[i] <-- inc(i);\n}" @ (3,1)
- loop var -> "i" @ (3,5)
- body dataflow stmt -> "x[i] <-- inc(i);" @ (4,5)  (TIGHT: no trailing newline)
- lvalue base -> "x" @ (4,5);  rhs call expr -> "inc(i)";  callee -> "inc" @ (4,14);  arg -> "i"
Plus: Spanned PartialEq+Hash proven to IGNORE span (two idents, different offsets, compare/hash equal).

GOTCHAS / lessons (feed-forward — stateless subagents):
1. chumsky 0.9: span type for char input IS Range<usize> (its own docs show `struct Spanned<T>(T, Range<usize>)`). .map_with_span(Spanned::new) works directly.
2. SPAN TIGHTNESS: naive pad(p).map_with_span(..) swallows trailing whitespace into the span (the newline after `;`) — wrong for a diagnostic underline. Fix = padded_spanned(p): map_with_span on the BARE token then then_ignore(comment_or_ws()) OFF-span. Statements/decls end at the BARE terminator (; / }) not pad(just(..)), span fixed there, trailing layout consumed outside. ident_or_call composes name.start..close-delim-end (captured the )/] end offset). This is why the test asserts TIGHT spans (no trailing layout).
3. AST shape preserved EXACTLY: bare ident stays Expr::LValue{indices:[]} (index_tail .repeated() empty), NOT Expr::Ident — so lower.rs bare-ident arms unchanged. Expr::Ident still never constructed by the parser (was already dead). Determinism byte-identical independently proves zero shape change reaches codegen.
4. Consumers needing .node plumbing: ONLY lower.rs in src (link.rs/acfg.rs/sidecar.rs/event.rs read IR not algo AST; ir.rs only imports Copy ScalarType/Purity; sched/* is the sibling AST = TASK-0086, untouched). Map-key lookups need .node (Spanned has no Borrow<str>); .clone() for LowerError(String) needs .node.clone(); deref-coercion gives &SpIdent->&str for &str params.
5. Test surface: algo_lower.rs asserts IR only -> ZERO edits (key proof point). algo_parser.rs needed mechanical .node + 2 helpers; structural/count assertions unchanged in meaning.
6. decision-0003: no new panic/expect/unwrap on user-reachable path; Spanned::new infallible; parser stays typed Result. Deserialize defaults span 0..0 (no panic).
7. ENV LIMITATION (honest): no sub-agent dispatch tool present in this environment, so the CLAUDE.md qa-test-runner+mped-architect parallel pre-commit review could not be run as agents; performed equivalent inline self-review (panic-not-diagnostic + comment-honesty defect classes + correctness) — clean.

ORCHESTRATOR review-gate close (phase3-ralph): both reviewers GO, no blocking findings. CORRECTED COUNTS (reviewer-measured is the fact of record — implementer self-report was transposed): algo_lower.rs = 18 tests, UNEDITED by either commit (git show empty), all 18 pass unchanged = the load-bearing AC#2 proof (Spanned PartialEq ignores span -> IR-equality suite needs zero edits); algo_parser.rs = 15 tests (14 original unchanged-meaning + 1 new span test). Workspace 400/0/2. mped-architect independently verified: Spanned PartialEq/Eq/Hash/Debug hand-impl forward to .node only (no derived_hash_with_manual_eq hazard; Eq/Hash consistent by construction); NO HashMap/HashSet/BTreeMap anywhere is keyed on Spanned<T> (every map keyed on String via .node) so span-ignoring Hash has zero opportunity to silently merge distinct nodes; lower.rs is the SOLE algo::ast consumer and is pure .node projection (no span-dependent control flow); error.rs/ir.rs/link.rs/acfg.rs/sched untouched (TASK-0090/0086 not bled; span never reaches IR/codegen); AST shape preserved (bare ident still Expr::LValue empty-indices); tightness fix (padded_spanned) principled & uniformly applied; AC#3 test reconstructs &src[node.span] byte-exact across 11 node kinds + line/col via error::offset_to_line_col; no new panic/unwrap/expect on user paths (decision-0003 upheld); doc comments honest; forward-carry to TASK-0090/0086/0096/0080/0092 accurate incl. the correct TASK-0096 caveat (link consumes AlgoIR not algo AST). qa-test-runner: determinism byte-identical x2 + e2e EXACTLY 30/26/0/4/0 + both negatives bite + clippy --all-targets clean + ci exit 0 + serde --all-features build OK. Optional non-blocking finding (Expr::Ident parser-unreachable, PREDATES this task) filed as TASK-0193+1. TASK-0082 Done stands — the diagnostics-cluster span substrate is correct, safe, bounded, tested, honest.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added per-node source-span tracking to the algorithm AST — the keystone substrate for the diagnostics-quality cluster (TASK-0090 LowerError, TASK-0080/0092 multi-error, TASK-0096 fuzzy-match).

What changed:
- New compiler/src/algo/span.rs: Spanned<T>{node, span: Range<usize>}. PartialEq/Eq/Hash/Debug are MANUALLY implemented to forward to .node only — span EXCLUDED, NOT derived (AC#2, the load-bearing regression control). Deref/DerefMut to T bound consumer churn; serde-transparent under the default-on feature.
- algo/ast.rs wrapped at the diagnostics-relevant granularity: Item, Stmt (incl. nested for-body), Expr (all recursive positions), and the identifier-bearing String fields errors name -> Spanned<String>. Copy leaves (ScalarType/Purity/UnaryOp/BinOp) and the Type/KernelSig containers deliberately NOT wrapped (bounded blast radius; their diagnosable content reachable via inner spans).
- algo/parser.rs: parse_algo populates every wrapped span via chumsky 0.9 .map_with_span. Spans are TIGHT (exclude trailing layout) via a padded_spanned primitive + bare-terminator statement/decl spans — the exact byte range a diagnostic underlines.
- algo/lower.rs (sole src consumer of algo::ast): mechanical .node plumbing; lowering semantics UNCHANGED, spans IGNORED (TASK-0090 owns LowerError wiring — scope not bled; TASK-0086 schedule AST untouched).

User impact: none yet — additive metadata; lowering ignores spans. Proven zero-behaviour-change by determinism byte-identical (x2) + e2e 30/26/0/4/0.

Tests: just test 400/0/2 (existing algo_parser structural + algo_lower IR-equality tests pass; algo_lower.rs needed ZERO edits, proving the wrapper is internal & PartialEq-ignores-span holds). New AC#3 test reconstructs each node's source substring from its span alone and validates (line,col) via error::offset_to_line_col.

Gate: e2e 30/26/0/4/0; determinism byte-identical x2; both negative falsifiers still bite; clippy --all-targets clean; ci exit 0.

Risks/follow-ups: spans not yet consumed (by design — TASK-0090). Substrate facts forward-carried to TASK-0090/0096 (and TASK-0086 is the sibling sched-AST). No new panic on user-reachable paths (decision-0003). Honest limitation: CLAUDE.md pre-commit sub-agent review (qa-test-runner/mped-architect) could not be dispatched (no agent-spawn tool in this environment); equivalent inline self-review performed and clean.
<!-- SECTION:FINAL_SUMMARY:END -->
